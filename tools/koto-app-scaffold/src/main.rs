use std::path::PathBuf;
use std::process::ExitCode;

use koto_app_scaffold::{scaffold_app, ScaffoldOptions};

fn main() -> ExitCode {
    let args = match CliArgs::parse(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(error) => {
            eprintln!("{error}");
            eprintln!(
                "usage: koto-app-scaffold --app-id APP_ID --name NAME [--dir apps/name] [--root DIR]"
            );
            return ExitCode::FAILURE;
        }
    };

    match scaffold_app(ScaffoldOptions {
        root: args.root,
        app_id: args.app_id,
        name: args.name,
        app_dir: args.app_dir,
    }) {
        Ok(result) => {
            println!("created app {}", result.app_id);
            println!("  descriptor: {}", result.descriptor.display());
            println!("  source: {}", result.source.display());
            println!("  helpers: {}", result.helpers.display());
            println!("  icon: {}", result.icon.display());
            println!("  scenario: {}", result.scenario.display());
            println!("build with: python harness\\build_apps.py");
            println!(
                "run with: cargo run -p koto-sim -- --app {} --app-script {}",
                result.app_id,
                result.scenario.display()
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("failed to scaffold app: {error}");
            ExitCode::FAILURE
        }
    }
}

struct CliArgs {
    root: PathBuf,
    app_id: String,
    name: String,
    app_dir: Option<PathBuf>,
}

impl CliArgs {
    fn parse(mut args: impl Iterator<Item = String>) -> Result<Self, String> {
        let mut root = PathBuf::from(".");
        let mut app_id = None;
        let mut name = None;
        let mut app_dir = None;

        while let Some(flag) = args.next() {
            match flag.as_str() {
                "--root" => root = Self::path_value(&mut args, "--root")?,
                "--app-id" => app_id = Some(Self::value(&mut args, "--app-id")?),
                "--name" => name = Some(Self::value(&mut args, "--name")?),
                "--dir" => app_dir = Some(Self::path_value(&mut args, "--dir")?),
                other => return Err(format!("unknown argument: {other}")),
            }
        }

        Ok(Self {
            root,
            app_id: app_id.ok_or_else(|| "--app-id is required".to_string())?,
            name: name.ok_or_else(|| "--name is required".to_string())?,
            app_dir,
        })
    }

    fn value(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
        args.next()
            .ok_or_else(|| format!("{flag} requires a value"))
    }

    fn path_value(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<PathBuf, String> {
        Self::value(args, flag).map(PathBuf::from)
    }
}
