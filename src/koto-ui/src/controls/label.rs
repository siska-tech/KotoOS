use crate::{PaintError, Painter, TextAlign, TextRun, Theme, UiContext, UiRect, WidgetId};

use super::clipped;

/// Borrowed, non-focusable single-line label.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct Label<'a> {
    id: WidgetId,
    bounds: UiRect,
    text: &'a str,
    alignment: TextAlign,
    disabled: bool,
}

impl<'a> Label<'a> {
    pub const fn new(id: WidgetId, bounds: UiRect, text: &'a str) -> Self {
        Self {
            id,
            bounds,
            text,
            alignment: TextAlign::Start,
            disabled: false,
        }
    }

    pub const fn with_alignment(mut self, alignment: TextAlign) -> Self {
        self.alignment = alignment;
        self
    }

    pub const fn id(&self) -> WidgetId {
        self.id
    }

    pub const fn bounds(&self) -> UiRect {
        self.bounds
    }

    pub const fn text(&self) -> &'a str {
        self.text
    }

    pub const fn semantic_label(&self) -> &'a str {
        self.text
    }

    pub const fn alignment(&self) -> TextAlign {
        self.alignment
    }

    pub const fn is_disabled(&self) -> bool {
        self.disabled
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

    pub fn set_text<const DAMAGE: usize>(
        &mut self,
        text: &'a str,
        context: &mut UiContext<DAMAGE>,
    ) {
        if self.text != text {
            self.text = text;
            context.damage(self.bounds);
        }
    }

    pub fn set_alignment<const DAMAGE: usize>(
        &mut self,
        alignment: TextAlign,
        context: &mut UiContext<DAMAGE>,
    ) {
        if self.alignment != alignment {
            self.alignment = alignment;
            context.damage(self.bounds);
        }
    }

    pub fn set_disabled<const DAMAGE: usize>(
        &mut self,
        disabled: bool,
        context: &mut UiContext<DAMAGE>,
    ) {
        if self.disabled != disabled {
            self.disabled = disabled;
            context.damage(self.bounds);
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
        let color = if self.disabled {
            theme.disabled.foreground
        } else {
            theme.normal.foreground
        };
        painter.draw_text(
            effective_clip,
            self.bounds,
            TextRun {
                text: self.text,
                color,
                align: self.alignment,
            },
        )
    }
}
