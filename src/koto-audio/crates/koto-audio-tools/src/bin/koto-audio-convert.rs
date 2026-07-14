use std::{env, fs, process::ExitCode};

use koto_audio::{ClipLoop, LoopCount};
use koto_audio_tools::{
    convert_wav_to_clip_asset, ConvertOptions, OutputCodec, Sldpcm4FallbackPolicy,
};

fn main() -> ExitCode {
    match run() {
        Ok(report) => {
            println!("{report}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<String, String> {
    let mut options = ConvertOptions::default();
    let mut positional = Vec::new();
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--codec" => {
                let codec = args.next().ok_or_else(|| usage("missing --codec value"))?;
                options.output_codec = parse_codec(&codec)?;
            }
            "--sldpcm4-fallback" => {
                let fallback = args
                    .next()
                    .ok_or_else(|| usage("missing --sldpcm4-fallback value"))?;
                let fallback = parse_sldpcm4_fallback(&fallback)?;
                options.output_codec = OutputCodec::ExperimentalSldpcm4 { fallback };
            }
            "--target-rate" => {
                let rate = args
                    .next()
                    .ok_or_else(|| usage("missing --target-rate value"))?;
                let rate = parse_target_rate(&rate)?;
                options.target_sample_rate_hz = rate;
                options.limits.sample_rate_hz = rate;
            }
            "--strict-input" | "--no-resample" => {
                options.strict_input = true;
            }
            "--max-samples" => {
                let samples = args
                    .next()
                    .ok_or_else(|| usage("missing --max-samples value"))?;
                options.max_output_samples = Some(parse_max_samples(&samples)?);
            }
            "--loop" => {
                options.loop_metadata = ClipLoop::Whole {
                    count: LoopCount::Infinite,
                };
            }
            _ if arg.starts_with('-') => return Err(usage("unknown option")),
            _ => positional.push(arg),
        }
    }

    if positional.len() != 2 {
        return Err(usage("expected input WAV path and output asset path"));
    }
    let input = positional.remove(0);
    let output = positional.remove(0);

    let wav = fs::read(&input).map_err(|error| format!("failed to read {input}: {error}"))?;
    let converted = convert_wav_to_clip_asset(&input, &wav, options)
        .map_err(|error| format!("conversion failed: {error:?}"))?;
    fs::write(&output, &converted.asset_bytes)
        .map_err(|error| format!("failed to write {output}: {error}"))?;

    Ok(converted.report.to_human_readable())
}

fn usage(reason: &str) -> String {
    format!(
        concat!(
            "usage: koto-audio-convert [--codec pcm16|experimental-sldpcm4] ",
            "[--sldpcm4-fallback pcm16|reject|force] [--target-rate hz] ",
            "[--max-samples count] [--loop] ",
            "[--strict-input|--no-resample] <input.wav> <output.kacl>\n",
            "error: {}"
        ),
        reason
    )
}

fn parse_codec(value: &str) -> Result<OutputCodec, String> {
    match value {
        "pcm16" => Ok(OutputCodec::Pcm16),
        "experimental-sldpcm4" | "sldpcm4" => Ok(OutputCodec::ExperimentalSldpcm4 {
            fallback: Sldpcm4FallbackPolicy::Pcm16,
        }),
        _ => Err(usage("unknown codec")),
    }
}

fn parse_sldpcm4_fallback(value: &str) -> Result<Sldpcm4FallbackPolicy, String> {
    match value {
        "pcm16" => Ok(Sldpcm4FallbackPolicy::Pcm16),
        "reject" => Ok(Sldpcm4FallbackPolicy::Reject),
        "force" => Ok(Sldpcm4FallbackPolicy::ForceExperimental),
        _ => Err(usage("unknown SLDPCM4 fallback policy")),
    }
}

fn parse_target_rate(value: &str) -> Result<u32, String> {
    let rate = value
        .parse::<u32>()
        .map_err(|_| usage("invalid --target-rate value"))?;
    if rate == 0 {
        return Err(usage("--target-rate must be greater than zero"));
    }
    Ok(rate)
}

fn parse_max_samples(value: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .ok()
        .filter(|count| *count > 0)
        .ok_or_else(|| usage("--max-samples must be greater than zero"))
}
