use std::{env, fs, process::ExitCode};

use koto_audio_tools::decode_clip_asset_to_wav;

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
    let positional: Vec<String> = env::args().skip(1).collect();
    if positional.len() != 2 {
        return Err(usage("expected input KACL path and output WAV path"));
    }

    let input = &positional[0];
    let output = &positional[1];
    let asset = fs::read(input).map_err(|error| format!("failed to read {input}: {error}"))?;
    let decoded = decode_clip_asset_to_wav(input, &asset)
        .map_err(|error| format!("decode failed: {error:?}"))?;
    fs::write(output, &decoded.wav_bytes)
        .map_err(|error| format!("failed to write {output}: {error}"))?;

    Ok(decoded.report.to_human_readable())
}

fn usage(reason: &str) -> String {
    format!(
        concat!(
            "usage: koto-audio-decode <input.kacl> <output.wav>\n",
            "error: {}"
        ),
        reason
    )
}
