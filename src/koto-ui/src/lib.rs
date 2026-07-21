//! KotoUI — a bounded, allocation-free GUI component foundation for KotoOS.
//!
//! The crate owns UI-neutral geometry, identity, visual state, theme, painter,
//! semantic response, and damage contracts. It deliberately does not own a
//! framebuffer, font cache, display service, input HAL, or application model.
//! Concrete controls and platform adapters are added by later KotoUI issues.
//!
//! All storage is caller-owned or fixed-capacity. The crate is `no_std` outside
//! tests and has no allocator dependency.

#![cfg_attr(not(test), no_std)]

mod context;
mod controls;
mod damage;
mod event;
mod focus;
mod geometry;
mod painter;
mod response;
mod theme;

pub use context::UiContext;
pub use controls::{
    BufferError, Button, Checkbox, Dialog, DialogAction, DialogError, DialogResult,
    DialogResultKind, ImeComposition, Insets, Label, LayoutError, List, ListError, ListModel,
    ListRow, Panel, PanelLayout, TextField, Utf8Buffer, DEFAULT_DIALOG_ACTION_CAPACITY,
    DEFAULT_DIALOG_CHILD_CAPACITY,
};
pub use damage::{DamageRects, DamageSet, DEFAULT_DAMAGE_CAPACITY};
pub use event::{EventBuffer, EventBufferFull, EventPhase, Navigation, UiAction, UiEvent};
pub use focus::{
    FocusEntry, FocusError, FocusManager, FocusScopeId, RegistrationError, DEFAULT_FOCUS_CAPACITY,
};
pub use geometry::{UiRect, WidgetId};
pub use painter::{GlyphRun, PaintError, Painter, TextAlign, TextMetrics, TextRun};
pub use response::{ResponseKind, UiResponse, VisualState};
pub use theme::{ControlStyle, Rgb565, Theme};
