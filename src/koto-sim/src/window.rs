//! Interactive `minifb` window backend for KotoSim (feature `window`).
//!
//! Presents the live framebuffer and maps PC keyboard events onto the PicoCalc
//! button model and the bytecode text-input ABI so the shell and bytecode apps
//! can be driven by hand. Compiled only when the `window` feature is enabled,
//! keeping headless builds and CI dependency-free.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use koto_core::runtime::{audio_id, text_intent};
use koto_core::shell::SHELL_SURFACE;
use koto_core::{
    BitmapFont, BootSplash, BootStep, BootStepStatus, Buttons, CanvasUiPainter, ConfigCapability,
    FontError, InputState, KotoConfigAction, KotoConfigUi, KotoConfigWifiUi, Rect, Rgb565,
    ShellAction, ShellCommandId, ShellSound, ShellState, VmInputSnapshot, WifiIntent, WifiKey,
    KOTOCONFIG_SURFACE, KOTOCONFIG_WIFI_SURFACE,
};
use minifb::{Key, KeyRepeat, Scale, Window, WindowOptions};

use crate::audio::{SimAudio, DEFAULT_SAMPLE_RATE};
use crate::fake_network::FakeNetworkUiSession;
use crate::{
    framebuffer_to_argb, parse_app_script, AppFailureSummary, BytecodeAppSession, Framebuffer,
    UiGallery,
};

/// Launch straight into an app instead of the shell (`--window --app`), with
/// the optional KOTO-0191 live-reload loop.
pub struct DirectApp {
    pub app_id: String,
    /// Directory tree to poll for changes (`--watch apps/<dir>`); a change
    /// rebuilds the app (`python harness/build_apps.py --app <id>`) and
    /// relaunches it in the same window.
    pub watch_dir: Option<PathBuf>,
    /// App input script replayed after every (re)launch (`--watch-replay`),
    /// landing back at the scene being iterated on.
    pub replay: Option<PathBuf>,
}

#[derive(Debug)]
pub enum WindowError {
    Font(FontError),
    Backend(minifb::Error),
}

/// Interactive developer-only KotoUI gallery (`--ui-gallery --window`).
pub fn run_ui_gallery(font_bytes: &[u8]) -> Result<(), WindowError> {
    let font = BitmapFont::from_bytes(font_bytes).map_err(WindowError::Font)?;
    let (width, height) = (320usize, 320usize);
    let mut window = Window::new(
        "KotoUI Gallery",
        width,
        height,
        WindowOptions {
            scale: Scale::X2,
            ..WindowOptions::default()
        },
    )?;
    window.set_target_fps(60);
    let mut gallery = UiGallery::new();
    let mut composition = false;
    println!(
        "KotoUI Gallery: arrows/Tab navigate, Enter/Space activate, type to edit, F1 composition, LCtrl cancel, F10 exit"
    );
    while window.is_open() && !window.is_key_down(Key::F10) {
        let mut events = Vec::new();
        for (key, action) in [
            (
                Key::Up,
                koto_ui::UiAction::Navigate(koto_ui::Navigation::Up),
            ),
            (
                Key::Down,
                koto_ui::UiAction::Navigate(koto_ui::Navigation::Down),
            ),
            (
                Key::Left,
                koto_ui::UiAction::Navigate(koto_ui::Navigation::Left),
            ),
            (
                Key::Right,
                koto_ui::UiAction::Navigate(koto_ui::Navigation::Right),
            ),
            (
                Key::Tab,
                koto_ui::UiAction::Navigate(koto_ui::Navigation::Next),
            ),
            (Key::Enter, koto_ui::UiAction::Activate),
            (Key::Space, koto_ui::UiAction::Activate),
            (Key::Backspace, koto_ui::UiAction::Backspace),
            (Key::Delete, koto_ui::UiAction::Delete),
            (Key::Home, koto_ui::UiAction::Home),
            (Key::End, koto_ui::UiAction::End),
            (Key::LeftCtrl, koto_ui::UiAction::Cancel),
        ] {
            if window.is_key_pressed(key, KeyRepeat::Yes) {
                events.push(koto_ui::UiEvent::pressed(action));
            }
        }
        if let Some(ch) = typed_char(&window, window.is_key_down(Key::LeftShift)) {
            events.push(koto_ui::UiEvent::pressed(koto_ui::UiAction::Text(ch)));
        }
        if window.is_key_pressed(Key::F1, KeyRepeat::No) {
            composition = !composition;
            gallery.set_composition(composition);
        }
        for event in events {
            gallery.handle_event(event);
        }
        let framebuffer = gallery.render(&font);
        window.update_with_buffer(&framebuffer_to_argb(&framebuffer), width, height)?;
    }
    Ok(())
}

impl From<minifb::Error> for WindowError {
    fn from(error: minifb::Error) -> Self {
        WindowError::Backend(error)
    }
}

/// Open the window and run the interactive loop until it is closed or Escape is
/// pressed. `font_bytes` is a `.kfont` blob (see [`crate::load_font_bytes`]).
pub fn run(
    mut shell: ShellState,
    font_bytes: &[u8],
    root: &std::path::Path,
    direct: Option<DirectApp>,
    fake_network_json: Option<String>,
) -> Result<(), WindowError> {
    let font = BitmapFont::from_bytes(font_bytes).map_err(WindowError::Font)?;

    let width = SHELL_SURFACE.width as usize;
    let height = SHELL_SURFACE.height as usize;
    let mut window = Window::new(
        "KotoSim",
        width,
        height,
        WindowOptions {
            scale: Scale::X2,
            ..WindowOptions::default()
        },
    )?;
    window.set_target_fps(60);

    // Shared host audio engine. The cpal output stream (kept alive for the whole
    // session) renders from it; each launched app drives it through the audio host
    // calls. If no audio device is available the engine still exists, apps just run
    // silently.
    let audio = Arc::new(Mutex::new(SimAudio::new(DEFAULT_SAMPLE_RATE)));
    let _audio_stream = start_audio(&audio);

    let mut framebuffer = Framebuffer::new(SHELL_SURFACE.width, SHELL_SURFACE.height);
    // KOTO-0181: open on the boot splash so the simulator mirrors the device's
    // boot moment. Steps resolve in real boot order; the device paces them by
    // its actual init phases, the simulator by a short fixed cadence.
    let mut splash = BootSplash::new();
    'splash: for (index, step) in BootStep::ALL.into_iter().enumerate() {
        splash.resolve(step, BootStepStatus::Ok);
        // ~8 frames per step at 60 fps, plus a short hold on the full checklist.
        let frames = if index + 1 == BootStep::ALL.len() {
            24
        } else {
            8
        };
        for _ in 0..frames {
            if !window.is_open() || window.is_key_down(Key::Escape) {
                break 'splash;
            }
            splash.paint(&mut framebuffer.as_canvas(), &font);
            let buffer = framebuffer_to_argb(&framebuffer);
            window.update_with_buffer(&buffer, width, height)?;
        }
    }
    let mut system_config = crate::load_system_config(root);
    shell.apply_config_snapshot(system_config.snapshot());
    // Force an initial paint before the first input.
    shell.paint(&mut framebuffer.as_canvas(), &font);
    let mut mode = WindowMode::Shell;

    // `--window --app`: skip the shell and start inside the app, optionally
    // with the KOTO-0191 watch loop (rebuild + relaunch on source change).
    let mut watcher = None;
    if let Some(direct) = direct {
        let name = shell
            .packages()
            .iter()
            .find(|package| package.app_id() == direct.app_id)
            .map(|package| package.name().to_string())
            .unwrap_or_else(|| direct.app_id.clone());
        println!("direct launch: {} ({})", name, direct.app_id);
        let mut run = AppRun::launch(&name, &direct.app_id, &audio);
        replay_script(&mut run, direct.replay.as_deref());
        paint_app_run(&mut framebuffer, &font, &run);
        mode = WindowMode::App(Box::new(run));
        if let Some(dir) = direct.watch_dir {
            println!(
                "watching {} (save a file to rebuild + relaunch{})",
                dir.display(),
                if direct.replay.is_some() {
                    " + replay"
                } else {
                    ""
                }
            );
            watcher = Some(WatchState::new(direct.app_id, name, dir, direct.replay));
        }
    }

    println!(
        "KotoSim window: arrows move/cursor, Enter launch/newline, Backspace toggle details pane, \
         F1 settings (shell) / IME on/off (app), F2 favorite, F3 sort, F4 category, letters type, Tab convert, \
         Shift sticky-shift, Space convert/cycle, Enter commit, Ctrl+G cancel, F2 save (prompts for name) \
         / F4 open file (memo), F5 new file (memo), F10 quit app, Esc quit"
    );

    while window.is_open() {
        if window.is_key_down(Key::Escape)
            && !matches!(mode, WindowMode::Config(_) | WindowMode::ConfigWifi(_))
        {
            break;
        }
        // KOTO-0191 watch loop: on the poll cadence, a change under the watched
        // tree rebuilds the app and relaunches it in this window. A failed
        // build keeps the running session (fix the source and save to retry);
        // its diagnostics go to the console in the `$koto` matcher format.
        if let Some(watch) = watcher.as_mut() {
            if watch.poll_changed() {
                println!("watch: change under {}, rebuilding...", watch.dir.display());
                let started = std::time::Instant::now();
                if watch.rebuild() {
                    println!(
                        "watch: rebuilt {} in {:.2?}; relaunching",
                        watch.app_id,
                        started.elapsed()
                    );
                    let mut run = AppRun::launch(&watch.name, &watch.app_id, &audio);
                    replay_script(&mut run, watch.replay.as_deref());
                    paint_app_run(&mut framebuffer, &font, &run);
                    mode = WindowMode::App(Box::new(run));
                } else {
                    println!("watch: build failed; keeping the running app (save again to retry)");
                }
            }
        }
        match &mut mode {
            WindowMode::Shell => {
                // Function-key shell commands (favorite / sort / category) persist
                // to the save-data area so they survive across sessions.
                let mut prefs_changed = false;
                let open_config = window.is_key_pressed(Key::F1, KeyRepeat::No);
                if window.is_key_pressed(Key::F2, KeyRepeat::No) {
                    shell.activate_command(ShellCommandId::Favorite);
                    prefs_changed = true;
                }
                if window.is_key_pressed(Key::F3, KeyRepeat::No) {
                    shell.activate_command(ShellCommandId::Sort);
                    prefs_changed = true;
                }
                if window.is_key_pressed(Key::F4, KeyRepeat::No) {
                    shell.activate_command(ShellCommandId::Category);
                    prefs_changed = true;
                }
                // F5 toggles the system/memory status overlay (KOTO-0182). The
                // shell full-repaints every frame here, so no relayout bookkeeping.
                if window.is_key_pressed(Key::F5, KeyRepeat::No) {
                    shell.activate_command(ShellCommandId::System);
                }
                if prefs_changed {
                    let _ = crate::save_shell_prefs(&shell, root);
                }

                let action = if open_config {
                    shell.activate_command(ShellCommandId::Settings)
                } else {
                    shell.update(&poll_shell_input(&window))
                };
                if let Some(sound) = shell.take_pending_sound() {
                    if let Ok(mut audio) = audio.lock() {
                        let id = match sound {
                            ShellSound::Navigation => audio_id::SFX_SHELL_NAV,
                            ShellSound::Confirm => audio_id::SFX_SHELL_CONFIRM,
                            ShellSound::Cancel => audio_id::SFX_SHELL_CANCEL,
                        };
                        audio.play_sfx(id);
                    }
                }
                if action == ShellAction::OpenConfig {
                    if let Some(json) = fake_network_json.as_deref() {
                        // Explicit development/test hook. The default path has
                        // no fake and therefore advertises no Wi-Fi capability.
                        // KOTO-0241 consumes these public observations without
                        // giving KotoConfig access to host-network APIs.
                        let trace = crate::fake_network::replay_trace(json)
                            .expect("fake fixture was validated before window launch");
                        println!(
                            "KotoConfig fake NetworkService attached: {} observations",
                            trace.len()
                        );
                    }
                    let capabilities = config_capabilities(fake_network_json.is_some());
                    let ui = KotoConfigUi::new_with_capabilities(&system_config, capabilities);
                    paint_koto_config(&mut framebuffer, &font, &ui);
                    mode = WindowMode::Config(ui);
                } else if let ShellAction::Launch(package) = action {
                    println!(
                        "launch requested: {} ({})",
                        package.name(),
                        package.app_id()
                    );
                    mode = WindowMode::App(Box::new(AppRun::launch(
                        package.name(),
                        package.app_id(),
                        &audio,
                    )));
                    if let WindowMode::App(run) = &mode {
                        paint_app_run(&mut framebuffer, &font, run);
                    }
                } else {
                    shell.paint(&mut framebuffer.as_canvas(), &font);
                    shell.advance_feedback();
                }
            }
            WindowMode::App(run) => {
                let back_to_shell = run.update(&window);
                if back_to_shell {
                    mode = WindowMode::Shell;
                    shell.paint(&mut framebuffer.as_canvas(), &font);
                } else {
                    paint_app_run(&mut framebuffer, &font, run);
                }
            }
            WindowMode::Config(ui) => {
                let exit_shortcut = window.is_key_pressed(Key::F1, KeyRepeat::No)
                    || window.is_key_pressed(Key::F10, KeyRepeat::No);
                let mut events = Vec::new();
                if window.is_key_pressed(Key::Tab, KeyRepeat::Yes) {
                    let navigation = if window.is_key_down(Key::LeftShift)
                        || window.is_key_down(Key::RightShift)
                    {
                        koto_ui::Navigation::Previous
                    } else {
                        koto_ui::Navigation::Next
                    };
                    events.push(koto_ui::UiEvent::pressed(koto_ui::UiAction::Navigate(
                        navigation,
                    )));
                }
                for (key, action) in [
                    (
                        Key::Up,
                        koto_ui::UiAction::Navigate(koto_ui::Navigation::Up),
                    ),
                    (
                        Key::Down,
                        koto_ui::UiAction::Navigate(koto_ui::Navigation::Down),
                    ),
                    (
                        Key::Left,
                        koto_ui::UiAction::Navigate(koto_ui::Navigation::Left),
                    ),
                    (
                        Key::Right,
                        koto_ui::UiAction::Navigate(koto_ui::Navigation::Right),
                    ),
                    (Key::Enter, koto_ui::UiAction::Activate),
                    (Key::Escape, koto_ui::UiAction::Cancel),
                    (Key::Backspace, koto_ui::UiAction::Cancel),
                    (Key::LeftCtrl, koto_ui::UiAction::Cancel),
                ] {
                    if window.is_key_pressed(key, KeyRepeat::Yes) {
                        events.push(koto_ui::UiEvent::pressed(action));
                    }
                }
                let mut repaint = false;
                let mut exit = exit_shortcut;
                let mut open_wifi = false;
                for event in events {
                    match ui.handle_event(event, &mut system_config) {
                        KotoConfigAction::None => {}
                        KotoConfigAction::LocaleChanged(_) => {
                            let _ = crate::save_system_config(&system_config, root);
                            shell.apply_config_snapshot(system_config.snapshot());
                            repaint = true;
                        }
                        KotoConfigAction::UtcOffsetChanged(_) => {
                            let _ = crate::save_system_config(&system_config, root);
                            repaint = true;
                        }
                        KotoConfigAction::SntpServerChanged(_) => {
                            let _ = crate::save_system_config(&system_config, root);
                            repaint = true;
                        }
                        KotoConfigAction::OpenWifi => open_wifi = true,
                        KotoConfigAction::Exit => exit = true,
                    }
                    repaint |= ui.damaged_rects().next().is_some();
                }
                if open_wifi {
                    if let Some(json) = fake_network_json.as_deref() {
                        let backend = FakeNetworkUiSession::new(json)
                            .expect("fake fixture was validated before window launch");
                        let mut wifi =
                            KotoConfigWifiUi::new(system_config.locale(), backend.snapshot());
                        let _ = wifi.update(backend.snapshot(), backend.results(), None);
                        paint_koto_config_wifi(&mut framebuffer, &font, &wifi);
                        wifi.clear_damage();
                        mode = WindowMode::ConfigWifi(Box::new((wifi, backend)));
                    }
                } else if exit {
                    mode = WindowMode::Shell;
                    shell.paint(&mut framebuffer.as_canvas(), &font);
                } else if repaint {
                    paint_koto_config(&mut framebuffer, &font, ui);
                    ui.clear_damage();
                }
            }
            WindowMode::ConfigWifi(state) => {
                let (ui, backend) = state.as_mut();
                let exit_shortcut = window.is_key_pressed(Key::F1, KeyRepeat::No)
                    || window.is_key_pressed(Key::F10, KeyRepeat::No);
                let advanced = backend.service_frame();
                let intent = ui.update(
                    backend.snapshot(),
                    backend.results(),
                    poll_wifi_key(&window),
                );
                let submitted = backend.submit(intent, ui.credential());
                if submitted {
                    ui.submission_complete(intent);
                }

                if exit_shortcut {
                    ui.reset();
                    mode = WindowMode::Shell;
                    shell.paint(&mut framebuffer.as_canvas(), &font);
                } else if intent == WifiIntent::Exit {
                    ui.reset();
                    let capabilities = config_capabilities(fake_network_json.is_some());
                    let config = KotoConfigUi::new_with_capabilities(&system_config, capabilities);
                    paint_koto_config(&mut framebuffer, &font, &config);
                    mode = WindowMode::Config(config);
                } else if advanced || ui.damaged_rects().next().is_some() {
                    paint_koto_config_wifi(&mut framebuffer, &font, ui);
                    ui.clear_damage();
                }
            }
        }

        let buffer = framebuffer_to_argb(&framebuffer);
        window.update_with_buffer(&buffer, width, height)?;
    }

    Ok(())
}

enum WindowMode {
    Shell,
    Config(KotoConfigUi),
    ConfigWifi(Box<(KotoConfigWifiUi, FakeNetworkUiSession)>),
    // Boxed: a live session embeds the VM heap and editor buffers, so the variant
    // is several KB larger than `Shell`.
    App(Box<AppRun>),
}

fn config_capabilities(fake_network: bool) -> ConfigCapability {
    if fake_network {
        ConfigCapability::LOCALE_CONFIG.union(ConfigCapability::WIFI_CONFIG)
    } else {
        ConfigCapability::LOCALE_CONFIG
    }
}

fn paint_koto_config(framebuffer: &mut Framebuffer, font: &BitmapFont<'_>, ui: &KotoConfigUi) {
    let mut canvas = framebuffer.as_canvas();
    let mut painter = CanvasUiPainter::new(&mut canvas, font);
    let _ = ui.paint(&mut painter, KOTOCONFIG_SURFACE);
}

fn paint_koto_config_wifi(
    framebuffer: &mut Framebuffer,
    font: &BitmapFont<'_>,
    ui: &KotoConfigWifiUi,
) {
    let mut canvas = framebuffer.as_canvas();
    let mut painter = CanvasUiPainter::new(&mut canvas, font);
    let _ = ui.paint(&mut painter, KOTOCONFIG_WIFI_SURFACE);
}

fn poll_wifi_key(window: &Window) -> Option<WifiKey> {
    for (key, wifi_key) in [
        (Key::Up, WifiKey::Up),
        (Key::Down, WifiKey::Down),
        (Key::Left, WifiKey::Left),
        (Key::Right, WifiKey::Right),
        (Key::Enter, WifiKey::Enter),
        (Key::Escape, WifiKey::Esc),
        (Key::Backspace, WifiKey::Backspace),
    ] {
        if window.is_key_pressed(key, KeyRepeat::Yes) {
            return Some(wifi_key);
        }
    }
    if window.is_key_pressed(Key::Tab, KeyRepeat::Yes) {
        return Some(
            if window.is_key_down(Key::LeftShift) || window.is_key_down(Key::RightShift) {
                WifiKey::Previous
            } else {
                WifiKey::Next
            },
        );
    }
    typed_char(window, window.is_key_down(Key::LeftShift))
        .and_then(|ch| ch.is_ascii().then_some(WifiKey::Char(ch as u8)))
}

/// Watch poll cadence in window frames (~250 ms at the 60 fps target).
const WATCH_POLL_FRAMES: u32 = 15;

/// KOTO-0191 watch loop state: an mtime snapshot of the app's source tree
/// plus what it takes to rebuild and relaunch the app.
struct WatchState {
    app_id: String,
    name: String,
    dir: PathBuf,
    replay: Option<PathBuf>,
    snapshot: Vec<(PathBuf, SystemTime)>,
    countdown: u32,
}

impl WatchState {
    fn new(app_id: String, name: String, dir: PathBuf, replay: Option<PathBuf>) -> Self {
        let snapshot = scan_tree_sorted(&dir);
        Self {
            app_id,
            name,
            dir,
            replay,
            snapshot,
            countdown: WATCH_POLL_FRAMES,
        }
    }

    /// Counts down on the window cadence; on the poll frame, rescans the tree
    /// and reports whether anything changed (add / remove / modify).
    fn poll_changed(&mut self) -> bool {
        self.countdown = self.countdown.saturating_sub(1);
        if self.countdown > 0 {
            return false;
        }
        self.countdown = WATCH_POLL_FRAMES;
        let current = scan_tree_sorted(&self.dir);
        if current == self.snapshot {
            return false;
        }
        self.snapshot = current;
        true
    }

    /// Rebuild just this app through the registry build (bytecode, images,
    /// maps, assets — `harness/build_apps.py --app`). Stdio is inherited so
    /// compiler diagnostics keep their `file:line:col` shape for both plain
    /// terminals and the VS Code `$koto` problem matcher.
    fn rebuild(&self) -> bool {
        match std::process::Command::new("python")
            .args(["harness/build_apps.py", "--app", &self.app_id])
            .status()
        {
            Ok(status) => status.success(),
            Err(error) => {
                eprintln!("watch: failed to run harness/build_apps.py: {error}");
                false
            }
        }
    }
}

/// Collect `(path, mtime)` for every file under `dir`, sorted for comparison.
/// Unreadable entries are skipped: a transient editor lock shows up as a
/// change on the next poll rather than an error.
fn scan_tree_sorted(dir: &Path) -> Vec<(PathBuf, SystemTime)> {
    fn walk(dir: &Path, out: &mut Vec<(PathBuf, SystemTime)>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if file_type.is_dir() {
                walk(&path, out);
            } else if file_type.is_file() {
                let modified = entry
                    .metadata()
                    .ok()
                    .and_then(|metadata| metadata.modified().ok())
                    .unwrap_or(SystemTime::UNIX_EPOCH);
                out.push((path, modified));
            }
        }
    }
    let mut files = Vec::new();
    walk(dir, &mut files);
    files.sort();
    files
}

/// Replay a recorded app script into a fresh session so a (re)launch lands
/// back at the scene being iterated on. Script problems are reported and
/// leave the session at whatever frame it reached; a trap during replay
/// surfaces exactly like a live trap.
fn replay_script(run: &mut AppRun, script: Option<&Path>) {
    let Some(path) = script else {
        return;
    };
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("watch replay: {}: {error}", path.display());
            return;
        }
    };
    let inputs = match parse_app_script(&text) {
        Ok(inputs) => inputs,
        Err(error) => {
            eprintln!("watch replay: {}: {error:?}", path.display());
            return;
        }
    };
    let mut failure = None;
    if let Ok(session) = &mut run.session {
        for input in inputs {
            if session.has_exited() {
                break;
            }
            if session.step_frame(input).is_err() {
                failure = Some(AppFailureSummary::trap(session.diagnostic()));
                break;
            }
        }
    }
    if let Some(failure) = failure {
        run.session = Err(failure);
    }
}

struct AppRun {
    name: String,
    app_id: String,
    session: Result<BytecodeAppSession, AppFailureSummary>,
}

impl AppRun {
    fn launch(name: &str, app_id: &str, audio: &Arc<Mutex<SimAudio>>) -> Self {
        // Start the new app from a clean audio state (no leftover BGM/SFX).
        if let Ok(mut audio) = audio.lock() {
            audio.reset();
        }
        let session =
            BytecodeAppSession::launch_with_audio("sdcard_mock", app_id, Arc::clone(audio))
                .map_err(|error| AppFailureSummary::launch(app_id, error));
        Self {
            name: name.to_string(),
            app_id: app_id.to_string(),
            session,
        }
    }

    /// Advance the app one frame from the current input. Returns `true` when the
    /// view should return to the shell.
    fn update(&mut self, window: &Window) -> bool {
        let back = window.is_key_pressed(Key::Backspace, KeyRepeat::No);
        let mut step_error = None;
        match &mut self.session {
            Ok(session) => {
                if session.has_exited() {
                    // Hold the final frame until the user steps back to the shell.
                    return back;
                }
                // F3 toggles the editor's soft-wrap mode (a host editor setting).
                if window.is_key_pressed(Key::F3, KeyRepeat::No) {
                    session.toggle_wrap();
                }
                let input = poll_app_input(window);
                if let Err(error) = session.step_frame(input) {
                    let _ = error;
                    step_error = Some(AppFailureSummary::trap(session.diagnostic()));
                }
            }
            // A failed launch or runtime error stays on screen until dismissed.
            Err(_) => return back,
        }
        if let Some(error) = step_error {
            self.session = Err(error);
        }
        false
    }
}

/// Open the default audio device and start a cpal output stream that renders from
/// the shared [`SimAudio`] engine. The engine's sample rate is matched to the device
/// so no resampling is needed. Returns the live `Stream` (which must be held to keep
/// playback running), or `None` when no device is available — apps then run silently.
fn start_audio(audio: &Arc<Mutex<SimAudio>>) -> Option<cpal::Stream> {
    let host = cpal::default_host();
    let device = host.default_output_device()?;
    let supported = device.default_output_config().ok()?;
    let sample_rate = supported.sample_rate().0;
    let channels = supported.channels() as usize;
    let sample_format = supported.sample_format();
    if let Ok(mut engine) = audio.lock() {
        engine.set_sample_rate(sample_rate);
    }

    let config: cpal::StreamConfig = supported.into();
    let err_fn = |err| eprintln!("audio stream error: {err}");
    let engine = Arc::clone(audio);
    let stream = match sample_format {
        cpal::SampleFormat::F32 => device
            .build_output_stream(
                &config,
                move |data: &mut [f32], _| {
                    let mut mono = vec![0i16; data.len() / channels.max(1)];
                    render_locked(&engine, &mut mono);
                    for (frame, &sample) in data.chunks_mut(channels.max(1)).zip(mono.iter()) {
                        let value = sample as f32 / 32_768.0;
                        frame.iter_mut().for_each(|out| *out = value);
                    }
                },
                err_fn,
                None,
            )
            .ok()?,
        cpal::SampleFormat::I16 => device
            .build_output_stream(
                &config,
                move |data: &mut [i16], _| {
                    let mut mono = vec![0i16; data.len() / channels.max(1)];
                    render_locked(&engine, &mut mono);
                    for (frame, &sample) in data.chunks_mut(channels.max(1)).zip(mono.iter()) {
                        frame.iter_mut().for_each(|out| *out = sample);
                    }
                },
                err_fn,
                None,
            )
            .ok()?,
        _ => return None,
    };
    stream.play().ok()?;
    Some(stream)
}

/// Render mono samples from the shared engine, leaving silence if the lock is
/// poisoned (an audio callback must never panic).
fn render_locked(audio: &Arc<Mutex<SimAudio>>, out: &mut [i16]) {
    if let Ok(mut engine) = audio.lock() {
        engine.render(out);
    }
}

/// Rendered pixel width of `text` in `font` (missing glyphs use the half width).
fn text_px_width(font: &BitmapFont<'_>, text: &str) -> i32 {
    text.chars()
        .map(|ch| {
            font.glyph(ch)
                .map(|glyph| glyph.width() as i32)
                .unwrap_or_else(|| font.half_width() as i32)
        })
        .sum()
}

fn paint_app_run(framebuffer: &mut Framebuffer, font: &BitmapFont<'_>, run: &AppRun) {
    let mut canvas = framebuffer.as_canvas();
    canvas.clear(Rgb565::from_rgb8(12, 14, 18));

    match &run.session {
        Ok(session) => {
            // A running app owns the whole screen, including its own command bar, so
            // the simulator paints no footer over it (the window controls are printed
            // to stdout at launch). An exited app gets a centered banner instead of a
            // bottom-row footer so it never collides with the app's frozen bar.
            // Composite the app's recorded draw lists (rects, pixel/tile blits,
            // then text) — the same path the `--app --image` frame dump uses.
            crate::paint_app_session(&mut canvas, font, session);
            if session.has_exited() {
                let message = "app exited - Backspace: shell / Esc: quit";
                let pad_x = 12;
                let cell = font.cell_height() as i32;
                let text_w = text_px_width(font, message);
                let box_w = (text_w + pad_x * 2).min(SHELL_SURFACE.width as i32);
                let box_h = cell + 12;
                let box_x = ((SHELL_SURFACE.width as i32 - box_w) / 2).max(0);
                let box_y = 148;
                canvas.fill_rect(
                    Rect {
                        x: box_x,
                        y: box_y,
                        w: box_w,
                        h: box_h,
                    },
                    Rgb565::from_rgb8(20, 28, 44),
                );
                canvas.draw_text(
                    box_x + pad_x,
                    box_y + (box_h - cell) / 2,
                    font,
                    message,
                    Rgb565::from_rgb8(220, 226, 236),
                );
            }
        }
        Err(summary) => {
            canvas.draw_text(8, 8, font, &run.name, Rgb565::from_rgb8(245, 247, 250));
            canvas.draw_text(8, 32, font, &run.app_id, Rgb565::from_rgb8(170, 180, 192));
            canvas.draw_text(
                8,
                64,
                font,
                summary.kind.as_str(),
                Rgb565::from_rgb8(240, 170, 170),
            );
            canvas.draw_text(
                8,
                84,
                font,
                &summary.detail,
                Rgb565::from_rgb8(240, 200, 200),
            );
            canvas.draw_text(
                8,
                304,
                font,
                "Backspace: shell / Esc: quit",
                Rgb565::from_rgb8(160, 168, 180),
            );
        }
    }
}

/// Shell navigation input: directional buttons plus confirm/cancel.
fn poll_shell_input(window: &Window) -> InputState {
    let mut pressed = Buttons::default();
    if window.is_key_pressed(Key::Up, KeyRepeat::Yes) {
        pressed.up = true;
    }
    if window.is_key_pressed(Key::Down, KeyRepeat::Yes) {
        pressed.down = true;
    }
    if window.is_key_pressed(Key::Left, KeyRepeat::Yes) {
        pressed.left = true;
    }
    if window.is_key_pressed(Key::Right, KeyRepeat::Yes) {
        pressed.right = true;
    }
    if window.is_key_pressed(Key::Enter, KeyRepeat::No)
        || window.is_key_pressed(Key::Space, KeyRepeat::No)
    {
        pressed.confirm = true;
    }
    if window.is_key_pressed(Key::Backspace, KeyRepeat::No) {
        pressed.cancel = true;
    }

    InputState {
        pressed,
        ..InputState::default()
    }
}

/// Map physical keys onto the bytecode text-input ABI: one typed character plus a
/// set of edit-intent flags for this frame. The interaction model is refined when
/// the interactive app is authored; this is the host-side key routing.
fn poll_app_input(window: &Window) -> VmInputSnapshot {
    let shift_held = window.is_key_down(Key::LeftShift);
    let typed = typed_char(window, shift_held);
    let text_codepoint = typed.map(|ch| ch as u32).unwrap_or(0);
    let enter_pressed = window.is_key_pressed(Key::Enter, KeyRepeat::No);

    let mut intent_bits = 0;
    let mut set = |pressed: bool, flag: u32| {
        if pressed {
            intent_bits |= flag;
        }
    };
    set(enter_pressed, text_intent::NEWLINE);
    set(
        window.is_key_pressed(Key::Backspace, KeyRepeat::Yes),
        text_intent::BACKSPACE,
    );
    set(
        window.is_key_pressed(Key::Delete, KeyRepeat::Yes),
        text_intent::DELETE,
    );
    set(
        window.is_key_pressed(Key::Left, KeyRepeat::Yes),
        text_intent::LEFT,
    );
    set(
        window.is_key_pressed(Key::Right, KeyRepeat::Yes),
        text_intent::RIGHT,
    );
    set(
        window.is_key_pressed(Key::Up, KeyRepeat::Yes),
        text_intent::UP,
    );
    set(
        window.is_key_pressed(Key::Down, KeyRepeat::Yes),
        text_intent::DOWN,
    );
    set(
        window.is_key_pressed(Key::Home, KeyRepeat::No),
        text_intent::HOME,
    );
    set(
        window.is_key_pressed(Key::End, KeyRepeat::No),
        text_intent::END,
    );
    set(
        window.is_key_pressed(Key::Tab, KeyRepeat::No),
        text_intent::CONVERT,
    );
    set(
        window.is_key_pressed(Key::Space, KeyRepeat::No),
        text_intent::CONVERT,
    );
    // Left Shift arms Sticky Shift (SKK), except when it is being used to type a
    // shifted symbol this frame — then it is a plain modifier, not a conversion.
    let typing_shifted_symbol = typed
        .map(|ch| !ch.is_ascii_alphabetic() && !ch.is_whitespace())
        .unwrap_or(false);
    set(
        window.is_key_pressed(Key::LeftShift, KeyRepeat::No) && !typing_shifted_symbol,
        text_intent::SHIFT,
    );
    let control_held = window.is_key_down(Key::LeftCtrl) || window.is_key_down(Key::RightCtrl);
    set(
        control_held && window.is_key_pressed(Key::G, KeyRepeat::No),
        text_intent::CANCEL,
    );
    set(
        window.is_key_pressed(Key::F1, KeyRepeat::No),
        text_intent::IME_TOGGLE,
    );
    set(
        window.is_key_pressed(Key::F2, KeyRepeat::No),
        text_intent::SAVE,
    );
    set(
        window.is_key_pressed(Key::F4, KeyRepeat::No),
        text_intent::OPEN,
    );
    set(
        window.is_key_pressed(Key::F5, KeyRepeat::No),
        text_intent::NEW,
    );
    // F10 is the only key that delivers EXIT — the firmware's scan-code
    // mapping (koto-core `keymap`) is host-tested against this contract, so
    // apps exit identically on both targets (KOTO-0177).
    set(
        window.is_key_pressed(Key::F10, KeyRepeat::No),
        text_intent::EXIT,
    );

    VmInputSnapshot {
        text_codepoint,
        intent_bits,
        // Match the device keyboard bridge: Enter is both a newline intent and
        // game/UI button A. KotoUI consumes button A as Activate.
        pressed_bits: u32::from(enter_pressed) << 4,
        ..VmInputSnapshot::empty()
    }
}

/// First typed character pressed this poll, if any. `shift` selects the shifted
/// (US-layout) variant for digit/symbol keys; letters are always lowercase here
/// (Shift+letter drives Sticky Shift conversion instead).
fn typed_char(window: &Window, shift: bool) -> Option<char> {
    const LETTERS: [(Key, char); 27] = [
        (Key::A, 'a'),
        (Key::B, 'b'),
        (Key::C, 'c'),
        (Key::D, 'd'),
        (Key::E, 'e'),
        (Key::F, 'f'),
        (Key::G, 'g'),
        (Key::H, 'h'),
        (Key::I, 'i'),
        (Key::J, 'j'),
        (Key::K, 'k'),
        (Key::L, 'l'),
        (Key::M, 'm'),
        (Key::N, 'n'),
        (Key::O, 'o'),
        (Key::P, 'p'),
        (Key::Q, 'q'),
        (Key::R, 'r'),
        (Key::S, 's'),
        (Key::T, 't'),
        (Key::U, 'u'),
        (Key::V, 'v'),
        (Key::W, 'w'),
        (Key::X, 'x'),
        (Key::Y, 'y'),
        (Key::Z, 'z'),
        (Key::Space, ' '),
    ];
    if let Some((_, ch)) = LETTERS
        .iter()
        .find(|(key, _)| window.is_key_pressed(*key, KeyRepeat::No))
    {
        return Some(*ch);
    }
    // Digit/symbol keys with their unshifted and US-layout shifted glyphs.
    const SYMBOLS: [(Key, char, char); 21] = [
        (Key::Key0, '0', ')'),
        (Key::Key1, '1', '!'),
        (Key::Key2, '2', '@'),
        (Key::Key3, '3', '#'),
        (Key::Key4, '4', '$'),
        (Key::Key5, '5', '%'),
        (Key::Key6, '6', '^'),
        (Key::Key7, '7', '&'),
        (Key::Key8, '8', '*'),
        (Key::Key9, '9', '('),
        (Key::Minus, '-', '_'),
        (Key::Equal, '=', '+'),
        (Key::LeftBracket, '[', '{'),
        (Key::RightBracket, ']', '}'),
        (Key::Backslash, '\\', '|'),
        (Key::Semicolon, ';', ':'),
        (Key::Apostrophe, '\'', '"'),
        (Key::Comma, ',', '<'),
        (Key::Period, '.', '>'),
        (Key::Slash, '/', '?'),
        (Key::Backquote, '`', '~'),
    ];
    SYMBOLS
        .iter()
        .find(|(key, _, _)| window.is_key_pressed(*key, KeyRepeat::No))
        .map(|(_, base, shifted)| if shift { *shifted } else { *base })
}
