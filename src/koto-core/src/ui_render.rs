//! KotoUI adapter over KotoGFX's shared RGB565 canvas and bitmap font.

use koto_ui::{GlyphRun, PaintError, Painter, TextAlign, TextMetrics, TextRun, UiContext, UiRect};

use crate::{
    BitmapFont, Canvas, PixelFormat, Rect, RenderCommand, RenderCommandList, RenderError,
    RenderSurface, Rgb565,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiRenderError {
    UnsupportedFormat,
    CommandCapacity,
    Render(RenderError),
}

impl From<RenderError> for UiRenderError {
    fn from(value: RenderError) -> Self {
        Self::Render(value)
    }
}

/// Borrowed adapter with no framebuffer or font storage of its own.
pub struct CanvasUiPainter<'a, 'pixels, 'font> {
    canvas: &'a mut Canvas<'pixels>,
    font: &'a BitmapFont<'font>,
}

impl<'a, 'pixels, 'font> CanvasUiPainter<'a, 'pixels, 'font> {
    pub fn new(canvas: &'a mut Canvas<'pixels>, font: &'a BitmapFont<'font>) -> Self {
        Self { canvas, font }
    }

    pub fn canvas(&self) -> &Canvas<'pixels> {
        self.canvas
    }

    pub fn canvas_mut(&mut self) -> &mut Canvas<'pixels> {
        self.canvas
    }

    fn effective_clip(&self, clip: UiRect, bounds: UiRect) -> Option<UiRect> {
        clip.intersection(bounds)?.intersection(UiRect::new(
            0,
            0,
            i32::from(self.canvas.width()),
            i32::from(self.canvas.height()),
        ))
    }

    fn measure_spaced(&self, text: &str, spacing: i32) -> Result<i32, PaintError> {
        if text.contains('\n') {
            return Err(PaintError::InvalidGeometry);
        }
        let mut width = 0i32;
        let mut count = 0usize;
        for ch in text.chars() {
            let advance = self
                .font
                .glyph(ch)
                .or_else(|| self.font.glyph('\u{fffd}'))
                .or_else(|| self.font.glyph('?'))
                .map_or(self.font.half_width(), |glyph| glyph.width());
            width = width
                .checked_add(i32::from(advance))
                .ok_or(PaintError::InvalidGeometry)?;
            count += 1;
        }
        if count > 1 {
            let gaps = i32::try_from(count - 1).map_err(|_| PaintError::InvalidGeometry)?;
            width = width
                .checked_add(
                    spacing
                        .checked_mul(gaps)
                        .ok_or(PaintError::InvalidGeometry)?,
                )
                .ok_or(PaintError::InvalidGeometry)?;
        }
        (width >= 0)
            .then_some(width)
            .ok_or(PaintError::InvalidGeometry)
    }

    fn draw_text_inner(
        &mut self,
        clip: UiRect,
        bounds: UiRect,
        text: &str,
        color: koto_ui::Rgb565,
        align: TextAlign,
        spacing: i32,
    ) -> Result<(), PaintError> {
        let Some(clip) = self.effective_clip(clip, bounds) else {
            return Ok(());
        };
        let width = self.measure_spaced(text, spacing)?;
        let mut x = match align {
            TextAlign::Start => bounds.x,
            TextAlign::Center => bounds.x.saturating_add((bounds.w - width) / 2),
            TextAlign::End => bounds.x.saturating_add(bounds.w - width),
        };
        let y = bounds
            .y
            .saturating_add((bounds.h - i32::from(self.font.cell_height())) / 2);
        for ch in text.chars() {
            if let Some(glyph) = self
                .font
                .glyph(ch)
                .or_else(|| self.font.glyph('\u{fffd}'))
                .or_else(|| self.font.glyph('?'))
            {
                for gy in 0..glyph.height() {
                    for gx in 0..glyph.width() {
                        let px = x.saturating_add(i32::from(gx));
                        let py = y.saturating_add(i32::from(gy));
                        if contains_point(clip, px, py) && glyph.pixel(gx, gy) {
                            self.canvas.put_pixel(px, py, Rgb565(color.0));
                        }
                    }
                }
                x = x.saturating_add(i32::from(glyph.width()));
            } else {
                x = x.saturating_add(i32::from(self.font.half_width()));
            }
            x = x.saturating_add(spacing);
        }
        Ok(())
    }

    fn stroke(&mut self, clip: UiRect, rect: UiRect, color: koto_ui::Rgb565, width: u8) {
        let width = i32::from(width).min(rect.w.max(0)).min(rect.h.max(0));
        if width <= 0 {
            return;
        }
        let edges = [
            UiRect::new(rect.x, rect.y, rect.w, width),
            UiRect::new(
                rect.x,
                rect.y.saturating_add(rect.h).saturating_sub(width),
                rect.w,
                width,
            ),
            UiRect::new(
                rect.x,
                rect.y.saturating_add(width),
                width,
                rect.h - width * 2,
            ),
            UiRect::new(
                rect.x.saturating_add(rect.w).saturating_sub(width),
                rect.y.saturating_add(width),
                width,
                rect.h - width * 2,
            ),
        ];
        for edge in edges {
            if let Some(fill) = self.effective_clip(clip, edge) {
                self.canvas.fill_rect(to_rect(fill), Rgb565(color.0));
            }
        }
    }
}

impl TextMetrics for CanvasUiPainter<'_, '_, '_> {
    fn measure_text(&mut self, text: &str) -> Result<i32, PaintError> {
        self.measure_spaced(text, 0)
    }

    fn supports_glyph(&self, ch: char) -> bool {
        self.font.glyph(ch).is_some()
    }

    fn line_height(&self) -> Option<i32> {
        Some(i32::from(self.font.cell_height()))
    }
}

impl Painter for CanvasUiPainter<'_, '_, '_> {
    fn fill_rect(
        &mut self,
        clip: UiRect,
        rect: UiRect,
        color: koto_ui::Rgb565,
    ) -> Result<(), PaintError> {
        if let Some(fill) = self.effective_clip(clip, rect) {
            self.canvas.fill_rect(to_rect(fill), Rgb565(color.0));
        }
        Ok(())
    }

    fn stroke_rect(
        &mut self,
        clip: UiRect,
        rect: UiRect,
        color: koto_ui::Rgb565,
        width: u8,
    ) -> Result<(), PaintError> {
        self.stroke(clip, rect, color, width);
        Ok(())
    }

    fn draw_text(
        &mut self,
        clip: UiRect,
        bounds: UiRect,
        run: TextRun<'_>,
    ) -> Result<(), PaintError> {
        self.draw_text_inner(clip, bounds, run.text, run.color, run.align, 0)
    }

    fn draw_glyphs(
        &mut self,
        clip: UiRect,
        bounds: UiRect,
        run: GlyphRun<'_>,
    ) -> Result<(), PaintError> {
        let mut x = bounds.x;
        for id in run.glyphs {
            let remaining = (i64::from(bounds.x) + i64::from(bounds.w) - i64::from(x))
                .clamp(0, i64::from(i32::MAX)) as i32;
            if remaining == 0 {
                break;
            }
            let ch = char::from_u32(u32::from(*id)).ok_or(PaintError::InvalidGeometry)?;
            let mut encoded = [0; 4];
            let text = ch.encode_utf8(&mut encoded);
            self.draw_text_inner(
                clip,
                UiRect::new(x, bounds.y, remaining, bounds.h),
                text,
                run.color,
                TextAlign::Start,
                0,
            )?;
            let advance = self
                .font
                .glyph(ch)
                .or_else(|| self.font.glyph('\u{fffd}'))
                .or_else(|| self.font.glyph('?'))
                .map_or(self.font.half_width(), |glyph| glyph.width());
            x = x
                .saturating_add(i32::from(advance))
                .saturating_add(i32::from(run.spacing));
        }
        Ok(())
    }

    fn draw_focus_mark(
        &mut self,
        clip: UiRect,
        rect: UiRect,
        color: koto_ui::Rgb565,
        width: u8,
    ) -> Result<(), PaintError> {
        self.stroke(clip, rect, color, width);
        Ok(())
    }
}

/// Replays the same component tree once per declared damage clip; idle is zero calls.
pub fn paint_ui_damage<const DAMAGE: usize, P: Painter>(
    context: &UiContext<DAMAGE>,
    painter: &mut P,
    mut paint: impl FnMut(&mut P, UiRect) -> Result<(), PaintError>,
) -> Result<usize, PaintError> {
    let mut count = 0;
    for clip in context.damaged_rects() {
        paint(painter, clip)?;
        count += 1;
    }
    Ok(count)
}

/// Converts damage into existing render requests, falling back to one full request.
pub fn ui_damage_commands<const DAMAGE: usize, const COMMANDS: usize>(
    context: &UiContext<DAMAGE>,
    surface: RenderSurface,
) -> Result<RenderCommandList<COMMANDS>, UiRenderError> {
    if surface.format != PixelFormat::Rgb565 {
        return Err(UiRenderError::UnsupportedFormat);
    }
    let visible = context
        .damaged_rects()
        .filter(|rect| clip_to_surface(*rect, surface).is_some())
        .count();
    let mut commands = RenderCommandList::new();
    if visible > COMMANDS {
        if COMMANDS == 0 {
            return Err(UiRenderError::CommandCapacity);
        }
        commands.push(RenderCommand::full(surface)?)?;
        return Ok(commands);
    }
    for rect in context.damaged_rects() {
        if let Some(rect) = clip_to_surface(rect, surface) {
            commands.push(RenderCommand::rect(surface, rect)?)?;
        }
    }
    Ok(commands)
}

fn clip_to_surface(rect: UiRect, surface: RenderSurface) -> Option<Rect> {
    Rect::clip(
        rect.x,
        rect.y,
        rect.w,
        rect.h,
        i32::from(surface.width),
        i32::from(surface.height),
    )
}

fn contains_point(rect: UiRect, x: i32, y: i32) -> bool {
    x >= rect.x
        && y >= rect.y
        && i64::from(x) < i64::from(rect.x) + i64::from(rect.w)
        && i64::from(y) < i64::from(rect.y) + i64::from(rect.h)
}

const fn to_rect(rect: UiRect) -> Rect {
    Rect {
        x: rect.x,
        y: rect.y,
        w: rect.w,
        h: rect.h,
    }
}

#[cfg(target_pointer_width = "32")]
const _: [(); 8] = [(); core::mem::size_of::<CanvasUiPainter<'static, 'static, 'static>>()];
