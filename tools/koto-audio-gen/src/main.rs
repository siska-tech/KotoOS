//! Offline generator for the Pico primary-audio cue tables (KOTO-0165).
//!
//! Scans `apps/*/audio/*.kmml`, parses each source with the koto-audio-tools MML
//! frontend using the native KotoAudio instrument, volume, and drum model, and
//! emits one vendored Rust module of `SequenceEvent` statics plus the per-app route
//! arrays consumed by `koto_pico::firmware::audio_cues`.
//!
//! Regeneration (from the KotoOS repo root):
//!
//! ```console
//! cargo run -p koto-audio-gen -- src/koto-pico/src/firmware/audio_cues_generated.rs
//! ```

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use koto_audio::{
    AudioLimits, MixerVolume, PolyphonicSequence, PolyphonicSequenceVoice, Sequence, SequenceEvent,
    BUILTIN_SEQUENCE_INSTRUMENTS, MAX_SEQUENCE_VOICES,
};
use koto_audio_gen::{convert_mml_text, ConvertedTrack};

/// One asset to convert: the MML source and its routing key.
struct AssetSpec {
    source: PathBuf,
    /// Routing key as apps declare it (`audio/<app>_<name>.kmml`).
    key: String,
    /// Generated static symbol base (`KOTOMINES_MOVE`).
    symbol: String,
    is_bgm: bool,
}

/// A converted asset ready to emit.
struct EmitAsset {
    key: String,
    symbol: String,
    is_bgm: bool,
    tick_rate_hz: u16,
    tracks: Vec<ConvertedTrack>,
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
    let mut output: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--root" => root = PathBuf::from(args.next().ok_or("missing --root value")?),
            _ if arg.starts_with('-') => return Err(format!("unknown option {arg}")),
            _ if output.is_none() => output = Some(PathBuf::from(arg)),
            _ => return Err("expected exactly one output path".to_string()),
        }
    }
    let output = output.ok_or("usage: koto-audio-gen [--root DIR] OUTPUT.rs")?;

    let specs = collect_specs(&root)?;
    let mut assets = Vec::new();
    for spec in &specs {
        assets.push(convert_asset(spec)?);
    }
    validate_assets(&assets)?;

    let rendered = render_module(&assets);
    std::fs::write(&output, rendered)
        .map_err(|error| format!("failed to write {}: {error}", output.display()))?;
    let status = Command::new("rustfmt")
        .args(["--edition", "2021"])
        .arg(&output)
        .status()
        .map_err(|error| format!("failed to run rustfmt on {}: {error}", output.display()))?;
    if !status.success() {
        return Err(format!("rustfmt failed for {}", output.display()));
    }

    let bgm_count = assets.iter().filter(|asset| asset.is_bgm).count();
    Ok(format!(
        "generated {} cues ({} BGM, {} SFX) -> {}",
        assets.len(),
        bgm_count,
        assets.len() - bgm_count,
        output.display()
    ))
}

fn collect_specs(root: &Path) -> Result<Vec<AssetSpec>, String> {
    let mut specs = Vec::new();

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
                source: file,
            });
        }
    }
    Ok(specs)
}

fn convert_asset(spec: &AssetSpec) -> Result<EmitAsset, String> {
    let source = std::fs::read_to_string(&spec.source)
        .map_err(|error| format!("failed to read {}: {error}", spec.source.display()))?;
    // The native conversion and BGM infinite-loop policy live in the crate
    // library so the audition CLI (koto-mml, KOTO-0188) shares them.
    let mut score = convert_mml_text(&source, spec.is_bgm)
        .map_err(|error| format!("{}: {error}", spec.source.display()))?;
    if !spec.is_bgm && score.tracks.len() > 1 {
        // SFX are monophonic in the primary cue model.
        score.tracks.truncate(1);
    }

    Ok(EmitAsset {
        key: spec.key.clone(),
        symbol: spec.symbol.clone(),
        is_bgm: spec.is_bgm,
        tick_rate_hz: score.tick_rate_hz,
        tracks: score.tracks,
    })
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
         //! native KotoAudio `apps/*/audio/*.kmml` sources.\n\
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
        MixerBlock, BUILTIN_INSTRUMENT_BASS_DRUM, BUILTIN_INSTRUMENT_CLOSED_HI_HAT,
        DEFAULT_MIXER_BLOCK_FRAMES,
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
    /// native KotoAudio `.kmml` converts, validates, and renders non-silent
    /// output through the same koto-audio service shape the Pico worker runs.
    #[test]
    fn converted_native_bgm_renders_non_silent_audio() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let spec = AssetSpec {
            source: root.join("apps/kotomines/audio/bgm.kmml"),
            key: "audio/kotomines_bgm.kmml".to_string(),
            symbol: "KOTOMINES_BGM".to_string(),
            is_bgm: true,
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
    fn every_koto_blocks_cue_uses_the_generic_per_app_scan() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let specs = collect_specs(&root).expect("shipped audio sources scan");
        let bgm = specs
            .iter()
            .find(|spec| spec.key == "audio/koto_blocks_bgm.kmml")
            .expect("KotoBlocks BGM is generated");

        assert_eq!(bgm.source, root.join("apps/koto_blocks/audio/bgm.kmml"));
        let asset = convert_asset(bgm).expect("native KotoBlocks BGM converts");
        assert_eq!(asset.tracks.len(), 4);
        let drum_events = &asset.tracks[3].events;
        assert!(drum_events.iter().any(|event| matches!(
            event,
            SequenceEvent::Note { instrument_id, .. }
                if *instrument_id == BUILTIN_INSTRUMENT_BASS_DRUM
        )));
        assert!(drum_events.iter().any(|event| matches!(
            event,
            SequenceEvent::Note { instrument_id, .. }
                if *instrument_id == BUILTIN_INSTRUMENT_CLOSED_HI_HAT
        )));
        assert!(specs
            .iter()
            .any(|spec| spec.key == "audio/koto_blocks_move.kmml"));
    }
}
