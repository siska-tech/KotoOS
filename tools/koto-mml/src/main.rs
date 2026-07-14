//! koto-mml CLI (KOTO-0188): render a `.kmml` score to WAV or play it now.

use std::process::ExitCode;

use koto_audio::{ClipLoop, LoopCount};
use koto_mml::{bake, render, Options, Rendered};

const USAGE: &str = "usage: koto-mml wav  IN.kmml OUT.wav  [options]\n\
                     \x20      koto-mml play IN.kmml [options]  (build with --features play)\n\
                     \x20      koto-mml bake IN.kmml OUT.kacl [options]  (native KotoAudio -> PCM16 clip)\n\
                     options:\n\
                     \x20 --loop              loop like BGM instead of playing once\n\
                     \x20 --seconds S         max render length in seconds (default: 10)\n\
                     \x20 --mute N            drop 0-based track N (repeatable)\n\
                     \x20 --clip-loop whole|START..END  (bake only) infinite KACL loop metadata,\n\
                     \x20                     samples at 16000 Hz; default: play once";

fn main() -> ExitCode {
    match run() {
        Ok(report) => {
            println!("{report}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("koto-mml: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<String, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((command, rest)) = args.split_first() else {
        return Err(USAGE.to_string());
    };
    let mut positional: Vec<&str> = Vec::new();
    let mut options = Options::default();
    let mut clip_loop = ClipLoop::None;
    let mut rest = rest.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--loop" => options.loop_playback = true,
            "--seconds" => {
                let value = rest.next().ok_or("--seconds expects a value")?;
                options.max_seconds = value
                    .parse::<f64>()
                    .ok()
                    .filter(|s| s.is_finite() && *s > 0.0)
                    .ok_or_else(|| format!("--seconds expects a positive number, got {value:?}"))?;
            }
            "--mute" => {
                let value = rest.next().ok_or("--mute expects a track index")?;
                options.mute.push(
                    value
                        .parse()
                        .map_err(|_| format!("--mute expects a track index, got {value:?}"))?,
                );
            }
            "--clip-loop" => {
                let value = rest.next().ok_or("--clip-loop expects whole|START..END")?;
                clip_loop = parse_clip_loop(value)?;
            }
            other if other.starts_with('-') => {
                return Err(format!("unknown option {other}\n{USAGE}"))
            }
            other => positional.push(other),
        }
    }

    let (input, output) = match (command.as_str(), positional.as_slice()) {
        ("wav", [input, output]) => (*input, Some(*output)),
        ("play", [input]) => (*input, None),
        ("bake", [input, output]) => return run_bake(input, output, &options, clip_loop),
        _ => return Err(USAGE.to_string()),
    };
    if !matches!(clip_loop, ClipLoop::None) {
        return Err("--clip-loop only applies to `bake`".to_string());
    }

    let text = std::fs::read_to_string(input).map_err(|error| format!("{input}: {error}"))?;
    let rendered = render(&text, &options).map_err(|error| format!("{input}: {error}"))?;

    let mut report = format!(
        "engine=kotoaudio rate={}Hz tracks={} rendered={:.2}s{}",
        rendered.sample_rate,
        rendered.track_count,
        rendered.samples.len() as f64 / f64::from(rendered.sample_rate),
        if options.loop_playback {
            " (looped)"
        } else {
            ""
        },
    );
    for note in &rendered.notes {
        report.push('\n');
        report.push_str(note);
    }

    match output {
        Some(path) => {
            koto_sim::audio::write_wav_mono(path, rendered.sample_rate, &rendered.samples)
                .map_err(|error| format!("{path}: {error}"))?;
            Ok(format!("{report}\n-> {path}"))
        }
        None => {
            play(&rendered)?;
            Ok(report)
        }
    }
}

/// `whole` or `START..END` (samples at the device rate), always infinite:
/// the bake use case is a sustained tone or looping jingle.
fn parse_clip_loop(value: &str) -> Result<ClipLoop, String> {
    if value == "whole" {
        return Ok(ClipLoop::Whole {
            count: LoopCount::Infinite,
        });
    }
    if let Some((start, end)) = value.split_once("..") {
        let parse = |s: &str| {
            s.parse::<u32>()
                .map_err(|_| format!("--clip-loop expects sample numbers, got {s:?}"))
        };
        return Ok(ClipLoop::Forward {
            start: parse(start)?,
            end: parse(end)?,
            count: LoopCount::Infinite,
        });
    }
    Err(format!(
        "--clip-loop expects whole|START..END, got {value:?}"
    ))
}

fn run_bake(
    input: &str,
    output: &str,
    options: &Options,
    clip_loop: ClipLoop,
) -> Result<String, String> {
    let text = std::fs::read_to_string(input).map_err(|error| format!("{input}: {error}"))?;
    let baked = bake(&text, options, clip_loop).map_err(|error| format!("{input}: {error}"))?;
    std::fs::write(output, &baked.kacl).map_err(|error| format!("{output}: {error}"))?;

    let seconds = baked.sample_count as f64 / f64::from(baked.sample_rate);
    let mut report = format!(
        "engine=kotoaudio-bake rate={}Hz rendered={seconds:.2}s \
         payload={} bytes ({} KB/s) total={} bytes loop={:?}",
        baked.sample_rate,
        baked.payload_bytes,
        baked.sample_rate * 2 / 1000,
        baked.kacl.len(),
        clip_loop,
    );
    for note in &baked.notes {
        report.push('\n');
        report.push_str(note);
    }
    Ok(format!("{report}\n-> {output}"))
}

#[cfg(feature = "play")]
fn play(rendered: &Rendered) -> Result<(), String> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or("no default audio output device")?;
    let config = device
        .default_output_config()
        .map_err(|error| format!("audio output config: {error}"))?;
    let channels = usize::from(config.channels());
    let out_rate = config.sample_rate().0;

    // Linear resample of the mono render to the device rate, as f32.
    let step = f64::from(rendered.sample_rate) / f64::from(out_rate);
    let out_len = (rendered.samples.len() as f64 / step) as usize;
    let mut resampled = Vec::with_capacity(out_len);
    for index in 0..out_len {
        let pos = index as f64 * step;
        let base = pos as usize;
        let frac = (pos - base as f64) as f32;
        let a = f32::from(rendered.samples[base]);
        let b = f32::from(*rendered.samples.get(base + 1).unwrap_or(&0));
        resampled.push((a + (b - a) * frac) / f32::from(i16::MAX));
    }

    let cursor = Arc::new(AtomicUsize::new(0));
    let feed = {
        let cursor = cursor.clone();
        let resampled = resampled.clone();
        move |out: &mut [f32], _: &cpal::OutputCallbackInfo| {
            for frame in out.chunks_mut(channels) {
                let index = cursor.fetch_add(1, Ordering::Relaxed);
                let sample = resampled.get(index).copied().unwrap_or(0.0);
                frame.fill(sample);
            }
        }
    };
    let stream = device
        .build_output_stream(
            &config.into(),
            feed,
            |error| eprintln!("koto-mml: stream error: {error}"),
            None,
        )
        .map_err(|error| format!("audio stream: {error}"))?;
    stream.play().map_err(|error| format!("play: {error}"))?;
    while cursor.load(std::sync::atomic::Ordering::Relaxed) < resampled.len() {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    // Let the tail drain before tearing the stream down.
    std::thread::sleep(std::time::Duration::from_millis(100));
    Ok(())
}

#[cfg(not(feature = "play"))]
fn play(_rendered: &Rendered) -> Result<(), String> {
    Err("playback needs the opt-in audio backend; rebuild with \
         `cargo run -p koto-mml --features play -- play ...` (or use `wav`)"
        .to_string())
}
