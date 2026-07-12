use std::path::PathBuf;
use std::process::ExitCode;

use koto_compiler::{
    compile_to_asm_with_options, compile_with_options, describe_slot_map, slot_map, CodegenOptions,
};

fn main() -> ExitCode {
    let args = match CliArgs::parse(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(error) => {
            eprintln!("{error}");
            eprintln!(
                "usage: koto-compiler SOURCE.koto OUTPUT.kbc [--emit-asm]\n\
                 \x20      koto-compiler SOURCE.koto --slot-map"
            );
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
    let file = args.source.display().to_string();

    if args.slot_map {
        return match slot_map(&file, &source) {
            Ok(map) => {
                println!("{}", describe_slot_map(&map));
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        };
    }

    let output = match &args.output {
        Some(output) => output.clone(),
        None => {
            eprintln!("expected an OUTPUT path");
            return ExitCode::FAILURE;
        }
    };

    if args.emit_asm {
        match compile_to_asm_with_options(&file, &source, args.options) {
            Ok(asm) => match std::fs::write(&output, asm.as_bytes()) {
                Ok(()) => {
                    println!("compiled {} -> {} (assembly)", file, output.display());
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("failed to write {}: {error}", output.display());
                    ExitCode::FAILURE
                }
            },
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        }
    } else {
        match compile_with_options(&file, &source, args.options) {
            Ok(bytecode) => match std::fs::write(&output, &bytecode) {
                Ok(()) => {
                    println!(
                        "compiled {} -> {} ({} bytes)",
                        file,
                        output.display(),
                        bytecode.len()
                    );
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("failed to write {}: {error}", output.display());
                    ExitCode::FAILURE
                }
            },
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        }
    }
}

struct CliArgs {
    source: PathBuf,
    output: Option<PathBuf>,
    emit_asm: bool,
    slot_map: bool,
    options: CodegenOptions,
}

impl CliArgs {
    fn parse(mut args: impl Iterator<Item = String>) -> Result<Self, String> {
        let mut emit_asm = false;
        let mut slot_map = false;
        let mut options = CodegenOptions::default();
        let mut positional = Vec::new();
        for arg in args.by_ref() {
            match arg.as_str() {
                "--emit-asm" => emit_asm = true,
                "--slot-map" => slot_map = true,
                // KOTO-0156 per-app code-window layout opt-ins (apps.json `codegen`).
                "--relocate-preamble" => options.relocate_preamble = true,
                "--outline-cold-blocks" => options.outline_cold_blocks = true,
                // KOTO-0169 Stage 4 per-app opt-OUT: pin the pre-Stage-4
                // boolean/comparison templates for apps whose code-window
                // tile layout regresses under the smaller default codegen.
                "--legacy-compare-templates" => options.legacy_compare_templates = true,
                other if other.starts_with("--") => {
                    return Err(format!("unknown argument: {other}"))
                }
                _ => positional.push(arg),
            }
        }
        if slot_map {
            // `--slot-map` only reads the source; it prints to stdout, no OUTPUT.
            if emit_asm {
                return Err("--slot-map cannot be combined with --emit-asm".to_string());
            }
            if positional.len() != 1 {
                return Err("expected a SOURCE path".to_string());
            }
            return Ok(Self {
                source: PathBuf::from(&positional[0]),
                output: None,
                emit_asm,
                slot_map,
                options,
            });
        }
        if positional.len() != 2 {
            return Err("expected SOURCE and OUTPUT paths".to_string());
        }
        Ok(Self {
            source: PathBuf::from(&positional[0]),
            output: Some(PathBuf::from(&positional[1])),
            emit_asm,
            slot_map,
            options,
        })
    }
}
