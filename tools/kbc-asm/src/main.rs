use std::path::PathBuf;
use std::process::ExitCode;

use kbc_asm::assemble;

fn main() -> ExitCode {
    let args = match CliArgs::parse(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(error) => {
            eprintln!("{error}");
            eprintln!("usage: kbc-asm SOURCE.asm OUTPUT.kbc");
            eprintln!("       kbc-asm --check SOURCE.asm EXPECTED.kbc");
            return ExitCode::FAILURE;
        }
    };

    let source = match std::fs::read_to_string(&args.source) {
        Ok(source) => source,
        Err(error) => {
            eprintln!("failed to read {}: {error}", args.source.display());
            return ExitCode::FAILURE;
        }
    };

    let bytecode = match assemble(&source) {
        Ok(bytecode) => bytecode,
        Err(error) => {
            eprintln!("{}: {error}", args.source.display());
            return ExitCode::FAILURE;
        }
    };

    if args.check {
        match std::fs::read(&args.output) {
            Ok(committed) if committed == bytecode => {
                println!(
                    "{} matches {} ({} bytes)",
                    args.output.display(),
                    args.source.display(),
                    bytecode.len()
                );
                ExitCode::SUCCESS
            }
            Ok(committed) => {
                eprintln!(
                    "{} is stale: {} assembles to {} bytes but the committed file is {} bytes; \
                     regenerate with `kbc-asm {} {}`",
                    args.output.display(),
                    args.source.display(),
                    bytecode.len(),
                    committed.len(),
                    args.source.display(),
                    args.output.display(),
                );
                ExitCode::FAILURE
            }
            Err(error) => {
                eprintln!("failed to read {}: {error}", args.output.display());
                ExitCode::FAILURE
            }
        }
    } else {
        match std::fs::write(&args.output, &bytecode) {
            Ok(()) => {
                println!(
                    "assembled {} -> {} ({} bytes)",
                    args.source.display(),
                    args.output.display(),
                    bytecode.len()
                );
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("failed to write {}: {error}", args.output.display());
                ExitCode::FAILURE
            }
        }
    }
}

struct CliArgs {
    source: PathBuf,
    output: PathBuf,
    check: bool,
}

impl CliArgs {
    fn parse(mut args: impl Iterator<Item = String>) -> Result<Self, String> {
        let mut check = false;
        let mut positional = Vec::new();
        for arg in args.by_ref() {
            match arg.as_str() {
                "--check" => check = true,
                other if other.starts_with("--") => {
                    return Err(format!("unknown argument: {other}"))
                }
                _ => positional.push(arg),
            }
        }
        if positional.len() != 2 {
            return Err("expected SOURCE and OUTPUT paths".to_string());
        }
        Ok(Self {
            source: PathBuf::from(&positional[0]),
            output: PathBuf::from(&positional[1]),
            check,
        })
    }
}
