/// Whether an input pulse is the initial press or a key-repeat pulse.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum EventPhase {
    #[default]
    Pressed,
    Repeated,
    Released,
}

/// Directional navigation intent independent of a physical key code.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Navigation {
    Up,
    Down,
    Left,
    Right,
    Next,
    Previous,
    PageUp,
    PageDown,
}

/// Device-independent input understood by KotoUI controls.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiAction {
    Navigate(Navigation),
    Activate,
    Cancel,
    Text(char),
    Backspace,
    Delete,
    Home,
    End,
    Submit,
}

/// One normalized input pulse.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiEvent {
    pub action: UiAction,
    pub phase: EventPhase,
}

impl UiEvent {
    pub const fn pressed(action: UiAction) -> Self {
        Self {
            action,
            phase: EventPhase::Pressed,
        }
    }

    pub const fn repeated(action: UiAction) -> Self {
        Self {
            action,
            phase: EventPhase::Repeated,
        }
    }

    pub const fn released(action: UiAction) -> Self {
        Self {
            action,
            phase: EventPhase::Released,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EventBufferFull;

/// Fixed-capacity event buffer used by input adapters without allocation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventBuffer<const N: usize> {
    events: [Option<UiEvent>; N],
    len: usize,
}

impl<const N: usize> EventBuffer<N> {
    pub const fn new() -> Self {
        Self {
            events: [None; N],
            len: 0,
        }
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn push(&mut self, event: UiEvent) -> Result<(), EventBufferFull> {
        if self.len >= N {
            return Err(EventBufferFull);
        }
        self.events[self.len] = Some(event);
        self.len += 1;
        Ok(())
    }

    pub fn clear(&mut self) {
        while self.len > 0 {
            self.len -= 1;
            self.events[self.len] = None;
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = UiEvent> + '_ {
        self.events[..self.len].iter().filter_map(|event| *event)
    }
}

impl<const N: usize> Default for EventBuffer<N> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_buffer_is_bounded_and_reusable() {
        let mut events = EventBuffer::<1>::new();
        assert_eq!(events.push(UiEvent::pressed(UiAction::Activate)), Ok(()));
        assert_eq!(
            events.push(UiEvent::pressed(UiAction::Cancel)),
            Err(EventBufferFull)
        );
        assert_eq!(events.iter().count(), 1);
        events.clear();
        assert!(events.is_empty());
        assert_eq!(events.push(UiEvent::repeated(UiAction::Backspace)), Ok(()));
    }
}
