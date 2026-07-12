//! Interactive `minifb` window backend for KotoSim (feature `window`).
//!
//! Presents the live framebuffer and maps PC keyboard events onto the PicoCalc
//! button model and the bytecode text-input ABI so the shell and bytecode apps
//! can be driven by hand. Compiled only when the `window` feature is enabled,
//! keeping headless builds and CI dependency-free.

use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use koto_core::runtime::{audio_id, text_intent};
use koto_core::shell::SHELL_SURFACE;
use koto_core::{
    BitmapFont, BootSplash, BootStep, BootStepStatus, Buttons, FontError, InputState, Rect, Rgb565,
    ShellAction, ShellSound, ShellState, VmInputSnapshot,
};
use minifb::{Key, KeyRepeat, Scale, Window, WindowOptions};

use crate::audio::{SimAudio, DEFAULT_SAMPLE_RATE};
use crate::{framebuffer_to_argb, AppFailureSummary, BytecodeAppSession, Framebuffer};

#[derive(Debug)]
pub enum WindowError {
    Font(FontError),
    Backend(minifb::Error),
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
    // Force an initial paint before the first input.
    shell.paint(&mut framebuffer.as_canvas(), &font);
    let mut mode = WindowMode::Shell;

    println!(
        "KotoSim window: arrows move/cursor, Enter launch/newline, Backspace toggle details pane, \
         F2 favorite, F3 sort, F4 category, letters type, F1 IME on/off, Tab convert, \
         Shift sticky-shift, RShift commit, LCtrl cancel, F2 save (prompts for name) \
         / F4 open file (memo), F5 new file (memo), F10 quit app, Esc quit"
    );

    while window.is_open() && !window.is_key_down(Key::Escape) {
        match &mut mode {
            WindowMode::Shell => {
                // Function-key shell commands (favorite / sort / category) persist
                // to the save-data area so they survive across sessions.
                let mut prefs_changed = false;
                if window.is_key_pressed(Key::F2, KeyRepeat::No) {
                    shell.toggle_selected_favorite();
                    prefs_changed = true;
                }
                if window.is_key_pressed(Key::F3, KeyRepeat::No) {
                    shell.cycle_sort();
                    prefs_changed = true;
                }
                if window.is_key_pressed(Key::F4, KeyRepeat::No) {
                    shell.cycle_category();
                    prefs_changed = true;
                }
                // F5 toggles the system/memory status overlay (KOTO-0182). The
                // shell full-repaints every frame here, so no relayout bookkeeping.
                if window.is_key_pressed(Key::F5, KeyRepeat::No) {
                    shell.toggle_system_view();
                }
                if prefs_changed {
                    let _ = crate::save_shell_prefs(&shell, root);
                }

                let action = shell.update(&poll_shell_input(&window));
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
                if let ShellAction::Launch(package) = action {
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
        }

        let buffer = framebuffer_to_argb(&framebuffer);
        window.update_with_buffer(&buffer, width, height)?;
    }

    Ok(())
}

enum WindowMode {
    Shell,
    // Boxed: a live session embeds the VM heap and editor buffers, so the variant
    // is several KB larger than `Shell`.
    App(Box<AppRun>),
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

    let mut intent_bits = 0;
    let mut set = |pressed: bool, flag: u32| {
        if pressed {
            intent_bits |= flag;
        }
    };
    set(
        window.is_key_pressed(Key::Enter, KeyRepeat::No),
        text_intent::NEWLINE,
    );
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
    // Left Shift arms Sticky Shift (SKK), except when it is being used to type a
    // shifted symbol this frame — then it is a plain modifier, not a conversion.
    let typing_shifted_symbol = typed
        .map(|ch| !ch.is_ascii_alphabetic() && !ch.is_whitespace())
        .unwrap_or(false);
    set(
        window.is_key_pressed(Key::LeftShift, KeyRepeat::No) && !typing_shifted_symbol,
        text_intent::SHIFT,
    );
    set(
        window.is_key_pressed(Key::RightShift, KeyRepeat::No)
            || window.is_key_down(Key::RightShift),
        text_intent::COMMIT,
    );
    set(
        window.is_key_pressed(Key::LeftCtrl, KeyRepeat::No),
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
