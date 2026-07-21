use super::*;
use koto_ui::Painter;

/// Default foreground colour for a bytecode `draw_text` (which carries no colour
/// of its own in the host ABI): ~rgb(240, 240, 240).
pub const APP_DEFAULT_TEXT_RGB565: u16 = 0xF79E;

/// Composite a session's recorded draw lists onto `canvas`: filled rectangles,
/// then RGB565 pixel/tile blits, then text (colourless `draw_text` uses the
/// default colour). The retained Game2D static/background layer (KOTO-0136) is
/// composited *beneath* each immediate list of the same primitive — static rects
/// under immediate rects, etc. — so the app's page/well/grid/panel chrome paints
/// behind the per-frame board/piece/overlay draws (the board tilemap re-emits
/// into the immediate `draw_pixels`, so it lands above the static layer). Shared
/// by the interactive window backend and the `--app --image` frame dump so both
/// render an app frame identically.
pub fn paint_app_session(
    canvas: &mut Canvas<'_>,
    font: &BitmapFont<'_>,
    session: &BytecodeAppSession,
) {
    canvas.blit_rgb565(0, 0, 320, 320, session.persistent_pixels());
    for &(x, y, w, h, rgb565) in session.static_rects() {
        canvas.fill_rect(Rect { x, y, w, h }, Rgb565(rgb565 as u16));
    }
    for &(x, y, w, h, rgb565) in session.draw_rects() {
        canvas.fill_rect(Rect { x, y, w, h }, Rgb565(rgb565 as u16));
    }
    for (x, y, w, h, pixels) in session.static_pixels() {
        canvas.blit_rgb565(*x, *y, *w, *h, pixels);
    }
    for (x, y, w, h, pixels) in session.draw_pixels() {
        canvas.blit_rgb565(*x, *y, *w, *h, pixels);
    }
    paint_text_list(
        canvas,
        font,
        session.static_text(),
        session.static_text_colors(),
    );
    paint_ui_commands(
        canvas,
        font,
        session.ui_rects(),
        session.ui_text(),
        session.ui_text_colors(),
        session.ui_text_layouts(),
        session.ui_commands(),
    );
    // Retained Game2D text layer (KOTO-0141): composited above the sprite layer
    // (re-emitted into `draw_pixels`) and below the per-frame immediate text, so an
    // id-keyed status value sits in the same fixed z-order on the device.
    for item in session.game2d_text().iter().flatten() {
        let color = match item.rgb565 {
            TEXT_COLOR_DEFAULT => Rgb565(APP_DEFAULT_TEXT_RGB565),
            rgb565 => Rgb565(rgb565 as u16),
        };
        canvas.draw_text(item.x, item.y, font, &item.text, color);
    }
    paint_text_list(canvas, font, session.text(), session.text_colors());
}

fn paint_ui_commands(
    canvas: &mut Canvas<'_>,
    font: &BitmapFont<'_>,
    rects: &[(i32, i32, i32, i32, i32)],
    text: &[(i32, i32, String)],
    colors: &[i32],
    layouts: &[super::host::SimUiTextLayout],
    commands: &[super::host::SimUiCommand],
) {
    let mut painter = koto_core::CanvasUiPainter::new(canvas, font);
    for command in commands {
        match *command {
            super::host::SimUiCommand::Rect(index) => {
                if let Some(&(x, y, w, h, rgb565)) = rects.get(index) {
                    painter
                        .canvas_mut()
                        .fill_rect(Rect { x, y, w, h }, Rgb565(rgb565 as u16));
                }
            }
            super::host::SimUiCommand::Text(index) => {
                let (Some((_, _, body)), Some(layout)) = (text.get(index), layouts.get(index))
                else {
                    continue;
                };
                let color = match colors.get(index).copied().unwrap_or(TEXT_COLOR_DEFAULT) {
                    TEXT_COLOR_DEFAULT => koto_ui::Rgb565(APP_DEFAULT_TEXT_RGB565),
                    rgb565 => koto_ui::Rgb565(rgb565 as u16),
                };
                let _ = painter.draw_text(
                    layout.clip,
                    layout.bounds,
                    koto_ui::TextRun {
                        text: body,
                        color,
                        align: layout.align,
                    },
                );
            }
        }
    }
}

/// Paint one text list with its index-aligned colour list, mapping the
/// colourless-`draw_text` sentinel to the default foreground colour.
fn paint_text_list(
    canvas: &mut Canvas<'_>,
    font: &BitmapFont<'_>,
    text: &[(i32, i32, String)],
    colors: &[i32],
) {
    for (index, (x, y, body)) in text.iter().enumerate() {
        let color = match colors.get(index).copied().unwrap_or(TEXT_COLOR_DEFAULT) {
            TEXT_COLOR_DEFAULT => Rgb565(APP_DEFAULT_TEXT_RGB565),
            rgb565 => Rgb565(rgb565 as u16),
        };
        canvas.draw_text(*x, *y, font, body, color);
    }
}

/// Run an app (scripted, or to idle/exit when `inputs` is empty) and composite
/// its final frame into a framebuffer. Used by `--app … --image` to capture a
/// running app's screen; end the script on a yielded frame to capture gameplay
/// rather than the cleared exit frame.
pub fn render_app_frame(
    root: impl AsRef<Path>,
    app_id: &str,
    inputs: &[VmInputSnapshot],
    font: &BitmapFont<'_>,
) -> Result<Framebuffer, AppRunError> {
    let mut session = BytecodeAppSession::launch(root, app_id)
        .map_err(|error| AppRunError::Launch(Box::new(AppFailureSummary::launch(app_id, error))))?;
    let trap = |session: &BytecodeAppSession| {
        AppRunError::Trap(Box::new(AppFailureSummary::trap(session.diagnostic())))
    };
    if inputs.is_empty() {
        while !session.has_exited() && session.frame() < SIM_APP_IDLE_FRAME_CAP {
            if session.step_frame(VmInputSnapshot::empty()).is_err() {
                return Err(trap(&session));
            }
        }
    } else {
        for input in inputs {
            if session.has_exited() {
                break;
            }
            if session.step_frame(*input).is_err() {
                return Err(trap(&session));
            }
        }
    }

    let mut framebuffer = Framebuffer::new(SHELL_SURFACE.width, SHELL_SURFACE.height);
    {
        let mut canvas = framebuffer.as_canvas();
        canvas.clear(Rgb565::from_rgb8(12, 14, 18));
        paint_app_session(&mut canvas, font, &session);
    }
    Ok(framebuffer)
}
