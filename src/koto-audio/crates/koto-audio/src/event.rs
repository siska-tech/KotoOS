use crate::{
    source::{SourceLifecycleEvent, SourceState},
    AudioError, SourceId,
};

/// Logical audio event kind delivered through a future bounded event queue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AudioEventKind {
    /// A source completed normally.
    Completed,
    /// A source was stopped by request or policy.
    Stopped,
    /// A source was dropped before or during playback.
    Dropped,
    /// A source was stolen by a higher-priority source.
    ///
    /// TODO(KA-M1-004): emit this from priority admission once stealing exists.
    Stolen,
    /// A logical source or asset error occurred.
    Error(AudioError),
    /// A decoder reached terminal end before lifecycle completion dispatch.
    DecoderEnded,
    /// A decoder completed a loop iteration.
    LoopCompleted,
    /// The mixer/backend produced or required silence due to underrun.
    Underrun,
    /// The backend rejected a submitted mixer block.
    BackendSubmitFailed,
}

/// Pollable logical audio event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AudioEvent {
    /// Kind of event.
    pub kind: AudioEventKind,
    /// Source associated with this event, if any.
    pub source_id: Option<SourceId>,
}

impl AudioEvent {
    /// Creates a source-scoped event.
    pub const fn for_source(kind: AudioEventKind, source_id: SourceId) -> Self {
        Self {
            kind,
            source_id: Some(source_id),
        }
    }

    /// Creates a runtime-level event with no source id.
    pub const fn runtime(kind: AudioEventKind) -> Self {
        Self {
            kind,
            source_id: None,
        }
    }
}

impl From<SourceLifecycleEvent> for AudioEvent {
    fn from(event: SourceLifecycleEvent) -> Self {
        let kind = match event.state {
            SourceState::Completed => AudioEventKind::Completed,
            SourceState::Stopping => AudioEventKind::Stopped,
            SourceState::Dropped => AudioEventKind::Dropped,
            SourceState::Stolen => AudioEventKind::Stolen,
            SourceState::Error => {
                AudioEventKind::Error(event.error.unwrap_or(AudioError::InvalidArgument))
            }
            SourceState::Queued | SourceState::Playing => {
                AudioEventKind::Error(AudioError::InvalidArgument)
            }
        };

        Self::for_source(kind, event.source_id)
    }
}

/// Bounded FIFO queue for pollable logical audio events.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AudioEventQueue<const MAX_EVENTS: usize> {
    items: [Option<AudioEvent>; MAX_EVENTS],
    head: usize,
    len: usize,
}

impl<const MAX_EVENTS: usize> AudioEventQueue<MAX_EVENTS> {
    /// Creates an empty event queue.
    pub const fn new() -> Self {
        Self {
            items: [None; MAX_EVENTS],
            head: 0,
            len: 0,
        }
    }

    /// Pushes an event to the back of the queue.
    pub fn push(&mut self, event: AudioEvent) -> Result<(), AudioEvent> {
        if self.len == MAX_EVENTS {
            return Err(event);
        }

        let index = (self.head + self.len) % MAX_EVENTS;
        self.items[index] = Some(event);
        self.len += 1;
        Ok(())
    }

    /// Pops the oldest event from the queue.
    pub fn pop(&mut self) -> Option<AudioEvent> {
        if self.len == 0 {
            return None;
        }

        let event = self.items[self.head].take();
        self.head = (self.head + 1) % MAX_EVENTS;
        self.len -= 1;
        event
    }

    /// Removes all queued events.
    pub fn clear(&mut self) {
        while self.pop().is_some() {}
    }
}

impl<const MAX_EVENTS: usize> Default for AudioEventQueue<MAX_EVENTS> {
    fn default() -> Self {
        Self::new()
    }
}
