use std::{env, fs, process::ExitCode};

use koto_audio_tools::{
    format_compact_sequence_table, mml::parse_mml_to_compact_sequence_table,
    CompactSequenceTableOptions,
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
    let mut options = CompactSequenceTableOptions::default();
    let mut positional = Vec::new();
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--symbol" => {
                options.sequence_symbol_name =
                    args.next().ok_or_else(|| usage("missing --symbol value"))?;
            }
            "--prefix" => {
                options.table_symbol_prefix =
                    args.next().ok_or_else(|| usage("missing --prefix value"))?;
            }
            _ if arg.starts_with('-') => return Err(usage("unknown option")),
            _ => positional.push(arg),
        }
    }

    if positional.len() != 2 {
        return Err(usage("expected input MML path and output Rust path"));
    }
    let input = positional.remove(0);
    let output = positional.remove(0);

    let source =
        fs::read_to_string(&input).map_err(|error| format!("failed to read {input}: {error}"))?;
    let table = parse_mml_to_compact_sequence_table(&source)
        .map_err(|error| format!("parse failed: {error:?}"))?;
    let generated = format_compact_sequence_table(&table, options)
        .map_err(|error| format!("generation failed: {error:?}"))?;
    fs::write(&output, &generated.rust_fragment)
        .map_err(|error| format!("failed to write {output}: {error}"))?;

    Ok(format!(
        concat!(
            "KotoAudio MML compact sequence generation report\n",
            "input file: {}\n",
            "output file: {}\n",
            "instrument count: {}\n",
            "track count: {}\n",
            "event counts: {:?}"
        ),
        input, output, generated.instrument_count, generated.track_count, generated.event_counts
    ))
}

fn usage(reason: &str) -> String {
    format!(
        concat!(
            "usage: koto-audio-mml-table [--symbol NAME] [--prefix NAME] ",
            "<input.mml> <output.rs>\n",
            "error: {}"
        ),
        reason
    )
}
