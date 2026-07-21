use crate::{
    EventPhase, Navigation, PaintError, Painter, ResponseKind, TextAlign, TextRun, Theme, UiAction,
    UiContext, UiEvent, UiRect, UiResponse, VisualState, WidgetId,
};

use super::{clipped, control_style, paint_frame};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BufferError {
    Capacity,
    InvalidBoundary,
    InvalidRange,
    InvalidUtf8,
}

/// A valid UTF-8 string stored in caller-owned, fixed-capacity memory.
pub struct Utf8Buffer<'a> {
    storage: &'a mut [u8],
    len: usize,
}

impl<'a> Utf8Buffer<'a> {
    pub fn new(storage: &'a mut [u8]) -> Self {
        Self { storage, len: 0 }
    }

    pub fn from_str(storage: &'a mut [u8], value: &str) -> Result<Self, BufferError> {
        if value.len() > storage.len() {
            return Err(BufferError::Capacity);
        }
        storage[..value.len()].copy_from_slice(value.as_bytes());
        Ok(Self {
            storage,
            len: value.len(),
        })
    }

    pub fn from_initialized(storage: &'a mut [u8], len: usize) -> Result<Self, BufferError> {
        let bytes = storage.get(..len).ok_or(BufferError::InvalidRange)?;
        core::str::from_utf8(bytes).map_err(|_| BufferError::InvalidUtf8)?;
        Ok(Self { storage, len })
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.storage[..self.len])
            .expect("Utf8Buffer operations preserve UTF-8")
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn capacity(&self) -> usize {
        self.storage.len()
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn insert_str(&mut self, index: usize, text: &str) -> Result<(), BufferError> {
        if !self.as_str().is_char_boundary(index) {
            return Err(BufferError::InvalidBoundary);
        }
        let new_len = self
            .len
            .checked_add(text.len())
            .filter(|len| *len <= self.storage.len())
            .ok_or(BufferError::Capacity)?;
        self.storage
            .copy_within(index..self.len, index + text.len());
        self.storage[index..index + text.len()].copy_from_slice(text.as_bytes());
        self.len = new_len;
        Ok(())
    }

    pub fn remove_range(&mut self, start: usize, end: usize) -> Result<(), BufferError> {
        if start > end || end > self.len {
            return Err(BufferError::InvalidRange);
        }
        if !self.as_str().is_char_boundary(start) || !self.as_str().is_char_boundary(end) {
            return Err(BufferError::InvalidBoundary);
        }
        self.storage.copy_within(end..self.len, start);
        self.len -= end - start;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ImeComposition<'a> {
    pub text: &'a str,
    pub candidate: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct TextField<'a> {
    id: WidgetId,
    bounds: UiRect,
    placeholder: &'a str,
    semantic_label: Option<&'a str>,
    cursor: usize,
    view_start: usize,
    enabled: bool,
    focused: bool,
    editing: bool,
}

#[cfg(target_pointer_width = "32")]
const _: [(); 12] = [(); core::mem::size_of::<Utf8Buffer<'static>>()];
#[cfg(target_pointer_width = "32")]
const _: [(); 48] = [(); core::mem::size_of::<TextField<'static>>()];
#[cfg(target_pointer_width = "32")]
const _: [(); 16] = [(); core::mem::size_of::<ImeComposition<'static>>()];

impl<'a> TextField<'a> {
    pub const fn new(id: WidgetId, bounds: UiRect, placeholder: &'a str) -> Self {
        Self {
            id,
            bounds,
            placeholder,
            semantic_label: None,
            cursor: 0,
            view_start: 0,
            enabled: true,
            focused: false,
            editing: false,
        }
    }

    pub const fn with_semantic_label(mut self, label: &'a str) -> Self {
        self.semantic_label = Some(label);
        self
    }

    pub const fn cursor(&self) -> usize {
        self.cursor
    }

    pub const fn view_start(&self) -> usize {
        self.view_start
    }

    pub const fn semantic_label(&self) -> &'a str {
        match self.semantic_label {
            Some(label) => label,
            None => self.placeholder,
        }
    }

    pub const fn visual_state(&self) -> VisualState {
        if !self.enabled {
            VisualState::Disabled
        } else if self.editing {
            VisualState::Focused
        } else {
            VisualState::Normal
        }
    }

    pub const fn is_editing(&self) -> bool {
        self.editing
    }

    pub fn set_enabled<const N: usize>(&mut self, enabled: bool, context: &mut UiContext<N>) {
        if self.enabled != enabled {
            self.enabled = enabled;
            if !enabled {
                self.editing = false;
            }
            context.damage(self.bounds);
        }
    }

    pub fn set_focused<const N: usize>(&mut self, focused: bool, context: &mut UiContext<N>) {
        if self.focused != focused {
            self.focused = focused;
            if !focused {
                self.editing = false;
            }
            context.damage(self.bounds);
        }
    }

    pub fn set_editing<const N: usize>(&mut self, editing: bool, context: &mut UiContext<N>) {
        let editing = editing && self.enabled && self.focused;
        if self.editing != editing {
            self.editing = editing;
            context.damage(self.bounds);
        }
    }

    pub fn set_cursor<const N: usize>(
        &mut self,
        value: &Utf8Buffer<'_>,
        cursor: usize,
        context: &mut UiContext<N>,
    ) -> Result<(), BufferError> {
        if !value.as_str().is_char_boundary(cursor) {
            return Err(BufferError::InvalidBoundary);
        }
        if self.cursor != cursor {
            self.cursor = cursor;
            context.damage(self.bounds);
        }
        Ok(())
    }

    /// Call when a separately-owned IME snapshot changes.
    pub fn invalidate_composition<const N: usize>(&self, context: &mut UiContext<N>) {
        context.damage(self.bounds);
    }

    /// Call after replacing the caller-owned value outside `handle_event`.
    pub fn invalidate_value<const N: usize>(
        &mut self,
        value: &Utf8Buffer<'_>,
        context: &mut UiContext<N>,
    ) {
        self.normalize_cursor(value.as_str());
        context.damage(self.bounds);
    }

    pub fn handle_event<const N: usize>(
        &mut self,
        value: &mut Utf8Buffer<'_>,
        event: UiEvent,
        context: &mut UiContext<N>,
    ) -> Option<UiResponse> {
        if !self.enabled || !self.focused || event.phase == EventPhase::Released {
            return None;
        }
        if event.action == UiAction::Activate && event.phase == EventPhase::Pressed {
            self.set_editing(true, context);
            return None;
        }
        if event.action == UiAction::Cancel && self.editing && event.phase == EventPhase::Pressed {
            self.set_editing(false, context);
            return Some(UiResponse::new(self.id, ResponseKind::Cancelled));
        }
        if !self.editing {
            return None;
        }
        self.normalize_cursor(value.as_str());

        let before_cursor = self.cursor;
        let response = match event.action {
            UiAction::Text(ch) if event.phase == EventPhase::Pressed => {
                let mut encoded = [0; 4];
                let text = ch.encode_utf8(&mut encoded);
                match value.insert_str(self.cursor, text) {
                    Ok(()) => {
                        self.cursor += text.len();
                        Some(ResponseKind::TextChanged(value.len()))
                    }
                    Err(BufferError::Capacity) => Some(ResponseKind::CapacityRejected),
                    Err(_) => None,
                }
            }
            UiAction::Backspace => self.backspace(value),
            UiAction::Delete => self.delete(value),
            UiAction::Navigate(Navigation::Left) => {
                self.cursor = previous_boundary(value.as_str(), self.cursor);
                None
            }
            UiAction::Navigate(Navigation::Right) => {
                self.cursor = next_boundary(value.as_str(), self.cursor);
                None
            }
            UiAction::Home => {
                self.cursor = 0;
                None
            }
            UiAction::End => {
                self.cursor = value.len();
                None
            }
            UiAction::Submit if event.phase == EventPhase::Pressed => Some(ResponseKind::Submitted),
            UiAction::Cancel if event.phase == EventPhase::Pressed => Some(ResponseKind::Cancelled),
            _ => None,
        };

        if self.cursor != before_cursor || matches!(response, Some(ResponseKind::TextChanged(_))) {
            context.damage(self.bounds);
        }
        response.map(|kind| UiResponse::new(self.id, kind))
    }

    pub fn paint(
        &mut self,
        painter: &mut impl Painter,
        clip: UiRect,
        theme: &Theme,
        value: &Utf8Buffer<'_>,
        composition: Option<ImeComposition<'_>>,
    ) -> Result<(), PaintError> {
        let Some(clip) = clipped(self.bounds, clip) else {
            return Ok(());
        };
        self.normalize_cursor(value.as_str());
        let style = control_style(theme, self.visual_state());
        paint_frame(painter, clip, self.bounds, style, theme)?;
        let Some(content) = self.bounds.inset(i32::from(theme.spacing)) else {
            return Ok(());
        };

        let composition_width =
            composition_advance(painter, composition, i32::from(theme.spacing))?;
        self.keep_cursor_visible(painter, value.as_str(), content.w, composition_width)?;
        if value.is_empty() && composition.is_none() {
            if !self.placeholder.is_empty() {
                painter.draw_text(
                    clip,
                    content,
                    TextRun {
                        text: self.placeholder,
                        color: theme.disabled.foreground,
                        align: TextAlign::Start,
                    },
                )?;
            }
        } else {
            self.paint_value(
                painter,
                clip,
                content,
                (
                    style.foreground,
                    theme.accent,
                    theme.focus,
                    i32::from(theme.spacing),
                ),
                value,
                composition,
            )?;
        }

        if self.editing && self.enabled {
            let prefix = &value.as_str()[self.view_start..self.cursor];
            let cursor_x = content
                .x
                .saturating_add(measured(painter, prefix)?)
                .saturating_add(composition_width)
                .min(content.x.saturating_add(content.w).saturating_sub(1));
            let cursor_height = painter
                .line_height()
                .unwrap_or(content.h)
                .clamp(1, content.h);
            let cursor_y = content
                .y
                .saturating_add((content.h - cursor_height).max(0) / 2);
            painter.fill_rect(
                clip,
                UiRect::new(cursor_x, cursor_y, 1, cursor_height),
                theme.focus,
            )?;
        }
        if self.focused && self.enabled {
            painter.draw_focus_mark(clip, self.bounds, theme.focus, theme.focus_width)?;
        }
        Ok(())
    }

    fn backspace(&mut self, value: &mut Utf8Buffer<'_>) -> Option<ResponseKind> {
        let start = previous_boundary(value.as_str(), self.cursor);
        if start == self.cursor {
            return None;
        }
        value.remove_range(start, self.cursor).ok()?;
        self.cursor = start;
        Some(ResponseKind::TextChanged(value.len()))
    }

    fn delete(&mut self, value: &mut Utf8Buffer<'_>) -> Option<ResponseKind> {
        let end = next_boundary(value.as_str(), self.cursor);
        if end == self.cursor {
            return None;
        }
        value.remove_range(self.cursor, end).ok()?;
        Some(ResponseKind::TextChanged(value.len()))
    }

    fn normalize_cursor(&mut self, value: &str) {
        self.cursor = self.cursor.min(value.len());
        while !value.is_char_boundary(self.cursor) {
            self.cursor -= 1;
        }
        self.view_start = self.view_start.min(self.cursor);
        while !value.is_char_boundary(self.view_start) {
            self.view_start -= 1;
        }
    }

    fn keep_cursor_visible(
        &mut self,
        painter: &mut impl Painter,
        value: &str,
        width: i32,
        trailing_width: i32,
    ) -> Result<(), PaintError> {
        if self.cursor < self.view_start {
            self.view_start = self.cursor;
        }
        while self.view_start < self.cursor
            && measured(painter, &value[self.view_start..self.cursor])?
                .saturating_add(trailing_width)
                >= width
        {
            self.view_start = next_boundary(value, self.view_start);
        }
        Ok(())
    }

    fn paint_value(
        &self,
        painter: &mut impl Painter,
        clip: UiRect,
        content: UiRect,
        colors: (crate::Rgb565, crate::Rgb565, crate::Rgb565, i32),
        value: &Utf8Buffer<'_>,
        composition: Option<ImeComposition<'_>>,
    ) -> Result<(), PaintError> {
        let (foreground, composition_color, candidate_color, _spacing) = colors;
        let text = value.as_str();
        let prefix = &text[self.view_start..self.cursor];
        draw_at(painter, clip, content, content.x, prefix, foreground)?;
        let mut x = content.x.saturating_add(measured(painter, prefix)?);
        if let Some(ime) = composition {
            if let Some(candidate) = ime.candidate {
                // An IME candidate replaces its reading in-place. Rendering
                // both side by side makes the converted word look appended
                // and moves the caret past text that will never be committed.
                draw_at(painter, clip, content, x, candidate, candidate_color)?;
                x = x.saturating_add(measured(painter, candidate)?);
            } else {
                draw_at(painter, clip, content, x, ime.text, composition_color)?;
                x = x.saturating_add(measured(painter, ime.text)?);
            }
        }
        draw_at(painter, clip, content, x, &text[self.cursor..], foreground)
    }
}

fn composition_advance(
    painter: &mut impl Painter,
    composition: Option<ImeComposition<'_>>,
    _spacing: i32,
) -> Result<i32, PaintError> {
    let Some(composition) = composition else {
        return Ok(0);
    };
    if let Some(candidate) = composition.candidate {
        return measured(painter, candidate);
    }
    measured(painter, composition.text)
}

fn measured(painter: &mut impl Painter, text: &str) -> Result<i32, PaintError> {
    let width = painter.measure_text(text)?;
    if width < 0 {
        Err(PaintError::InvalidGeometry)
    } else {
        Ok(width)
    }
}

fn draw_at(
    painter: &mut impl Painter,
    clip: UiRect,
    content: UiRect,
    x: i32,
    text: &str,
    color: crate::Rgb565,
) -> Result<(), PaintError> {
    if text.is_empty() {
        return Ok(());
    }
    painter.draw_text(
        clip,
        UiRect::new(x, content.y, content.w, content.h),
        TextRun {
            text,
            color,
            align: TextAlign::Start,
        },
    )
}

fn previous_boundary(value: &str, index: usize) -> usize {
    value[..index]
        .char_indices()
        .next_back()
        .map_or(0, |(i, _)| i)
}

fn next_boundary(value: &str, index: usize) -> usize {
    value[index..]
        .char_indices()
        .nth(1)
        .map_or(value.len(), |(offset, _)| index + offset)
}

#[cfg(test)]
mod tests {
    use core::mem::size_of;

    use super::*;
    use crate::{GlyphRun, Rgb565, TextMetrics};

    #[derive(Clone, Debug, Eq, PartialEq)]
    enum Op {
        Fill(UiRect, Rgb565),
        Text(String, Rgb565, i32),
        Focus,
    }

    #[derive(Default)]
    struct Recorder {
        ops: Vec<Op>,
    }

    impl TextMetrics for Recorder {
        fn measure_text(&mut self, text: &str) -> Result<i32, PaintError> {
            Ok(text
                .chars()
                .map(|ch| if ch.is_ascii() { 6 } else { 12 })
                .sum())
        }

        fn line_height(&self) -> Option<i32> {
            Some(12)
        }
    }

    impl Painter for Recorder {
        fn fill_rect(&mut self, _: UiRect, rect: UiRect, color: Rgb565) -> Result<(), PaintError> {
            self.ops.push(Op::Fill(rect, color));
            Ok(())
        }
        fn stroke_rect(
            &mut self,
            _: UiRect,
            _: UiRect,
            _: Rgb565,
            _: u8,
        ) -> Result<(), PaintError> {
            Ok(())
        }
        fn draw_text(
            &mut self,
            _: UiRect,
            bounds: UiRect,
            run: TextRun<'_>,
        ) -> Result<(), PaintError> {
            self.ops
                .push(Op::Text(run.text.into(), run.color, bounds.x));
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
            self.ops.push(Op::Focus);
            Ok(())
        }
    }

    const ID: WidgetId = WidgetId::new(31);
    const BOUNDS: UiRect = UiRect::new(10, 20, 50, 20);
    const SURFACE: UiRect = UiRect::new(0, 0, 320, 320);

    fn context() -> UiContext<8> {
        UiContext::new(SURFACE, Theme::DARK)
    }

    fn focused<'a>(placeholder: &'a str, context: &mut UiContext<8>) -> TextField<'a> {
        let mut field = TextField::new(ID, BOUNDS, placeholder);
        field.set_focused(true, context);
        field.set_editing(true, context);
        context.clear_damage();
        field
    }

    #[test]
    fn buffer_is_bounded_and_rejects_invalid_utf8_boundaries() {
        let mut bytes = [0; 6];
        let mut value = Utf8Buffer::from_str(&mut bytes, "あ").unwrap();
        assert_eq!(value.insert_str(1, "x"), Err(BufferError::InvalidBoundary));
        assert_eq!(value.insert_str(3, "い"), Ok(()));
        assert_eq!(value.as_str(), "あい");
        assert_eq!(value.insert_str(6, "x"), Err(BufferError::Capacity));
        assert_eq!(value.remove_range(1, 3), Err(BufferError::InvalidBoundary));
    }

    #[test]
    fn ascii_and_kana_edit_only_on_boundaries() {
        let mut bytes = [0; 16];
        let mut value = Utf8Buffer::from_str(&mut bytes, "aあb").unwrap();
        let mut context = context();
        let mut field = focused("", &mut context);
        field.set_cursor(&value, 4, &mut context).unwrap();
        context.clear_damage();
        assert_eq!(
            field.handle_event(
                &mut value,
                UiEvent::pressed(UiAction::Backspace),
                &mut context
            ),
            Some(UiResponse::new(ID, ResponseKind::TextChanged(2)))
        );
        assert_eq!(value.as_str(), "ab");
        assert_eq!(field.cursor(), 1);
        field.handle_event(
            &mut value,
            UiEvent::pressed(UiAction::Text('い')),
            &mut context,
        );
        assert_eq!(value.as_str(), "aいb");
        assert_eq!(field.cursor(), 4);
    }

    #[test]
    fn full_buffer_rejects_without_damage_or_mutation() {
        let mut bytes = [0; 1];
        let mut value = Utf8Buffer::from_str(&mut bytes, "x").unwrap();
        let mut context = context();
        let mut field = focused("", &mut context);
        field.set_cursor(&value, 1, &mut context).unwrap();
        context.clear_damage();
        assert_eq!(
            field.handle_event(
                &mut value,
                UiEvent::pressed(UiAction::Text('y')),
                &mut context
            ),
            Some(UiResponse::new(ID, ResponseKind::CapacityRejected))
        );
        assert_eq!(value.as_str(), "x");
        assert!(!context.has_damage());
    }

    #[test]
    fn cursor_edges_submit_cancel_release_disabled_and_focus_loss_are_stable() {
        let mut bytes = [0; 8];
        let mut value = Utf8Buffer::from_str(&mut bytes, "ab").unwrap();
        let mut context = context();
        let mut field = focused("", &mut context);
        assert_eq!(
            field.handle_event(
                &mut value,
                UiEvent::pressed(UiAction::Backspace),
                &mut context
            ),
            None
        );
        assert_eq!(
            field.handle_event(&mut value, UiEvent::pressed(UiAction::Submit), &mut context),
            Some(UiResponse::new(ID, ResponseKind::Submitted))
        );
        assert_eq!(
            field.handle_event(&mut value, UiEvent::pressed(UiAction::Cancel), &mut context),
            Some(UiResponse::new(ID, ResponseKind::Cancelled))
        );
        assert!(!field.is_editing());
        assert_eq!(
            field.handle_event(
                &mut value,
                UiEvent::released(UiAction::Text('x')),
                &mut context
            ),
            None
        );
        field.set_focused(false, &mut context);
        assert_eq!(
            field.handle_event(
                &mut value,
                UiEvent::pressed(UiAction::Text('x')),
                &mut context
            ),
            None
        );
        field.set_focused(true, &mut context);
        field.set_enabled(false, &mut context);
        assert_eq!(
            field.handle_event(
                &mut value,
                UiEvent::pressed(UiAction::Text('x')),
                &mut context
            ),
            None
        );
        assert_eq!(value.as_str(), "ab");
    }

    #[test]
    fn horizontal_scroll_uses_measured_width_and_keeps_cursor_visible() {
        let mut bytes = [0; 32];
        let value = Utf8Buffer::from_str(&mut bytes, "abあいう").unwrap();
        let mut context = context();
        let mut field = focused("", &mut context);
        field.set_cursor(&value, value.len(), &mut context).unwrap();
        let mut painter = Recorder::default();
        field
            .paint(&mut painter, SURFACE, &Theme::DARK, &value, None)
            .unwrap();
        assert!(field.view_start() > 0);
        assert!(value.as_str().is_char_boundary(field.view_start()));
    }

    #[test]
    fn placeholder_composition_candidate_and_cursor_have_distinct_styles() {
        let mut bytes = [0; 16];
        let empty = Utf8Buffer::new(&mut bytes);
        let mut context = context();
        let mut field = focused("入力", &mut context);
        let mut painter = Recorder::default();
        field
            .paint(&mut painter, SURFACE, &Theme::DARK, &empty, None)
            .unwrap();
        assert!(painter.ops.contains(&Op::Text(
            "入力".into(),
            Theme::DARK.disabled.foreground,
            14
        )));

        painter.ops.clear();
        field
            .paint(
                &mut painter,
                SURFACE,
                &Theme::DARK,
                &empty,
                Some(ImeComposition {
                    text: "かな",
                    candidate: Some("仮名"),
                }),
            )
            .unwrap();
        assert!(!painter
            .ops
            .iter()
            .any(|op| matches!(op, Op::Text(text, _, _) if text == "かな")));
        assert!(painter.ops.iter().any(|op| matches!(op, Op::Text(text, color, _) if text == "仮名" && *color == Theme::DARK.focus)));
        assert!(painter.ops.iter().any(
            |op| matches!(op, Op::Fill(rect, color) if rect.w == 1 && *color == Theme::DARK.focus)
        ));
        assert!(painter.ops.iter().any(
            |op| matches!(op, Op::Fill(rect, color) if rect.x == 38 && *color == Theme::DARK.focus)
        ));
    }

    #[test]
    fn focus_and_editing_are_distinct_and_caret_matches_the_font_line() {
        let mut bytes = [0; 8];
        let mut value = Utf8Buffer::from_str(&mut bytes, "ab").unwrap();
        let mut context = context();
        let mut field = TextField::new(ID, UiRect::new(10, 20, 50, 28), "");
        field.set_focused(true, &mut context);

        assert!(!field.is_editing());
        assert_eq!(field.visual_state(), VisualState::Normal);
        assert_eq!(
            field.handle_event(
                &mut value,
                UiEvent::pressed(UiAction::Navigate(Navigation::Right)),
                &mut context,
            ),
            None
        );
        assert_eq!(field.cursor(), 0);

        field.handle_event(
            &mut value,
            UiEvent::pressed(UiAction::Activate),
            &mut context,
        );
        assert!(field.is_editing());
        assert_eq!(field.visual_state(), VisualState::Focused);
        let mut painter = Recorder::default();
        field
            .paint(&mut painter, SURFACE, &Theme::DARK, &value, None)
            .unwrap();
        assert!(painter.ops.iter().any(
            |op| matches!(op, Op::Fill(rect, color) if rect.y == 28 && rect.h == 12 && *color == Theme::DARK.focus)
        ));
    }

    #[test]
    fn composition_invalidation_is_explicit_and_idle_navigation_does_not_damage() {
        let mut bytes = [0; 4];
        let mut value = Utf8Buffer::new(&mut bytes);
        let mut context = context();
        let mut field = focused("", &mut context);
        assert_eq!(
            field.handle_event(&mut value, UiEvent::pressed(UiAction::Home), &mut context),
            None
        );
        assert!(!context.has_damage());
        field.invalidate_composition(&mut context);
        assert_eq!(context.damaged_rects().collect::<Vec<_>>(), [BOUNDS]);
    }

    #[test]
    fn memory_costs_are_fixed_and_documentable() {
        assert_eq!(size_of::<Utf8Buffer<'static>>(), 24);
        assert_eq!(size_of::<TextField<'static>>(), 80);
        assert_eq!(size_of::<ImeComposition<'static>>(), 32);
    }
}
