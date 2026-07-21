use crate::{PaintError, Painter, Rgb565, TextAlign, TextRun, Theme, UiRect};

use super::{clipped, paint_frame};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LayoutError {
    Empty,
    Overflow,
    OutOfBounds,
    ZeroCount,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(C)]
pub struct Insets {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Insets {
    pub const fn uniform(value: i32) -> Self {
        Self {
            left: value,
            top: value,
            right: value,
            bottom: value,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PanelLayout {
    pub frame: UiRect,
    pub title: Option<UiRect>,
    pub content: UiRect,
}

impl PanelLayout {
    pub fn inset(rect: UiRect, insets: Insets) -> Result<UiRect, LayoutError> {
        if rect.is_empty()
            || insets.left < 0
            || insets.top < 0
            || insets.right < 0
            || insets.bottom < 0
        {
            return Err(LayoutError::Empty);
        }
        rect_from_edges(
            i64::from(rect.x) + i64::from(insets.left),
            i64::from(rect.y) + i64::from(insets.top),
            i64::from(rect.x) + i64::from(rect.w) - i64::from(insets.right),
            i64::from(rect.y) + i64::from(rect.h) - i64::from(insets.bottom),
        )
    }

    pub fn row(
        content: UiRect,
        index: usize,
        height: i32,
        gap: i32,
    ) -> Result<UiRect, LayoutError> {
        if content.is_empty() || height <= 0 || gap < 0 {
            return Err(LayoutError::Empty);
        }
        let stride = i64::from(height)
            .checked_add(i64::from(gap))
            .ok_or(LayoutError::Overflow)?;
        let offset = stride
            .checked_mul(i64::try_from(index).map_err(|_| LayoutError::Overflow)?)
            .ok_or(LayoutError::Overflow)?;
        let top = i64::from(content.y)
            .checked_add(offset)
            .ok_or(LayoutError::Overflow)?;
        let row = rect_from_edges(
            i64::from(content.x),
            top,
            i64::from(content.x) + i64::from(content.w),
            top.checked_add(i64::from(height))
                .ok_or(LayoutError::Overflow)?,
        )?;
        if !content.contains(row) {
            return Err(LayoutError::OutOfBounds);
        }
        Ok(row)
    }

    pub fn button(
        row: UiRect,
        index: usize,
        count: usize,
        gap: i32,
    ) -> Result<UiRect, LayoutError> {
        if row.is_empty() || gap < 0 {
            return Err(LayoutError::Empty);
        }
        if count == 0 || index >= count {
            return Err(LayoutError::ZeroCount);
        }
        let count_i64 = i64::try_from(count).map_err(|_| LayoutError::Overflow)?;
        let total_gap = i64::from(gap)
            .checked_mul(count_i64 - 1)
            .ok_or(LayoutError::Overflow)?;
        let available = i64::from(row.w) - total_gap;
        if available < count_i64 {
            return Err(LayoutError::OutOfBounds);
        }
        let base = available / count_i64;
        let remainder = available % count_i64;
        let index_i64 = i64::try_from(index).map_err(|_| LayoutError::Overflow)?;
        let prior_extra = index_i64.min(remainder);
        let width = base + i64::from(index_i64 < remainder);
        let left = i64::from(row.x)
            + index_i64
                .checked_mul(base + i64::from(gap))
                .ok_or(LayoutError::Overflow)?
            + prior_extra;
        rect_from_edges(
            left,
            i64::from(row.y),
            left + width,
            i64::from(row.y) + i64::from(row.h),
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct Panel<'a> {
    bounds: UiRect,
    backdrop: Option<UiRect>,
    title: Option<&'a str>,
    padding: u8,
    title_height: u8,
}

impl<'a> Panel<'a> {
    pub const fn new(bounds: UiRect) -> Self {
        Self {
            bounds,
            backdrop: None,
            title: None,
            padding: 4,
            title_height: 20,
        }
    }

    pub const fn with_title(mut self, title: &'a str) -> Self {
        self.title = Some(title);
        self
    }

    pub const fn with_padding(mut self, padding: u8) -> Self {
        self.padding = padding;
        self
    }

    pub const fn with_title_height(mut self, height: u8) -> Self {
        self.title_height = height;
        self
    }

    pub const fn with_dimmed_backdrop(mut self, backdrop: UiRect) -> Self {
        self.backdrop = Some(backdrop);
        self
    }

    pub const fn bounds(&self) -> UiRect {
        self.bounds
    }

    pub const fn backdrop(&self) -> Option<UiRect> {
        self.backdrop
    }

    pub fn repaint_region(&self) -> UiRect {
        self.backdrop
            .and_then(|backdrop| backdrop.union(self.bounds))
            .unwrap_or(self.bounds)
    }

    pub fn layout(&self, theme: &Theme) -> Result<PanelLayout, LayoutError> {
        if self.bounds.is_empty() {
            return Err(LayoutError::Empty);
        }
        let inset = i32::from(self.padding)
            .checked_add(i32::from(theme.border_width))
            .ok_or(LayoutError::Overflow)?;
        let inner = PanelLayout::inset(self.bounds, Insets::uniform(inset))?;
        let (title, content) = if self.title.is_some() {
            if self.title_height == 0 {
                return Err(LayoutError::Empty);
            }
            let title = PanelLayout::row(inner, 0, i32::from(self.title_height), 0)?;
            let content = rect_from_edges(
                i64::from(inner.x),
                i64::from(title.y) + i64::from(title.h),
                i64::from(inner.x) + i64::from(inner.w),
                i64::from(inner.y) + i64::from(inner.h),
            )?;
            (Some(title), content)
        } else {
            (None, inner)
        };
        Ok(PanelLayout {
            frame: self.bounds,
            title,
            content,
        })
    }

    pub fn paint(
        &self,
        painter: &mut impl Painter,
        clip: UiRect,
        theme: &Theme,
    ) -> Result<(), PaintError> {
        let layout = self
            .layout(theme)
            .map_err(|_| PaintError::InvalidGeometry)?;
        if let Some(backdrop) = self.backdrop.and_then(|bounds| clipped(bounds, clip)) {
            painter.fill_rect(backdrop, backdrop, Rgb565::from_rgb8(8, 10, 16))?;
        }
        let Some(effective_clip) = clipped(self.bounds, clip) else {
            return Ok(());
        };
        paint_frame(painter, effective_clip, self.bounds, theme.normal, theme)?;
        if let (Some(title), Some(title_bounds)) = (self.title, layout.title) {
            painter.draw_text(
                effective_clip,
                title_bounds,
                TextRun {
                    text: title,
                    color: theme.normal.foreground,
                    align: TextAlign::Start,
                },
            )?;
            painter.fill_rect(
                effective_clip,
                UiRect::new(
                    title_bounds.x,
                    title_bounds.y + title_bounds.h - 1,
                    title_bounds.w,
                    1,
                ),
                theme.normal.border,
            )?;
        }
        Ok(())
    }
}

fn rect_from_edges(left: i64, top: i64, right: i64, bottom: i64) -> Result<UiRect, LayoutError> {
    let width = right.checked_sub(left).ok_or(LayoutError::Overflow)?;
    let height = bottom.checked_sub(top).ok_or(LayoutError::Overflow)?;
    if width <= 0 || height <= 0 {
        return Err(LayoutError::Empty);
    }
    Ok(UiRect::new(
        i32::try_from(left).map_err(|_| LayoutError::Overflow)?,
        i32::try_from(top).map_err(|_| LayoutError::Overflow)?,
        i32::try_from(width).map_err(|_| LayoutError::Overflow)?,
        i32::try_from(height).map_err(|_| LayoutError::Overflow)?,
    ))
}

#[cfg(target_pointer_width = "32")]
const _: [(); 48] = [(); core::mem::size_of::<Panel<'static>>()];

#[cfg(test)]
mod tests {
    use core::mem::size_of;

    use super::*;
    use crate::{GlyphRun, TextMetrics};

    #[derive(Clone, Debug, Eq, PartialEq)]
    enum Op {
        Fill(UiRect, UiRect, Rgb565),
        Stroke(UiRect, UiRect),
        Text(UiRect, UiRect, String),
    }

    #[derive(Default)]
    struct Recorder {
        ops: Vec<Op>,
    }

    impl TextMetrics for Recorder {
        fn measure_text(&mut self, text: &str) -> Result<i32, PaintError> {
            Ok(text.len() as i32 * 6)
        }
    }

    impl Painter for Recorder {
        fn fill_rect(
            &mut self,
            clip: UiRect,
            rect: UiRect,
            color: Rgb565,
        ) -> Result<(), PaintError> {
            self.ops.push(Op::Fill(clip, rect, color));
            Ok(())
        }

        fn stroke_rect(
            &mut self,
            clip: UiRect,
            rect: UiRect,
            _: Rgb565,
            _: u8,
        ) -> Result<(), PaintError> {
            self.ops.push(Op::Stroke(clip, rect));
            Ok(())
        }

        fn draw_text(
            &mut self,
            clip: UiRect,
            bounds: UiRect,
            run: TextRun<'_>,
        ) -> Result<(), PaintError> {
            self.ops.push(Op::Text(clip, bounds, run.text.into()));
            Ok(())
        }

        fn draw_glyphs(&mut self, _: UiRect, _: UiRect, _: GlyphRun<'_>) -> Result<(), PaintError> {
            Ok(())
        }

        fn draw_focus_mark(
            &mut self,
            _: UiRect,
            _: UiRect,
            _: Rgb565,
            _: u8,
        ) -> Result<(), PaintError> {
            Ok(())
        }
    }

    #[test]
    fn titled_panel_computes_absolute_content_geometry() {
        let panel = Panel::new(UiRect::new(10, 20, 100, 80))
            .with_title("Open")
            .with_padding(4)
            .with_title_height(16);
        assert_eq!(
            panel.layout(&Theme::DARK),
            Ok(PanelLayout {
                frame: UiRect::new(10, 20, 100, 80),
                title: Some(UiRect::new(15, 25, 90, 16)),
                content: UiRect::new(15, 41, 90, 54),
            })
        );
    }

    #[test]
    fn rows_and_buttons_are_deterministic_and_tile_available_width() {
        let content = UiRect::new(10, 20, 101, 70);
        assert_eq!(
            PanelLayout::row(content, 1, 20, 5),
            Ok(UiRect::new(10, 45, 101, 20))
        );
        let row = UiRect::new(10, 70, 101, 20);
        let buttons = [
            PanelLayout::button(row, 0, 3, 4).unwrap(),
            PanelLayout::button(row, 1, 3, 4).unwrap(),
            PanelLayout::button(row, 2, 3, 4).unwrap(),
        ];
        assert_eq!(
            buttons,
            [
                UiRect::new(10, 70, 31, 20),
                UiRect::new(45, 70, 31, 20),
                UiRect::new(80, 70, 31, 20)
            ]
        );
    }

    #[test]
    fn impossible_layouts_fail_without_partial_geometry() {
        assert_eq!(
            PanelLayout::inset(UiRect::new(0, 0, 4, 4), Insets::uniform(2)),
            Err(LayoutError::Empty)
        );
        assert_eq!(
            PanelLayout::row(UiRect::new(0, 0, 20, 20), 1, 15, 2),
            Err(LayoutError::OutOfBounds)
        );
        assert_eq!(
            PanelLayout::button(UiRect::new(0, 0, 3, 10), 0, 3, 2),
            Err(LayoutError::OutOfBounds)
        );
    }

    #[test]
    fn backdrop_and_partially_visible_panel_use_surface_clip() {
        let surface = UiRect::new(0, 0, 40, 40);
        let panel = Panel::new(UiRect::new(20, 20, 40, 40))
            .with_title("Clip")
            .with_dimmed_backdrop(UiRect::new(-10, -10, 70, 70));
        let mut painter = Recorder::default();
        panel.paint(&mut painter, surface, &Theme::DARK).unwrap();
        assert!(
            matches!(painter.ops[0], Op::Fill(clip, rect, _) if clip == surface && rect == surface)
        );
        assert!(painter.ops.iter().skip(1).all(|op| match op {
            Op::Fill(clip, _, _) | Op::Stroke(clip, _) | Op::Text(clip, _, _) =>
                *clip == UiRect::new(20, 20, 20, 20),
        }));
    }

    #[test]
    fn panel_memory_is_fixed() {
        assert_eq!(size_of::<Panel<'static>>(), 64);
    }
}
