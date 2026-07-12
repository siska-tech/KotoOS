//! Primary-audio cue tables and route lookup for the Pico firmware (KOTO-0165).
//!
//! The device side of the primary audio model (`docs/architecture/PRIMARY_AUDIO_CUE_MODEL.md`):
//! an app's audio asset path is a *routing key*, not a parseable format.
//! `app_host` resolves a `play_bgm_asset` / `play_sfx_asset` path here and hands
//! the resolved KotoAudio sequence to the CPU1 worker
//! ([`crate::firmware::audio`]); nothing is read from SD and nothing is parsed
//! at runtime. An unrouted path plays nothing (safe drop) — the legacy runtime
//! MML / `.kwt` / tone chain was removed with KOTO-0165.
//!
//! Cue sources:
//!
//! * **Generated tables** ([`super::audio_cues_generated`]): every shipped app's
//!   BGM and SFX, converted offline by `tools/koto-audio-gen` from the
//!   `apps/*/audio/*.kmml` sources.
//! * **Authored KotoBlocks SFX** (this module): verbatim ports of the SIM bridge
//!   sequences (`koto_sim::koto_blocks_audio`), which use envelope/drum voices
//!   the generated compact path cannot express. Keep the two in sync.
//! * **Hostcall blips** (this module): the small fixed cues behind the legacy
//!   `play_sfx(id)` hostcall, replacing the removed tone path.

use koto_audio::{
    MixerVolume, PolyphonicSequence, PolyphonicSequenceVoice, Sequence, SequenceDrum,
    SequenceEvent, SequenceInstrument, SequenceWaveform, BUILTIN_INSTRUMENT_SQUARE,
    BUILTIN_INSTRUMENT_TRIANGLE, BUILTIN_SEQUENCE_INSTRUMENTS, SEQUENCE_REPEAT_INFINITE,
};

use super::audio_cues_generated::{GENERATED_BGM_ROUTES, GENERATED_SFX_ROUTES};

/// What a routed asset path plays on the primary KotoAudio path. Returned by
/// [`primary_audio_route`] so `app_host` dispatches without `.kmml` magic
/// strings of its own. The device mirror of SIM `PrimaryCue`, now carrying the
/// sequence itself instead of an app-specific cue id.
#[derive(Clone, Copy, Debug)]
pub enum PicoPrimaryCue {
    /// Start a looping generated BGM sequence (`seq-bgm` in diagnostics).
    Bgm(&'static PolyphonicSequence<'static>),
    /// Play a one-shot SFX sequence (`seq-sfx` in diagnostics).
    Sfx(&'static Sequence<'static>),
}

/// Resolves an audio asset `path` to its primary-audio cue, or `None` if no
/// route claims it (`app_host` then drops the call safely). The single place
/// the firmware matches primary-audio asset paths.
pub fn primary_audio_route(path: &str) -> Option<PicoPrimaryCue> {
    if let Some((_, sequence)) = KOTO_BLOCKS_SFX_ROUTES.iter().find(|(key, _)| *key == path) {
        return Some(PicoPrimaryCue::Sfx(sequence));
    }
    if let Some((_, sequence)) = GENERATED_BGM_ROUTES.iter().find(|(key, _)| *key == path) {
        return Some(PicoPrimaryCue::Bgm(sequence));
    }
    if let Some((_, sequence)) = GENERATED_SFX_ROUTES.iter().find(|(key, _)| *key == path) {
        return Some(PicoPrimaryCue::Sfx(sequence));
    }
    None
}

/// Returns the fixed blip cue behind the `play_sfx(id)` hostcall. The id set
/// mirrors the removed `tone_for_sfx_id` tone table (6/7/8 plus a default).
pub fn sfx_id_cue(id: i32) -> &'static Sequence<'static> {
    match id {
        6 => &BLIP_HIGH_SEQ,
        7 => &BLIP_TOP_SEQ,
        8 => &BLIP_LOW_SEQ,
        _ => &BLIP_MID_SEQ,
    }
}

/// Returns the built-in loop behind the `play_bgm(id)` hostcall, replacing the
/// removed built-in MML strings with equivalent authored sequences (id 0 plus a
/// default, as before).
pub fn builtin_bgm_cue(id: i32) -> &'static PolyphonicSequence<'static> {
    match id {
        0 => &BUILTIN_BGM_0,
        _ => &BUILTIN_BGM_1,
    }
}

// --- Authored KotoBlocks SFX (SIM parity) ----------------------------------
//
// Verbatim ports of the SIM bridge statics in
// `src/koto-sim/src/koto_blocks_audio.rs` ("Authored static SFX sequences").
// Durations are in ticks at `SFX_TICK_RATE`; pitches are in hertz. Edit the SIM
// module first, then mirror here.

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

static TETRIS_EVENTS: [SequenceEvent; 6] = [
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

/// KotoBlocks SFX routes: asset routing key -> authored sequence. The keys
/// mirror the SIM `KOTO_BLOCKS_*_ASSET` constants and the routing table in
/// `docs/architecture/PRIMARY_AUDIO_CUE_MODEL.md`; the KotoBlocks BGM key routes through the
/// generated tables instead.
static KOTO_BLOCKS_SFX_ROUTES: [(&str, &Sequence<'static>); 6] = [
    ("audio/koto_blocks_move.kmml", &MOVE_SEQ),
    ("audio/koto_blocks_rotate.kmml", &ROTATE_SEQ),
    ("audio/koto_blocks_lock.kmml", &LOCK_SEQ),
    ("audio/koto_blocks_clear.kmml", &CLEAR_SEQ),
    ("audio/koto_blocks_tetris.kmml", &TETRIS_SEQ),
    ("audio/koto_blocks_over.kmml", &OVER_SEQ),
];

// --- Hostcall blip cues -----------------------------------------------------
//
// Fixed one-shot square blips behind `play_sfx(id)`, replacing the removed tone
// path. Pitches and lengths approximate the old `tone_for_sfx_id` tones
// (frequency in hertz; the old durations of 70-100 ms round to 4-6 ticks).

static BLIP_HIGH_EVENTS: [SequenceEvent; 2] = [
    SequenceEvent::Note {
        pitch: 660,
        duration_ticks: 4,
        volume: 200,
        instrument_id: 0,
    },
    SequenceEvent::End,
];
static BLIP_HIGH_SEQ: Sequence<'static> =
    Sequence::new(&BLIP_HIGH_EVENTS, &SFX_SQUARE, SFX_TICK_RATE);

static BLIP_TOP_EVENTS: [SequenceEvent; 2] = [
    SequenceEvent::Note {
        pitch: 880,
        duration_ticks: 6,
        volume: 200,
        instrument_id: 0,
    },
    SequenceEvent::End,
];
static BLIP_TOP_SEQ: Sequence<'static> =
    Sequence::new(&BLIP_TOP_EVENTS, &SFX_SQUARE, SFX_TICK_RATE);

static BLIP_LOW_EVENTS: [SequenceEvent; 2] = [
    SequenceEvent::Note {
        pitch: 440,
        duration_ticks: 6,
        volume: 200,
        instrument_id: 0,
    },
    SequenceEvent::End,
];
static BLIP_LOW_SEQ: Sequence<'static> =
    Sequence::new(&BLIP_LOW_EVENTS, &SFX_SQUARE, SFX_TICK_RATE);

static BLIP_MID_EVENTS: [SequenceEvent; 2] = [
    SequenceEvent::Note {
        pitch: 720,
        duration_ticks: 4,
        volume: 200,
        instrument_id: 0,
    },
    SequenceEvent::End,
];
static BLIP_MID_SEQ: Sequence<'static> =
    Sequence::new(&BLIP_MID_EVENTS, &SFX_SQUARE, SFX_TICK_RATE);

// --- Built-in `play_bgm(id)` loops ------------------------------------------
//
// Authored equivalents of the removed built-in MML strings: a square lead over
// a triangle bass, one tick per eighth note (id 0 at T120 -> 4 ticks/s, the
// default at T150 -> 5 ticks/s), looping forever.

const fn bgm_note(pitch: u16, duration_ticks: u16, volume: u8, instrument_id: u8) -> SequenceEvent {
    SequenceEvent::Note {
        pitch,
        duration_ticks,
        volume,
        instrument_id,
    }
}

const BGM_LEAD_VOLUME: u8 = 136; // legacy V8
const BGM_BASS_VOLUME: u8 = 85; // legacy V5

static BUILTIN_BGM_0_LEAD_EVENTS: [SequenceEvent; 11] = [
    SequenceEvent::LoopStart,
    bgm_note(523, 1, BGM_LEAD_VOLUME, BUILTIN_INSTRUMENT_SQUARE), // C5
    bgm_note(659, 1, BGM_LEAD_VOLUME, BUILTIN_INSTRUMENT_SQUARE), // E5
    bgm_note(784, 1, BGM_LEAD_VOLUME, BUILTIN_INSTRUMENT_SQUARE), // G5
    bgm_note(659, 1, BGM_LEAD_VOLUME, BUILTIN_INSTRUMENT_SQUARE), // E5
    bgm_note(587, 1, BGM_LEAD_VOLUME, BUILTIN_INSTRUMENT_SQUARE), // D5
    bgm_note(699, 1, BGM_LEAD_VOLUME, BUILTIN_INSTRUMENT_SQUARE), // F5
    bgm_note(880, 1, BGM_LEAD_VOLUME, BUILTIN_INSTRUMENT_SQUARE), // A5
    bgm_note(699, 1, BGM_LEAD_VOLUME, BUILTIN_INSTRUMENT_SQUARE), // F5
    SequenceEvent::LoopEnd {
        repeat_count: SEQUENCE_REPEAT_INFINITE,
    },
    SequenceEvent::End,
];
static BUILTIN_BGM_0_BASS_EVENTS: [SequenceEvent; 7] = [
    SequenceEvent::LoopStart,
    bgm_note(131, 2, BGM_BASS_VOLUME, BUILTIN_INSTRUMENT_TRIANGLE), // C3
    bgm_note(196, 2, BGM_BASS_VOLUME, BUILTIN_INSTRUMENT_TRIANGLE), // G3
    bgm_note(147, 2, BGM_BASS_VOLUME, BUILTIN_INSTRUMENT_TRIANGLE), // D3
    bgm_note(196, 2, BGM_BASS_VOLUME, BUILTIN_INSTRUMENT_TRIANGLE), // G3
    SequenceEvent::LoopEnd {
        repeat_count: SEQUENCE_REPEAT_INFINITE,
    },
    SequenceEvent::End,
];
static BUILTIN_BGM_0_VOICES: [PolyphonicSequenceVoice<'static>; 2] = [
    PolyphonicSequenceVoice::new(
        Sequence::new(&BUILTIN_BGM_0_LEAD_EVENTS, &BUILTIN_SEQUENCE_INSTRUMENTS, 4),
        MixerVolume::UNITY,
    ),
    PolyphonicSequenceVoice::new(
        Sequence::new(&BUILTIN_BGM_0_BASS_EVENTS, &BUILTIN_SEQUENCE_INSTRUMENTS, 4),
        MixerVolume::UNITY,
    ),
];
static BUILTIN_BGM_0: PolyphonicSequence<'static> = PolyphonicSequence::new(&BUILTIN_BGM_0_VOICES);

static BUILTIN_BGM_1_LEAD_EVENTS: [SequenceEvent; 11] = [
    SequenceEvent::LoopStart,
    bgm_note(659, 1, BGM_LEAD_VOLUME, BUILTIN_INSTRUMENT_SQUARE), // E5
    bgm_note(784, 1, BGM_LEAD_VOLUME, BUILTIN_INSTRUMENT_SQUARE), // G5
    bgm_note(988, 1, BGM_LEAD_VOLUME, BUILTIN_INSTRUMENT_SQUARE), // B5
    bgm_note(784, 1, BGM_LEAD_VOLUME, BUILTIN_INSTRUMENT_SQUARE), // G5
    bgm_note(659, 1, BGM_LEAD_VOLUME, BUILTIN_INSTRUMENT_SQUARE), // E5
    bgm_note(784, 1, BGM_LEAD_VOLUME, BUILTIN_INSTRUMENT_SQUARE), // G5
    bgm_note(988, 1, BGM_LEAD_VOLUME, BUILTIN_INSTRUMENT_SQUARE), // B5
    bgm_note(784, 1, BGM_LEAD_VOLUME, BUILTIN_INSTRUMENT_SQUARE), // G5
    SequenceEvent::LoopEnd {
        repeat_count: SEQUENCE_REPEAT_INFINITE,
    },
    SequenceEvent::End,
];
static BUILTIN_BGM_1_BASS_EVENTS: [SequenceEvent; 7] = [
    SequenceEvent::LoopStart,
    bgm_note(131, 2, BGM_BASS_VOLUME, BUILTIN_INSTRUMENT_TRIANGLE), // C3
    bgm_note(131, 2, BGM_BASS_VOLUME, BUILTIN_INSTRUMENT_TRIANGLE), // C3
    bgm_note(196, 2, BGM_BASS_VOLUME, BUILTIN_INSTRUMENT_TRIANGLE), // G3
    bgm_note(196, 2, BGM_BASS_VOLUME, BUILTIN_INSTRUMENT_TRIANGLE), // G3
    SequenceEvent::LoopEnd {
        repeat_count: SEQUENCE_REPEAT_INFINITE,
    },
    SequenceEvent::End,
];
static BUILTIN_BGM_1_VOICES: [PolyphonicSequenceVoice<'static>; 2] = [
    PolyphonicSequenceVoice::new(
        Sequence::new(&BUILTIN_BGM_1_LEAD_EVENTS, &BUILTIN_SEQUENCE_INSTRUMENTS, 5),
        MixerVolume::UNITY,
    ),
    PolyphonicSequenceVoice::new(
        Sequence::new(&BUILTIN_BGM_1_BASS_EVENTS, &BUILTIN_SEQUENCE_INSTRUMENTS, 5),
        MixerVolume::UNITY,
    ),
];
static BUILTIN_BGM_1: PolyphonicSequence<'static> = PolyphonicSequence::new(&BUILTIN_BGM_1_VOICES);
