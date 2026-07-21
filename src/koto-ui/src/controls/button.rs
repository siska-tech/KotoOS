use crate::{
    EventPhase, PaintError, Painter, ResponseKind, TextAlign, TextRun, Theme, UiAction, UiContext,
    UiEvent, UiRect, UiResponse, VisualState, WidgetId,
};

use super::{clipped, control_style, paint_frame};

/// Borrowed-text push button with explicit focus and press state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct Button<'a> {
    id: WidgetId,
    bounds: UiRect,
    label: &'a str,
    semantic_label: Option<&'a str>,
    enabled: bool,
    focused: bool,
    pressed: bool,
}

impl<'a> Button<'a> {
    pub const fn new(id: WidgetId, bounds: UiRect, label: &'a str) -> Self {
        Self {
            id,
            bounds,
            label,
            semantic_label: None,
            enabled: true,
            focused: false,
            pressed: false,
        }
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
                Some(UiResponse::new(self.id, ResponseKind::Activated))
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
        let text_bounds = self
            .bounds
            .inset(i32::from(theme.spacing))
            .unwrap_or(self.bounds);
        painter.draw_text(
            effective_clip,
            text_bounds,
            TextRun {
                text: self.label,
                color: style.foreground,
                align: TextAlign::Center,
            },
        )?;
        if self.focused && self.enabled {
            painter.draw_focus_mark(effective_clip, self.bounds, theme.focus, theme.focus_width)?;
        }
        Ok(())
    }
}
