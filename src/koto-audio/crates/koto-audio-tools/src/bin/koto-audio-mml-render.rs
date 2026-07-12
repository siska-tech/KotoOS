use std::{env, fs, process::ExitCode};

use koto_audio_tools::{render_mml_to_wav, MmlRenderOptions};

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
    let mut options = MmlRenderOptions::default();
    let mut positional = Vec::new();
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--seconds" => {
                let value = args
                    .next()
                    .ok_or_else(|| usage("missing --seconds value"))?;
                options.seconds = Some(
                    value
                        .parse::<u32>()
                        .map_err(|_| usage("invalid --seconds value"))?,
                );
            }
            "--sample-rate" => {
                let value = args
                    .next()
                    .ok_or_else(|| usage("missing --sample-rate value"))?;
                options.sample_rate_hz = value
                    .parse::<u32>()
                    .map_err(|_| usage("invalid --sample-rate value"))?;
            }
            "--bgm" => options.bgm = true,
            _ if arg.starts_with('-') => return Err(usage("unknown option")),
            _ => positional.push(arg),
        }
    }

    if positional.len() != 2 {
        return Err(usage("expected input MML path and output WAV path"));
    }
    let input = positional.remove(0);
    let output = positional.remove(0);

    let source =
        fs::read_to_string(&input).map_err(|error| format!("failed to read {input}: {error}"))?;
    let rendered = render_mml_to_wav(&input, &source, options)
        .map_err(|error| format!("render failed: {error:?}"))?;
    fs::write(&output, &rendered.wav_bytes)
        .map_err(|error| format!("failed to write {output}: {error}"))?;

    Ok(rendered.report.to_human_readable())
}

fn usage(reason: &str) -> String {
    format!(
        concat!(
            "usage: koto-audio-mml-render [--seconds N] [--sample-rate HZ] [--bgm] ",
            "<input.mml> <output.wav>\n",
            "supported sample rates: 16000, 22050, 44100\n",
            "error: {}"
        ),
        reason
    )
}
