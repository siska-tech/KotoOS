use std::path::PathBuf;
use std::process::ExitCode;

use kpa_packer::{pack_manifest, PackOptions};

fn main() -> ExitCode {
    let args = match CliArgs::parse(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(error) => {
            eprintln!("{error}");
            eprintln!(
                "usage: kpa-packer --manifest PATH --assets-root DIR [--out PATH] [--dry-run]"
            );
            return ExitCode::FAILURE;
        }
    };

    let manifest_bytes = match std::fs::read(&args.manifest) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!(
                "failed to read manifest {}: {error}",
                args.manifest.display()
            );
            return ExitCode::FAILURE;
        }
    };

    let package = match pack_manifest(
        &manifest_bytes,
        PackOptions {
            assets_root: args.assets_root,
        },
    ) {
        Ok(package) => package,
        Err(error) => {
            eprintln!("failed to pack manifest: {error}");
            return ExitCode::FAILURE;
        }
    };

    if args.dry_run {
        print!("{}", package.layout_report());
    }

    if let Some(out) = args.out {
        if let Err(error) = std::fs::write(&out, package.bytes()) {
            eprintln!("failed to write package {}: {error}", out.display());
            return ExitCode::FAILURE;
        }
        println!("wrote {} bytes -> {}", package.bytes().len(), out.display());
    }

    ExitCode::SUCCESS
}

struct CliArgs {
    manifest: PathBuf,
    assets_root: PathBuf,
    out: Option<PathBuf>,
    dry_run: bool,
}

impl CliArgs {
    fn parse(mut args: impl Iterator<Item = String>) -> Result<Self, String> {
        let mut manifest = None;
        let mut assets_root = None;
        let mut out = None;
        let mut dry_run = false;

        while let Some(flag) = args.next() {
            match flag.as_str() {
                "--manifest" => manifest = Some(Self::path_value(&mut args, "--manifest")?),
                "--assets-root" => {
                    assets_root = Some(Self::path_value(&mut args, "--assets-root")?)
                }
                "--out" => out = Some(Self::path_value(&mut args, "--out")?),
                "--dry-run" => dry_run = true,
                other => return Err(format!("unknown argument: {other}")),
            }
        }

        Ok(Self {
            manifest: manifest.ok_or_else(|| "--manifest is required".to_string())?,
            assets_root: assets_root.ok_or_else(|| "--assets-root is required".to_string())?,
            out,
            dry_run,
        })
    }

    fn path_value(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<PathBuf, String> {
        args.next()
            .map(PathBuf::from)
            .ok_or_else(|| format!("{flag} requires a path"))
    }
}
