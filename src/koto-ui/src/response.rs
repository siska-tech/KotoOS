use crate::{UiEvent, WidgetId};

/// Backend-independent visual interaction state for a control.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum VisualState {
    #[default]
    Normal,
    Focused,
    Pressed,
    Disabled,
}

/// Semantic result emitted by a component instead of invoking callbacks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResponseKind {
    FocusChanged,
    Activated,
    ValueChanged(i32),
    TextChanged(usize),
    SelectionChanged(usize),
    SelectionActivated(usize),
    Submitted,
    Cancelled,
    CapacityRejected,
    Input(UiEvent),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiResponse {
    pub widget: WidgetId,
    pub kind: ResponseKind,
}

impl UiResponse {
    pub const fn new(widget: WidgetId, kind: ResponseKind) -> Self {
        Self { widget, kind }
    }
}
