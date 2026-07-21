use core::mem::size_of;

use koto_core::{
    paint_ui_damage, ui_damage_commands, BitmapFont, Canvas, CanvasUiPainter, PixelFormat, Rect,
    RenderSurface, RenderUpdate, UiRenderError,
};
use koto_ui::{
    Button, Checkbox, GlyphRun, PaintError, Painter, ResponseKind, TextMetrics, TextRun, Theme,
    UiAction, UiContext, UiEvent, UiRect, WidgetId,
};

fn font_bytes() -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(b"KFNT");
    data.extend_from_slice(&1u16.to_le_bytes());
    data.extend_from_slice(&0u16.to_le_bytes());
    data.extend_from_slice(&[5, 5, 3, 6]);
    data.extend_from_slice(&1u32.to_le_bytes());
    data.extend_from_slice(&(u32::from('A')).to_le_bytes());
    data.extend_from_slice(&[3, 1]);
    data.extend_from_slice(&0u32.to_le_bytes());
    data.extend_from_slice(&[
        0b0100_0000,
        0b1010_0000,
        0b1110_0000,
        0b1010_0000,
        0b1010_0000,
    ]);
    data
}

fn pixel(pixels: &[u8], width: usize, x: usize, y: usize) -> u16 {
    let index = (y * width + x) * 2;
    u16::from_le_bytes([pixels[index], pixels[index + 1]])
}

#[derive(Default)]
struct OperationRecorder {
    fills: usize,
    strokes: usize,
    texts: usize,
    focuses: usize,
}

impl TextMetrics for OperationRecorder {
    fn measure_text(&mut self, text: &str) -> Result<i32, PaintError> {
        Ok(text.len() as i32 * 3)
    }
}

impl Painter for OperationRecorder {
    fn fill_rect(&mut self, _: UiRect, _: UiRect, _: koto_ui::Rgb565) -> Result<(), PaintError> {
        self.fills += 1;
        Ok(())
    }

    fn stroke_rect(
        &mut self,
        _: UiRect,
        _: UiRect,
        _: koto_ui::Rgb565,
        _: u8,
    ) -> Result<(), PaintError> {
        self.strokes += 1;
        Ok(())
    }

    fn draw_text(&mut self, _: UiRect, _: UiRect, _: TextRun<'_>) -> Result<(), PaintError> {
        self.texts += 1;
        Ok(())
    }

    fn draw_glyphs(&mut self, _: UiRect, _: UiRect, _: GlyphRun<'_>) -> Result<(), PaintError> {
        Ok(())
    }

    fn draw_focus_mark(
        &mut self,
        _: UiRect,
        _: UiRect,
        _: koto_ui::Rgb565,
        _: u8,
    ) -> Result<(), PaintError> {
        self.focuses += 1;
        Ok(())
    }
}

#[test]
fn recording_operations_map_to_rgb565_pixels() {
    let bytes = font_bytes();
    let font = BitmapFont::from_bytes(&bytes).unwrap();
    let surface = UiRect::new(0, 0, 24, 14);
    let bounds = UiRect::new(2, 2, 20, 10);
    let mut context = UiContext::<8>::new(surface, Theme::DARK);
    let mut button = Button::new(WidgetId::new(1), bounds, "A");
    button.set_focused(true, &mut context);

    let mut recording = OperationRecorder::default();
    button.paint(&mut recording, surface, &Theme::DARK).unwrap();
    assert_eq!(
        (
            recording.fills,
            recording.strokes,
            recording.texts,
            recording.focuses
        ),
        (1, 1, 1, 1)
    );

    let mut pixels = vec![0; 24 * 14 * 2];
    let mut canvas = Canvas::new(&mut pixels, 24, 14).unwrap();
    let mut painter = CanvasUiPainter::new(&mut canvas, &font);
    button.paint(&mut painter, surface, &Theme::DARK).unwrap();
    assert_eq!(pixel(&pixels, 24, 2, 2), Theme::DARK.focus.0);
    assert_eq!(pixel(&pixels, 24, 3, 3), Theme::DARK.focused.background.0);
    assert!(pixels.chunks_exact(2).any(|bytes| {
        u16::from_le_bytes([bytes[0], bytes[1]]) == Theme::DARK.focused.foreground.0
    }));
}

#[test]
fn simulator_full_canvas_and_device_viewport_are_pixel_identical() {
    let bytes = font_bytes();
    let font = BitmapFont::from_bytes(&bytes).unwrap();
    let damage = UiRect::new(4, 3, 12, 8);
    let checkbox = Checkbox::new(WidgetId::new(2), UiRect::new(2, 2, 20, 10), "A");
    let mut full = vec![0; 24 * 14 * 2];
    let mut strip = vec![0; damage.w as usize * damage.h as usize * 2];
    {
        let mut canvas = Canvas::new(&mut full, 24, 14).unwrap();
        let mut painter = CanvasUiPainter::new(&mut canvas, &font);
        checkbox.paint(&mut painter, damage, &Theme::DARK).unwrap();
    }
    {
        let mut canvas = Canvas::new_viewport(
            &mut strip,
            24,
            14,
            Rect {
                x: damage.x,
                y: damage.y,
                w: damage.w,
                h: damage.h,
            },
        )
        .unwrap();
        let mut painter = CanvasUiPainter::new(&mut canvas, &font);
        checkbox.paint(&mut painter, damage, &Theme::DARK).unwrap();
    }
    for y in 0..damage.h as usize {
        let full_start = ((damage.y as usize + y) * 24 + damage.x as usize) * 2;
        let strip_start = y * damage.w as usize * 2;
        assert_eq!(
            &full[full_start..full_start + damage.w as usize * 2],
            &strip[strip_start..strip_start + damage.w as usize * 2]
        );
    }
}

#[test]
fn damage_conversion_is_idle_precise_and_bounded() {
    let surface = RenderSurface::new(32, 24, PixelFormat::Rgb565);
    let mut context = UiContext::<4>::new(UiRect::new(0, 0, 32, 24), Theme::DARK);
    assert!(ui_damage_commands::<4, 2>(&context, surface)
        .unwrap()
        .is_empty());
    context.damage(UiRect::new(-2, 2, 6, 5));
    assert_eq!(
        ui_damage_commands::<4, 2>(&context, surface)
            .unwrap()
            .iter()
            .next()
            .unwrap()
            .update,
        RenderUpdate::Rect(Rect {
            x: 0,
            y: 2,
            w: 4,
            h: 5
        })
    );
    context.clear_damage();
    context.damage(UiRect::new(0, 0, 2, 2));
    context.damage(UiRect::new(10, 10, 2, 2));
    assert_eq!(
        ui_damage_commands::<4, 1>(&context, surface)
            .unwrap()
            .iter()
            .next()
            .unwrap()
            .update,
        RenderUpdate::Full
    );
    assert_eq!(
        ui_damage_commands::<4, 0>(&context, surface),
        Err(UiRenderError::CommandCapacity)
    );
}

#[test]
fn idle_and_checkbox_transition_replay_only_declared_clips() {
    let mut context = UiContext::<8>::new(UiRect::new(0, 0, 32, 24), Theme::DARK);
    let mut recorder = OperationRecorder::default();
    let mut clips = Vec::new();
    assert_eq!(
        paint_ui_damage(&context, &mut recorder, |_, clip| {
            clips.push(clip);
            Ok(())
        })
        .unwrap(),
        0
    );
    let mut checkbox = Checkbox::new(WidgetId::new(3), UiRect::new(4, 5, 12, 8), "A");
    checkbox.set_focused(true, &mut context);
    context.clear_damage();
    assert_eq!(
        checkbox
            .handle_event(UiEvent::pressed(UiAction::Activate), &mut context)
            .unwrap()
            .kind,
        ResponseKind::ValueChanged(1)
    );
    assert_eq!(
        paint_ui_damage(&context, &mut recorder, |_, clip| {
            clips.push(clip);
            Ok(())
        })
        .unwrap(),
        1
    );
    assert_eq!(clips, [UiRect::new(4, 5, 12, 8)]);
}

#[test]
fn adapter_has_no_owned_surface_storage() {
    assert_eq!(size_of::<CanvasUiPainter<'static, 'static, 'static>>(), 16);
}

#[test]
fn dependency_direction_keeps_ui_below_core_integration() {
    let ui_manifest = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../koto-ui/Cargo.toml"
    ))
    .unwrap();
    assert!(!ui_manifest.contains("koto-core"));
    assert!(!ui_manifest.contains("koto-gfx"));
    let core_manifest =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml")).unwrap();
    assert!(core_manifest.contains("koto-gfx"));
    assert!(core_manifest.contains("koto-ui"));
}
