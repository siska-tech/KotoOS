//! Offline generator for the Pico primary-audio cue tables (KOTO-0165).
//!
//! Scans `apps/*/audio/*.kmml`, parses each source with the koto-audio-tools MML
//! frontend, converts the **legacy KotoMML dialect** (0-15 `V` volumes, legacy
//! synth `@` instrument ids) to the KotoAudio builtin-instrument model, and emits
//! one vendored Rust module of `SequenceEvent` statics plus the per-app route
//! arrays consumed by `koto_pico::firmware::audio_cues`.
//!
//! KotoBlocks is special-cased for SIM parity: its BGM is generated from the same
//! `examples/mml/blocks_like_bgm.mml` source (native dialect, no conversion) that
//! produced the SIM `BLOCKS_LIKE_BGM_COMPACT` table, and its SFX cues are the
//! hand-authored statics in `audio_cues.rs`, so they are skipped here.
//!
//! Regeneration (from the KotoOS repo root, sibling `koto-audio` checkout):
//!
//! ```console
//! cargo run -p koto-audio-gen -- src/koto-pico/src/firmware/audio_cues_generated.rs
//! ```

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use koto_audio::{
    AudioLimits, CompactEvent, MixerVolume, PolyphonicSequence, PolyphonicSequenceVoice, Sequence,
    SequenceEvent, BUILTIN_INSTRUMENT_CLOSED_HI_HAT, BUILTIN_INSTRUMENT_SAW,
    BUILTIN_INSTRUMENT_SQUARE, BUILTIN_INSTRUMENT_SQUARE_FAST, BUILTIN_INSTRUMENT_TRIANGLE,
    BUILTIN_SEQUENCE_INSTRUMENTS, MAX_SEQUENCE_VOICES, SEQUENCE_REPEAT_INFINITE,
};
use koto_audio_tools::mml::{parse_mml_to_compact_sequence_table_with_options, MmlParseOptions};
use koto_audio_tools::CompactSequenceTable;

/// Which MML dialect a source file was authored in.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Dialect {
    /// Legacy KotoOS `.kmml`: `V` is 0-15, `@` ids name the legacy Pico synth
    /// voices, quarter-note resolution assumed 96 ticks (matches the removed
    /// runtime parser exactly, including dotted lengths).
    Legacy,
    /// koto-audio-tools native MML: `V` is 0-127, `@` ids are builtin ids.
    Native,
}

/// Maps a legacy Pico synth `@` id to the closest KotoAudio builtin instrument.
///
/// Legacy voices (see the removed `pico_instrument` table): 0 square, 1 pulse25,
/// 2/8 triangle, 3 saw, 4 noise, 5 pulse12, 6 warm saw lead, 7 warm pulse bass,
/// 9 short noise. Custom `.kwt` instruments occupied ids 6-31 per app; the
/// shipped apps used 6 (lead), 7 (bass), and 9 (drum), mapped to the same
/// builtin timbres as their synth fallbacks. There is no pulse or pitched-noise
/// builtin, so pulses become squares and noise becomes the closed hi-hat.
fn map_legacy_instrument(id: u8) -> u8 {
    match id {
        0 => BUILTIN_INSTRUMENT_SQUARE,
        1 | 5 => BUILTIN_INSTRUMENT_SQUARE_FAST,
        2 | 7 | 8 => BUILTIN_INSTRUMENT_TRIANGLE,
        3 | 6 => BUILTIN_INSTRUMENT_SAW,
        4 | 9 => BUILTIN_INSTRUMENT_CLOSED_HI_HAT,
        other => {
            if usize::from(other) < BUILTIN_SEQUENCE_INSTRUMENTS.len() {
                other
            } else {
                BUILTIN_INSTRUMENT_SQUARE
            }
        }
    }
}

/// Rescales a legacy 0-15 `V` volume onto the 0-255 note-volume scale.
fn rescale_legacy_volume(volume: u8) -> u8 {
    volume.saturating_mul(17)
}

/// One asset to convert: the MML source, its routing key, and its dialect.
struct AssetSpec {
    source: PathBuf,
    /// Routing key as apps declare it (`audio/<app>_<name>.kmml`).
    key: String,
    /// Generated static symbol base (`KOTOMINES_MOVE`).
    symbol: String,
    is_bgm: bool,
    dialect: Dialect,
}

/// A converted track ready to emit: adapted events plus voice gain.
struct EmitTrack {
    events: Vec<SequenceEvent>,
    gain: u16,
}

/// A converted asset ready to emit.
struct EmitAsset {
    key: String,
    symbol: String,
    is_bgm: bool,
    tick_rate_hz: u16,
    tracks: Vec<EmitTrack>,
}

fn main() -> ExitCode {
    match run() {
        Ok(report) => {
            println!("{report}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("koto-audio-gen: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<String, String> {
    let mut root = PathBuf::from(".");
    let mut koto_audio_root = PathBuf::from("../koto-audio");
    let mut output: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--root" => root = PathBuf::from(args.next().ok_or("missing --root value")?),
            "--koto-audio" => {
                koto_audio_root = PathBuf::from(args.next().ok_or("missing --koto-audio value")?)
            }
            _ if arg.starts_with('-') => return Err(format!("unknown option {arg}")),
            _ if output.is_none() => output = Some(PathBuf::from(arg)),
            _ => return Err("expected exactly one output path".to_string()),
        }
    }
    let output = output.ok_or("usage: koto-audio-gen [--root DIR] [--koto-audio DIR] OUTPUT.rs")?;

    let specs = collect_specs(&root, &koto_audio_root)?;
    let mut assets = Vec::new();
    for spec in &specs {
        assets.push(convert_asset(spec)?);
    }
    validate_assets(&assets)?;

    let rendered = render_module(&assets);
    std::fs::write(&output, rendered)
        .map_err(|error| format!("failed to write {}: {error}", output.display()))?;

    let bgm_count = assets.iter().filter(|asset| asset.is_bgm).count();
    Ok(format!(
        "generated {} cues ({} BGM, {} SFX) -> {}",
        assets.len(),
        bgm_count,
        assets.len() - bgm_count,
        output.display()
    ))
}

fn collect_specs(root: &Path, koto_audio_root: &Path) -> Result<Vec<AssetSpec>, String> {
    let mut specs = Vec::new();

    // KotoBlocks BGM: the same native-dialect source that generated the SIM
    // `BLOCKS_LIKE_BGM_COMPACT` table, so SIM and device play identical cues.
    let blocks_like = koto_audio_root.join("examples/mml/blocks_like_bgm.mml");
    if !blocks_like.is_file() {
        return Err(format!(
            "missing {} (pass --koto-audio pointing at the sibling koto-audio checkout)",
            blocks_like.display()
        ));
    }
    specs.push(AssetSpec {
        source: blocks_like,
        key: "audio/koto_blocks_bgm.kmml".to_string(),
        symbol: "KOTO_BLOCKS_BGM".to_string(),
        is_bgm: true,
        dialect: Dialect::Native,
    });

    let apps_dir = root.join("apps");
    let mut app_dirs: Vec<PathBuf> = std::fs::read_dir(&apps_dir)
        .map_err(|error| format!("failed to read {}: {error}", apps_dir.display()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.is_dir() && path.join("audio").is_dir())
        .collect();
    app_dirs.sort();

    for app_dir in app_dirs {
        let app = app_dir
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| format!("non-UTF8 app dir {}", app_dir.display()))?
            .to_string();
        // KotoBlocks SFX are the hand-authored statics in `audio_cues.rs`
        // (SIM-parity envelopes/drums the compact MML path cannot express).
        if app == "koto_blocks" {
            continue;
        }
        let mut files: Vec<PathBuf> = std::fs::read_dir(app_dir.join("audio"))
            .map_err(|error| format!("failed to read {}/audio: {error}", app_dir.display()))?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.extension().is_some_and(|ext| ext == "kmml"))
            .collect();
        files.sort();
        for file in files {
            let name = file
                .file_stem()
                .and_then(|stem| stem.to_str())
                .ok_or_else(|| format!("non-UTF8 asset name {}", file.display()))?
                .to_string();
            specs.push(AssetSpec {
                key: format!("audio/{app}_{name}.kmml"),
                symbol: format!("{}_{}", app.to_uppercase(), name.to_uppercase()),
                is_bgm: name.starts_with("bgm"),
                dialect: Dialect::Legacy,
                source: file,
            });
        }
    }
    Ok(specs)
}

fn convert_asset(spec: &AssetSpec) -> Result<EmitAsset, String> {
    let source = std::fs::read_to_string(&spec.source)
        .map_err(|error| format!("failed to read {}: {error}", spec.source.display()))?;
    let options = match spec.dialect {
        // 96 ticks per quarter mirrors the removed legacy runtime parser exactly
        // (L32 and dotted lengths stay integral); defaults match its initial
        // state (T120 V10 O4 L4 @0).
        Dialect::Legacy => MmlParseOptions {
            ticks_per_beat: 96,
            default_volume: 10,
            default_octave: 4,
            ..MmlParseOptions::default()
        },
        Dialect::Native => MmlParseOptions::default(),
    };
    let table = parse_mml_to_compact_sequence_table_with_options(&source, options)
        .map_err(|error| format!("{}: parse failed: {error:?}", spec.source.display()))?;

    let mut tracks = convert_tracks(&table, spec.dialect);
    if !spec.is_bgm && tracks.len() > 1 {
        // The legacy SFX path played track 0 only; keep that behavior.
        tracks.truncate(1);
    }
    if spec.is_bgm {
        for track in &mut tracks {
            ensure_infinite_loop(&mut track.events);
        }
    }

    Ok(EmitAsset {
        key: spec.key.clone(),
        symbol: spec.symbol.clone(),
        is_bgm: spec.is_bgm,
        tick_rate_hz: table.tempo.tick_rate_hz,
        tracks,
    })
}

/// Adapts compact tracks to `SequenceEvent`s against the builtin instrument
/// table, applying the legacy-dialect conversions where requested. This mirrors
/// `CompactTrack::adapt_to_sequence` (instrument volume is 255 in every table
/// the MML frontend emits, so note volumes pass through unscaled).
fn convert_tracks(table: &CompactSequenceTable, dialect: Dialect) -> Vec<EmitTrack> {
    table
        .tracks
        .iter()
        .map(|track| EmitTrack {
            gain: track.gain.get(),
            events: track
                .events
                .iter()
                .map(|event| convert_event(*event, table, dialect))
                .collect(),
        })
        .collect()
}

fn convert_event(
    event: CompactEvent,
    table: &CompactSequenceTable,
    dialect: Dialect,
) -> SequenceEvent {
    match event {
        CompactEvent::Note {
            pitch,
            duration_ticks,
            volume,
            instrument_id,
        } => {
            let builtin_id = table
                .instruments
                .get(usize::from(instrument_id))
                .map_or(BUILTIN_INSTRUMENT_SQUARE, |instrument| {
                    instrument.builtin_id
                });
            let (volume, instrument_id) = match dialect {
                Dialect::Legacy => (
                    rescale_legacy_volume(volume),
                    map_legacy_instrument(builtin_id),
                ),
                Dialect::Native => (volume, builtin_id),
            };
            SequenceEvent::Note {
                pitch,
                duration_ticks,
                volume,
                instrument_id,
            }
        }
        CompactEvent::Rest { duration_ticks } => SequenceEvent::Rest { duration_ticks },
        CompactEvent::LoopStart => SequenceEvent::LoopStart,
        CompactEvent::LoopEnd { repeat_count } => SequenceEvent::LoopEnd { repeat_count },
        CompactEvent::End => SequenceEvent::End,
    }
}

/// BGM must loop forever like the removed legacy player did even without `[ ]`:
/// wrap the whole track when the source has no loop region, and force finite
/// loop regions to infinite.
fn ensure_infinite_loop(events: &mut Vec<SequenceEvent>) {
    let mut has_loop = false;
    for event in events.iter_mut() {
        if let SequenceEvent::LoopEnd { repeat_count } = event {
            *repeat_count = SEQUENCE_REPEAT_INFINITE;
            has_loop = true;
        }
    }
    if !has_loop {
        let end = events
            .iter()
            .position(|event| matches!(event, SequenceEvent::End))
            .unwrap_or(events.len());
        events.insert(
            end,
            SequenceEvent::LoopEnd {
                repeat_count: SEQUENCE_REPEAT_INFINITE,
            },
        );
        events.insert(0, SequenceEvent::LoopStart);
    }
}

/// Rebuilds each emitted asset in memory and runs the runtime validators, so a
/// bad conversion fails generation instead of failing on device.
fn validate_assets(assets: &[EmitAsset]) -> Result<(), String> {
    let limits = AudioLimits::v0_default();
    for asset in assets {
        if asset.tracks.is_empty() {
            return Err(format!("{}: no tracks", asset.key));
        }
        if asset.is_bgm && asset.tracks.len() > MAX_SEQUENCE_VOICES {
            return Err(format!(
                "{}: {} tracks exceeds MAX_SEQUENCE_VOICES ({MAX_SEQUENCE_VOICES})",
                asset.key,
                asset.tracks.len()
            ));
        }
        let sequences: Vec<Sequence<'_>> = asset
            .tracks
            .iter()
            .map(|track| {
                Sequence::new(
                    &track.events,
                    &BUILTIN_SEQUENCE_INSTRUMENTS,
                    asset.tick_rate_hz,
                )
            })
            .collect();
        if asset.is_bgm {
            let voices: Vec<PolyphonicSequenceVoice<'_>> = sequences
                .iter()
                .zip(&asset.tracks)
                .map(|(sequence, track)| {
                    PolyphonicSequenceVoice::new(*sequence, MixerVolume::new(track.gain))
                })
                .collect();
            PolyphonicSequence::new(&voices)
                .validate(limits)
                .map_err(|error| format!("{}: BGM validation failed: {error:?}", asset.key))?;
        } else {
            sequences[0]
                .validate(limits)
                .map_err(|error| format!("{}: SFX validation failed: {error:?}", asset.key))?;
        }
    }
    Ok(())
}

fn render_module(assets: &[EmitAsset]) -> String {
    let mut out = String::new();
    out.push_str(
        "//! Generated primary-audio cue tables for the Pico firmware (KOTO-0165).\n\
         //!\n\
         //! GENERATED FILE - DO NOT EDIT. Produced by `tools/koto-audio-gen` from the\n\
         //! per-app `apps/*/audio/*.kmml` sources (legacy-dialect conversion) and the\n\
         //! koto-audio `examples/mml/blocks_like_bgm.mml` KotoBlocks BGM source.\n\
         //! Regenerate from the KotoOS repo root with:\n\
         //!\n\
         //! ```console\n\
         //! cargo run -p koto-audio-gen -- src/koto-pico/src/firmware/audio_cues_generated.rs\n\
         //! ```\n\n\
         use koto_audio::{\n    MixerVolume, PolyphonicSequence, PolyphonicSequenceVoice, Sequence, SequenceEvent,\n    BUILTIN_SEQUENCE_INSTRUMENTS,\n};\n\n",
    );

    for asset in assets {
        render_asset(&mut out, asset);
    }

    let bgm: Vec<&EmitAsset> = assets.iter().filter(|asset| asset.is_bgm).collect();
    let sfx: Vec<&EmitAsset> = assets.iter().filter(|asset| !asset.is_bgm).collect();

    let _ = writeln!(
        out,
        "/// Generated looping BGM routes: asset routing key -> polyphonic sequence.\npub static GENERATED_BGM_ROUTES: [(&str, &PolyphonicSequence<'static>); {}] = [",
        bgm.len()
    );
    for asset in &bgm {
        let _ = writeln!(out, "    (\"{}\", &{}),", asset.key, asset.symbol);
    }
    out.push_str("];\n\n");

    let _ = writeln!(
        out,
        "/// Generated one-shot SFX routes: asset routing key -> monophonic sequence.\npub static GENERATED_SFX_ROUTES: [(&str, &Sequence<'static>); {}] = [",
        sfx.len()
    );
    for asset in &sfx {
        let _ = writeln!(out, "    (\"{}\", &{}),", asset.key, asset.symbol);
    }
    out.push_str("];\n");
    out
}

fn render_asset(out: &mut String, asset: &EmitAsset) {
    for (index, track) in asset.tracks.iter().enumerate() {
        let _ = writeln!(
            out,
            "static {}_T{}_EVENTS: [SequenceEvent; {}] = [",
            asset.symbol,
            index,
            track.events.len()
        );
        for event in &track.events {
            let _ = writeln!(out, "    {},", format_event(*event));
        }
        out.push_str("];\n");
    }

    if asset.is_bgm {
        let _ = writeln!(
            out,
            "static {}_VOICES: [PolyphonicSequenceVoice<'static>; {}] = [",
            asset.symbol,
            asset.tracks.len()
        );
        for (index, track) in asset.tracks.iter().enumerate() {
            let _ = writeln!(
                out,
                "    PolyphonicSequenceVoice::new(\n        Sequence::new(&{}_T{}_EVENTS, &BUILTIN_SEQUENCE_INSTRUMENTS, {}),\n        MixerVolume::new({}),\n    ),",
                asset.symbol, index, asset.tick_rate_hz, track.gain
            );
        }
        out.push_str("];\n");
        let _ = writeln!(
            out,
            "static {}: PolyphonicSequence<'static> = PolyphonicSequence::new(&{}_VOICES);\n",
            asset.symbol, asset.symbol
        );
    } else {
        let _ = writeln!(
            out,
            "static {}: Sequence<'static> = Sequence::new(&{}_T0_EVENTS, &BUILTIN_SEQUENCE_INSTRUMENTS, {});\n",
            asset.symbol, asset.symbol, asset.tick_rate_hz
        );
    }
}

fn format_event(event: SequenceEvent) -> String {
    match event {
        SequenceEvent::Note {
            pitch,
            duration_ticks,
            volume,
            instrument_id,
        } => format!(
            "SequenceEvent::Note {{ pitch: {pitch}, duration_ticks: {duration_ticks}, volume: {volume}, instrument_id: {instrument_id} }}"
        ),
        SequenceEvent::Rest { duration_ticks } => {
            format!("SequenceEvent::Rest {{ duration_ticks: {duration_ticks} }}")
        }
        SequenceEvent::LoopStart => "SequenceEvent::LoopStart".to_string(),
        SequenceEvent::LoopEnd { repeat_count } => {
            format!("SequenceEvent::LoopEnd {{ repeat_count: {repeat_count} }}")
        }
        SequenceEvent::End => "SequenceEvent::End".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use koto_audio::{
        AudioBackend, AudioPolicy, BackendReport, BackendResult, BackendState, DefaultAudioService,
        MixerBlock, DEFAULT_MIXER_BLOCK_FRAMES,
    };

    /// Capture backend: records whether any submitted block was non-silent.
    struct CaptureBackend {
        state: BackendState,
        heard: std::rc::Rc<std::cell::Cell<bool>>,
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
        fn submit_block(
            &mut self,
            block: &MixerBlock<DEFAULT_MIXER_BLOCK_FRAMES>,
        ) -> BackendResult {
            if block.as_pcm16_mono().iter().any(|&sample| sample != 0) {
                self.heard.set(true);
            }
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

    /// End-to-end conversion check against a real shipped app source: a
    /// legacy-dialect `.kmml` converts, validates, and renders non-silent
    /// output through the same koto-audio service shape the Pico worker runs.
    #[test]
    fn converted_legacy_bgm_renders_non_silent_audio() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let spec = AssetSpec {
            source: root.join("apps/kotomines/audio/bgm.kmml"),
            key: "audio/kotomines_bgm.kmml".to_string(),
            symbol: "KOTOMINES_BGM".to_string(),
            is_bgm: true,
            dialect: Dialect::Legacy,
        };
        let asset = convert_asset(&spec).expect("shipped BGM converts");
        validate_assets(std::slice::from_ref(&asset)).expect("shipped BGM validates");

        let sequences: Vec<Sequence<'_>> = asset
            .tracks
            .iter()
            .map(|track| {
                Sequence::new(
                    &track.events,
                    &BUILTIN_SEQUENCE_INSTRUMENTS,
                    asset.tick_rate_hz,
                )
            })
            .collect();
        let voices: Vec<PolyphonicSequenceVoice<'_>> = sequences
            .iter()
            .zip(&asset.tracks)
            .map(|(sequence, track)| {
                PolyphonicSequenceVoice::new(*sequence, MixerVolume::new(track.gain))
            })
            .collect();

        let heard = std::rc::Rc::new(std::cell::Cell::new(false));
        let backend = CaptureBackend {
            state: BackendState::Stopped,
            heard: heard.clone(),
        };
        let mut service = DefaultAudioService::new(AudioPolicy::v0_default(), backend)
            .expect("v0 policy service");
        service.start().expect("backend starts");
        service
            .play_bgm_sequence(PolyphonicSequence::new(&voices))
            .expect("BGM admits");
        // One second of blocks at the device rate/block size.
        for _ in 0..(16_000 / DEFAULT_MIXER_BLOCK_FRAMES) {
            service.tick().expect("tick renders");
        }
        assert!(heard.get(), "converted BGM must render non-silent blocks");
        assert!(
            service.counter_snapshot().active_source_count > 0,
            "looping BGM must stay active"
        );
    }

    #[test]
    fn legacy_instrument_map_targets_valid_builtins() {
        for id in 0..=u8::MAX {
            let mapped = map_legacy_instrument(id);
            assert!(usize::from(mapped) < BUILTIN_SEQUENCE_INSTRUMENTS.len());
        }
    }

    #[test]
    fn legacy_volume_rescale_spans_full_note_volume() {
        assert_eq!(rescale_legacy_volume(0), 0);
        assert_eq!(rescale_legacy_volume(10), 170);
        assert_eq!(rescale_legacy_volume(15), 255);
    }

    #[test]
    fn missing_loop_region_is_wrapped_infinite() {
        let mut events = vec![
            SequenceEvent::Note {
                pitch: 440,
                duration_ticks: 4,
                volume: 200,
                instrument_id: 3,
            },
            SequenceEvent::End,
        ];
        ensure_infinite_loop(&mut events);
        assert!(matches!(events[0], SequenceEvent::LoopStart));
        assert!(matches!(
            events[2],
            SequenceEvent::LoopEnd {
                repeat_count: SEQUENCE_REPEAT_INFINITE
            }
        ));
        assert!(matches!(events[3], SequenceEvent::End));
    }

    #[test]
    fn finite_loop_regions_are_forced_infinite() {
        let mut events = vec![
            SequenceEvent::LoopStart,
            SequenceEvent::Rest { duration_ticks: 4 },
            SequenceEvent::LoopEnd { repeat_count: 3 },
            SequenceEvent::End,
        ];
        ensure_infinite_loop(&mut events);
        assert!(matches!(
            events[2],
            SequenceEvent::LoopEnd {
                repeat_count: SEQUENCE_REPEAT_INFINITE
            }
        ));
    }
}
