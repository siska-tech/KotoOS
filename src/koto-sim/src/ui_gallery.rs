use koto_core::{BitmapFont, CanvasUiPainter};
use koto_ui::{
    Button, Checkbox, Dialog, DialogAction, DialogResult, EventPhase, FocusEntry, FocusManager,
    FocusScopeId, ImeComposition, Label, List, ListModel, ListRow, Navigation, Panel, ResponseKind,
    TextField, Theme, UiAction, UiContext, UiEvent, UiRect, UiResponse, Utf8Buffer, WidgetId,
};

use crate::Framebuffer;

pub const GALLERY_SURFACE: UiRect = UiRect::new(0, 0, 320, 320);
const BUTTON: WidgetId = WidgetId::new(101);
const CHECKBOX: WidgetId = WidgetId::new(102);
const LIST: WidgetId = WidgetId::new(103);
const FIELD: WidgetId = WidgetId::new(104);
const DISABLED: WidgetId = WidgetId::new(105);
const DIALOG: WidgetId = WidgetId::new(110);
const OK: WidgetId = WidgetId::new(111);
const CANCEL: WidgetId = WidgetId::new(112);
const MODAL: FocusScopeId = FocusScopeId::new(10);
const ROWS: [&str; 12] = [
    "Alpha", "Bravo", "Charlie", "Delta", "Echo", "Foxtrot", "Golf", "Hotel", "India", "Juliet",
    "Kilo", "Lima",
];

struct Rows;
impl ListModel for Rows {
    fn len(&self) -> usize {
        ROWS.len()
    }
    fn row(&self, index: usize) -> Option<ListRow<'_>> {
        ROWS.get(index).map(|label| ListRow::new(label))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GalleryResponse {
    Control(UiResponse),
    Dialog(DialogResult),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GalleryStep {
    pub focused: Option<WidgetId>,
    pub response: Option<GalleryResponse>,
    pub damage: Vec<UiRect>,
}

/// Simulator developer surface; not a KPA package or shell catalog entry.
pub struct UiGallery {
    context: UiContext<16>,
    focus: FocusManager<16>,
    panel: Panel<'static>,
    label: Label<'static>,
    button: Button<'static>,
    checkbox: Checkbox<'static>,
    list: List,
    field: TextField<'static>,
    disabled: Button<'static>,
    dialog: Dialog<'static, 4, 2>,
    ok: Button<'static>,
    cancel: Button<'static>,
    text: [u8; 64],
    text_len: usize,
    composition: bool,
}

impl UiGallery {
    pub fn new() -> Self {
        let mut context = UiContext::new(GALLERY_SURFACE, Theme::DARK);
        let mut focus = FocusManager::new();
        for (id, bounds, enabled) in [
            (BUTTON, UiRect::new(20, 66, 120, 24), true),
            (CHECKBOX, UiRect::new(164, 66, 136, 24), true),
            (LIST, UiRect::new(20, 104, 140, 112), true),
            (FIELD, UiRect::new(176, 104, 124, 24), true),
            (DISABLED, UiRect::new(176, 146, 124, 24), false),
        ] {
            let mut entry = FocusEntry::new(id, bounds, FocusScopeId::ROOT);
            entry.enabled = enabled;
            focus.register(entry).unwrap();
        }
        for (id, bounds) in [
            (OK, UiRect::new(88, 190, 64, 24)),
            (CANCEL, UiRect::new(168, 190, 64, 24)),
        ] {
            focus.register(FocusEntry::new(id, bounds, MODAL)).unwrap();
        }
        focus.focus(BUTTON, &mut context).unwrap();
        let mut dialog = Dialog::new(
            DIALOG,
            MODAL,
            Panel::new(UiRect::new(56, 112, 208, 120))
                .with_title("Modal dialog")
                .with_dimmed_backdrop(GALLERY_SURFACE),
        );
        dialog.add_action(DialogAction::new(OK)).unwrap();
        dialog.add_action(DialogAction::new(CANCEL)).unwrap();
        dialog.set_default_action(OK).unwrap();
        let mut gallery = Self {
            context,
            focus,
            panel: Panel::new(UiRect::new(8, 8, 304, 304)).with_title("KotoUI Gallery"),
            label: Label::new(
                WidgetId::new(100),
                UiRect::new(20, 42, 280, 18),
                "Component states",
            ),
            button: Button::new(BUTTON, UiRect::new(20, 66, 120, 24), "Open dialog"),
            checkbox: Checkbox::new(CHECKBOX, UiRect::new(164, 66, 136, 24), "Checked"),
            list: List::new(LIST, UiRect::new(20, 104, 140, 112), 16),
            field: TextField::new(FIELD, UiRect::new(176, 104, 124, 24), "Type text"),
            disabled: Button::new(DISABLED, UiRect::new(176, 146, 124, 24), "Disabled"),
            dialog,
            ok: Button::new(OK, UiRect::new(88, 190, 64, 24), "OK"),
            cancel: Button::new(CANCEL, UiRect::new(168, 190, 64, 24), "Cancel"),
            text: [0; 64],
            text_len: 0,
            composition: false,
        };
        gallery.disabled.set_enabled(false, &mut gallery.context);
        gallery.list.sync_model(&Rows, &mut gallery.context);
        gallery.sync_focus();
        gallery.context.clear_damage();
        gallery
    }

    pub fn focused(&self) -> Option<WidgetId> {
        self.focus.focused()
    }
    pub fn dialog_is_open(&self) -> bool {
        self.dialog.is_open()
    }
    pub fn checkbox_is_checked(&self) -> bool {
        self.checkbox.is_checked()
    }
    pub fn list_first_visible(&self) -> usize {
        self.list.first_visible()
    }
    pub fn composition_is_visible(&self) -> bool {
        self.composition
    }
    pub fn text_is_editing(&self) -> bool {
        self.field.is_editing()
    }
    pub fn text(&self) -> &str {
        core::str::from_utf8(&self.text[..self.text_len]).expect("valid gallery UTF-8")
    }
    pub fn clear_damage(&mut self) {
        self.context.clear_damage();
    }

    pub fn set_composition(&mut self, visible: bool) -> GalleryStep {
        self.context.clear_damage();
        if self.composition != visible {
            self.composition = visible;
            self.field.invalidate_composition(&mut self.context);
        }
        self.step(None)
    }

    pub fn handle_event(&mut self, event: UiEvent) -> GalleryStep {
        self.context.clear_damage();
        let response = if self.dialog.is_open() {
            self.handle_dialog(event)
        } else {
            self.handle_root(event)
        };
        self.sync_focus();
        self.step(response)
    }

    pub fn render(&mut self, font: &BitmapFont<'_>) -> Framebuffer {
        let mut framebuffer = Framebuffer::new(320, 320);
        let mut canvas = framebuffer.as_canvas();
        let mut painter = CanvasUiPainter::new(&mut canvas, font);
        self.paint(&mut painter, GALLERY_SURFACE).unwrap();
        framebuffer
    }

    pub fn paint_damage(&mut self, font: &BitmapFont<'_>, framebuffer: &mut Framebuffer) -> usize {
        let damage: Vec<_> = self.context.damaged_rects().collect();
        let mut canvas = framebuffer.as_canvas();
        let mut painter = CanvasUiPainter::new(&mut canvas, font);
        for clip in &damage {
            self.paint(&mut painter, *clip).unwrap();
        }
        damage.len()
    }

    fn handle_root(&mut self, event: UiEvent) -> Option<GalleryResponse> {
        if event.phase != EventPhase::Released {
            if let UiAction::Navigate(direction) = event.action {
                if self.focus.focused() == Some(FIELD)
                    && self.field.is_editing()
                    && matches!(direction, Navigation::Left | Navigation::Right)
                {
                    let mut value =
                        Utf8Buffer::from_initialized(&mut self.text, self.text_len).unwrap();
                    let response = self
                        .field
                        .handle_event(&mut value, event, &mut self.context);
                    self.text_len = value.len();
                    return response.map(GalleryResponse::Control);
                }
                if self.focus.focused() == Some(LIST)
                    && matches!(direction, Navigation::Up | Navigation::Down)
                {
                    if let Some(response) = self.list.handle_event(&Rows, event, &mut self.context)
                    {
                        return Some(GalleryResponse::Control(response));
                    }
                }
                match direction {
                    Navigation::Next => {
                        let _ = self.focus.move_id_next(&mut self.context);
                        return None;
                    }
                    Navigation::Previous => {
                        let _ = self.focus.move_id_previous(&mut self.context);
                        return None;
                    }
                    Navigation::Up | Navigation::Down | Navigation::Left | Navigation::Right => {
                        if matches!(
                            self.focus.move_spatial(direction, &mut self.context),
                            Ok(Some(_))
                        ) {
                            return None;
                        }
                    }
                    Navigation::PageUp | Navigation::PageDown => {}
                }
            }
        }
        let focused = self.focus.focused();
        let response = match focused {
            Some(BUTTON) => self.button.handle_event(event, &mut self.context),
            Some(CHECKBOX) => self.checkbox.handle_event(event, &mut self.context),
            Some(LIST) => self.list.handle_event(&Rows, event, &mut self.context),
            Some(FIELD) => {
                let mut value =
                    Utf8Buffer::from_initialized(&mut self.text, self.text_len).unwrap();
                let response = self
                    .field
                    .handle_event(&mut value, event, &mut self.context);
                self.text_len = value.len();
                response
            }
            _ => None,
        };
        if focused == Some(BUTTON)
            && matches!(
                response.map(|value| value.kind),
                Some(ResponseKind::Activated)
            )
            && event.phase == EventPhase::Pressed
        {
            self.dialog
                .open(&mut self.focus, &mut self.context)
                .unwrap();
        }
        response.map(GalleryResponse::Control)
    }

    fn handle_dialog(&mut self, event: UiEvent) -> Option<GalleryResponse> {
        if event.phase != EventPhase::Released {
            if let UiAction::Navigate(direction) = event.action {
                match direction {
                    Navigation::Next => {
                        let _ = self.focus.move_id_next(&mut self.context);
                        return None;
                    }
                    Navigation::Previous => {
                        let _ = self.focus.move_id_previous(&mut self.context);
                        return None;
                    }
                    Navigation::Up | Navigation::Down | Navigation::Left | Navigation::Right => {
                        if matches!(
                            self.focus.move_spatial(direction, &mut self.context),
                            Ok(Some(_))
                        ) {
                            return None;
                        }
                    }
                    Navigation::PageUp | Navigation::PageDown => {}
                }
            }
        }
        self.dialog
            .handle_event(event, &mut self.focus, &mut self.context)
            .unwrap()
            .map(GalleryResponse::Dialog)
    }

    fn sync_focus(&mut self) {
        let focused = self.focus.focused();
        self.button
            .set_focused(focused == Some(BUTTON), &mut self.context);
        self.checkbox
            .set_focused(focused == Some(CHECKBOX), &mut self.context);
        self.list
            .set_focused(focused == Some(LIST), &mut self.context);
        self.field
            .set_focused(focused == Some(FIELD), &mut self.context);
        self.ok.set_focused(focused == Some(OK), &mut self.context);
        self.cancel
            .set_focused(focused == Some(CANCEL), &mut self.context);
    }

    fn step(&self, response: Option<GalleryResponse>) -> GalleryStep {
        GalleryStep {
            focused: self.focus.focused(),
            response,
            damage: self.context.damaged_rects().collect(),
        }
    }

    fn paint(
        &mut self,
        painter: &mut CanvasUiPainter<'_, '_, '_>,
        clip: UiRect,
    ) -> Result<(), koto_ui::PaintError> {
        self.panel.paint(painter, clip, &Theme::DARK)?;
        self.label.paint(painter, clip, &Theme::DARK)?;
        self.button.paint(painter, clip, &Theme::DARK)?;
        self.checkbox.paint(painter, clip, &Theme::DARK)?;
        self.list.paint(&Rows, painter, clip, &Theme::DARK)?;
        let value = Utf8Buffer::from_initialized(&mut self.text, self.text_len).unwrap();
        let composition = self.composition.then_some(ImeComposition {
            text: "かな",
            candidate: Some("仮名"),
        });
        self.field
            .paint(painter, clip, &Theme::DARK, &value, composition)?;
        self.disabled.paint(painter, clip, &Theme::DARK)?;
        if self.dialog.is_open() {
            self.dialog.panel().paint(painter, clip, &Theme::DARK)?;
            self.ok.paint(painter, clip, &Theme::DARK)?;
            self.cancel.paint(painter, clip, &Theme::DARK)?;
        }
        Ok(())
    }
}

impl Default for UiGallery {
    fn default() -> Self {
        Self::new()
    }
}
