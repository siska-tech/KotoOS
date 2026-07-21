use crate::{
    EventPhase, Navigation, PaintError, Painter, ResponseKind, TextAlign, TextRun, Theme, UiAction,
    UiContext, UiEvent, UiRect, UiResponse, VisualState, WidgetId,
};

use super::{clipped, control_style};

const NO_SELECTION: usize = usize::MAX;

/// Borrowed presentation data for one list row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ListRow<'a> {
    pub label: &'a str,
    pub semantic_label: Option<&'a str>,
    pub enabled: bool,
}

impl<'a> ListRow<'a> {
    pub const fn new(label: &'a str) -> Self {
        Self {
            label,
            semantic_label: None,
            enabled: true,
        }
    }

    pub const fn disabled(label: &'a str) -> Self {
        Self {
            label,
            semantic_label: None,
            enabled: false,
        }
    }

    pub const fn with_semantic_label(mut self, label: &'a str) -> Self {
        self.semantic_label = Some(label);
        self
    }

    pub const fn semantic_label(self) -> &'a str {
        match self.semantic_label {
            Some(label) => label,
            None => self.label,
        }
    }
}

/// Caller-owned item source. Rows are borrowed only for the method call.
pub trait ListModel {
    fn len(&self) -> usize;
    fn row(&self, index: usize) -> Option<ListRow<'_>>;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ListError {
    EmptyModel,
    InvalidIndex,
    DisabledItem,
}

/// Fixed-state list viewport over a caller-owned [`ListModel`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct List {
    id: WidgetId,
    bounds: UiRect,
    row_height: i32,
    selected: usize,
    first_visible: usize,
    model_len: usize,
    enabled: bool,
    focused: bool,
}

impl List {
    pub const fn new(id: WidgetId, bounds: UiRect, row_height: i32) -> Self {
        Self {
            id,
            bounds,
            row_height: if row_height > 0 { row_height } else { 1 },
            selected: NO_SELECTION,
            first_visible: 0,
            model_len: 0,
            enabled: true,
            focused: false,
        }
    }

    pub const fn id(&self) -> WidgetId {
        self.id
    }

    pub const fn bounds(&self) -> UiRect {
        self.bounds
    }

    pub const fn row_height(&self) -> i32 {
        self.row_height
    }

    pub const fn selected(&self) -> Option<usize> {
        if self.selected == NO_SELECTION {
            None
        } else {
            Some(self.selected)
        }
    }

    pub const fn first_visible(&self) -> usize {
        self.first_visible
    }

    pub const fn visible_rows(&self) -> usize {
        if self.bounds.h <= 0 {
            0
        } else {
            (self.bounds.h / self.row_height) as usize
        }
    }

    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub const fn is_focused(&self) -> bool {
        self.focused
    }

    pub fn set_bounds<const DAMAGE: usize>(
        &mut self,
        bounds: UiRect,
        context: &mut UiContext<DAMAGE>,
    ) {
        if self.bounds != bounds {
            context.damage_transition(self.bounds, bounds);
            self.bounds = bounds;
            self.ensure_selected_visible();
        }
    }

    pub fn set_row_height<const DAMAGE: usize>(
        &mut self,
        row_height: i32,
        context: &mut UiContext<DAMAGE>,
    ) {
        let row_height = row_height.max(1);
        if self.row_height != row_height {
            self.row_height = row_height;
            self.ensure_selected_visible();
            context.damage(self.bounds);
        }
    }

    pub fn set_enabled<const DAMAGE: usize>(
        &mut self,
        enabled: bool,
        context: &mut UiContext<DAMAGE>,
    ) {
        if self.enabled != enabled {
            self.enabled = enabled;
            if !enabled {
                self.focused = false;
            }
            context.damage(self.bounds);
        }
    }

    pub fn set_focused<const DAMAGE: usize>(
        &mut self,
        focused: bool,
        context: &mut UiContext<DAMAGE>,
    ) {
        let focused = focused && self.enabled;
        if self.focused != focused {
            self.focused = focused;
            // Focus changes affect the frame and every row's visual treatment,
            // not only the selected row marker.
            context.damage(self.bounds);
        }
    }

    /// Reconciles cached selection/viewport state after caller model changes.
    pub fn sync_model<const DAMAGE: usize>(
        &mut self,
        model: &impl ListModel,
        context: &mut UiContext<DAMAGE>,
    ) -> Option<UiResponse> {
        let old_len = self.model_len;
        let old_selected = self.selected();
        let old_first = self.first_visible;
        self.model_len = model.len();

        let next = old_selected
            .filter(|index| self.row_enabled(model, *index))
            .or_else(|| {
                let target = old_selected
                    .unwrap_or(0)
                    .min(self.model_len.saturating_sub(1));
                self.nearest_enabled(model, target)
            });
        self.selected = next.unwrap_or(NO_SELECTION);
        self.clamp_viewport(self.model_len);
        self.ensure_selected_visible();

        if old_len != self.model_len || old_first != self.first_visible {
            context.damage(self.bounds);
        } else if old_selected != next {
            self.damage_selection_change(old_selected, next, context);
        }

        if old_selected != next {
            next.map(|index| UiResponse::new(self.id, ResponseKind::SelectionChanged(index)))
        } else {
            None
        }
    }

    pub fn set_selected<const DAMAGE: usize>(
        &mut self,
        index: usize,
        model: &impl ListModel,
        context: &mut UiContext<DAMAGE>,
    ) -> Result<Option<UiResponse>, ListError> {
        if model.is_empty() {
            return Err(ListError::EmptyModel);
        }
        if index >= model.len() || model.row(index).is_none() {
            return Err(ListError::InvalidIndex);
        }
        if !self.row_enabled(model, index) {
            return Err(ListError::DisabledItem);
        }
        self.model_len = model.len();
        Ok(self.select(Some(index), context))
    }

    pub fn invalidate_row<const DAMAGE: usize>(
        &self,
        index: usize,
        context: &mut UiContext<DAMAGE>,
    ) {
        if let Some(rect) = self.row_rect(index) {
            context.damage(rect);
        }
    }

    pub fn handle_event<const DAMAGE: usize>(
        &mut self,
        model: &impl ListModel,
        event: UiEvent,
        context: &mut UiContext<DAMAGE>,
    ) -> Option<UiResponse> {
        let sync_response = self.sync_model(model, context);
        if !self.enabled || !self.focused || event.phase == EventPhase::Released {
            return sync_response;
        }

        let event_response = match event.action {
            UiAction::Navigate(Navigation::Up) => self.move_relative(model, false, 1, context),
            UiAction::Navigate(Navigation::Down) => self.move_relative(model, true, 1, context),
            UiAction::Navigate(Navigation::PageUp) => {
                self.move_relative(model, false, self.visible_rows().max(1), context)
            }
            UiAction::Navigate(Navigation::PageDown) => {
                self.move_relative(model, true, self.visible_rows().max(1), context)
            }
            UiAction::Home => self.select(self.first_enabled(model), context),
            UiAction::End => self.select(self.last_enabled(model), context),
            UiAction::Activate if event.phase == EventPhase::Pressed => self
                .selected()
                .filter(|index| self.row_enabled(model, *index))
                .map(|index| UiResponse::new(self.id, ResponseKind::SelectionActivated(index))),
            _ => None,
        };
        event_response.or(sync_response)
    }

    pub fn paint(
        &self,
        model: &impl ListModel,
        painter: &mut impl Painter,
        clip: UiRect,
        theme: &Theme,
    ) -> Result<(), PaintError> {
        let Some(effective_clip) = clipped(self.bounds, clip) else {
            return Ok(());
        };
        let frame_style = if self.enabled && self.focused {
            theme.normal
        } else {
            theme.disabled
        };
        painter.fill_rect(effective_clip, self.bounds, frame_style.background)?;
        painter.stroke_rect(
            effective_clip,
            self.bounds,
            frame_style.border,
            theme.border_width,
        )?;

        let end = self
            .first_visible
            .saturating_add(self.visible_rows())
            .min(model.len());
        for index in self.first_visible..end {
            let Some(row) = model.row(index) else {
                continue;
            };
            let Some(rect) = self.row_rect(index) else {
                continue;
            };
            let selected = self.selected() == Some(index);
            let state = if !self.enabled || !self.focused || !row.enabled {
                VisualState::Disabled
            } else if selected {
                VisualState::Focused
            } else {
                VisualState::Normal
            };
            let style = control_style(theme, state);
            painter.fill_rect(effective_clip, rect, style.background)?;
            let text_x = rect.x.saturating_add(i32::from(theme.spacing));
            let text_w = rect.w.saturating_sub(i32::from(theme.spacing) * 2);
            if text_w > 0 {
                painter.draw_text(
                    effective_clip,
                    UiRect::new(text_x, rect.y, text_w, rect.h),
                    TextRun {
                        text: row.label,
                        color: style.foreground,
                        align: TextAlign::Start,
                    },
                )?;
            }
            if selected {
                painter.stroke_rect(effective_clip, rect, style.border, theme.border_width)?;
                if self.focused && self.enabled && row.enabled {
                    painter.draw_focus_mark(
                        effective_clip,
                        rect,
                        theme.focus,
                        theme.focus_width,
                    )?;
                }
            }
        }
        self.paint_scrollbar(model.len(), painter, effective_clip, theme)
    }

    fn move_relative<const DAMAGE: usize>(
        &mut self,
        model: &impl ListModel,
        forward: bool,
        distance: usize,
        context: &mut UiContext<DAMAGE>,
    ) -> Option<UiResponse> {
        let Some(current) = self.selected() else {
            return self.select(self.first_enabled(model), context);
        };
        let target = if forward {
            current
                .saturating_add(distance)
                .min(model.len().saturating_sub(1))
        } else {
            current.saturating_sub(distance)
        };
        let next = if forward {
            self.enabled_from(model, target, true)
                .or_else(|| self.last_enabled(model))
        } else {
            self.enabled_from(model, target, false)
                .or_else(|| self.first_enabled(model))
        };
        self.select(next, context)
    }

    fn select<const DAMAGE: usize>(
        &mut self,
        next: Option<usize>,
        context: &mut UiContext<DAMAGE>,
    ) -> Option<UiResponse> {
        let old = self.selected();
        if old == next {
            return None;
        }
        let old_first = self.first_visible;
        self.selected = next.unwrap_or(NO_SELECTION);
        self.ensure_selected_visible();
        if old_first != self.first_visible {
            context.damage(self.bounds);
        } else {
            self.damage_selection_change(old, next, context);
        }
        next.map(|index| UiResponse::new(self.id, ResponseKind::SelectionChanged(index)))
    }

    fn damage_selection_change<const DAMAGE: usize>(
        &self,
        old: Option<usize>,
        new: Option<usize>,
        context: &mut UiContext<DAMAGE>,
    ) {
        if let Some(rect) = old.and_then(|index| self.row_rect(index)) {
            context.damage(rect);
        }
        if let Some(rect) = new.and_then(|index| self.row_rect(index)) {
            context.damage(rect);
        }
    }

    fn ensure_selected_visible(&mut self) {
        let Some(selected) = self.selected() else {
            self.first_visible = 0;
            return;
        };
        let rows = self.visible_rows();
        if rows == 0 || selected < self.first_visible {
            self.first_visible = selected;
        } else if selected >= self.first_visible.saturating_add(rows) {
            self.first_visible = selected.saturating_add(1).saturating_sub(rows);
        }
        self.clamp_viewport(self.model_len);
    }

    fn clamp_viewport(&mut self, len: usize) {
        let max_first = len.saturating_sub(self.visible_rows());
        self.first_visible = self.first_visible.min(max_first);
    }

    fn row_rect(&self, index: usize) -> Option<UiRect> {
        if index < self.first_visible {
            return None;
        }
        let offset = index - self.first_visible;
        if offset >= self.visible_rows() {
            return None;
        }
        let y_offset = i64::try_from(offset)
            .ok()?
            .checked_mul(i64::from(self.row_height))?;
        let y = i64::from(self.bounds.y).checked_add(y_offset)?;
        Some(UiRect::new(
            self.bounds.x,
            i32::try_from(y).ok()?,
            self.bounds.w,
            self.row_height,
        ))
    }

    fn row_enabled(&self, model: &impl ListModel, index: usize) -> bool {
        index < model.len() && model.row(index).is_some_and(|row| row.enabled)
    }

    fn first_enabled(&self, model: &impl ListModel) -> Option<usize> {
        self.enabled_from(model, 0, true)
    }

    fn last_enabled(&self, model: &impl ListModel) -> Option<usize> {
        model
            .len()
            .checked_sub(1)
            .and_then(|index| self.enabled_from(model, index, false))
    }

    fn nearest_enabled(&self, model: &impl ListModel, target: usize) -> Option<usize> {
        self.enabled_from(model, target, true)
            .or_else(|| self.enabled_from(model, target, false))
    }

    fn enabled_from(&self, model: &impl ListModel, start: usize, forward: bool) -> Option<usize> {
        if forward {
            (start..model.len()).find(|index| self.row_enabled(model, *index))
        } else {
            (0..=start.min(model.len().saturating_sub(1)))
                .rev()
                .find(|index| self.row_enabled(model, *index))
        }
    }

    fn paint_scrollbar(
        &self,
        len: usize,
        painter: &mut impl Painter,
        clip: UiRect,
        theme: &Theme,
    ) -> Result<(), PaintError> {
        let rows = self.visible_rows();
        if len <= rows || rows == 0 || self.bounds.w < 3 {
            return Ok(());
        }
        let track = UiRect::new(
            self.bounds
                .x
                .saturating_add(self.bounds.w)
                .saturating_sub(3),
            self.bounds.y,
            3,
            self.bounds.h,
        );
        painter.fill_rect(clip, track, theme.disabled.border)?;
        let thumb_h = ((i64::from(self.bounds.h) * i64::try_from(rows).unwrap_or(i64::MAX))
            / i64::try_from(len).unwrap_or(i64::MAX))
        .max(4)
        .min(i64::from(self.bounds.h));
        let travel = i64::from(self.bounds.h) - thumb_h;
        let denominator = len.saturating_sub(rows);
        let thumb_y = if denominator == 0 {
            0
        } else {
            let numerator = (travel as u128).saturating_mul(self.first_visible as u128);
            i64::try_from(numerator / denominator as u128).unwrap_or(travel)
        };
        painter.fill_rect(
            clip,
            UiRect::new(
                track.x,
                track
                    .y
                    .saturating_add(i32::try_from(thumb_y).unwrap_or(i32::MAX)),
                track.w,
                i32::try_from(thumb_h).unwrap_or(self.bounds.h),
            ),
            if self.enabled && self.focused {
                theme.accent
            } else {
                theme.disabled.foreground
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use core::mem::size_of;

    use super::*;
    use crate::{GlyphRun, Rgb565, TextMetrics};

    struct TestModel<'a> {
        rows: &'a [(&'a str, bool)],
    }

    impl ListModel for TestModel<'_> {
        fn len(&self) -> usize {
            self.rows.len()
        }

        fn row(&self, index: usize) -> Option<ListRow<'_>> {
            self.rows.get(index).map(|(label, enabled)| ListRow {
                label,
                semantic_label: None,
                enabled: *enabled,
            })
        }
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    enum Op {
        Fill(UiRect, UiRect, Rgb565),
        Stroke,
        Text(UiRect, UiRect, std::string::String, Rgb565),
        Focus(UiRect),
    }

    #[derive(Default)]
    struct Recorder {
        ops: std::vec::Vec<Op>,
    }

    impl TextMetrics for Recorder {
        fn measure_text(&mut self, text: &str) -> Result<i32, PaintError> {
            Ok(text
                .chars()
                .map(|ch| if ch.is_ascii() { 6 } else { 12 })
                .sum())
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
            _clip: UiRect,
            _rect: UiRect,
            _color: Rgb565,
            _width: u8,
        ) -> Result<(), PaintError> {
            self.ops.push(Op::Stroke);
            Ok(())
        }

        fn draw_text(
            &mut self,
            clip: UiRect,
            bounds: UiRect,
            run: TextRun<'_>,
        ) -> Result<(), PaintError> {
            self.ops
                .push(Op::Text(clip, bounds, run.text.into(), run.color));
            Ok(())
        }

        fn draw_glyphs(
            &mut self,
            _clip: UiRect,
            _bounds: UiRect,
            _run: GlyphRun<'_>,
        ) -> Result<(), PaintError> {
            Ok(())
        }

        fn draw_focus_mark(
            &mut self,
            _clip: UiRect,
            rect: UiRect,
            _color: Rgb565,
            _width: u8,
        ) -> Result<(), PaintError> {
            self.ops.push(Op::Focus(rect));
            Ok(())
        }
    }

    const ID: WidgetId = WidgetId::new(21);
    const BOUNDS: UiRect = UiRect::new(10, 20, 100, 30);
    const SURFACE: UiRect = UiRect::new(0, 0, 320, 320);

    fn context() -> UiContext<8> {
        UiContext::new(SURFACE, Theme::DARK)
    }

    fn list() -> List {
        List::new(ID, BOUNDS, 10)
    }

    #[test]
    fn empty_and_all_disabled_models_are_safe() {
        let empty = TestModel { rows: &[] };
        let disabled = TestModel {
            rows: &[("one", false), ("two", false)],
        };
        let mut list = list();
        let mut context = context();
        assert_eq!(list.sync_model(&empty, &mut context), None);
        assert_eq!(list.selected(), None);
        assert_eq!(
            list.handle_event(
                &empty,
                UiEvent::pressed(UiAction::Navigate(Navigation::Down)),
                &mut context,
            ),
            None
        );
        assert_eq!(list.sync_model(&disabled, &mut context), None);
        assert_eq!(list.selected(), None);
        assert_eq!(
            list.set_selected(0, &disabled, &mut context),
            Err(ListError::DisabledItem)
        );
    }

    #[test]
    fn navigation_skips_disabled_rows_and_accepts_repeat() {
        let model = TestModel {
            rows: &[("zero", true), ("disabled", false), ("two", true)],
        };
        let mut list = list();
        let mut context = context();
        list.sync_model(&model, &mut context);
        list.set_focused(true, &mut context);
        context.clear_damage();
        assert_eq!(
            list.handle_event(
                &model,
                UiEvent::repeated(UiAction::Navigate(Navigation::Down)),
                &mut context,
            ),
            Some(UiResponse::new(ID, ResponseKind::SelectionChanged(2)))
        );
        assert_eq!(list.selected(), Some(2));
        assert_eq!(
            context.damaged_rects().collect::<std::vec::Vec<_>>(),
            [UiRect::new(10, 20, 100, 10), UiRect::new(10, 40, 100, 10)]
        );
        context.clear_damage();
        assert_eq!(
            list.handle_event(
                &model,
                UiEvent::pressed(UiAction::Navigate(Navigation::Down)),
                &mut context,
            ),
            None
        );
        assert!(!context.has_damage());
    }

    #[test]
    fn page_home_end_navigation_is_deterministic() {
        let rows = [
            ("0", true),
            ("1", true),
            ("2", true),
            ("3", true),
            ("4", true),
            ("5", true),
            ("6", true),
        ];
        let model = TestModel { rows: &rows };
        let mut list = list();
        let mut context = context();
        list.sync_model(&model, &mut context);
        list.set_focused(true, &mut context);
        list.handle_event(
            &model,
            UiEvent::pressed(UiAction::Navigate(Navigation::PageDown)),
            &mut context,
        );
        assert_eq!(list.selected(), Some(3));
        assert_eq!(list.first_visible(), 1);
        list.handle_event(&model, UiEvent::pressed(UiAction::End), &mut context);
        assert_eq!(list.selected(), Some(6));
        assert_eq!(list.first_visible(), 4);
        list.handle_event(
            &model,
            UiEvent::pressed(UiAction::Navigate(Navigation::PageUp)),
            &mut context,
        );
        assert_eq!(list.selected(), Some(3));
        list.handle_event(&model, UiEvent::pressed(UiAction::Home), &mut context);
        assert_eq!(list.selected(), Some(0));
        assert_eq!(list.first_visible(), 0);
    }

    #[test]
    fn scrolling_damages_viewport_while_in_view_move_damages_rows() {
        let rows = [("0", true), ("1", true), ("2", true), ("3", true)];
        let model = TestModel { rows: &rows };
        let mut list = List::new(ID, UiRect::new(10, 20, 100, 20), 10);
        let mut context = context();
        list.sync_model(&model, &mut context);
        list.set_focused(true, &mut context);
        context.clear_damage();
        list.handle_event(
            &model,
            UiEvent::pressed(UiAction::Navigate(Navigation::Down)),
            &mut context,
        );
        assert_eq!(
            context.damaged_rects().collect::<std::vec::Vec<_>>(),
            [UiRect::new(10, 20, 100, 20)]
        );
        context.clear_damage();
        list.handle_event(
            &model,
            UiEvent::pressed(UiAction::Navigate(Navigation::Down)),
            &mut context,
        );
        assert_eq!(list.first_visible(), 1);
        assert_eq!(
            context.damaged_rects().collect::<std::vec::Vec<_>>(),
            [UiRect::new(10, 20, 100, 20)]
        );
    }

    #[test]
    fn model_shrink_clamps_and_growth_preserves_selection() {
        let large_rows = [("0", true), ("1", true), ("2", true), ("3", true)];
        let small_rows = [("0", true), ("1", true)];
        let large = TestModel { rows: &large_rows };
        let small = TestModel { rows: &small_rows };
        let mut list = list();
        let mut context = context();
        list.sync_model(&large, &mut context);
        list.set_selected(3, &large, &mut context).unwrap();
        assert_eq!(
            list.sync_model(&small, &mut context),
            Some(UiResponse::new(ID, ResponseKind::SelectionChanged(1)))
        );
        assert_eq!(list.selected(), Some(1));
        assert_eq!(list.sync_model(&large, &mut context), None);
        assert_eq!(list.selected(), Some(1));
    }

    #[test]
    fn activation_reports_selected_index_and_ignores_repeat() {
        let model = TestModel {
            rows: &[("zero", true), ("one", true)],
        };
        let mut list = list();
        let mut context = context();
        list.sync_model(&model, &mut context);
        list.set_focused(true, &mut context);
        assert_eq!(
            list.handle_event(&model, UiEvent::pressed(UiAction::Activate), &mut context,),
            Some(UiResponse::new(ID, ResponseKind::SelectionActivated(0)))
        );
        assert_eq!(
            list.handle_event(&model, UiEvent::repeated(UiAction::Activate), &mut context,),
            None
        );
    }

    #[test]
    fn paint_clips_rows_marks_disabled_and_draws_scrollbar() {
        let rows = [
            ("zero", true),
            ("disabled", false),
            ("two", true),
            ("three", true),
        ];
        let model = TestModel { rows: &rows };
        let mut list = List::new(ID, UiRect::new(0, 0, 40, 20), 10);
        let mut context = context();
        list.sync_model(&model, &mut context);
        list.set_focused(true, &mut context);
        let clip = UiRect::new(5, 0, 20, 20);
        let mut painter = Recorder::default();
        list.paint(&model, &mut painter, clip, &Theme::DARK)
            .unwrap();
        assert!(painter.ops.iter().all(|op| match op {
            Op::Fill(actual, _, _) | Op::Text(actual, _, _, _) => *actual == clip,
            Op::Stroke | Op::Focus(_) => true,
        }));
        assert!(painter.ops.iter().any(|op| matches!(
            op,
            Op::Text(_, _, text, color)
                if text == "disabled" && *color == Theme::DARK.disabled.foreground
        )));
        assert!(
            painter
                .ops
                .iter()
                .filter(|op| matches!(op, Op::Fill(..)))
                .count()
                >= 5
        );
        assert!(painter.ops.iter().any(|op| matches!(op, Op::Focus(_))));
    }

    #[test]
    fn unfocused_list_uses_disabled_frame_and_row_colors() {
        let rows = [
            ("zero", true),
            ("one", true),
            ("two", true),
            ("three", true),
        ];
        let model = TestModel { rows: &rows };
        let mut list = list();
        let mut context = context();
        list.sync_model(&model, &mut context);
        let mut painter = Recorder::default();

        list.paint(&model, &mut painter, SURFACE, &Theme::DARK)
            .unwrap();

        assert!(matches!(
            painter.ops.first(),
            Some(Op::Fill(_, rect, color))
                if *rect == BOUNDS && *color == Theme::DARK.disabled.background
        ));
        assert!(painter
            .ops
            .iter()
            .filter_map(|op| match op {
                Op::Text(_, _, _, color) => Some(*color),
                _ => None,
            })
            .all(|color| color == Theme::DARK.disabled.foreground));
        assert!(!painter
            .ops
            .iter()
            .any(|op| matches!(op, Op::Fill(_, _, color) if *color == Theme::DARK.accent)));
        assert!(!painter.ops.iter().any(|op| matches!(op, Op::Focus(_))));
    }

    #[test]
    fn resize_and_no_op_updates_have_bounded_damage() {
        let model = TestModel {
            rows: &[("zero", true), ("one", true), ("two", true)],
        };
        let mut list = list();
        let mut context = context();
        list.sync_model(&model, &mut context);
        context.clear_damage();
        list.set_bounds(BOUNDS, &mut context);
        list.set_row_height(10, &mut context);
        list.invalidate_row(99, &mut context);
        assert!(!context.has_damage());
        list.set_bounds(UiRect::new(10, 20, 100, 20), &mut context);
        assert!(context.has_damage());
        assert_eq!(list.visible_rows(), 2);
    }

    #[test]
    fn list_state_size_matches_documented_host_measurement() {
        assert_eq!(size_of::<List>(), 56);
    }
}
