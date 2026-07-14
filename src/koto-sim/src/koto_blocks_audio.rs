//! SIM bridge for built-in KotoAudio cues.
//!
//! This is the **primary** KotoOS audio path (KOTO-0162): the KotoAudio bounded
//! runtime driving host-owned fixed cues. Package KMML is handled by the owned
//! runtime-cue path in `SimAudio`, after reading the mounted SD payload.
//!
//! An *additive* integration of the `koto-audio` bounded audio runtime: its mixed
//! output is folded into `SimAudio`'s single stream. For the full picture —
//! asset-path routing table, BGM regeneration command, and known limitations — see
//! `docs/devlog/KOTO_KOTOBLOCKS_AUDIO_BRIDGE.md`.
//!
//! # Responsibilities
//!
//! This module is the **SIM-side bridge** between app audio events and the bounded
//! [`koto_audio::AudioService`]. It owns:
//!
//! * generic [`PolyphonicSequence`] BGM and [`Sequence`] SFX playback;
//! * the **SimAudio merge**: the service mixes fixed blocks into a shared queue via
//!   [`BlockSink`], and [`KotoBlocksAudio::mix_into`] folds that queue additively
//!   into `SimAudio`'s single output stream.
//!
//! # What this module is *not*
//!
//! * **Not** the package `.kmml` loader. `SimRuntimeHost` reads and compiles it.
//! * BGM/SFX use koto-audio's built-in voices and runtime-ready assets.
//! * **Not** the hostcall dispatcher.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};

use koto_audio::{
    AudioBackend, AudioCounterSnapshot, AudioLimits, AudioPolicy, BackendError, BackendReport,
    BackendResult, BackendState, CompactEvent, CompactInstrument, CompactSequence, CompactTempo,
    CompactTrack, DefaultAudioService, DropPolicy, MixerBlock, MixerVolume, PolyphonicSequence,
    PolyphonicSequenceVoice, Sequence, SequenceDrum, SequenceEvent, SequenceInstrument,
    SequenceWaveform, DEFAULT_MIXER_BLOCK_FRAMES, MAX_SEQUENCE_VOICES,
};

/// Fixed mixer block length shared by the service, backend, and this bridge.
const BLOCK_FRAMES: usize = DEFAULT_MIXER_BLOCK_FRAMES;

// --- Default bus volumes --------------------------------------------------
//
// Initial per-bus gains applied by [`KotoBlocksAudio::new`]. On koto-audio's
// `MixerVolume` scale 256 = unity; both sit below unity for mixing headroom and
// are deliberately unequal so the one-shot SFX bus cuts through the sustained
// BGM. The game side may rebalance either at runtime (see the setters below and
// `SimAudio::set_seq_volumes`).

/// Default BGM bus gain (150/256 ≈ 0.59). Sits well below unity so one-shot SFX
/// cut through the music rather than being buried. Overridable via
/// [`KotoBlocksAudio::set_bgm_volume`] / `SimAudio::set_seq_volumes`.
pub const DEFAULT_BGM_VOLUME: MixerVolume = MixerVolume::new(150);

/// Default SFX bus gain (200/256 ≈ 0.78). Sits well above the BGM bus so gameplay
/// cues cut through, but below full so a cue landing on a BGM peak leaves headroom
/// rather than clamping the summed mix to full scale (KOTO-0163: at 230 the
/// SFX-on-BGM sum reached 100% FS during play; 200 holds the peak near 91% with no
/// audible loss of presence). Overridable via [`KotoBlocksAudio::set_sfx_volume`].
pub const DEFAULT_SFX_VOLUME: MixerVolume = MixerVolume::new(200);

/// Upper bound on samples buffered in the [`BlockSink`] queue. The render path
/// drains every call, so this only guards against a stalled consumer; it never
/// clips normal playback.
const MAX_BUFFERED_SAMPLES: usize = BLOCK_FRAMES * 16;

/// The KotoBlocks game-event SFX cues, each backed by a small authored static
/// [`Sequence`]. `Lock`/`HardDrop` share the landing thud, and `Tetris` is the
/// longer four-line fanfare.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SeqSfx {
    /// Horizontal move: a short high blip.
    Move,
    /// Rotate: a quick two-note rise.
    Rotate,
    /// Hard drop / piece lock: a low landing thud.
    HardDrop,
    /// Single/double/triple line clear: an ascending arpeggio.
    LineClear,
    /// Four-line clear ("tetris"): a longer triumphant arpeggio.
    Tetris,
    /// Game over: a descending phrase (played at high priority like all SFX).
    GameOver,
}

impl SeqSfx {
    /// Returns the authored static sequence for this cue.
    fn sequence(self) -> Sequence<'static> {
        match self {
            SeqSfx::Move => MOVE_SEQ,
            SeqSfx::Rotate => ROTATE_SEQ,
            SeqSfx::HardDrop => LOCK_SEQ,
            SeqSfx::LineClear => CLEAR_SEQ,
            SeqSfx::Tetris => TETRIS_SEQ,
            SeqSfx::GameOver => OVER_SEQ,
        }
    }
}

/// A [`koto_audio::AudioBackend`] that copies each mixed block into a shared
/// mono sample queue, which the render path drains. No hardware output; the
/// real device output stays SimAudio's cpal stream.
#[derive(Clone, Debug)]
struct BlockSink {
    state: BackendState,
    queue: Arc<Mutex<VecDeque<i16>>>,
}

impl AudioBackend<BLOCK_FRAMES> for BlockSink {
    fn start(&mut self) -> BackendResult {
        self.state = BackendState::Running;
        Ok(BackendReport::backend_restart())
    }

    fn stop(&mut self) -> BackendResult {
        self.state = BackendState::Stopped;
        Ok(BackendReport::default())
    }

    fn submit_block(&mut self, block: &MixerBlock<BLOCK_FRAMES>) -> BackendResult {
        if self.state != BackendState::Running {
            return Err(BackendError::NotRunning);
        }
        if let Ok(mut queue) = self.queue.lock() {
            queue.extend(block.as_pcm16_mono().iter().copied());
            while queue.len() > MAX_BUFFERED_SAMPLES {
                queue.pop_front();
            }
        }
        Ok(BackendReport::submitted_block())
    }

    fn suspend(&mut self) -> BackendResult {
        self.state = BackendState::Suspended;
        Ok(BackendReport::default())
    }

    fn resume(&mut self) -> BackendResult {
        self.state = BackendState::Running;
        Ok(BackendReport::backend_restart())
    }

    fn query_state(&self) -> BackendState {
        self.state
    }
}

/// The KotoBlocks audio bridge: a bounded [`koto_audio::AudioService`] plus the
/// generated BGM sequence, driven by game events and drained into SimAudio.
#[derive(Debug)]
pub struct KotoBlocksAudio {
    service: DefaultAudioService<'static, BlockSink>,
    queue: Arc<Mutex<VecDeque<i16>>>,
    bgm: PolyphonicSequence<'static>,
    /// True once BGM is playing; gates against double-starting the loop.
    bgm_started: bool,
    sample_rate: u32,
}

impl KotoBlocksAudio {
    /// Creates a started bridge rendering at `sample_rate`. Returns `None` if the
    /// bounded runtime rejects the policy (e.g. a zero sample rate).
    pub fn new(sample_rate: u32) -> Option<Self> {
        let sample_rate = sample_rate.max(1);
        let queue = Arc::new(Mutex::new(VecDeque::new()));
        let backend = BlockSink {
            state: BackendState::Stopped,
            queue: Arc::clone(&queue),
        };
        let policy = AudioPolicy {
            limits: AudioLimits {
                sample_rate_hz: sample_rate,
                block_frames: BLOCK_FRAMES as u16,
                // BGM (1) + up to 3 SFX fits the default 4 active-source budget.
                max_sfx_sources: 3,
                source_queue_depth: 8,
                event_queue_depth: 16,
            },
            // Drop the *new* SFX when the queue is full so move/rotate spam never
            // panics and never steals the BGM slot.
            drop_policy: DropPolicy::DropNew,
            min_volume: 0,
            max_volume: 256,
            default_volume: 256,
        };
        let mut service = DefaultAudioService::new(policy, backend).ok()?;
        service.start().ok()?;

        let mut bridge = Self {
            service,
            queue,
            bgm: blocks_bgm_sequence(),
            bgm_started: false,
            sample_rate,
        };
        bridge.set_bgm_volume(DEFAULT_BGM_VOLUME);
        bridge.set_sfx_volume(DEFAULT_SFX_VOLUME);
        Some(bridge)
    }

    /// The sample rate this bridge renders at.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Starts the looping BGM sequence, unless it is already playing. Replaces
    /// any existing BGM (the service stops the old BGM bus first), so this can
    /// never stack a second copy of the loop.
    pub fn start_bgm(&mut self) {
        if self.bgm_started {
            return;
        }
        if self.service.play_bgm_sequence(self.bgm).is_ok() {
            self.bgm_started = true;
        }
    }

    /// Starts a fixed static BGM cue.
    pub fn start_bgm_cue(&mut self, sequence: &'static PolyphonicSequence<'static>) {
        if self.bgm_started {
            return;
        }
        if self.service.play_bgm_sequence(*sequence).is_ok() {
            self.bgm_started = true;
        }
    }

    /// Stops only the BGM bus. Active SFX sources keep playing.
    pub fn stop_bgm(&mut self) {
        let _ = self.service.stop_bgm();
        self.bgm_started = false;
    }

    /// Plays a one-shot SFX cue on the SFX bus. A full source queue silently
    /// drops the cue (never panics).
    pub fn sfx(&mut self, kind: SeqSfx) {
        let _ = self.service.play_sequence(kind.sequence());
    }

    /// Plays a fixed static SFX cue.
    pub fn sfx_cue(&mut self, sequence: &'static Sequence<'static>) {
        let _ = self.service.play_sequence(*sequence);
    }

    /// Sets the BGM bus gain (256 = unity).
    pub fn set_bgm_volume(&mut self, volume: MixerVolume) {
        let _ = self.service.set_bgm_volume(volume);
    }

    /// Sets the SFX bus gain (256 = unity).
    pub fn set_sfx_volume(&mut self, volume: MixerVolume) {
        let _ = self.service.set_sfx_volume(volume);
    }

    /// True while at least one BGM source is active.
    pub fn is_bgm_active(&self) -> bool {
        self.service.counter_snapshot().active_bgm_source_count > 0
    }

    /// The service counter snapshot (used by tests and diagnostics).
    pub fn counter_snapshot(&self) -> AudioCounterSnapshot {
        self.service.counter_snapshot()
    }

    /// Advances the service far enough to supply `out.len()` mono samples and
    /// mixes them additively into `out` (clamped to the i16 range). Intended to
    /// be called by `SimAudio::render` after its own mixing.
    pub fn mix_into(&mut self, out: &mut [i16]) {
        if out.is_empty() {
            return;
        }
        // Drain incrementally: pull any buffered samples, then tick one block and
        // pull again. Draining *before* each tick keeps the shared queue small so
        // the backend's overflow cap never discards a short cue's leading samples.
        // One tick emits BLOCK_FRAMES samples, so this bound cannot spin.
        let mut produced = 0;
        let max_ticks = out.len() / BLOCK_FRAMES + 4;
        let mut ticks = 0;
        while produced < out.len() {
            if let Ok(mut queue) = self.queue.lock() {
                while produced < out.len() {
                    let Some(sample) = queue.pop_front() else {
                        break;
                    };
                    out[produced] = clamp_i16(i32::from(out[produced]) + i32::from(sample));
                    produced += 1;
                }
            }
            if produced >= out.len() || ticks >= max_ticks {
                break;
            }
            if self.service.tick().is_err() {
                break;
            }
            ticks += 1;
        }
    }

    /// Test-only: surface the raw play result so tests can distinguish a rejected
    /// enqueue from a queued-but-not-yet-promoted source.
    #[cfg(test)]
    fn try_start_bgm(&mut self) -> koto_audio::AudioResult<koto_audio::SourceId> {
        self.service.play_bgm_sequence(self.bgm)
    }

    #[cfg(test)]
    fn try_sfx(&mut self, kind: SeqSfx) -> koto_audio::AudioResult<koto_audio::SourceId> {
        self.service.play_sequence(kind.sequence())
    }
}

fn clamp_i16(sample: i32) -> i16 {
    sample.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
}

/// Builds (once) the looping BGM [`PolyphonicSequence`] from the generated
/// compact table. Each compact track is adapted into a monophonic [`Sequence`]
/// whose event buffer is leaked to `'static` (a bounded, one-time allocation).
fn blocks_bgm_sequence() -> PolyphonicSequence<'static> {
    static VOICES: OnceLock<&'static [PolyphonicSequenceVoice<'static>]> = OnceLock::new();
    let voices = *VOICES.get_or_init(|| {
        let compact = BLOCKS_LIKE_BGM_COMPACT;
        let mut voices: Vec<PolyphonicSequenceVoice<'static>> = Vec::new();
        for track in compact.tracks.iter().take(MAX_SEQUENCE_VOICES) {
            let buffer = vec![SequenceEvent::End; track.events.len()];
            let leaked: &'static mut [SequenceEvent] = Box::leak(buffer.into_boxed_slice());
            if let Ok(sequence) = track.adapt_to_sequence(compact, leaked) {
                voices.push(PolyphonicSequenceVoice::unity(sequence));
            }
        }
        &*Box::leak(voices.into_boxed_slice())
    });
    PolyphonicSequence::new(voices)
}

// --- Authored static SFX sequences ----------------------------------------
//
// Small, hand-written cues (no runtime MML). Durations are in ticks at
// `SFX_TICK_RATE`, so real time is `ticks / SFX_TICK_RATE` seconds regardless
// of the output sample rate. Pitches are in hertz.

/// SFX tick rate: 64 ticks/second, so one tick ≈ 15.6 ms.
const SFX_TICK_RATE: u16 = 64;

/// Bright square lead for melodic cues.
static SFX_SQUARE: [SequenceInstrument; 1] = [SequenceInstrument::with_envelope(
    SequenceWaveform::Square,
    200,
    1,
    2,
)];
/// Soft triangle for the game-over phrase.
static SFX_TRIANGLE: [SequenceInstrument; 1] = [SequenceInstrument::with_envelope(
    SequenceWaveform::Triangle,
    200,
    1,
    3,
)];
/// Percussive bass-drum thud for the piece lock / hard drop.
static SFX_THUD: [SequenceInstrument; 1] = [SequenceInstrument::drum(SequenceDrum::BassDrum, 255)];

static MOVE_EVENTS: [SequenceEvent; 2] = [
    SequenceEvent::Note {
        pitch: 988,
        duration_ticks: 2,
        volume: 180,
        instrument_id: 0,
    },
    SequenceEvent::End,
];
static MOVE_SEQ: Sequence<'static> = Sequence::new(&MOVE_EVENTS, &SFX_SQUARE, SFX_TICK_RATE);

static ROTATE_EVENTS: [SequenceEvent; 3] = [
    SequenceEvent::Note {
        pitch: 784,
        duration_ticks: 2,
        volume: 200,
        instrument_id: 0,
    },
    SequenceEvent::Note {
        pitch: 1047,
        duration_ticks: 2,
        volume: 200,
        instrument_id: 0,
    },
    SequenceEvent::End,
];
static ROTATE_SEQ: Sequence<'static> = Sequence::new(&ROTATE_EVENTS, &SFX_SQUARE, SFX_TICK_RATE);

static LOCK_EVENTS: [SequenceEvent; 2] = [
    SequenceEvent::Note {
        pitch: 110,
        duration_ticks: 4,
        volume: 255,
        instrument_id: 0,
    },
    SequenceEvent::End,
];
static LOCK_SEQ: Sequence<'static> = Sequence::new(&LOCK_EVENTS, &SFX_THUD, SFX_TICK_RATE);

static CLEAR_EVENTS: [SequenceEvent; 5] = [
    SequenceEvent::Note {
        pitch: 523,
        duration_ticks: 2,
        volume: 210,
        instrument_id: 0,
    },
    SequenceEvent::Note {
        pitch: 659,
        duration_ticks: 2,
        volume: 210,
        instrument_id: 0,
    },
    SequenceEvent::Note {
        pitch: 784,
        duration_ticks: 2,
        volume: 210,
        instrument_id: 0,
    },
    SequenceEvent::Note {
        pitch: 1047,
        duration_ticks: 3,
        volume: 210,
        instrument_id: 0,
    },
    SequenceEvent::End,
];
static CLEAR_SEQ: Sequence<'static> = Sequence::new(&CLEAR_EVENTS, &SFX_SQUARE, SFX_TICK_RATE);

static TETRIS_EVENTS: [SequenceEvent; 7] = [
    SequenceEvent::Note {
        pitch: 659,
        duration_ticks: 2,
        volume: 220,
        instrument_id: 0,
    },
    SequenceEvent::Note {
        pitch: 784,
        duration_ticks: 2,
        volume: 220,
        instrument_id: 0,
    },
    SequenceEvent::Note {
        pitch: 1047,
        duration_ticks: 2,
        volume: 220,
        instrument_id: 0,
    },
    SequenceEvent::Note {
        pitch: 1319,
        duration_ticks: 2,
        volume: 220,
        instrument_id: 0,
    },
    SequenceEvent::Note {
        pitch: 1568,
        duration_ticks: 4,
        volume: 220,
        instrument_id: 0,
    },
    SequenceEvent::End,
    SequenceEvent::End,
];
static TETRIS_SEQ: Sequence<'static> = Sequence::new(&TETRIS_EVENTS, &SFX_SQUARE, SFX_TICK_RATE);

static OVER_EVENTS: [SequenceEvent; 5] = [
    SequenceEvent::Note {
        pitch: 523,
        duration_ticks: 4,
        volume: 210,
        instrument_id: 0,
    },
    SequenceEvent::Note {
        pitch: 440,
        duration_ticks: 4,
        volume: 210,
        instrument_id: 0,
    },
    SequenceEvent::Note {
        pitch: 349,
        duration_ticks: 4,
        volume: 210,
        instrument_id: 0,
    },
    SequenceEvent::Note {
        pitch: 262,
        duration_ticks: 8,
        volume: 210,
        instrument_id: 0,
    },
    SequenceEvent::End,
];
static OVER_SEQ: Sequence<'static> = Sequence::new(&OVER_EVENTS, &SFX_TRIANGLE, SFX_TICK_RATE);

// --- Generated BGM compact table ------------------------------------------
//
// Vendored verbatim from koto-audio `examples/generated/blocks_like_bgm.rs`
// (generated by koto-audio-tools from a validated compact sequence table).
// Runtime representation: borrowed CompactSequence static tables. The crate is
// unchanged; only the imports at the top of this module differ from the example.
static BLOCKS_LIKE_BGM_COMPACT_INSTRUMENTS: [CompactInstrument; 9] = [
    CompactInstrument {
        builtin_id: 0,
        volume: 255,
        attack_ticks: 0,
        release_ticks: 0,
        decay_ticks: 0,
    },
    CompactInstrument {
        builtin_id: 5,
        volume: 255,
        attack_ticks: 0,
        release_ticks: 0,
        decay_ticks: 0,
    },
    CompactInstrument {
        builtin_id: 6,
        volume: 255,
        attack_ticks: 0,
        release_ticks: 0,
        decay_ticks: 0,
    },
    CompactInstrument {
        builtin_id: 10,
        volume: 255,
        attack_ticks: 0,
        release_ticks: 0,
        decay_ticks: 0,
    },
    CompactInstrument {
        builtin_id: 8,
        volume: 255,
        attack_ticks: 0,
        release_ticks: 0,
        decay_ticks: 0,
    },
    CompactInstrument {
        builtin_id: 7,
        volume: 255,
        attack_ticks: 0,
        release_ticks: 0,
        decay_ticks: 0,
    },
    CompactInstrument {
        builtin_id: 9,
        volume: 255,
        attack_ticks: 0,
        release_ticks: 0,
        decay_ticks: 0,
    },
    CompactInstrument {
        builtin_id: 16,
        volume: 255,
        attack_ticks: 0,
        release_ticks: 0,
        decay_ticks: 0,
    },
    CompactInstrument {
        builtin_id: 11,
        volume: 255,
        attack_ticks: 0,
        release_ticks: 0,
        decay_ticks: 0,
    },
];

static BLOCKS_LIKE_BGM_COMPACT_TRACK_0_EVENTS: [CompactEvent; 33] = [
    CompactEvent::LoopStart,
    CompactEvent::Note {
        pitch: 523,
        duration_ticks: 2,
        volume: 104,
        instrument_id: 0,
    },
    CompactEvent::Note {
        pitch: 659,
        duration_ticks: 2,
        volume: 104,
        instrument_id: 0,
    },
    CompactEvent::Note {
        pitch: 784,
        duration_ticks: 2,
        volume: 104,
        instrument_id: 0,
    },
    CompactEvent::Note {
        pitch: 1047,
        duration_ticks: 2,
        volume: 104,
        instrument_id: 0,
    },
    CompactEvent::Note {
        pitch: 933,
        duration_ticks: 2,
        volume: 104,
        instrument_id: 0,
    },
    CompactEvent::Note {
        pitch: 784,
        duration_ticks: 2,
        volume: 104,
        instrument_id: 0,
    },
    CompactEvent::Note {
        pitch: 659,
        duration_ticks: 2,
        volume: 104,
        instrument_id: 0,
    },
    CompactEvent::Note {
        pitch: 523,
        duration_ticks: 2,
        volume: 104,
        instrument_id: 0,
    },
    CompactEvent::Note {
        pitch: 587,
        duration_ticks: 2,
        volume: 104,
        instrument_id: 0,
    },
    CompactEvent::Note {
        pitch: 699,
        duration_ticks: 2,
        volume: 104,
        instrument_id: 0,
    },
    CompactEvent::Note {
        pitch: 880,
        duration_ticks: 2,
        volume: 104,
        instrument_id: 0,
    },
    CompactEvent::Note {
        pitch: 1175,
        duration_ticks: 2,
        volume: 104,
        instrument_id: 0,
    },
    CompactEvent::Note {
        pitch: 554,
        duration_ticks: 2,
        volume: 104,
        instrument_id: 0,
    },
    CompactEvent::Note {
        pitch: 880,
        duration_ticks: 2,
        volume: 104,
        instrument_id: 0,
    },
    CompactEvent::Note {
        pitch: 699,
        duration_ticks: 2,
        volume: 104,
        instrument_id: 0,
    },
    CompactEvent::Note {
        pitch: 587,
        duration_ticks: 2,
        volume: 104,
        instrument_id: 0,
    },
    CompactEvent::Note {
        pitch: 659,
        duration_ticks: 2,
        volume: 104,
        instrument_id: 0,
    },
    CompactEvent::Note {
        pitch: 784,
        duration_ticks: 2,
        volume: 104,
        instrument_id: 0,
    },
    CompactEvent::Note {
        pitch: 988,
        duration_ticks: 2,
        volume: 104,
        instrument_id: 0,
    },
    CompactEvent::Note {
        pitch: 1319,
        duration_ticks: 2,
        volume: 104,
        instrument_id: 0,
    },
    CompactEvent::Note {
        pitch: 587,
        duration_ticks: 2,
        volume: 104,
        instrument_id: 0,
    },
    CompactEvent::Note {
        pitch: 784,
        duration_ticks: 2,
        volume: 104,
        instrument_id: 0,
    },
    CompactEvent::Note {
        pitch: 988,
        duration_ticks: 2,
        volume: 104,
        instrument_id: 0,
    },
    CompactEvent::Note {
        pitch: 784,
        duration_ticks: 2,
        volume: 104,
        instrument_id: 0,
    },
    CompactEvent::Note {
        pitch: 699,
        duration_ticks: 2,
        volume: 104,
        instrument_id: 0,
    },
    CompactEvent::Note {
        pitch: 880,
        duration_ticks: 2,
        volume: 104,
        instrument_id: 0,
    },
    CompactEvent::Note {
        pitch: 1047,
        duration_ticks: 2,
        volume: 104,
        instrument_id: 0,
    },
    CompactEvent::Note {
        pitch: 880,
        duration_ticks: 2,
        volume: 104,
        instrument_id: 0,
    },
    CompactEvent::Note {
        pitch: 784,
        duration_ticks: 6,
        volume: 104,
        instrument_id: 0,
    },
    CompactEvent::Rest { duration_ticks: 2 },
    CompactEvent::LoopEnd { repeat_count: 0 },
    CompactEvent::End,
];

static BLOCKS_LIKE_BGM_COMPACT_TRACK_1_EVENTS: [CompactEvent; 33] = [
    CompactEvent::LoopStart,
    CompactEvent::Note {
        pitch: 65,
        duration_ticks: 2,
        volume: 96,
        instrument_id: 1,
    },
    CompactEvent::Rest { duration_ticks: 2 },
    CompactEvent::Note {
        pitch: 65,
        duration_ticks: 2,
        volume: 96,
        instrument_id: 1,
    },
    CompactEvent::Note {
        pitch: 196,
        duration_ticks: 2,
        volume: 96,
        instrument_id: 1,
    },
    CompactEvent::Note {
        pitch: 65,
        duration_ticks: 2,
        volume: 96,
        instrument_id: 1,
    },
    CompactEvent::Rest { duration_ticks: 2 },
    CompactEvent::Note {
        pitch: 65,
        duration_ticks: 2,
        volume: 96,
        instrument_id: 1,
    },
    CompactEvent::Note {
        pitch: 196,
        duration_ticks: 2,
        volume: 96,
        instrument_id: 1,
    },
    CompactEvent::Note {
        pitch: 147,
        duration_ticks: 2,
        volume: 96,
        instrument_id: 1,
    },
    CompactEvent::Rest { duration_ticks: 2 },
    CompactEvent::Note {
        pitch: 147,
        duration_ticks: 2,
        volume: 96,
        instrument_id: 1,
    },
    CompactEvent::Note {
        pitch: 440,
        duration_ticks: 2,
        volume: 96,
        instrument_id: 1,
    },
    CompactEvent::Note {
        pitch: 147,
        duration_ticks: 2,
        volume: 96,
        instrument_id: 1,
    },
    CompactEvent::Rest { duration_ticks: 2 },
    CompactEvent::Note {
        pitch: 147,
        duration_ticks: 2,
        volume: 96,
        instrument_id: 1,
    },
    CompactEvent::Note {
        pitch: 440,
        duration_ticks: 2,
        volume: 96,
        instrument_id: 1,
    },
    CompactEvent::Note {
        pitch: 330,
        duration_ticks: 2,
        volume: 96,
        instrument_id: 1,
    },
    CompactEvent::Rest { duration_ticks: 2 },
    CompactEvent::Note {
        pitch: 330,
        duration_ticks: 2,
        volume: 96,
        instrument_id: 1,
    },
    CompactEvent::Note {
        pitch: 988,
        duration_ticks: 2,
        volume: 96,
        instrument_id: 1,
    },
    CompactEvent::Note {
        pitch: 330,
        duration_ticks: 2,
        volume: 96,
        instrument_id: 1,
    },
    CompactEvent::Rest { duration_ticks: 2 },
    CompactEvent::Note {
        pitch: 330,
        duration_ticks: 2,
        volume: 96,
        instrument_id: 1,
    },
    CompactEvent::Note {
        pitch: 988,
        duration_ticks: 2,
        volume: 96,
        instrument_id: 1,
    },
    CompactEvent::Note {
        pitch: 699,
        duration_ticks: 2,
        volume: 96,
        instrument_id: 1,
    },
    CompactEvent::Rest { duration_ticks: 2 },
    CompactEvent::Note {
        pitch: 699,
        duration_ticks: 2,
        volume: 96,
        instrument_id: 1,
    },
    CompactEvent::Note {
        pitch: 1047,
        duration_ticks: 2,
        volume: 96,
        instrument_id: 1,
    },
    CompactEvent::Note {
        pitch: 699,
        duration_ticks: 2,
        volume: 96,
        instrument_id: 1,
    },
    CompactEvent::Rest { duration_ticks: 6 },
    CompactEvent::LoopEnd { repeat_count: 0 },
    CompactEvent::End,
];

static BLOCKS_LIKE_BGM_COMPACT_TRACK_2_EVENTS: [CompactEvent; 35] = [
    CompactEvent::LoopStart,
    CompactEvent::Note {
        pitch: 262,
        duration_ticks: 1,
        volume: 118,
        instrument_id: 2,
    },
    CompactEvent::Note {
        pitch: 262,
        duration_ticks: 1,
        volume: 118,
        instrument_id: 3,
    },
    CompactEvent::Note {
        pitch: 262,
        duration_ticks: 1,
        volume: 118,
        instrument_id: 4,
    },
    CompactEvent::Note {
        pitch: 262,
        duration_ticks: 1,
        volume: 118,
        instrument_id: 3,
    },
    CompactEvent::Note {
        pitch: 262,
        duration_ticks: 1,
        volume: 118,
        instrument_id: 2,
    },
    CompactEvent::Note {
        pitch: 262,
        duration_ticks: 1,
        volume: 118,
        instrument_id: 3,
    },
    CompactEvent::Note {
        pitch: 262,
        duration_ticks: 1,
        volume: 118,
        instrument_id: 5,
    },
    CompactEvent::Note {
        pitch: 262,
        duration_ticks: 1,
        volume: 118,
        instrument_id: 6,
    },
    CompactEvent::Note {
        pitch: 262,
        duration_ticks: 1,
        volume: 118,
        instrument_id: 2,
    },
    CompactEvent::Note {
        pitch: 262,
        duration_ticks: 1,
        volume: 118,
        instrument_id: 3,
    },
    CompactEvent::Note {
        pitch: 262,
        duration_ticks: 1,
        volume: 118,
        instrument_id: 4,
    },
    CompactEvent::Note {
        pitch: 262,
        duration_ticks: 1,
        volume: 118,
        instrument_id: 3,
    },
    CompactEvent::Note {
        pitch: 262,
        duration_ticks: 1,
        volume: 118,
        instrument_id: 2,
    },
    CompactEvent::Note {
        pitch: 262,
        duration_ticks: 1,
        volume: 118,
        instrument_id: 3,
    },
    CompactEvent::Note {
        pitch: 262,
        duration_ticks: 1,
        volume: 118,
        instrument_id: 7,
    },
    CompactEvent::Note {
        pitch: 262,
        duration_ticks: 1,
        volume: 118,
        instrument_id: 3,
    },
    CompactEvent::Note {
        pitch: 262,
        duration_ticks: 1,
        volume: 118,
        instrument_id: 2,
    },
    CompactEvent::Note {
        pitch: 262,
        duration_ticks: 1,
        volume: 118,
        instrument_id: 3,
    },
    CompactEvent::Note {
        pitch: 262,
        duration_ticks: 1,
        volume: 118,
        instrument_id: 4,
    },
    CompactEvent::Note {
        pitch: 262,
        duration_ticks: 1,
        volume: 118,
        instrument_id: 3,
    },
    CompactEvent::Note {
        pitch: 262,
        duration_ticks: 1,
        volume: 118,
        instrument_id: 2,
    },
    CompactEvent::Note {
        pitch: 262,
        duration_ticks: 1,
        volume: 118,
        instrument_id: 3,
    },
    CompactEvent::Note {
        pitch: 262,
        duration_ticks: 1,
        volume: 118,
        instrument_id: 5,
    },
    CompactEvent::Note {
        pitch: 262,
        duration_ticks: 1,
        volume: 118,
        instrument_id: 6,
    },
    CompactEvent::Note {
        pitch: 262,
        duration_ticks: 1,
        volume: 118,
        instrument_id: 2,
    },
    CompactEvent::Note {
        pitch: 262,
        duration_ticks: 1,
        volume: 118,
        instrument_id: 3,
    },
    CompactEvent::Note {
        pitch: 262,
        duration_ticks: 1,
        volume: 118,
        instrument_id: 4,
    },
    CompactEvent::Note {
        pitch: 262,
        duration_ticks: 1,
        volume: 118,
        instrument_id: 3,
    },
    CompactEvent::Note {
        pitch: 262,
        duration_ticks: 1,
        volume: 118,
        instrument_id: 2,
    },
    CompactEvent::Note {
        pitch: 262,
        duration_ticks: 1,
        volume: 118,
        instrument_id: 3,
    },
    CompactEvent::Note {
        pitch: 262,
        duration_ticks: 1,
        volume: 118,
        instrument_id: 8,
    },
    CompactEvent::Note {
        pitch: 262,
        duration_ticks: 1,
        volume: 118,
        instrument_id: 3,
    },
    CompactEvent::LoopEnd { repeat_count: 0 },
    CompactEvent::End,
];

static BLOCKS_LIKE_BGM_COMPACT_TRACKS: [CompactTrack<'static>; 3] = [
    CompactTrack::new(
        &BLOCKS_LIKE_BGM_COMPACT_TRACK_0_EVENTS,
        MixerVolume::new(256),
        0,
    ),
    CompactTrack::new(
        &BLOCKS_LIKE_BGM_COMPACT_TRACK_1_EVENTS,
        MixerVolume::new(256),
        0,
    ),
    CompactTrack::new(
        &BLOCKS_LIKE_BGM_COMPACT_TRACK_2_EVENTS,
        MixerVolume::new(256),
        0,
    ),
];

static BLOCKS_LIKE_BGM_COMPACT: CompactSequence<'static> = CompactSequence::new(
    &BLOCKS_LIKE_BGM_COMPACT_INSTRUMENTS,
    &BLOCKS_LIKE_BGM_COMPACT_TRACKS,
    CompactTempo {
        tick_rate_hz: 9,
        bpm: 132,
        ticks_per_beat: 4,
    },
);

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 22_050;

    /// Renders `frames` mono samples from a fresh, silent buffer.
    fn render(bridge: &mut KotoBlocksAudio, frames: usize) -> Vec<i16> {
        let mut out = vec![0i16; frames];
        bridge.mix_into(&mut out);
        out
    }

    #[test]
    fn generated_bgm_table_validates_and_adapts_to_three_voices() {
        assert!(BLOCKS_LIKE_BGM_COMPACT.validate().is_ok());
        let bgm = blocks_bgm_sequence();
        assert_eq!(bgm.voices.len(), 3);
    }

    #[test]
    fn bgm_start_produces_non_silent_output() {
        let mut bridge = KotoBlocksAudio::new(RATE).expect("bridge builds");
        bridge.try_start_bgm().expect("BGM enqueues");
        // A source is queued until a tick promotes it; render one block first.
        let _ = render(&mut bridge, BLOCK_FRAMES);
        assert!(bridge.is_bgm_active());
        let out = render(&mut bridge, RATE as usize / 4);
        assert!(out.iter().any(|&s| s != 0), "BGM rendered only silence");
    }

    #[test]
    fn bgm_does_not_double_start() {
        let mut bridge = KotoBlocksAudio::new(RATE).expect("bridge builds");
        bridge.start_bgm();
        bridge.start_bgm();
        bridge.start_bgm();
        // Exactly one admitted BGM start, no stacking or replacement.
        let snapshot = bridge.counter_snapshot();
        assert_eq!(snapshot.bgm_start_count, 1);
        assert_eq!(snapshot.bgm_replaced_count, 0);
        // Promote and confirm a single active BGM source.
        let _ = render(&mut bridge, BLOCK_FRAMES);
        assert_eq!(bridge.counter_snapshot().active_bgm_source_count, 1);
    }

    #[test]
    fn each_sfx_cue_renders_non_silent() {
        for kind in [
            SeqSfx::Move,
            SeqSfx::Rotate,
            SeqSfx::HardDrop,
            SeqSfx::LineClear,
            SeqSfx::Tetris,
            SeqSfx::GameOver,
        ] {
            let mut bridge = KotoBlocksAudio::new(RATE).expect("bridge builds");
            bridge.try_sfx(kind).expect("SFX enqueues");
            let out = render(&mut bridge, RATE as usize / 2);
            assert!(
                out.iter().any(|&s| s != 0),
                "{kind:?} rendered only silence"
            );
        }
    }

    #[test]
    fn stop_bgm_stops_only_bgm_and_keeps_sfx() {
        let mut bridge = KotoBlocksAudio::new(RATE).expect("bridge builds");
        bridge.start_bgm();
        bridge.sfx(SeqSfx::GameOver);
        // Promote queued sources to active.
        let _ = render(&mut bridge, BLOCK_FRAMES);
        assert!(bridge.is_bgm_active());
        let sfx_active_before = bridge.counter_snapshot().active_sfx_source_count;
        assert!(sfx_active_before >= 1);

        bridge.stop_bgm();

        assert!(!bridge.is_bgm_active());
        assert_eq!(
            bridge.counter_snapshot().active_sfx_source_count,
            sfx_active_before,
            "stop_bgm must not stop SFX"
        );
    }

    #[test]
    fn rapid_sfx_spam_never_panics() {
        let mut bridge = KotoBlocksAudio::new(RATE).expect("bridge builds");
        bridge.start_bgm();
        for _ in 0..500 {
            bridge.sfx(SeqSfx::Move);
            bridge.sfx(SeqSfx::Rotate);
            // Occasionally drain a block so some sources complete and free slots.
            let _ = render(&mut bridge, BLOCK_FRAMES / 2);
        }
        // Full queues drop new SFX rather than panicking.
        assert!(bridge.counter_snapshot().dropped_source_count > 0);
    }

    #[test]
    fn silenced_buses_produce_silence() {
        let mut bridge = KotoBlocksAudio::new(RATE).expect("bridge builds");
        bridge.set_bgm_volume(MixerVolume::SILENCE);
        bridge.set_sfx_volume(MixerVolume::SILENCE);
        bridge.start_bgm();
        bridge.sfx(SeqSfx::LineClear);
        let out = render(&mut bridge, RATE as usize / 4);
        assert!(
            out.iter().all(|&s| s == 0),
            "silenced buses should be silent"
        );
    }

    #[test]
    fn mix_into_is_additive_over_existing_audio() {
        let mut bridge = KotoBlocksAudio::new(RATE).expect("bridge builds");
        bridge.start_bgm();
        let mut out = vec![1000i16; BLOCK_FRAMES];
        bridge.mix_into(&mut out);
        // The pre-existing 1000 baseline is preserved plus the BGM contribution;
        // at least some samples differ from a plain 1000 fill.
        assert!(out.iter().any(|&s| s != 1000));
    }
}
