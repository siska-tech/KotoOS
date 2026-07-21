use crate::{
    EventPhase, FocusError, FocusManager, FocusScopeId, UiAction, UiContext, UiEvent, WidgetId,
};

use super::{LayoutError, Panel};

pub const DEFAULT_DIALOG_CHILD_CAPACITY: usize = 8;
pub const DEFAULT_DIALOG_ACTION_CAPACITY: usize = 4;

const EMPTY_ID: WidgetId = WidgetId::new(0);
const EMPTY_ACTION: DialogAction = DialogAction {
    id: EMPTY_ID,
    enabled: false,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct DialogAction {
    pub id: WidgetId,
    pub enabled: bool,
}

impl DialogAction {
    pub const fn new(id: WidgetId) -> Self {
        Self { id, enabled: true }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DialogResultKind {
    Accepted,
    Cancelled,
    Closed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DialogResult {
    pub dialog: WidgetId,
    pub action: Option<WidgetId>,
    pub kind: DialogResultKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DialogError {
    ChildCapacity,
    ActionCapacity,
    DuplicateId,
    UnknownAction,
    AlreadyOpen,
    NotOpen,
    Layout(LayoutError),
    Focus(FocusError),
}

impl From<FocusError> for DialogError {
    fn from(value: FocusError) -> Self {
        Self::Focus(value)
    }
}

/// Flat, fixed-capacity modal composition metadata.
///
/// The caller owns actual controls and registers their focus entries. Dialog
/// retains only stable IDs, action availability, and modal lifecycle state.
#[derive(Clone, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct Dialog<
    'a,
    const CHILDREN: usize = DEFAULT_DIALOG_CHILD_CAPACITY,
    const ACTIONS: usize = DEFAULT_DIALOG_ACTION_CAPACITY,
> {
    id: WidgetId,
    scope: FocusScopeId,
    panel: Panel<'a>,
    children: [WidgetId; CHILDREN],
    child_len: usize,
    actions: [DialogAction; ACTIONS],
    action_len: usize,
    default_action: Option<WidgetId>,
    open: bool,
}

impl<'a, const CHILDREN: usize, const ACTIONS: usize> Dialog<'a, CHILDREN, ACTIONS> {
    pub const fn new(id: WidgetId, scope: FocusScopeId, panel: Panel<'a>) -> Self {
        Self {
            id,
            scope,
            panel,
            children: [EMPTY_ID; CHILDREN],
            child_len: 0,
            actions: [EMPTY_ACTION; ACTIONS],
            action_len: 0,
            default_action: None,
            open: false,
        }
    }

    pub const fn id(&self) -> WidgetId {
        self.id
    }

    pub const fn scope(&self) -> FocusScopeId {
        self.scope
    }

    pub const fn panel(&self) -> &Panel<'a> {
        &self.panel
    }

    pub const fn is_open(&self) -> bool {
        self.open
    }

    pub const fn child_count(&self) -> usize {
        self.child_len
    }

    pub const fn action_count(&self) -> usize {
        self.action_len
    }

    pub fn children(&self) -> impl Iterator<Item = WidgetId> + '_ {
        self.children[..self.child_len].iter().copied()
    }

    pub fn actions(&self) -> impl Iterator<Item = DialogAction> + '_ {
        self.actions[..self.action_len].iter().copied()
    }

    pub fn add_child(&mut self, id: WidgetId) -> Result<(), DialogError> {
        if self.contains(id) {
            return Err(DialogError::DuplicateId);
        }
        if self.child_len >= CHILDREN {
            return Err(DialogError::ChildCapacity);
        }
        self.children[self.child_len] = id;
        self.child_len += 1;
        Ok(())
    }

    pub fn add_action(&mut self, action: DialogAction) -> Result<(), DialogError> {
        if self.contains(action.id) {
            return Err(DialogError::DuplicateId);
        }
        if self.action_len >= ACTIONS {
            return Err(DialogError::ActionCapacity);
        }
        if self.child_len >= CHILDREN {
            return Err(DialogError::ChildCapacity);
        }
        self.children[self.child_len] = action.id;
        self.child_len += 1;
        self.actions[self.action_len] = action;
        self.action_len += 1;
        Ok(())
    }

    pub fn set_default_action(&mut self, id: WidgetId) -> Result<(), DialogError> {
        self.action_index(id).ok_or(DialogError::UnknownAction)?;
        self.default_action = Some(id);
        Ok(())
    }

    pub fn set_action_enabled(&mut self, id: WidgetId, enabled: bool) -> Result<(), DialogError> {
        let index = self.action_index(id).ok_or(DialogError::UnknownAction)?;
        self.actions[index].enabled = enabled;
        Ok(())
    }

    pub fn open<const FOCUS: usize, const DAMAGE: usize>(
        &mut self,
        focus: &mut FocusManager<FOCUS>,
        context: &mut UiContext<DAMAGE>,
    ) -> Result<(), DialogError> {
        if self.open {
            return Err(DialogError::AlreadyOpen);
        }
        self.panel
            .layout(context.theme())
            .map_err(DialogError::Layout)?;
        let initial = self.initial_action();
        focus.open_modal(self.scope, initial, context)?;
        self.open = true;
        self.damage_full(context);
        Ok(())
    }

    pub fn handle_event<const FOCUS: usize, const DAMAGE: usize>(
        &mut self,
        event: UiEvent,
        focus: &mut FocusManager<FOCUS>,
        context: &mut UiContext<DAMAGE>,
    ) -> Result<Option<DialogResult>, DialogError> {
        if !self.open {
            return Err(DialogError::NotOpen);
        }
        if event.phase != EventPhase::Pressed {
            return Ok(None);
        }
        match event.action {
            UiAction::Activate => {
                let Some(id) = focus.focused() else {
                    return Ok(None);
                };
                let Some(index) = self.action_index(id) else {
                    return Ok(None);
                };
                if !self.actions[index].enabled {
                    return Ok(None);
                }
                self.finish(DialogResultKind::Accepted, Some(id), focus, context)
                    .map(Some)
            }
            UiAction::Cancel => self
                .finish(DialogResultKind::Cancelled, None, focus, context)
                .map(Some),
            _ => Ok(None),
        }
    }

    pub fn close<const FOCUS: usize, const DAMAGE: usize>(
        &mut self,
        focus: &mut FocusManager<FOCUS>,
        context: &mut UiContext<DAMAGE>,
    ) -> Result<DialogResult, DialogError> {
        if !self.open {
            return Err(DialogError::NotOpen);
        }
        let action = focus
            .focused()
            .filter(|id| self.action_index(*id).is_some());
        self.finish(DialogResultKind::Closed, action, focus, context)
    }

    fn finish<const FOCUS: usize, const DAMAGE: usize>(
        &mut self,
        kind: DialogResultKind,
        action: Option<WidgetId>,
        focus: &mut FocusManager<FOCUS>,
        context: &mut UiContext<DAMAGE>,
    ) -> Result<DialogResult, DialogError> {
        focus.close_modal(context)?;
        self.open = false;
        self.damage_full(context);
        Ok(DialogResult {
            dialog: self.id,
            action,
            kind,
        })
    }

    fn initial_action(&self) -> Option<WidgetId> {
        self.default_action
            .and_then(|id| self.action_index(id).map(|index| (id, index)))
            .filter(|(_, index)| self.actions[*index].enabled)
            .map(|(id, _)| id)
            .or_else(|| {
                self.actions[..self.action_len]
                    .iter()
                    .find(|action| action.enabled)
                    .map(|action| action.id)
            })
    }

    fn damage_full<const DAMAGE: usize>(&self, context: &mut UiContext<DAMAGE>) {
        if let Some(backdrop) = self.panel.backdrop() {
            context.damage(backdrop);
        }
        context.damage(self.panel.bounds());
    }

    fn contains(&self, id: WidgetId) -> bool {
        self.children[..self.child_len].contains(&id)
    }

    fn action_index(&self, id: WidgetId) -> Option<usize> {
        self.actions[..self.action_len]
            .iter()
            .position(|action| action.id == id)
    }
}

#[cfg(target_pointer_width = "32")]
const _: [(); 100] = [(); core::mem::size_of::<Dialog<'static, 8, 4>>()];

#[cfg(test)]
mod tests {
    use core::mem::size_of;

    use super::*;
    use crate::{FocusEntry, Theme, UiRect};

    const ROOT: WidgetId = WidgetId::new(1);
    const DIALOG: WidgetId = WidgetId::new(20);
    const LABEL: WidgetId = WidgetId::new(21);
    const LIST: WidgetId = WidgetId::new(22);
    const FIELD: WidgetId = WidgetId::new(23);
    const OK: WidgetId = WidgetId::new(24);
    const CANCEL: WidgetId = WidgetId::new(25);
    const SCOPE: FocusScopeId = FocusScopeId::new(2);
    const SURFACE: UiRect = UiRect::new(0, 0, 320, 320);
    const PANEL_BOUNDS: UiRect = UiRect::new(40, 60, 240, 160);

    fn context() -> UiContext<8> {
        UiContext::new(SURFACE, Theme::DARK)
    }

    fn focus() -> FocusManager<8> {
        let mut focus = FocusManager::new();
        focus
            .register(FocusEntry::new(
                ROOT,
                UiRect::new(0, 0, 20, 20),
                FocusScopeId::ROOT,
            ))
            .unwrap();
        for (id, x) in [(LIST, 60), (FIELD, 80), (OK, 100), (CANCEL, 120)] {
            focus
                .register(FocusEntry::new(id, UiRect::new(x, 180, 20, 20), SCOPE))
                .unwrap();
        }
        focus
    }

    fn dialog() -> Dialog<'static, 8, 4> {
        let panel = Panel::new(PANEL_BOUNDS)
            .with_title("Confirm")
            .with_dimmed_backdrop(SURFACE);
        let mut dialog = Dialog::new(DIALOG, SCOPE, panel);
        dialog.add_child(LABEL).unwrap();
        dialog.add_action(DialogAction::new(OK)).unwrap();
        dialog.add_action(DialogAction::new(CANCEL)).unwrap();
        dialog.set_default_action(OK).unwrap();
        dialog
    }

    #[test]
    fn confirmation_accepts_default_and_restores_root_focus() {
        let mut context = context();
        let mut focus = focus();
        focus.focus(ROOT, &mut context).unwrap();
        let mut dialog = dialog();
        dialog.open(&mut focus, &mut context).unwrap();
        assert_eq!(focus.focused(), Some(OK));
        assert_eq!(focus.active_scope(), SCOPE);
        let result = dialog
            .handle_event(
                UiEvent::pressed(UiAction::Activate),
                &mut focus,
                &mut context,
            )
            .unwrap();
        assert_eq!(
            result,
            Some(DialogResult {
                dialog: DIALOG,
                action: Some(OK),
                kind: DialogResultKind::Accepted,
            })
        );
        assert_eq!(focus.focused(), Some(ROOT));
        assert!(!dialog.is_open());
    }

    #[test]
    fn disabled_default_falls_back_and_disabled_action_cannot_close() {
        let mut context = context();
        let mut focus = focus();
        focus.focus(ROOT, &mut context).unwrap();
        let mut dialog = dialog();
        dialog.set_action_enabled(OK, false).unwrap();
        let mut ok_entry = focus.entry(OK).unwrap();
        ok_entry.enabled = false;
        focus.update(ok_entry, &mut context).unwrap();
        dialog.open(&mut focus, &mut context).unwrap();
        assert_eq!(focus.focused(), Some(CANCEL));

        dialog.set_action_enabled(CANCEL, false).unwrap();
        assert_eq!(
            dialog
                .handle_event(
                    UiEvent::pressed(UiAction::Activate),
                    &mut focus,
                    &mut context
                )
                .unwrap(),
            None
        );
        assert!(dialog.is_open());
    }

    #[test]
    fn list_picker_and_text_prompt_keep_flat_child_ids_and_trap_focus() {
        let mut context = context();
        let mut focus = focus();
        focus.focus(ROOT, &mut context).unwrap();
        let mut picker: Dialog<4, 2> = Dialog::new(DIALOG, SCOPE, Panel::new(PANEL_BOUNDS));
        picker.add_child(LIST).unwrap();
        picker.add_action(DialogAction::new(OK)).unwrap();
        picker.open(&mut focus, &mut context).unwrap();
        assert_eq!(picker.children().collect::<Vec<_>>(), [LIST, OK]);
        assert_eq!(focus.focus(ROOT, &mut context), Err(FocusError::WrongScope));
        picker.close(&mut focus, &mut context).unwrap();

        let mut prompt: Dialog<4, 2> = Dialog::new(DIALOG, SCOPE, Panel::new(PANEL_BOUNDS));
        prompt.add_child(FIELD).unwrap();
        prompt.add_action(DialogAction::new(OK)).unwrap();
        prompt.open(&mut focus, &mut context).unwrap();
        assert_eq!(prompt.children().collect::<Vec<_>>(), [FIELD, OK]);
    }

    #[test]
    fn cancel_and_programmatic_close_identify_dialog_and_restore_focus() {
        let mut context = context();
        let mut focus = focus();
        focus.focus(ROOT, &mut context).unwrap();
        let mut dialog = dialog();
        dialog.open(&mut focus, &mut context).unwrap();
        assert_eq!(
            dialog
                .handle_event(UiEvent::pressed(UiAction::Cancel), &mut focus, &mut context)
                .unwrap()
                .unwrap()
                .kind,
            DialogResultKind::Cancelled
        );
        dialog.open(&mut focus, &mut context).unwrap();
        let closed = dialog.close(&mut focus, &mut context).unwrap();
        assert_eq!(closed.dialog, DIALOG);
        assert_eq!(closed.kind, DialogResultKind::Closed);
        assert_eq!(closed.action, Some(OK));
    }

    #[test]
    fn open_close_damage_backdrop_but_child_damage_stays_small() {
        let mut context = context();
        let mut focus = focus();
        focus.focus(ROOT, &mut context).unwrap();
        context.clear_damage();
        let mut dialog = dialog();
        dialog.open(&mut focus, &mut context).unwrap();
        assert_eq!(context.damaged_rects().collect::<Vec<_>>(), [SURFACE]);
        context.clear_damage();
        context.damage(UiRect::new(50, 80, 10, 10));
        assert_eq!(
            context.damaged_rects().collect::<Vec<_>>(),
            [UiRect::new(50, 80, 10, 10)]
        );
        context.clear_damage();
        dialog.close(&mut focus, &mut context).unwrap();
        assert_eq!(context.damaged_rects().collect::<Vec<_>>(), [SURFACE]);
    }

    #[test]
    fn impossible_geometry_and_capacity_fail_without_partial_open() {
        let mut context = context();
        let mut focus = focus();
        focus.focus(ROOT, &mut context).unwrap();
        context.clear_damage();
        let panel = Panel::new(UiRect::new(0, 0, 4, 4)).with_title("x");
        let mut invalid: Dialog<1, 1> = Dialog::new(DIALOG, SCOPE, panel);
        assert!(matches!(
            invalid.open(&mut focus, &mut context),
            Err(DialogError::Layout(_))
        ));
        assert!(!invalid.is_open());
        assert!(!focus.modal_is_open());
        assert!(!context.has_damage());

        let mut bounded: Dialog<1, 1> = Dialog::new(DIALOG, SCOPE, Panel::new(PANEL_BOUNDS));
        bounded.add_child(LABEL).unwrap();
        assert_eq!(
            bounded.add_action(DialogAction::new(OK)),
            Err(DialogError::ChildCapacity)
        );
        assert_eq!(bounded.action_count(), 0);
    }

    #[test]
    fn dialog_memory_is_fixed() {
        assert_eq!(size_of::<Dialog<'static, 8, 4>>(), 128);
    }
}
