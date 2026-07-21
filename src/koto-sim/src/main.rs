use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use koto_core::shell::{MemoryStatus, SaveStatus, ShellClock, StorageStatus, SHELL_SURFACE};
use koto_core::{BitmapFont, BootSplash, Buttons, InputState, PowerState, ShellAction, ShellState};
use koto_sim::{
    audio::write_wav_mono, capture_app_audio, describe_app_budget_report,
    describe_app_scenario_report, describe_host_input, describe_inspector_report,
    describe_launch_report, describe_memo_validation_report, describe_render_command,
    describe_save_data_clear_report, describe_save_data_namespace, describe_shell_action,
    golden_frame_trace, launch_package, list_save_data, load_font_bytes, load_packages,
    load_system_config, parse_app_script, parse_input_script, render_app_frame,
    render_splash_frame, run_app_scenario, run_memo_validation, run_shell_script, write_bmp,
    Framebuffer, RenderRecorder, UiGallery,
};

const DEFAULT_FONT: &str = "assets/fonts/mplus12.kfont";

/// Representative memory snapshot for the sim system view (KOTO-0182). Values
/// echo hardware captures (KOTO-0170 `free_min`, KOTO-0172 stack peak, the 8 MiB
/// PSRAM with a 2-slot code window) so the overlay looks like the real thing.
const SIM_MEMORY_STATUS: MemoryStatus = MemoryStatus {
    sram_total: 264 * 1024,
    sram_static_used: Some(184 * 1024),
    sram_free_min: Some(7620),
    stack_peak_used: Some(26_616),
    core1_stack_free_min: Some(3_912),
    app_heap_total: Some(64 * 1024),
    app_heap_last_used: Some(41 * 1024),
    psram_total: 8 * 1024 * 1024,
    psram_present: true,
    code_window_slots: 2,
};

fn main() -> ExitCode {
    let args = match CliArgs::parse(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(error) => {
            eprintln!("{error}");
            eprintln!(
                "usage: koto-sim [--script PATH] [--image PATH] [--font PATH] [--window] [--memo-validation]\n\
                 \x20               [--app APP_ID] [--app-script PATH] [--inspect] [--budget] [--audio PATH]\n\
                 \x20               [--watch DIR] [--watch-replay PATH]\n\
                 \x20               [--fake-network PATH]\n\
                 \x20               [--golden-frames] [--ui-gallery] [--splash] [--system-view]\n\
                 \x20               [--save-list] [--save-clear APP_ID]\n\
                 \x20               [--battery PERCENT] [--charging] [--voltage MV] [--power-unknown]"
            );
            return ExitCode::FAILURE;
        }
    };

    if args.save_list {
        return list_save_data_cli();
    }

    if let Some(app_id) = &args.save_clear {
        return clear_save_data_cli(app_id);
    }

    if args.memo_validation {
        return run_memo_validation_cli();
    }

    let fake_network_json = match args.fake_network.as_deref() {
        Some(path) => match std::fs::read_to_string(path) {
            Ok(json) => match koto_sim::fake_network::replay_trace(&json) {
                Ok(trace) => {
                    println!(
                        "fake NetworkService: replayed {} snapshot boundaries from {path}",
                        trace.len()
                    );
                    Some(json)
                }
                Err(error) => {
                    eprintln!("invalid fake-network fixture {path}: {error}");
                    return ExitCode::FAILURE;
                }
            },
            Err(error) => {
                eprintln!("failed to read fake-network fixture {path}: {error}");
                return ExitCode::FAILURE;
            }
        },
        None => None,
    };

    // Headless fake replay is a focused CI/development mode. In window mode
    // the checked fixture stays attached to the native KotoConfig launch path.
    if fake_network_json.is_some() && !args.window {
        return ExitCode::SUCCESS;
    }

    if args.golden_frames {
        return golden_frames_cli();
    }

    if args.ui_gallery {
        return ui_gallery_cli(&args.font, args.image.as_deref(), args.window);
    }

    if args.splash {
        let Some(image_path) = &args.image else {
            eprintln!("--splash requires --image PATH");
            return ExitCode::FAILURE;
        };
        return render_splash_cli(&args.font, image_path);
    }

    // `--window --app` launches the app directly inside window mode (with the
    // optional KOTO-0191 watch loop); only headless `--app` runs dispatch here.
    if !args.window {
        if let Some(app_id) = &args.app {
            if let Some(audio_path) = &args.audio {
                return run_app_audio_cli(app_id, args.app_script.as_deref(), audio_path);
            }
            if let Some(image_path) = &args.image {
                return run_app_image_cli(
                    app_id,
                    args.app_script.as_deref(),
                    &args.font,
                    image_path,
                );
            }
            return run_app_cli(
                app_id,
                args.app_script.as_deref(),
                args.inspect,
                args.budget,
            );
        }
    }

    let packages = match load_packages("sdcard_mock") {
        Ok(packages) => packages,
        Err(error) => {
            eprintln!("failed to load packages from sdcard_mock: {error:?}");
            return ExitCode::FAILURE;
        }
    };
    let mut shell = ShellState::new(packages);
    shell.apply_config_snapshot(load_system_config("sdcard_mock").snapshot());
    if let Some(power_state) = args.power_state {
        shell.set_power_state(power_state);
    }
    // Deterministic header indicators for the demo/simulator views.
    shell.set_clock(ShellClock {
        year: 2025,
        month: 5,
        day: 18,
        hour: 10,
        minute: 42,
    });
    shell.set_storage_status(StorageStatus::Present);
    shell.set_save_status(SaveStatus::Saved);
    // Representative device-typical figures for the F5 system view (KOTO-0182).
    // The sim has no live SRAM/PSRAM to measure, so this mirrors the shape of a
    // real capture (the free_min/stack/PSRAM values seen on hardware) so the
    // overlay is inspectable and golden-testable without a device.
    shell.set_memory_status(SIM_MEMORY_STATUS);
    if args.system_view {
        shell.set_system_view_visible(true);
    }
    let mut recorder = RenderRecorder::new();

    println!("KotoSim text harness");
    if let Some(status) = shell.status_text().as_str() {
        println!("battery status: {status}");
    }
    if let Err(error) = recorder.record_shell_list(&shell) {
        eprintln!("failed to record shell redraw: {error:?}");
        return ExitCode::FAILURE;
    }
    let list_command_count = recorder.commands().len();
    println!("packages:");
    for (index, package) in shell.packages().iter().enumerate() {
        let marker = if index == shell.selected_index() {
            ">"
        } else {
            " "
        };
        println!("{marker} {} ({})", package.name(), package.app_id());
    }

    println!("render commands:");
    for command in recorder.commands() {
        println!("- {}", describe_render_command(command));
    }

    if args.window {
        return run_window_mode(
            shell,
            &args.font,
            args.app.as_deref(),
            args.watch.as_deref(),
            args.watch_replay.as_deref(),
            fake_network_json.as_deref(),
        );
    }

    if let Some(path) = &args.script {
        let script = match std::fs::read_to_string(path) {
            Ok(script) => script,
            Err(error) => {
                eprintln!("failed to read input script {path}: {error}");
                return ExitCode::FAILURE;
            }
        };
        let inputs = match parse_input_script(&script) {
            Ok(inputs) => inputs,
            Err(error) => {
                eprintln!("failed to parse input script {path}: {error:?}");
                return ExitCode::FAILURE;
            }
        };

        println!("script actions:");
        for event in run_shell_script(&mut shell, &inputs) {
            println!(
                "- input {} -> {} (selected={})",
                describe_host_input(event.input),
                describe_shell_action(event.action),
                event.selected_index
            );
            if let ShellAction::Launch(package) = event.action {
                print_launch_result(&package);
            }
        }
    } else {
        let previous_selected = shell.selected_index();
        shell.update(&InputState {
            pressed: Buttons {
                down: true,
                ..Buttons::default()
            },
            ..InputState::default()
        });
        if let Err(error) = recorder.record_shell_selection_change(&shell, previous_selected) {
            eprintln!("failed to record shell selection redraw: {error:?}");
            return ExitCode::FAILURE;
        }
        for command in &recorder.commands()[list_command_count..] {
            println!("- {}", describe_render_command(command));
        }

        let action = shell.update(&InputState {
            pressed: Buttons {
                confirm: true,
                ..Buttons::default()
            },
            ..InputState::default()
        });

        match action {
            ShellAction::None => println!("no action"),
            ShellAction::Launch(package) => {
                println!(
                    "launch requested: {} ({})",
                    package.name(),
                    package.app_id()
                );
                print_launch_result(&package);
            }
            ShellAction::OpenConfig => println!("open KotoConfig"),
        }
    }

    if let Some(image_path) = &args.image {
        // The demo input above drives a confirm press, which dismisses the
        // overlay; re-assert it so `--system-view --image` screenshots it.
        if args.system_view {
            shell.set_system_view_visible(true);
        }
        if let Err(code) = render_image(&shell, &args.font, image_path) {
            return code;
        }
    }

    ExitCode::SUCCESS
}

fn print_launch_result(package: &koto_core::PackageInfo) {
    match launch_package("sdcard_mock", package) {
        Ok(report) => println!("- {}", describe_launch_report(&report)),
        Err(error) => println!("- runtime launch failed: {error:?}"),
    }
}

fn run_memo_validation_cli() -> ExitCode {
    let root = memo_validation_root();
    if let Err(error) = copy_dir_all(Path::new("sdcard_mock"), &root) {
        eprintln!("failed to prepare memo validation root: {error}");
        return ExitCode::FAILURE;
    }

    match run_memo_validation(&root) {
        Ok(report) => {
            println!("{}", describe_memo_validation_report(&report));
            if let Err(error) = std::fs::remove_dir_all(&root) {
                eprintln!("warning: failed to remove {}: {error}", root.display());
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("memo validation failed: {error:?}");
            let _ = std::fs::remove_dir_all(&root);
            ExitCode::FAILURE
        }
    }
}

/// Render the boot splash (all steps ok, KOTO-0181) to a BMP so docs and
/// screenshots match the device's boot moment.
fn render_splash_cli(font_path: &str, image_path: &str) -> ExitCode {
    let font_bytes = match load_font_bytes(font_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("failed to read font {font_path}: {error:?}");
            return ExitCode::FAILURE;
        }
    };
    let font = match BitmapFont::from_bytes(&font_bytes) {
        Ok(font) => font,
        Err(error) => {
            eprintln!("invalid font {font_path}: {error:?}");
            return ExitCode::FAILURE;
        }
    };
    let framebuffer = render_splash_frame(&BootSplash::complete(), &font);
    if let Err(error) = write_bmp(image_path, &framebuffer) {
        eprintln!("failed to write image {image_path}: {error:?}");
        return ExitCode::FAILURE;
    }
    println!(
        "wrote {}x{} splash frame -> {image_path}",
        framebuffer.width(),
        framebuffer.height()
    );
    ExitCode::SUCCESS
}

fn golden_frames_cli() -> ExitCode {
    match golden_frame_trace("sdcard_mock") {
        Ok(trace) => {
            print!("{trace}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("golden frame trace failed: {error:?}");
            ExitCode::FAILURE
        }
    }
}

fn ui_gallery_cli(font_path: &str, image: Option<&str>, window: bool) -> ExitCode {
    let font_bytes = match load_font_bytes(font_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("failed to load font: {error:?}");
            return ExitCode::FAILURE;
        }
    };
    if window {
        #[cfg(feature = "window")]
        return match koto_sim::window::run_ui_gallery(&font_bytes) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("gallery window failed: {error:?}");
                ExitCode::FAILURE
            }
        };
        #[cfg(not(feature = "window"))]
        {
            eprintln!("--window requires cargo run -p koto-sim --features window");
            return ExitCode::FAILURE;
        }
    }
    let font = match BitmapFont::from_bytes(&font_bytes) {
        Ok(font) => font,
        Err(error) => {
            eprintln!("invalid font: {error:?}");
            return ExitCode::FAILURE;
        }
    };
    let mut gallery = UiGallery::new();
    let framebuffer = gallery.render(&font);
    if let Some(path) = image {
        if let Err(error) = write_bmp(path, &framebuffer) {
            eprintln!("failed to write gallery image: {error:?}");
            return ExitCode::FAILURE;
        }
        println!("wrote KotoUI gallery to {path}");
    } else {
        println!("KotoUI gallery ready; add --image PATH or --window");
    }
    ExitCode::SUCCESS
}

fn memo_validation_root() -> PathBuf {
    sim_temp_root("koto-memo-validation")
}

fn list_save_data_cli() -> ExitCode {
    match list_save_data("sdcard_mock") {
        Ok(namespaces) => {
            if namespaces.is_empty() {
                println!("save-data empty");
            } else {
                for namespace in namespaces {
                    println!("{}", describe_save_data_namespace(&namespace));
                }
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("failed to list save data: {error:?}");
            ExitCode::FAILURE
        }
    }
}

fn clear_save_data_cli(app_id: &str) -> ExitCode {
    match koto_sim::clear_save_data("sdcard_mock", app_id) {
        Ok(report) => {
            println!("{}", describe_save_data_clear_report(&report));
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("failed to clear save data for {app_id}: {error:?}");
            ExitCode::FAILURE
        }
    }
}

/// Launch a single app by ID directly (no shell navigation), optionally driving
/// it with a per-frame input script, and print a scenario report or a diagnostic.
/// With `inspect`, also print the runtime inspector snapshot from the final frame.
fn run_app_cli(app_id: &str, script: Option<&str>, inspect: bool, budget: bool) -> ExitCode {
    let inputs = match script {
        Some(path) => match std::fs::read_to_string(path) {
            Ok(text) => match parse_app_script(&text) {
                Ok(inputs) => inputs,
                Err(error) => {
                    eprintln!("invalid app script {path}: {error:?}");
                    return ExitCode::FAILURE;
                }
            },
            Err(error) => {
                eprintln!("failed to read app script {path}: {error}");
                return ExitCode::FAILURE;
            }
        },
        None => Vec::new(),
    };

    // Run against a throwaway copy so app saves do not touch the repo tree.
    let root = sim_temp_root("koto-app");
    if let Err(error) = copy_dir_all(Path::new("sdcard_mock"), &root) {
        eprintln!("failed to prepare app root: {error}");
        return ExitCode::FAILURE;
    }
    let outcome = run_app_scenario(&root, app_id, &inputs);
    let _ = std::fs::remove_dir_all(&root);
    match outcome {
        Ok(report) => {
            println!("{}", describe_app_scenario_report(&report));
            if inspect {
                println!("{}", describe_inspector_report(&report.inspector));
            }
            if budget {
                println!("{}", describe_app_budget_report(&report.budget));
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{}", error.describe());
            ExitCode::FAILURE
        }
    }
}

/// Run an app (scripted or to idle/exit) and write its host audio timeline to a
/// mono 16-bit WAV. Deterministic capture — no device required — so scripted runs
/// can assert sound output.
fn run_app_audio_cli(app_id: &str, script: Option<&str>, audio_path: &str) -> ExitCode {
    let inputs = match script {
        Some(path) => match std::fs::read_to_string(path) {
            Ok(text) => match parse_app_script(&text) {
                Ok(inputs) => inputs,
                Err(error) => {
                    eprintln!("invalid app script {path}: {error:?}");
                    return ExitCode::FAILURE;
                }
            },
            Err(error) => {
                eprintln!("failed to read app script {path}: {error}");
                return ExitCode::FAILURE;
            }
        },
        None => Vec::new(),
    };

    let root = sim_temp_root("koto-audio");
    if let Err(error) = copy_dir_all(Path::new("sdcard_mock"), &root) {
        eprintln!("failed to prepare app root: {error}");
        return ExitCode::FAILURE;
    }
    let outcome = capture_app_audio(&root, app_id, &inputs);
    let _ = std::fs::remove_dir_all(&root);
    match outcome {
        Ok((sample_rate, samples)) => {
            if let Err(error) = write_wav_mono(audio_path, sample_rate, &samples) {
                eprintln!("failed to write audio {audio_path}: {error}");
                return ExitCode::FAILURE;
            }
            let nonzero = samples.iter().filter(|&&s| s != 0).count();
            println!(
                "audio captured: {audio_path} ({} samples @ {sample_rate} Hz, {nonzero} non-silent)",
                samples.len()
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{}", error.describe());
            ExitCode::FAILURE
        }
    }
}

/// Run an app (scripted or to idle/exit) and write its final frame to a BMP, so a
/// running app's screen can be captured headlessly. End the script on a yielded
/// frame to capture gameplay rather than the cleared exit frame.
fn run_app_image_cli(
    app_id: &str,
    script: Option<&str>,
    font_path: &str,
    image_path: &str,
) -> ExitCode {
    let inputs = match script {
        Some(path) => match std::fs::read_to_string(path) {
            Ok(text) => match parse_app_script(&text) {
                Ok(inputs) => inputs,
                Err(error) => {
                    eprintln!("invalid app script {path}: {error:?}");
                    return ExitCode::FAILURE;
                }
            },
            Err(error) => {
                eprintln!("failed to read app script {path}: {error}");
                return ExitCode::FAILURE;
            }
        },
        None => Vec::new(),
    };

    let font_bytes = match load_font_bytes(font_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("failed to read font {font_path}: {error:?}");
            return ExitCode::FAILURE;
        }
    };
    let font = match BitmapFont::from_bytes(&font_bytes) {
        Ok(font) => font,
        Err(error) => {
            eprintln!("invalid font {font_path}: {error:?}");
            return ExitCode::FAILURE;
        }
    };

    let root = sim_temp_root("koto-app-image");
    if let Err(error) = copy_dir_all(Path::new("sdcard_mock"), &root) {
        eprintln!("failed to prepare app root: {error}");
        return ExitCode::FAILURE;
    }
    let outcome = render_app_frame(&root, app_id, &inputs, &font);
    let _ = std::fs::remove_dir_all(&root);
    match outcome {
        Ok(framebuffer) => {
            if let Err(error) = write_bmp(image_path, &framebuffer) {
                eprintln!("failed to write image {image_path}: {error:?}");
                return ExitCode::FAILURE;
            }
            println!(
                "wrote {}x{} app frame -> {image_path}",
                framebuffer.width(),
                framebuffer.height()
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{}", error.describe());
            ExitCode::FAILURE
        }
    }
}

fn sim_temp_root(prefix: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("{prefix}-{unique}"))
}

fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let target = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_all(&entry.path(), &target)?;
        } else if file_type.is_file() {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

#[cfg(feature = "window")]
fn run_window_mode(
    mut shell: ShellState,
    font_path: &str,
    app: Option<&str>,
    watch: Option<&str>,
    watch_replay: Option<&str>,
    fake_network_json: Option<&str>,
) -> ExitCode {
    let font_bytes = match load_font_bytes(font_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("failed to read font {font_path}: {error:?}");
            return ExitCode::FAILURE;
        }
    };
    // Restore persisted favorites / sort / category for the interactive session.
    koto_sim::apply_shell_prefs(&mut shell, "sdcard_mock");
    let direct = app.map(|app_id| koto_sim::window::DirectApp {
        app_id: app_id.to_string(),
        watch_dir: watch.map(PathBuf::from),
        replay: watch_replay.map(PathBuf::from),
    });
    match koto_sim::window::run(
        shell,
        &font_bytes,
        Path::new("sdcard_mock"),
        direct,
        fake_network_json.map(str::to_owned),
    ) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("window error: {error:?}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(not(feature = "window"))]
fn run_window_mode(
    _shell: ShellState,
    _font_path: &str,
    _app: Option<&str>,
    _watch: Option<&str>,
    _watch_replay: Option<&str>,
    _fake_network_json: Option<&str>,
) -> ExitCode {
    eprintln!("--window requires building with the `window` feature:");
    eprintln!("    cargo run -p koto-sim --features window -- --window");
    ExitCode::FAILURE
}

fn render_image(shell: &ShellState, font_path: &str, image_path: &str) -> Result<(), ExitCode> {
    let font_bytes = match load_font_bytes(font_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("failed to read font {font_path}: {error:?}");
            return Err(ExitCode::FAILURE);
        }
    };
    let font = match BitmapFont::from_bytes(&font_bytes) {
        Ok(font) => font,
        Err(error) => {
            eprintln!("invalid font {font_path}: {error:?}");
            return Err(ExitCode::FAILURE);
        }
    };

    let mut framebuffer = Framebuffer::new(SHELL_SURFACE.width, SHELL_SURFACE.height);
    shell.paint(&mut framebuffer.as_canvas(), &font);

    if let Err(error) = write_bmp(image_path, &framebuffer) {
        eprintln!("failed to write image {image_path}: {error:?}");
        return Err(ExitCode::FAILURE);
    }
    println!(
        "wrote {}x{} frame -> {image_path}",
        framebuffer.width(),
        framebuffer.height()
    );
    Ok(())
}

struct CliArgs {
    script: Option<String>,
    image: Option<String>,
    font: String,
    window: bool,
    power_state: Option<PowerState>,
    memo_validation: bool,
    app: Option<String>,
    app_script: Option<String>,
    inspect: bool,
    budget: bool,
    save_list: bool,
    save_clear: Option<String>,
    golden_frames: bool,
    ui_gallery: bool,
    splash: bool,
    audio: Option<String>,
    system_view: bool,
    watch: Option<String>,
    watch_replay: Option<String>,
    fake_network: Option<String>,
}

impl CliArgs {
    fn parse(mut args: impl Iterator<Item = String>) -> Result<Self, String> {
        let mut script = None;
        let mut image = None;
        let mut font = None;
        let mut window = false;
        let mut battery = None;
        let mut charging = false;
        let mut voltage = None;
        let mut power_unknown = false;
        let mut memo_validation = false;
        let mut app = None;
        let mut app_script = None;
        let mut inspect = false;
        let mut budget = false;
        let mut save_list = false;
        let mut save_clear = None;
        let mut golden_frames = false;
        let mut ui_gallery = false;
        let mut splash = false;
        let mut audio = None;
        let mut system_view = false;
        let mut watch = None;
        let mut watch_replay = None;
        let mut fake_network = None;

        while let Some(flag) = args.next() {
            match flag.as_str() {
                "--script" => script = Some(Self::value(&mut args, "--script")?),
                "--image" => image = Some(Self::value(&mut args, "--image")?),
                "--font" => font = Some(Self::value(&mut args, "--font")?),
                "--window" => window = true,
                "--battery" => battery = Some(Self::percent(&mut args)?),
                "--charging" => charging = true,
                "--voltage" => voltage = Some(Self::millivolts(&mut args)?),
                "--power-unknown" => power_unknown = true,
                "--memo-validation" => memo_validation = true,
                "--app" => app = Some(Self::value(&mut args, "--app")?),
                "--app-script" => app_script = Some(Self::value(&mut args, "--app-script")?),
                "--inspect" => inspect = true,
                "--budget" => budget = true,
                "--save-list" => save_list = true,
                "--save-clear" => save_clear = Some(Self::value(&mut args, "--save-clear")?),
                "--golden-frames" => golden_frames = true,
                "--ui-gallery" => ui_gallery = true,
                "--splash" => splash = true,
                "--audio" => audio = Some(Self::value(&mut args, "--audio")?),
                "--system-view" => system_view = true,
                "--watch" => watch = Some(Self::value(&mut args, "--watch")?),
                "--watch-replay" => watch_replay = Some(Self::value(&mut args, "--watch-replay")?),
                "--fake-network" => fake_network = Some(Self::value(&mut args, "--fake-network")?),
                other => return Err(format!("unknown argument: {other}")),
            }
        }

        if save_list && save_clear.is_some() {
            return Err("--save-list cannot be combined with --save-clear".to_string());
        }
        if save_list && app.is_some() {
            return Err("--save-list cannot be combined with --app".to_string());
        }
        if save_clear.is_some() && app.is_some() {
            return Err("--save-clear cannot be combined with --app".to_string());
        }
        if golden_frames
            && (script.is_some()
                || image.is_some()
                || window
                || memo_validation
                || app.is_some()
                || app_script.is_some()
                || inspect
                || budget
                || save_list
                || save_clear.is_some()
                || audio.is_some())
        {
            return Err("--golden-frames cannot be combined with other modes".to_string());
        }
        if ui_gallery
            && (script.is_some()
                || app.is_some()
                || app_script.is_some()
                || memo_validation
                || golden_frames
                || splash)
        {
            return Err("--ui-gallery only combines with --window/--image/--font".to_string());
        }
        if audio.is_some() && app.is_none() {
            return Err("--audio requires --app".to_string());
        }
        if splash && (window || app.is_some() || script.is_some() || golden_frames) {
            return Err("--splash only combines with --image/--font".to_string());
        }
        if watch.is_some() && (!window || app.is_none()) {
            return Err("--watch requires --window and --app".to_string());
        }
        if watch_replay.is_some() && watch.is_none() {
            return Err("--watch-replay requires --watch".to_string());
        }
        if fake_network.is_some()
            && (script.is_some()
                || image.is_some()
                || memo_validation
                || app.is_some()
                || app_script.is_some()
                || inspect
                || budget
                || save_list
                || save_clear.is_some()
                || golden_frames
                || ui_gallery
                || splash
                || audio.is_some()
                || system_view
                || watch.is_some()
                || watch_replay.is_some())
        {
            return Err("--fake-network only combines with --window/--font".to_string());
        }

        let power_state = Self::resolve_power_state(battery, charging, voltage, power_unknown)?;

        Ok(Self {
            script,
            image,
            font: font.unwrap_or_else(|| DEFAULT_FONT.to_string()),
            window,
            power_state,
            memo_validation,
            app,
            app_script,
            inspect,
            budget,
            save_list,
            save_clear,
            golden_frames,
            ui_gallery,
            splash,
            audio,
            system_view,
            watch,
            watch_replay,
            fake_network,
        })
    }

    /// Combine the power-related flags into a single [`PowerState`]. `--voltage`
    /// takes precedence; `--charging` pairs with an optional `--battery` percent;
    /// a bare `--battery` is a discharging percent; `--power-unknown` is the
    /// fallback. With no power flags the shell keeps its `Unsupported` default
    /// (no status text).
    fn resolve_power_state(
        battery: Option<u8>,
        charging: bool,
        voltage: Option<u16>,
        power_unknown: bool,
    ) -> Result<Option<PowerState>, String> {
        if let Some(millivolts) = voltage {
            if battery.is_some() {
                return Err("--voltage cannot be combined with --battery".to_string());
            }
            return Ok(Some(PowerState::voltage(millivolts)));
        }
        if charging {
            return Ok(Some(PowerState::charging(battery, None)));
        }
        if let Some(percent) = battery {
            return Ok(Some(PowerState::percent(percent, None)));
        }
        if power_unknown {
            return Ok(Some(PowerState::unknown()));
        }
        Ok(None)
    }

    fn value(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
        args.next().ok_or_else(|| format!("{flag} requires a path"))
    }

    fn percent(args: &mut impl Iterator<Item = String>) -> Result<u8, String> {
        let raw = args
            .next()
            .ok_or_else(|| "--battery requires a percent (0-100)".to_string())?;
        match raw.parse::<u8>() {
            Ok(percent) if percent <= 100 => Ok(percent),
            _ => Err(format!("--battery percent must be 0-100, got: {raw}")),
        }
    }

    fn millivolts(args: &mut impl Iterator<Item = String>) -> Result<u16, String> {
        let raw = args
            .next()
            .ok_or_else(|| "--voltage requires a value in millivolts".to_string())?;
        raw.parse::<u16>()
            .map_err(|_| format!("--voltage must be a millivolt value, got: {raw}"))
    }
}
