use crate::{
    EventPhase, PaintError, Painter, ResponseKind, TextAlign, TextRun, Theme, UiAction, UiContext,
    UiEvent, UiRect, UiResponse, VisualState, WidgetId,
};

use super::{clipped, control_style, paint_frame};

/// Borrowed-text checkbox with an explicit shape mark for checked state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct Checkbox<'a> {
    id: WidgetId,
    bounds: UiRect,
    label: &'a str,
    semantic_label: Option<&'a str>,
    enabled: bool,
    focused: bool,
    pressed: bool,
    checked: bool,
    mark_offset_x: i16,
    mark_offset_y: i16,
}

impl<'a> Checkbox<'a> {
    pub const fn new(id: WidgetId, bounds: UiRect, label: &'a str) -> Self {
        Self {
            id,
            bounds,
            label,
            semantic_label: None,
            enabled: true,
            focused: false,
            pressed: false,
            checked: false,
            mark_offset_x: 0,
            mark_offset_y: 0,
        }
    }

    /// Offsets the square mark from its default left-aligned, vertically
    /// centered position. Callers must keep the mark inside `bounds`.
    pub const fn with_mark_offset(mut self, x: i16, y: i16) -> Self {
        self.mark_offset_x = x;
        self.mark_offset_y = y;
        self
    }

    pub const fn with_semantic_label(mut self, label: &'a str) -> Self {
        self.semantic_label = Some(label);
        self
    }

    pub const fn id(&self) -> WidgetId {
        self.id
    }

    pub const fn bounds(&self) -> UiRect {
        self.bounds
    }

    pub const fn label(&self) -> &'a str {
        self.label
    }

    pub const fn semantic_label(&self) -> &'a str {
        match self.semantic_label {
            Some(label) => label,
            None => self.label,
        }
    }

    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub const fn is_focused(&self) -> bool {
        self.focused
    }

    pub const fn is_pressed(&self) -> bool {
        self.pressed
    }

    pub const fn is_checked(&self) -> bool {
        self.checked
    }

    pub const fn mark_offset(&self) -> (i16, i16) {
        (self.mark_offset_x, self.mark_offset_y)
    }

    pub fn set_mark_offset<const DAMAGE: usize>(
        &mut self,
        x: i16,
        y: i16,
        context: &mut UiContext<DAMAGE>,
    ) {
        if self.mark_offset() != (x, y) {
            self.mark_offset_x = x;
            self.mark_offset_y = y;
            context.damage(self.bounds);
        }
    }

    pub const fn visual_state(&self) -> VisualState {
        if !self.enabled {
            VisualState::Disabled
        } else if self.pressed {
            VisualState::Pressed
        } else if self.focused {
            VisualState::Focused
        } else {
            VisualState::Normal
        }
    }

    pub fn set_bounds<const DAMAGE: usize>(
        &mut self,
        bounds: UiRect,
        context: &mut UiContext<DAMAGE>,
    ) {
        if self.bounds != bounds {
            context.damage_transition(self.bounds, bounds);
            self.bounds = bounds;
        }
    }

    pub fn set_label<const DAMAGE: usize>(
        &mut self,
        label: &'a str,
        context: &mut UiContext<DAMAGE>,
    ) {
        if self.label != label {
            self.label = label;
            context.damage(self.bounds);
        }
    }

    pub fn set_semantic_label(&mut self, label: Option<&'a str>) {
        self.semantic_label = label;
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
                self.pressed = false;
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
            if !focused {
                self.pressed = false;
            }
            context.damage(self.bounds);
        }
    }

    pub fn set_pressed<const DAMAGE: usize>(
        &mut self,
        pressed: bool,
        context: &mut UiContext<DAMAGE>,
    ) {
        let pressed = pressed && self.enabled && self.focused;
        if self.pressed != pressed {
            self.pressed = pressed;
            context.damage(self.bounds);
        }
    }

    pub fn set_checked<const DAMAGE: usize>(
        &mut self,
        checked: bool,
        context: &mut UiContext<DAMAGE>,
    ) {
        if self.checked != checked {
            self.checked = checked;
            context.damage(self.bounds);
        }
    }

    pub fn handle_event<const DAMAGE: usize>(
        &mut self,
        event: UiEvent,
        context: &mut UiContext<DAMAGE>,
    ) -> Option<UiResponse> {
        if event.action != UiAction::Activate || !self.enabled || !self.focused {
            return None;
        }
        match event.phase {
            EventPhase::Pressed if !self.pressed => {
                self.set_pressed(true, context);
                self.set_checked(!self.checked, context);
                Some(UiResponse::new(
                    self.id,
                    ResponseKind::ValueChanged(i32::from(self.checked)),
                ))
            }
            EventPhase::Released => {
                self.set_pressed(false, context);
                None
            }
            EventPhase::Pressed | EventPhase::Repeated => None,
        }
    }

    pub fn paint(
        &self,
        painter: &mut impl Painter,
        clip: UiRect,
        theme: &Theme,
    ) -> Result<(), PaintError> {
        let Some(effective_clip) = clipped(self.bounds, clip) else {
            return Ok(());
        };
        let style = control_style(theme, self.visual_state());
        paint_frame(painter, effective_clip, self.bounds, style, theme)?;

        let side = self.bounds.h.clamp(1, 12);
        let box_x = self.bounds.x.saturating_add(i32::from(self.mark_offset_x));
        let box_y = self
            .bounds
            .y
            .saturating_add((self.bounds.h - side) / 2)
            .saturating_add(i32::from(self.mark_offset_y));
        let box_rect = UiRect::new(box_x, box_y, side, side);
        painter.stroke_rect(
            effective_clip,
            box_rect,
            style.foreground,
            theme.border_width,
        )?;
        if self.checked {
            let inner = box_rect.inset(3).unwrap_or(box_rect);
            painter.fill_rect(effective_clip, inner, style.foreground)?;
        }
        let label_x = box_rect
            .x
            .saturating_add(side)
            .saturating_add(i32::from(theme.spacing));
        let label_w = self
            .bounds
            .x
            .saturating_add(self.bounds.w)
            .saturating_sub(label_x);
        if label_w > 0 {
            painter.draw_text(
                effective_clip,
                UiRect::new(label_x, self.bounds.y, label_w, self.bounds.h),
                TextRun {
                    text: self.label,
                    color: style.foreground,
                    align: TextAlign::Start,
                },
            )?;
        }
        if self.focused && self.enabled {
            painter.draw_focus_mark(effective_clip, self.bounds, theme.focus, theme.focus_width)?;
        }
        Ok(())
    }
}
