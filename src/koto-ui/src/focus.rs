use crate::{ResponseKind, UiAction, UiContext, UiEvent, UiRect, UiResponse, WidgetId};

pub const DEFAULT_FOCUS_CAPACITY: usize = 16;

/// Caller-assigned focus scope. Scope zero is the root surface.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(transparent)]
pub struct FocusScopeId(u16);

impl FocusScopeId {
    pub const ROOT: Self = Self(0);

    pub const fn new(value: u16) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u16 {
        self.0
    }
}

/// One entry in explicit caller registration order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct FocusEntry {
    pub id: WidgetId,
    pub bounds: UiRect,
    pub scope: FocusScopeId,
    pub enabled: bool,
    pub visible: bool,
}

impl FocusEntry {
    pub const fn new(id: WidgetId, bounds: UiRect, scope: FocusScopeId) -> Self {
        Self {
            id,
            bounds,
            scope,
            enabled: true,
            visible: true,
        }
    }

    pub const fn is_focusable(self) -> bool {
        self.enabled && self.visible && !self.bounds.is_empty()
    }
}

const EMPTY_ENTRY: FocusEntry = FocusEntry {
    id: WidgetId::new(0),
    bounds: UiRect::EMPTY,
    scope: FocusScopeId::ROOT,
    enabled: false,
    visible: false,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegistrationError {
    Capacity,
    DuplicateId,
    UnknownWidget,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FocusError {
    UnknownWidget,
    NotFocusable,
    WrongScope,
    NoFocusableTarget,
    ModalAlreadyOpen,
    NoModalOpen,
    RootCannotBeModal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ModalFrame {
    scope: FocusScopeId,
    prior_scope: FocusScopeId,
    prior_focus: Option<WidgetId>,
}

/// Fixed-capacity focus registry for a flat component collection.
///
/// Sequential traversal can follow registration order or stable widget IDs.
/// Spatial traversal selects the nearest focusable entry in the requested
/// direction within the active scope.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FocusManager<const N: usize = DEFAULT_FOCUS_CAPACITY> {
    entries: [FocusEntry; N],
    len: usize,
    focused: Option<WidgetId>,
    active_scope: FocusScopeId,
    modal: Option<ModalFrame>,
}

impl<const N: usize> FocusManager<N> {
    pub const fn new() -> Self {
        Self {
            entries: [EMPTY_ENTRY; N],
            len: 0,
            focused: None,
            active_scope: FocusScopeId::ROOT,
            modal: None,
        }
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub const fn focused(&self) -> Option<WidgetId> {
        self.focused
    }

    pub const fn active_scope(&self) -> FocusScopeId {
        self.active_scope
    }

    pub const fn modal_is_open(&self) -> bool {
        self.modal.is_some()
    }

    pub fn entry(&self, id: WidgetId) -> Option<FocusEntry> {
        self.index_of(id).map(|index| self.entries[index])
    }

    pub fn register(&mut self, entry: FocusEntry) -> Result<(), RegistrationError> {
        if self.index_of(entry.id).is_some() {
            return Err(RegistrationError::DuplicateId);
        }
        if self.len >= N {
            return Err(RegistrationError::Capacity);
        }
        self.entries[self.len] = entry;
        self.len += 1;
        Ok(())
    }

    pub fn unregister<const DAMAGE: usize>(
        &mut self,
        id: WidgetId,
        context: &mut UiContext<DAMAGE>,
    ) -> Result<Option<UiResponse>, RegistrationError> {
        let index = self.index_of(id).ok_or(RegistrationError::UnknownWidget)?;
        let was_focused = self.focused == Some(id);
        let old_bounds = self.entries[index].bounds;
        self.remove(index);

        if !was_focused {
            return Ok(None);
        }

        let next = self.find_from(index, true);
        self.focused = next.map(|entry_index| self.entries[entry_index].id);
        context.damage(old_bounds);
        if let Some(entry_index) = next {
            context.damage(self.entries[entry_index].bounds);
            Ok(Some(UiResponse::new(
                self.entries[entry_index].id,
                ResponseKind::FocusChanged,
            )))
        } else {
            Ok(None)
        }
    }

    pub fn update<const DAMAGE: usize>(
        &mut self,
        entry: FocusEntry,
        context: &mut UiContext<DAMAGE>,
    ) -> Result<Option<UiResponse>, RegistrationError> {
        let index = self
            .index_of(entry.id)
            .ok_or(RegistrationError::UnknownWidget)?;
        let previous = self.entries[index];
        self.entries[index] = entry;

        if self.focused != Some(entry.id) {
            return Ok(None);
        }
        if entry.scope == self.active_scope && entry.is_focusable() {
            if previous.bounds != entry.bounds {
                context.damage(previous.bounds);
                context.damage(entry.bounds);
            }
            return Ok(None);
        }

        let next = self.find_from(index + 1, true);
        context.damage(previous.bounds);
        self.focused = next.map(|next_index| self.entries[next_index].id);
        if let Some(next_index) = next {
            context.damage(self.entries[next_index].bounds);
            Ok(Some(UiResponse::new(
                self.entries[next_index].id,
                ResponseKind::FocusChanged,
            )))
        } else {
            Ok(None)
        }
    }

    pub fn focus<const DAMAGE: usize>(
        &mut self,
        id: WidgetId,
        context: &mut UiContext<DAMAGE>,
    ) -> Result<Option<UiResponse>, FocusError> {
        let index = self.index_of(id).ok_or(FocusError::UnknownWidget)?;
        let entry = self.entries[index];
        if entry.scope != self.active_scope {
            return Err(FocusError::WrongScope);
        }
        if !entry.is_focusable() {
            return Err(FocusError::NotFocusable);
        }
        self.apply_focus_index(Some(index), context)
    }

    pub fn focus_first<const DAMAGE: usize>(
        &mut self,
        context: &mut UiContext<DAMAGE>,
    ) -> Result<Option<UiResponse>, FocusError> {
        let next = self.find_from(0, true);
        if next.is_none() {
            return Err(FocusError::NoFocusableTarget);
        }
        self.apply_focus_index(next, context)
    }

    pub fn move_next<const DAMAGE: usize>(
        &mut self,
        context: &mut UiContext<DAMAGE>,
    ) -> Result<Option<UiResponse>, FocusError> {
        self.move_by(true, context)
    }

    pub fn move_previous<const DAMAGE: usize>(
        &mut self,
        context: &mut UiContext<DAMAGE>,
    ) -> Result<Option<UiResponse>, FocusError> {
        self.move_by(false, context)
    }

    /// Moves to the next focusable widget ID, wrapping at the largest ID.
    pub fn move_id_next<const DAMAGE: usize>(
        &mut self,
        context: &mut UiContext<DAMAGE>,
    ) -> Result<Option<UiResponse>, FocusError> {
        self.move_by_id(true, context)
    }

    /// Moves to the previous focusable widget ID, wrapping at the smallest ID.
    pub fn move_id_previous<const DAMAGE: usize>(
        &mut self,
        context: &mut UiContext<DAMAGE>,
    ) -> Result<Option<UiResponse>, FocusError> {
        self.move_by_id(false, context)
    }

    /// Moves to the nearest focusable widget wholly beyond the requested edge.
    /// Spatial navigation does not wrap when no widget exists in that direction.
    pub fn move_spatial<const DAMAGE: usize>(
        &mut self,
        direction: crate::Navigation,
        context: &mut UiContext<DAMAGE>,
    ) -> Result<Option<UiResponse>, FocusError> {
        let current = self
            .focused
            .and_then(|id| self.index_of(id))
            .ok_or(FocusError::NoFocusableTarget)?;
        let next = self
            .find_spatial(current, direction)
            .ok_or(FocusError::NoFocusableTarget)?;
        self.apply_focus_index(Some(next), context)
    }

    pub fn open_modal<const DAMAGE: usize>(
        &mut self,
        scope: FocusScopeId,
        initial: Option<WidgetId>,
        context: &mut UiContext<DAMAGE>,
    ) -> Result<Option<UiResponse>, FocusError> {
        if scope == FocusScopeId::ROOT {
            return Err(FocusError::RootCannotBeModal);
        }
        if self.modal.is_some() {
            return Err(FocusError::ModalAlreadyOpen);
        }

        let target = if let Some(id) = initial {
            let index = self.index_of(id).ok_or(FocusError::UnknownWidget)?;
            let entry = self.entries[index];
            if entry.scope != scope {
                return Err(FocusError::WrongScope);
            }
            if !entry.is_focusable() {
                return Err(FocusError::NotFocusable);
            }
            index
        } else {
            self.find_in_scope(scope, 0, true)
                .ok_or(FocusError::NoFocusableTarget)?
        };

        self.modal = Some(ModalFrame {
            scope,
            prior_scope: self.active_scope,
            prior_focus: self.focused,
        });
        self.active_scope = scope;
        self.apply_focus_index(Some(target), context)
    }

    pub fn close_modal<const DAMAGE: usize>(
        &mut self,
        context: &mut UiContext<DAMAGE>,
    ) -> Result<Option<UiResponse>, FocusError> {
        let modal = self.modal.take().ok_or(FocusError::NoModalOpen)?;
        self.active_scope = modal.prior_scope;
        let target = modal
            .prior_focus
            .and_then(|id| self.index_of(id))
            .filter(|index| self.eligible(*index))
            .or_else(|| self.find_from(0, true));
        self.apply_focus_index(target, context)
    }

    /// Routes an event to the focused component and returns a semantic result.
    ///
    /// Directional input is not consumed as focus traversal automatically;
    /// controls such as lists and text fields receive it first. Activate and
    /// cancel are normalized into their semantic response kinds. Cancelling an
    /// open modal also restores the previous focus.
    pub fn dispatch<const DAMAGE: usize>(
        &mut self,
        event: UiEvent,
        context: &mut UiContext<DAMAGE>,
    ) -> Option<UiResponse> {
        let target = self.focused?;
        match event.action {
            UiAction::Activate => Some(UiResponse::new(target, ResponseKind::Activated)),
            UiAction::Cancel => {
                if self.modal.is_some() {
                    let _ = self.close_modal(context);
                }
                Some(UiResponse::new(target, ResponseKind::Cancelled))
            }
            _ => Some(UiResponse::new(target, ResponseKind::Input(event))),
        }
    }

    fn move_by<const DAMAGE: usize>(
        &mut self,
        forward: bool,
        context: &mut UiContext<DAMAGE>,
    ) -> Result<Option<UiResponse>, FocusError> {
        let start = self
            .focused
            .and_then(|id| self.index_of(id))
            .map(|index| if forward { index + 1 } else { index })
            .unwrap_or(if forward { 0 } else { self.len });
        let next = self.find_from(start, forward);
        if next.is_none() {
            return Err(FocusError::NoFocusableTarget);
        }
        self.apply_focus_index(next, context)
    }

    fn move_by_id<const DAMAGE: usize>(
        &mut self,
        forward: bool,
        context: &mut UiContext<DAMAGE>,
    ) -> Result<Option<UiResponse>, FocusError> {
        let current = self.focused.map_or(0, WidgetId::get);
        let mut wrapped: Option<(u16, usize)> = None;
        let mut adjacent: Option<(u16, usize)> = None;
        for index in 0..self.len {
            if !self.eligible(index) {
                continue;
            }
            let id = self.entries[index].id.get();
            let wrapped_is_better =
                wrapped.is_none_or(|(best, _)| if forward { id < best } else { id > best });
            if wrapped_is_better {
                wrapped = Some((id, index));
            }
            let follows = if forward { id > current } else { id < current };
            let adjacent_is_better =
                adjacent.is_none_or(|(best, _)| if forward { id < best } else { id > best });
            if follows && adjacent_is_better {
                adjacent = Some((id, index));
            }
        }
        let next = adjacent.or(wrapped).map(|(_, index)| index);
        if next.is_none() {
            return Err(FocusError::NoFocusableTarget);
        }
        self.apply_focus_index(next, context)
    }

    fn find_spatial(&self, current: usize, direction: crate::Navigation) -> Option<usize> {
        let origin = self.entries[current].bounds;
        let origin_x = i64::from(origin.x) * 2 + i64::from(origin.w);
        let origin_y = i64::from(origin.y) * 2 + i64::from(origin.h);
        let mut best: Option<((i64, i64, i64, u16), usize)> = None;
        for index in 0..self.len {
            if index == current || !self.eligible(index) {
                continue;
            }
            let bounds = self.entries[index].bounds;
            let dx = i64::from(bounds.x) * 2 + i64::from(bounds.w) - origin_x;
            let dy = i64::from(bounds.y) * 2 + i64::from(bounds.h) - origin_y;
            let origin_left = i64::from(origin.x);
            let origin_top = i64::from(origin.y);
            let origin_right = origin_left + i64::from(origin.w);
            let origin_bottom = origin_top + i64::from(origin.h);
            let candidate_left = i64::from(bounds.x);
            let candidate_top = i64::from(bounds.y);
            let candidate_right = candidate_left + i64::from(bounds.w);
            let candidate_bottom = candidate_top + i64::from(bounds.h);
            let (in_direction, primary, cross) = match direction {
                crate::Navigation::Up => (candidate_bottom <= origin_top, -dy, dx.abs()),
                crate::Navigation::Down => (candidate_top >= origin_bottom, dy, dx.abs()),
                crate::Navigation::Left => (candidate_right <= origin_left, -dx, dy.abs()),
                crate::Navigation::Right => (candidate_left >= origin_right, dx, dy.abs()),
                _ => return None,
            };
            if !in_direction {
                continue;
            }
            let distance = dx.saturating_mul(dx).saturating_add(dy.saturating_mul(dy));
            let score = (distance, cross, primary, self.entries[index].id.get());
            if best.is_none_or(|(best_score, _)| score < best_score) {
                best = Some((score, index));
            }
        }
        best.map(|(_, index)| index)
    }

    fn apply_focus_index<const DAMAGE: usize>(
        &mut self,
        next: Option<usize>,
        context: &mut UiContext<DAMAGE>,
    ) -> Result<Option<UiResponse>, FocusError> {
        let next_id = next.map(|index| self.entries[index].id);
        if self.focused == next_id {
            return Ok(None);
        }
        if let Some(old) = self.focused.and_then(|id| self.index_of(id)) {
            context.damage(self.entries[old].bounds);
        }
        self.focused = next_id;
        if let Some(index) = next {
            context.damage(self.entries[index].bounds);
            Ok(Some(UiResponse::new(
                self.entries[index].id,
                ResponseKind::FocusChanged,
            )))
        } else {
            Ok(None)
        }
    }

    fn find_from(&self, start: usize, forward: bool) -> Option<usize> {
        self.find_in_scope(self.active_scope, start, forward)
    }

    fn find_in_scope(&self, scope: FocusScopeId, start: usize, forward: bool) -> Option<usize> {
        if self.len == 0 {
            return None;
        }
        for offset in 0..self.len {
            let index = if forward {
                (start + offset) % self.len
            } else {
                (start + self.len - 1 - offset) % self.len
            };
            let entry = self.entries[index];
            if entry.scope == scope && entry.is_focusable() {
                return Some(index);
            }
        }
        None
    }

    fn eligible(&self, index: usize) -> bool {
        let entry = self.entries[index];
        entry.scope == self.active_scope && entry.is_focusable()
    }

    fn index_of(&self, id: WidgetId) -> Option<usize> {
        (0..self.len).find(|index| self.entries[*index].id == id)
    }

    fn remove(&mut self, index: usize) {
        let mut cursor = index;
        while cursor + 1 < self.len {
            self.entries[cursor] = self.entries[cursor + 1];
            cursor += 1;
        }
        self.len -= 1;
        self.entries[self.len] = EMPTY_ENTRY;
    }
}

impl<const N: usize> Default for FocusManager<N> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Theme;

    const ROOT_A: WidgetId = WidgetId::new(1);
    const ROOT_B: WidgetId = WidgetId::new(2);
    const ROOT_C: WidgetId = WidgetId::new(3);
    const MODAL_A: WidgetId = WidgetId::new(10);
    const MODAL_B: WidgetId = WidgetId::new(11);
    const MODAL: FocusScopeId = FocusScopeId::new(1);

    fn context() -> UiContext<8> {
        UiContext::new(UiRect::new(0, 0, 320, 320), Theme::DARK)
    }

    fn entry(id: WidgetId, x: i32, scope: FocusScopeId) -> FocusEntry {
        FocusEntry::new(id, UiRect::new(x, 0, 10, 10), scope)
    }

    fn manager() -> FocusManager<8> {
        let mut manager = FocusManager::new();
        manager
            .register(entry(ROOT_A, 0, FocusScopeId::ROOT))
            .unwrap();
        manager
            .register(entry(ROOT_B, 20, FocusScopeId::ROOT))
            .unwrap();
        manager
            .register(entry(ROOT_C, 40, FocusScopeId::ROOT))
            .unwrap();
        manager.register(entry(MODAL_A, 100, MODAL)).unwrap();
        manager.register(entry(MODAL_B, 120, MODAL)).unwrap();
        manager
    }

    #[test]
    fn empty_and_full_registries_fail_without_panicking() {
        let mut empty = FocusManager::<0>::new();
        assert_eq!(
            empty.register(entry(ROOT_A, 0, FocusScopeId::ROOT)),
            Err(RegistrationError::Capacity)
        );
        assert_eq!(
            empty.focus_first(&mut context()),
            Err(FocusError::NoFocusableTarget)
        );

        let mut one = FocusManager::<1>::new();
        one.register(entry(ROOT_A, 0, FocusScopeId::ROOT)).unwrap();
        assert_eq!(
            one.register(entry(ROOT_B, 20, FocusScopeId::ROOT)),
            Err(RegistrationError::Capacity)
        );
        assert_eq!(
            one.register(entry(ROOT_A, 0, FocusScopeId::ROOT)),
            Err(RegistrationError::DuplicateId)
        );
    }

    #[test]
    fn traversal_skips_disabled_and_hidden_entries_and_wraps() {
        let mut manager = manager();
        let mut context = context();
        let mut disabled = manager.entry(ROOT_B).unwrap();
        disabled.enabled = false;
        manager.update(disabled, &mut context).unwrap();
        let mut hidden = manager.entry(ROOT_C).unwrap();
        hidden.visible = false;
        manager.update(hidden, &mut context).unwrap();

        manager.focus_first(&mut context).unwrap();
        assert_eq!(manager.focused(), Some(ROOT_A));
        assert_eq!(manager.move_next(&mut context).unwrap(), None);
        assert_eq!(manager.focused(), Some(ROOT_A));
        assert_eq!(manager.move_previous(&mut context).unwrap(), None);
    }

    #[test]
    fn disabling_focused_entry_selects_successor_and_damages_old_bounds() {
        let mut manager = manager();
        let mut context = context();
        manager.focus(ROOT_B, &mut context).unwrap();
        context.clear_damage();

        let mut disabled = manager.entry(ROOT_B).unwrap();
        disabled.bounds = UiRect::new(200, 0, 10, 10);
        disabled.enabled = false;
        manager.update(disabled, &mut context).unwrap();

        assert_eq!(manager.focused(), Some(ROOT_C));
        assert_eq!(
            context.damaged_rects().collect::<std::vec::Vec<_>>(),
            [UiRect::new(20, 0, 10, 10), UiRect::new(40, 0, 10, 10)]
        );
    }

    #[test]
    fn focus_changes_damage_only_old_and_new_indicators() {
        let mut manager = manager();
        let mut context = context();
        manager.focus(ROOT_A, &mut context).unwrap();
        context.clear_damage();
        manager.focus(ROOT_C, &mut context).unwrap();
        assert_eq!(
            context.damaged_rects().collect::<std::vec::Vec<_>>(),
            [UiRect::new(0, 0, 10, 10), UiRect::new(40, 0, 10, 10)]
        );
    }

    #[test]
    fn removing_focus_selects_the_next_eligible_registration() {
        let mut manager = manager();
        let mut context = context();
        manager.focus(ROOT_B, &mut context).unwrap();
        context.clear_damage();
        manager.unregister(ROOT_B, &mut context).unwrap();
        assert_eq!(manager.focused(), Some(ROOT_C));
        assert_eq!(
            context.damaged_rects().collect::<std::vec::Vec<_>>(),
            [UiRect::new(20, 0, 10, 10), UiRect::new(40, 0, 10, 10)]
        );
    }

    #[test]
    fn modal_traps_traversal_and_restores_prior_focus() {
        let mut manager = manager();
        let mut context = context();
        manager.focus(ROOT_B, &mut context).unwrap();
        manager
            .open_modal(MODAL, Some(MODAL_A), &mut context)
            .unwrap();
        assert_eq!(manager.focused(), Some(MODAL_A));
        manager.move_previous(&mut context).unwrap();
        assert_eq!(manager.focused(), Some(MODAL_B));
        assert_eq!(
            manager.focus(ROOT_A, &mut context),
            Err(FocusError::WrongScope)
        );
        manager.close_modal(&mut context).unwrap();
        assert_eq!(manager.focused(), Some(ROOT_B));
    }

    #[test]
    fn modal_restore_uses_first_root_entry_when_prior_focus_is_gone() {
        let mut manager = manager();
        let mut context = context();
        manager.focus(ROOT_B, &mut context).unwrap();
        manager.open_modal(MODAL, None, &mut context).unwrap();
        manager.unregister(ROOT_B, &mut context).unwrap();
        manager.close_modal(&mut context).unwrap();
        assert_eq!(manager.focused(), Some(ROOT_A));
    }

    #[test]
    fn id_traversal_is_stable_when_registration_order_differs() {
        let mut manager = FocusManager::<4>::new();
        manager
            .register(entry(ROOT_C, 40, FocusScopeId::ROOT))
            .unwrap();
        manager
            .register(entry(ROOT_A, 0, FocusScopeId::ROOT))
            .unwrap();
        manager
            .register(entry(ROOT_B, 20, FocusScopeId::ROOT))
            .unwrap();
        let mut context = context();
        manager.focus(ROOT_A, &mut context).unwrap();
        manager.move_id_next(&mut context).unwrap();
        assert_eq!(manager.focused(), Some(ROOT_B));
        manager.move_id_previous(&mut context).unwrap();
        assert_eq!(manager.focused(), Some(ROOT_A));
        manager.move_id_previous(&mut context).unwrap();
        assert_eq!(manager.focused(), Some(ROOT_C));
    }

    #[test]
    fn spatial_traversal_chooses_nearest_center_in_requested_direction() {
        let mut manager = FocusManager::<4>::new();
        manager
            .register(FocusEntry::new(
                ROOT_A,
                UiRect::new(40, 40, 20, 20),
                FocusScopeId::ROOT,
            ))
            .unwrap();
        manager
            .register(FocusEntry::new(
                ROOT_B,
                UiRect::new(70, 42, 20, 20),
                FocusScopeId::ROOT,
            ))
            .unwrap();
        manager
            .register(FocusEntry::new(
                ROOT_C,
                UiRect::new(48, 80, 20, 20),
                FocusScopeId::ROOT,
            ))
            .unwrap();
        let mut context = context();
        manager.focus(ROOT_A, &mut context).unwrap();
        manager
            .move_spatial(crate::Navigation::Right, &mut context)
            .unwrap();
        assert_eq!(manager.focused(), Some(ROOT_B));
        manager.focus(ROOT_A, &mut context).unwrap();
        manager
            .move_spatial(crate::Navigation::Down, &mut context)
            .unwrap();
        assert_eq!(manager.focused(), Some(ROOT_C));
        assert_eq!(
            manager.move_spatial(crate::Navigation::Down, &mut context),
            Err(FocusError::NoFocusableTarget)
        );
        assert_eq!(manager.focused(), Some(ROOT_C));
    }

    #[test]
    fn dispatch_returns_responses_and_cancel_closes_modal() {
        let mut manager = manager();
        let mut context = context();
        manager.focus(ROOT_A, &mut context).unwrap();
        assert_eq!(
            manager.dispatch(UiEvent::pressed(UiAction::Activate), &mut context),
            Some(UiResponse::new(ROOT_A, ResponseKind::Activated))
        );
        let repeated = UiEvent::repeated(UiAction::Navigate(crate::Navigation::Down));
        assert_eq!(
            manager.dispatch(repeated, &mut context),
            Some(UiResponse::new(ROOT_A, ResponseKind::Input(repeated)))
        );

        manager
            .open_modal(MODAL, Some(MODAL_A), &mut context)
            .unwrap();
        assert_eq!(
            manager.dispatch(UiEvent::pressed(UiAction::Cancel), &mut context),
            Some(UiResponse::new(MODAL_A, ResponseKind::Cancelled))
        );
        assert!(!manager.modal_is_open());
        assert_eq!(manager.focused(), Some(ROOT_A));
    }
}
