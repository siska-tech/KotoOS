use crate::{Rgb565, UiRect};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PaintError {
    InvalidClip,
    InvalidGeometry,
    Backend,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TextAlign {
    #[default]
    Start,
    Center,
    End,
}

/// A borrowed UTF-8 text run. Font selection remains backend-owned.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextRun<'a> {
    pub text: &'a str,
    pub color: Rgb565,
    pub align: TextAlign,
}

/// A borrowed pre-resolved glyph run for font-cache-aware callers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GlyphRun<'a> {
    pub glyphs: &'a [u16],
    pub color: Rgb565,
    pub spacing: i16,
}

/// Backend-neutral drawing contract used by KotoUI components.
///
/// Every operation carries an explicit clip. Implementations must not paint
/// outside `clip.intersection(bounds)` and must not retain borrowed run data.
pub trait TextMetrics {
    /// Measures a borrowed UTF-8 run in pixels using the active backend font.
    fn measure_text(&mut self, text: &str) -> Result<i32, PaintError>;

    /// Returns the active font's line height when the backend exposes it.
    fn line_height(&self) -> Option<i32> {
        None
    }

    /// Reports optional glyph support when the backend can answer cheaply.
    /// Callers must retain a deterministic fallback when this returns false.
    fn supports_glyph(&self, _ch: char) -> bool {
        false
    }
}

pub trait Painter: TextMetrics {
    fn fill_rect(&mut self, clip: UiRect, rect: UiRect, color: Rgb565) -> Result<(), PaintError>;

    fn stroke_rect(
        &mut self,
        clip: UiRect,
        rect: UiRect,
        color: Rgb565,
        width: u8,
    ) -> Result<(), PaintError>;

    fn draw_text(
        &mut self,
        clip: UiRect,
        bounds: UiRect,
        run: TextRun<'_>,
    ) -> Result<(), PaintError>;

    fn draw_glyphs(
        &mut self,
        clip: UiRect,
        bounds: UiRect,
        run: GlyphRun<'_>,
    ) -> Result<(), PaintError>;

    fn draw_focus_mark(
        &mut self,
        clip: UiRect,
        rect: UiRect,
        color: Rgb565,
        width: u8,
    ) -> Result<(), PaintError>;
}
