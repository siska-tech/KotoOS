use std::{env, fs, process::ExitCode};

use koto_audio_tools::{generate_drum_table_from_wav, DrumTableOptions};

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
    let mut options = DrumTableOptions::default();
    let mut positional = Vec::new();
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--symbol" => {
                options.symbol_name = args.next().ok_or_else(|| usage("missing --symbol value"))?;
            }
            "--target-rate" => {
                let rate = args
                    .next()
                    .ok_or_else(|| usage("missing --target-rate value"))?;
                options.target_sample_rate_hz = parse_target_rate(&rate)?;
            }
            "--strict-input" | "--no-resample" => {
                options.strict_input = true;
            }
            _ if arg.starts_with('-') => return Err(usage("unknown option")),
            _ => positional.push(arg),
        }
    }

    if positional.len() != 2 {
        return Err(usage("expected input WAV path and output Rust path"));
    }
    let input = positional.remove(0);
    let output = positional.remove(0);

    let wav = fs::read(&input).map_err(|error| format!("failed to read {input}: {error}"))?;
    let generated = generate_drum_table_from_wav(&wav, options)
        .map_err(|error| format!("generation failed: {error:?}"))?;
    fs::write(&output, &generated.rust_fragment)
        .map_err(|error| format!("failed to write {output}: {error}"))?;

    Ok(format!(
        concat!(
            "KotoAudio drum table generation report\n",
            "input file: {}\n",
            "output file: {}\n",
            "sample rate: {} Hz\n",
            "sample count: {}\n",
            "downmix applied: {}\n",
            "resample applied: {}"
        ),
        input,
        output,
        generated.sample_rate_hz,
        generated.sample_count,
        generated.downmix_applied,
        generated.resample_applied
    ))
}

fn usage(reason: &str) -> String {
    format!(
        concat!(
            "usage: koto-audio-drum-table [--symbol NAME] [--target-rate hz] ",
            "[--strict-input|--no-resample] <input.wav> <output.rs>\n",
            "error: {}"
        ),
        reason
    )
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
