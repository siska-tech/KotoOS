use std::{env, fs, process::ExitCode};

use koto_audio_tools::{generate_sldpcm4_drum_table_module, parse_pcm16_drum_table_module};

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
    let mut positional = Vec::new();
    for arg in env::args().skip(1) {
        if arg.starts_with('-') {
            return Err(usage("unknown option"));
        }
        positional.push(arg);
    }
    if positional.len() != 2 {
        return Err(usage(
            "expected input PCM16 drum-table Rust path and output Rust path",
        ));
    }
    let input = positional.remove(0);
    let output = positional.remove(0);

    let source =
        fs::read_to_string(&input).map_err(|error| format!("failed to read {input}: {error}"))?;
    let tables = parse_pcm16_drum_table_module(&source)
        .map_err(|error| format!("parse failed: {error:?}"))?;
    let generated = generate_sldpcm4_drum_table_module(&tables);
    fs::write(&output, &generated.rust_module)
        .map_err(|error| format!("failed to write {output}: {error}"))?;

    let mut report = String::from("KotoAudio SLDPCM4 drum table generation report\n");
    report.push_str(&format!("input file: {input}\noutput file: {output}\n"));
    let mut pcm16_bytes = 0u64;
    let mut payload_bytes = 0u64;
    for drum in &generated.reports {
        pcm16_bytes += u64::from(drum.sample_count) * 2;
        payload_bytes += u64::from(drum.payload_bytes);
        report.push_str(&format!(
            "{}: samples={} payload_bytes={} saturations={}\n",
            drum.symbol, drum.sample_count, drum.payload_bytes, drum.saturation_count
        ));
    }
    report.push_str(&format!(
        "total: pcm16_bytes={pcm16_bytes} sldpcm4_bytes={payload_bytes}"
    ));
    Ok(report)
}

fn usage(reason: &str) -> String {
    format!(
        concat!(
            "usage: koto-audio-drum-sldpcm4-table <builtin_drums_generated.rs> ",
            "<builtin_drums_sldpcm4_generated.rs>\n",
            "error: {}"
        ),
        reason
    )
}
