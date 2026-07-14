//! koto-mml: `.kmml` audition renderer (KOTO-0188).
//!
//! Renders native KotoAudio KotoMML to mono PCM16 samples without launching an
//! app. The same conversion and service path is used by the Pico cue tables.

use std::cell::RefCell;
use std::rc::Rc;

use koto_audio::{
    parse_clip_asset, AudioBackend, AudioPolicy, BackendReport, BackendResult, BackendState,
    ClipLoop, DefaultAudioService, MixerBlock, MixerVolume, PolyphonicSequence,
    PolyphonicSequenceVoice, Sequence, BUILTIN_SEQUENCE_INSTRUMENTS, DEFAULT_MIXER_BLOCK_FRAMES,
    MAX_SEQUENCE_VOICES,
};
use koto_audio_gen::convert_mml_text;
use koto_audio_tools::{convert_wav_to_clip_asset, ConvertOptions};
/// Native KotoAudio audition options.
pub struct Options {
    /// Loop like BGM (bounded by `max_seconds`) instead of playing once.
    pub loop_playback: bool,
    /// Upper bound on the rendered length; a score that finishes earlier stops
    /// at its natural end.
    pub max_seconds: f64,
    /// 0-based track indices to drop before rendering.
    pub mute: Vec<usize>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            loop_playback: false,
            max_seconds: 10.0,
            mute: Vec::new(),
        }
    }
}

/// A finished render: mono PCM16 plus the report lines the CLI prints.
#[derive(Debug)]
pub struct Rendered {
    pub sample_rate: u32,
    pub samples: Vec<i16>,
    pub track_count: usize,
    /// Human-readable notes such as voice-cap truncation.
    pub notes: Vec<String>,
}

/// Render native KotoAudio KotoMML per `options`.
pub fn render(text: &str, options: &Options) -> Result<Rendered, String> {
    render_native(text, options)
}

/// A baked KACL clip plus the numbers an author needs to size it.
#[derive(Debug)]
pub struct Baked {
    /// Runtime-ready KACL bytes (already through runtime validation).
    pub kacl: Vec<u8>,
    pub sample_rate: u32,
    pub sample_count: usize,
    /// PCM16 payload size — the number that matters on the SD budget.
    pub payload_bytes: usize,
    pub notes: Vec<String>,
}

/// Bake a native KotoAudio score into a PCM16 mono KACL clip.
/// `clip_loop` becomes the KACL loop metadata so a sustained tone can loop
/// on device instead of storing seconds of samples.
pub fn bake(text: &str, options: &Options, clip_loop: ClipLoop) -> Result<Baked, String> {
    let sample_rate = AudioPolicy::v0_default().limits.sample_rate_hz;
    let rendered = render_native(text, options)?;
    let wav = koto_sim::audio::wav_mono_bytes(sample_rate, &rendered.samples);
    let converted = convert_wav_to_clip_asset(
        "koto-mml-bake",
        &wav,
        ConvertOptions {
            loop_metadata: clip_loop,
            // Already mono at the target rate; any mismatch is a bug here,
            // not input to repair.
            strict_input: true,
            ..ConvertOptions::default()
        },
    )
    .map_err(|error| format!("KACL conversion failed: {error:?}"))?;
    parse_clip_asset(&converted.asset_bytes, AudioPolicy::v0_default().limits)
        .map_err(|error| format!("baked asset failed runtime validation: {error:?}"))?;
    Ok(Baked {
        kacl: converted.asset_bytes,
        sample_rate,
        sample_count: rendered.samples.len(),
        payload_bytes: rendered.samples.len() * 2,
        notes: rendered.notes,
    })
}

fn apply_mute<T>(tracks: &mut Vec<T>, mute: &[usize]) -> Result<(), String> {
    for &index in mute {
        if index >= tracks.len() {
            return Err(format!(
                "--mute {index}: score has {} track(s), indices are 0-based",
                tracks.len()
            ));
        }
    }
    let mut index = 0;
    tracks.retain(|_| {
        let keep = !mute.contains(&index);
        index += 1;
        keep
    });
    if tracks.is_empty() {
        return Err("every track is muted; nothing to render".to_string());
    }
    Ok(())
}

/// Capture backend: collects every mixed block as mono PCM16.
struct CaptureBackend {
    state: BackendState,
    samples: Rc<RefCell<Vec<i16>>>,
}

impl AudioBackend<DEFAULT_MIXER_BLOCK_FRAMES> for CaptureBackend {
    fn start(&mut self) -> BackendResult {
        self.state = BackendState::Running;
        Ok(BackendReport::backend_restart())
    }
    fn stop(&mut self) -> BackendResult {
        self.state = BackendState::Stopped;
        Ok(BackendReport::default())
    }
    fn submit_block(&mut self, block: &MixerBlock<DEFAULT_MIXER_BLOCK_FRAMES>) -> BackendResult {
        self.samples
            .borrow_mut()
            .extend_from_slice(block.as_pcm16_mono());
        Ok(BackendReport::submitted_block())
    }
    fn suspend(&mut self) -> BackendResult {
        Ok(BackendReport::default())
    }
    fn resume(&mut self) -> BackendResult {
        Ok(BackendReport::backend_restart())
    }
    fn query_state(&self) -> BackendState {
        self.state
    }
    fn reset(&mut self) -> BackendResult {
        Ok(BackendReport::default())
    }
}

/// Device-parity render: `koto-audio-gen` conversion into the koto-audio
/// service, the same shape the Pico core1 worker runs.
fn render_native(text: &str, options: &Options) -> Result<Rendered, String> {
    let mut notes = Vec::new();
    let mut score = convert_mml_text(text, options.loop_playback)?;
    apply_mute(&mut score.tracks, &options.mute)?;
    if score.tracks.len() > MAX_SEQUENCE_VOICES {
        notes.push(format!(
            "note: {} tracks truncated to the device voice cap of {MAX_SEQUENCE_VOICES}",
            score.tracks.len()
        ));
        score.tracks.truncate(MAX_SEQUENCE_VOICES);
    }
    let track_count = score.tracks.len();

    let sequences: Vec<Sequence<'_>> = score
        .tracks
        .iter()
        .map(|track| {
            Sequence::new(
                &track.events,
                &BUILTIN_SEQUENCE_INSTRUMENTS,
                score.tick_rate_hz,
            )
        })
        .collect();
    let voices: Vec<PolyphonicSequenceVoice<'_>> = sequences
        .iter()
        .zip(&score.tracks)
        .map(|(sequence, track)| {
            PolyphonicSequenceVoice::new(*sequence, MixerVolume::new(track.gain))
        })
        .collect();

    let policy = AudioPolicy::v0_default();
    let sample_rate = policy.limits.sample_rate_hz;
    let captured = Rc::new(RefCell::new(Vec::new()));
    let backend = CaptureBackend {
        state: BackendState::Stopped,
        samples: captured.clone(),
    };
    let mut service = DefaultAudioService::new(policy, backend)
        .map_err(|error| format!("audio service rejected the v0 policy: {error:?}"))?;
    service
        .start()
        .map_err(|error| format!("backend start failed: {error:?}"))?;
    service
        .play_bgm_sequence(PolyphonicSequence::new(&voices))
        .map_err(|error| format!("sequence rejected: {error:?}"))?;

    let max_samples = (options.max_seconds * f64::from(sample_rate)) as usize;
    while captured.borrow().len() < max_samples {
        service
            .tick()
            .map_err(|error| format!("mixer tick failed: {error:?}"))?;
        if service.counter_snapshot().active_source_count == 0 {
            break;
        }
    }
    let mut samples = std::mem::take(&mut *captured.borrow_mut());
    samples.truncate(max_samples);

    Ok(Rendered {
        sample_rate,
        samples,
        track_count,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn repo_path(rel: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(rel)
    }

    fn render_fixture(rel: &str, options: &Options) -> Rendered {
        let path = repo_path(rel);
        let text = std::fs::read_to_string(&path).expect("read fixture");
        render(&text, options).expect("render")
    }

    fn fnv1a(samples: &[i16]) -> u64 {
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for sample in samples {
            for byte in sample.to_le_bytes() {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        hash
    }

    /// The harness determinism gate: the committed fixture renders the same
    /// samples every run, and the render is audible. If a
    /// deliberate synth or conversion change moves these hashes, re-pin them
    /// from the test failure output.
    #[test]
    fn twinkle_fixture_renders_are_deterministic_and_audible() {
        let options = Options {
            // The fixture is 21 beats at T120 = 10.5 s; leave headroom so
            // the render ends at the score's natural end, not the bound.
            max_seconds: 15.0,
            ..Options::default()
        };
        let first = render_fixture("harness/fixtures/kotomml/twinkle.kmml", &options);
        let second = render_fixture("harness/fixtures/kotomml/twinkle.kmml", &options);
        assert_eq!(first.sample_rate, 16_000);
        assert_eq!(
            first.samples, second.samples,
            "native render nondeterministic"
        );
        assert!(first.samples.iter().any(|&sample| sample != 0));
        let rate = first.sample_rate as usize;
        assert!(first.samples.len() > 10 * rate && first.samples.len() < 12 * rate);
        assert_eq!(fnv1a(&first.samples), TWINKLE_NATIVE_HASH);
    }

    const TWINKLE_NATIVE_HASH: u64 = 0x1ce1_3fc5_88af_fdb9;

    /// Looping playback is bounded by `max_seconds` exactly.
    #[test]
    fn looped_render_is_bounded() {
        let options = Options {
            loop_playback: true,
            max_seconds: 1.0,
            ..Options::default()
        };
        let rendered = render_fixture("harness/fixtures/kotomml/twinkle.kmml", &options);
        assert_eq!(rendered.samples.len(), rendered.sample_rate as usize);
    }

    /// Shipped app KMML uses native builtin instruments and renders through the
    /// same path as generated runtime cues.
    #[test]
    fn kotosnake_bgm_renders_as_native_kotoaudio() {
        let rendered = render_fixture(
            "apps/kotosnake/audio/bgm.kmml",
            &Options {
                max_seconds: 2.0,
                loop_playback: true,
                ..Options::default()
            },
        );
        assert!(rendered.samples.iter().any(|&sample| sample != 0));
        assert_eq!(rendered.sample_rate, 16_000);
    }

    #[test]
    fn mute_validates_indices_and_rejects_all_muted() {
        let text = std::fs::read_to_string(repo_path("harness/fixtures/kotomml/twinkle.kmml"))
            .expect("read fixture");
        let out_of_range = render(
            &text,
            &Options {
                mute: vec![3],
                ..Options::default()
            },
        );
        assert!(out_of_range.unwrap_err().contains("1 track(s)"));
        let all_muted = render(
            &text,
            &Options {
                mute: vec![0],
                ..Options::default()
            },
        );
        assert!(all_muted.unwrap_err().contains("every track is muted"));
    }

    /// KOTO-0189 proving case: the committed Native KotoAudio jingle bakes to
    /// the committed KACL byte-for-byte (drift check), the payload numbers
    /// are consistent, and the baked clip plays through the koto-audio
    /// service path — the device-parity backend shape.
    #[test]
    fn baked_jingle_matches_committed_fixture_and_plays_through_the_service() {
        let path = repo_path("harness/fixtures/kacl_bake/native_pcm16_jingle.kmml");
        let text = std::fs::read_to_string(&path).expect("read fixture");
        let baked = bake(&text, &Options::default(), ClipLoop::None).expect("bake");
        assert_eq!(baked.sample_rate, 16_000);
        assert_eq!(baked.payload_bytes, baked.sample_count * 2);
        assert!(baked.notes.is_empty());

        let committed = std::fs::read(repo_path(
            "harness/fixtures/kacl_bake/native_pcm16_jingle.kacl",
        ))
        .expect("read committed kacl");
        assert_eq!(
            baked.kacl, committed,
            "baked clip drifted from the committed fixture; rebake it \
             (see the .kmml header) if the change is intentional"
        );

        let clip = parse_clip_asset(&baked.kacl, AudioPolicy::v0_default().limits)
            .expect("runtime validation");
        let captured = Rc::new(RefCell::new(Vec::new()));
        let backend = CaptureBackend {
            state: BackendState::Stopped,
            samples: captured.clone(),
        };
        let mut service =
            DefaultAudioService::new(AudioPolicy::v0_default(), backend).expect("service");
        service.start().expect("backend starts");
        service.play_clip(clip).expect("clip admits");
        // Tick first: the clip is queued and only becomes an active source
        // once the mixer admits it. Cap well past the 1 s jingle.
        for _ in 0..(4 * 16_000 / DEFAULT_MIXER_BLOCK_FRAMES) {
            service.tick().expect("tick renders");
            if service.counter_snapshot().active_source_count == 0 {
                break;
            }
        }
        assert!(
            captured.borrow().iter().any(|&sample| sample != 0),
            "baked clip must be audible through the service"
        );
    }

    /// KACL loop metadata: `whole` writes an infinite loop over the full
    /// range; an inverted forward range fails the converter's loop policy.
    #[test]
    fn bake_writes_loop_metadata_and_rejects_bad_ranges() {
        let path = repo_path("harness/fixtures/kacl_bake/native_pcm16_jingle.kmml");
        let text = std::fs::read_to_string(&path).expect("read fixture");
        let looped = bake(
            &text,
            &Options::default(),
            ClipLoop::Whole {
                count: koto_audio::LoopCount::Infinite,
            },
        )
        .expect("bake with whole-clip loop");
        // Header fields: loop start (20..24), loop end (24..28), loop count
        // (28..32). A whole-clip loop is encoded as start=0, end=0.
        let field =
            |kacl: &[u8], o: usize| u32::from_le_bytes(kacl[o..o + 4].try_into().expect("field"));
        assert_eq!(field(&looped.kacl, 20), 0);
        assert_eq!(field(&looped.kacl, 24), 0);
        assert_eq!(field(&looped.kacl, 28), u32::MAX);

        let forward = bake(
            &text,
            &Options::default(),
            ClipLoop::Forward {
                start: 100,
                end: 500,
                count: koto_audio::LoopCount::Infinite,
            },
        )
        .expect("bake with forward loop");
        assert_eq!(field(&forward.kacl, 20), 100);
        assert_eq!(field(&forward.kacl, 24), 500);
        assert_eq!(field(&forward.kacl, 28), u32::MAX);

        let inverted = bake(
            &text,
            &Options::default(),
            ClipLoop::Forward {
                start: 500,
                end: 100,
                count: koto_audio::LoopCount::Infinite,
            },
        );
        assert!(inverted.is_err(), "inverted loop range must be rejected");
    }
}
