use crate::{
    counters::AudioCounters, decoder::DecoderState, AudioError, AudioLimits, AudioResult,
    ClipAsset, Decoder, DropPolicy, MixerVolume, PolyphonicSequence, Sequence,
};

/// Generation tag used to reject stale logical source identifiers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourceGeneration(u16);

impl SourceGeneration {
    /// Initial non-zero generation value.
    pub const INITIAL: Self = Self(1);

    /// Returns the next generation, wrapping past zero to keep zero reserved.
    pub const fn next(self) -> Self {
        let next = self.0.wrapping_add(1);
        if next == 0 {
            Self::INITIAL
        } else {
            Self(next)
        }
    }

    /// Returns the generation as a compact integer.
    pub const fn get(self) -> u16 {
        self.0
    }
}

impl Default for SourceGeneration {
    fn default() -> Self {
        Self::INITIAL
    }
}

/// Opaque logical source identifier returned by source admission APIs.
///
/// The slot index remains crate-private so normal callers cannot depend on the
/// internal source table layout.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourceId {
    slot: u16,
    generation: SourceGeneration,
}

impl SourceId {
    pub(crate) const fn new(slot: u16, generation: SourceGeneration) -> Self {
        Self { slot, generation }
    }

    pub(crate) const fn slot(self) -> u16 {
        self.slot
    }

    /// Returns the generation tag carried by this source id.
    pub const fn generation(self) -> SourceGeneration {
        self.generation
    }
}

/// Logical source lifecycle state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceState {
    /// The source is admitted to the bounded queue.
    Queued,
    /// The source has an active playback slot.
    Playing,
    /// The source has received a stop request.
    Stopping,
    /// The source completed normally.
    Completed,
    /// The source was dropped before it could complete.
    Dropped,
    /// The source was stolen by a future priority admission path.
    Stolen,
    /// The source failed with a logical audio error.
    Error,
}

/// Admission priority carried by source requests.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
#[allow(dead_code)]
pub enum SourcePriority {
    /// Low-priority effect.
    Low,
    /// Normal-priority effect.
    #[default]
    Normal,
    /// High-priority effect.
    High,
}

/// Future owner/app scope tag for a logical source.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SourceOwner {
    /// Opaque owner scope value.
    pub scope: u16,
}

/// Source play request metadata owned by the lifecycle layer.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SourceRequest<'a> {
    /// Runtime-ready clip to decode when the source becomes active.
    pub clip: Option<ClipAsset<'a>>,
    /// Static sequence to synthesize when the source becomes active.
    pub sequence: Option<Sequence<'a>>,
    /// Fixed-capacity polyphonic sequence to synthesize when active.
    pub poly_sequence: Option<PolyphonicSequence<'a>>,
    /// Source-local logical volume.
    pub volume: MixerVolume,
    /// Logical mix bus used for bus gain and BGM/SFX policy.
    pub(crate) bus: SourceBus,
    /// Admission priority. v0 records it but only rejects/drops on overflow.
    pub priority: SourcePriority,
    /// Future app/owner scope. v0 does not enforce permissions.
    pub owner: SourceOwner,
}

/// Minimal logical mix bus tag for BGM/SFX gain groundwork.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum SourceBus {
    /// Short effect or raw PCM clip path.
    #[default]
    Sfx,
    /// Long-running music path.
    Bgm,
}

/// A source promoted from the queue into an active slot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PromotedSource {
    /// Logical source identifier promoted to playing.
    pub source_id: SourceId,
    /// Opaque active slot identifier for crate/runtime internals.
    pub active_slot: ActiveSourceSlotId,
}

/// Opaque active source slot identifier.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActiveSourceSlotId(u16);

impl ActiveSourceSlotId {
    pub(crate) const fn new(slot: u16) -> Self {
        Self(slot)
    }

    pub(crate) const fn get(self) -> u16 {
        self.0
    }
}

/// Lightweight lifecycle event candidate for a future AudioEvent queue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourceLifecycleEvent {
    /// Source associated with the lifecycle event.
    pub source_id: SourceId,
    /// Resulting lifecycle state.
    pub state: SourceState,
    /// Optional logical error for stale ids or source failures.
    pub error: Option<AudioError>,
}

/// Bounded source lifecycle table, queue, and active slots.
///
/// `MAX_RECORDS` should be at least `MAX_ACTIVE + MAX_QUEUE` if all active
/// slots and queued requests may coexist at the configured limit.
#[derive(Debug)]
pub struct SourceLifecycle<
    'a,
    const MAX_RECORDS: usize,
    const MAX_ACTIVE: usize,
    const MAX_QUEUE: usize,
> {
    limits: AudioLimits,
    records: [SourceRecord<'a>; MAX_RECORDS],
    active: [ActiveSourceSlot<'a>; MAX_ACTIVE],
    queue: SourceQueue<MAX_QUEUE>,
}

impl<'a, const MAX_RECORDS: usize, const MAX_ACTIVE: usize, const MAX_QUEUE: usize>
    SourceLifecycle<'a, MAX_RECORDS, MAX_ACTIVE, MAX_QUEUE>
{
    /// Creates an empty source lifecycle using the supplied runtime limits.
    pub fn new(limits: AudioLimits) -> AudioResult<Self> {
        limits.validate()?;

        if usize::from(limits.max_sfx_sources) > MAX_ACTIVE
            || usize::from(limits.source_queue_depth) > MAX_QUEUE
            || usize::from(limits.max_sfx_sources) + usize::from(limits.source_queue_depth)
                > MAX_RECORDS
        {
            return Err(AudioError::InvalidArgument);
        }

        Ok(Self {
            limits,
            records: [SourceRecord::EMPTY; MAX_RECORDS],
            active: [ActiveSourceSlot::EMPTY; MAX_ACTIVE],
            queue: SourceQueue::new(),
        })
    }

    /// Clears every record, active slot, and queued id back to the empty
    /// freshly-constructed state **in place**, reusing this table's storage.
    ///
    /// Behaviourally identical to replacing the value with `Self::new(limits)`
    /// for the same (immutable, already-validated) limits, but it never
    /// materializes a whole replacement `SourceLifecycle` — whose records array
    /// dominates the ~5 KiB temporary that overflowed the core1 audio worker
    /// stack under LTO (KOTO-0186). Each element is a small `Copy` value, so the
    /// per-element fills touch only the live storage.
    pub fn reset(&mut self) {
        self.records.fill(SourceRecord::EMPTY);
        self.active.fill(ActiveSourceSlot::EMPTY);
        self.queue.clear();
    }

    /// Enqueues a source request without blocking.
    pub fn enqueue(
        &mut self,
        request: SourceRequest<'a>,
        counters: &mut AudioCounters,
        policy: DropPolicy,
    ) -> AudioResult<SourceId> {
        if let Some(clip) = request.clip {
            clip.validate(self.limits)?;
        }
        if let Some(sequence) = request.sequence {
            sequence.validate(self.limits)?;
        }
        if let Some(poly_sequence) = request.poly_sequence {
            poly_sequence.validate(self.limits)?;
        }
        let source_kind_count = request.clip.is_some() as u8
            + request.sequence.is_some() as u8
            + request.poly_sequence.is_some() as u8;
        if source_kind_count > 1 {
            return Err(AudioError::InvalidArgument);
        }

        if self.queue.len() >= usize::from(self.limits.source_queue_depth) {
            counters.queue_full_count = counters.queue_full_count.saturating_add(1);
            if matches!(policy, DropPolicy::DropNew) {
                counters.dropped_source_count = counters.dropped_source_count.saturating_add(1);
            }
            return Err(match policy {
                DropPolicy::RejectNew | DropPolicy::AllowSteal => AudioError::QueueFull,
                DropPolicy::DropNew => AudioError::AdmissionRejected,
            });
        }

        let record_index = self.allocate_record(SourceState::Queued, request)?;
        let source_id = SourceId::new(
            record_index,
            self.records[usize::from(record_index)].generation,
        );

        if self.queue.push(source_id).is_err() {
            self.release_record(source_id)?;
            counters.queue_full_count = counters.queue_full_count.saturating_add(1);
            return Err(AudioError::QueueFull);
        }

        counters.queued_source_count = counters.queued_source_count.saturating_add(1);
        Ok(source_id)
    }

    /// Promotes the oldest queued source into the first free active slot.
    pub fn promote_next(&mut self, counters: &mut AudioCounters) -> Option<PromotedSource> {
        let active_slot = self.first_free_active_slot()?;

        while let Some(source_id) = self.queue.pop() {
            counters.queued_source_count = counters.queued_source_count.saturating_sub(1);

            if self.is_valid_in_state(source_id, SourceState::Queued) {
                let record_index = usize::from(source_id.slot());
                self.records[record_index].state = Some(SourceState::Playing);
                self.records[record_index].active_slot = Some(active_slot);
                self.active[usize::from(active_slot)].source_id = Some(source_id);
                self.active[usize::from(active_slot)].volume =
                    self.records[record_index].request.volume;
                let request = self.records[record_index].request;
                self.active[usize::from(active_slot)].bus = request.bus;
                self.active[usize::from(active_slot)].decoder = if let Some(clip) = request.clip {
                    Some(
                        DecoderState::new(clip, self.limits)
                            .expect("queued clip was validated before promotion"),
                    )
                } else if let Some(poly_sequence) = request.poly_sequence {
                    Some(
                        DecoderState::new_polyphonic_sequence(poly_sequence, self.limits)
                            .expect("queued polyphonic sequence was validated before promotion"),
                    )
                } else {
                    request.sequence.map(|sequence| {
                        DecoderState::new_sequence(sequence, self.limits)
                            .expect("queued sequence was validated before promotion")
                    })
                };
                counters.active_source_count = counters.active_source_count.saturating_add(1);
                counters.increment_active_bus(request.bus);

                return Some(PromotedSource {
                    source_id,
                    active_slot: ActiveSourceSlotId::new(active_slot),
                });
            }
        }

        None
    }

    /// Stops a queued or playing source by id.
    pub fn stop(
        &mut self,
        source_id: SourceId,
        counters: &mut AudioCounters,
    ) -> AudioResult<SourceLifecycleEvent> {
        let record_index = self.validate_id(source_id)?;
        match self.records[record_index].state {
            Some(SourceState::Queued) => {
                self.queue.remove(source_id);
                counters.queued_source_count = counters.queued_source_count.saturating_sub(1);
            }
            Some(SourceState::Playing) => {
                self.clear_active_for_record(record_index, counters);
            }
            Some(SourceState::Stopping) => return Err(AudioError::InvalidArgument),
            Some(SourceState::Completed)
            | Some(SourceState::Dropped)
            | Some(SourceState::Stolen)
            | Some(SourceState::Error) => return Err(AudioError::StaleSourceId),
            None => return Err(AudioError::StaleSourceId),
        }

        self.records[record_index].state = Some(SourceState::Stopping);
        Ok(SourceLifecycleEvent {
            source_id,
            state: SourceState::Stopping,
            error: None,
        })
    }

    /// Returns the logical bus assigned to a live or terminal-but-unreleased source.
    pub(crate) fn source_bus(&self, source_id: SourceId) -> AudioResult<SourceBus> {
        let record_index = self.validate_id(source_id)?;
        Ok(self.records[record_index].request.bus)
    }

    /// Stops all queued or playing sources assigned to the requested bus.
    pub(crate) fn stop_all_on_bus(
        &mut self,
        bus: SourceBus,
        counters: &mut AudioCounters,
    ) -> SourceLifecycleEvents<MAX_RECORDS> {
        let mut events = SourceLifecycleEvents::new();

        for record_index in 0..self.records.len() {
            if self.records[record_index].request.bus != bus {
                continue;
            }

            let state = self.records[record_index].state;
            if !matches!(state, Some(SourceState::Queued | SourceState::Playing)) {
                continue;
            }

            let source_id = SourceId::new(
                u16::try_from(record_index).expect("source record index fits in u16"),
                self.records[record_index].generation,
            );

            if state == Some(SourceState::Queued) {
                if self.queue.remove(source_id) {
                    counters.queued_source_count = counters.queued_source_count.saturating_sub(1);
                }
            } else {
                self.clear_active_for_record(record_index, counters);
            }

            self.records[record_index].state = Some(SourceState::Stopping);
            events.push(SourceLifecycleEvent {
                source_id,
                state: SourceState::Stopping,
                error: None,
            });
        }

        events
    }

    /// Marks a playing source complete by id.
    pub fn complete(
        &mut self,
        source_id: SourceId,
        counters: &mut AudioCounters,
    ) -> AudioResult<SourceLifecycleEvent> {
        let record_index = self.validate_id(source_id)?;
        if self.records[record_index].state != Some(SourceState::Playing) {
            return Err(AudioError::InvalidArgument);
        }

        self.clear_active_for_record(record_index, counters);
        self.records[record_index].state = Some(SourceState::Completed);
        Ok(SourceLifecycleEvent {
            source_id,
            state: SourceState::Completed,
            error: None,
        })
    }

    /// Marks a playing source complete by active slot.
    pub fn complete_active_slot(
        &mut self,
        active_slot: ActiveSourceSlotId,
        counters: &mut AudioCounters,
    ) -> AudioResult<SourceLifecycleEvent> {
        let Some(slot) = self.active.get(usize::from(active_slot.get())) else {
            return Err(AudioError::InvalidArgument);
        };
        let Some(source_id) = slot.source_id else {
            return Err(AudioError::InvalidArgument);
        };

        self.complete(source_id, counters)
    }

    /// Updates source-local volume for a queued or active source.
    pub fn set_source_volume(
        &mut self,
        source_id: SourceId,
        volume: MixerVolume,
    ) -> AudioResult<()> {
        let record_index = self.validate_id(source_id)?;
        match self.records[record_index].state {
            Some(SourceState::Queued) | Some(SourceState::Playing) => {}
            Some(SourceState::Stopping) => return Err(AudioError::InvalidArgument),
            Some(SourceState::Completed)
            | Some(SourceState::Dropped)
            | Some(SourceState::Stolen)
            | Some(SourceState::Error)
            | None => return Err(AudioError::StaleSourceId),
        }
        self.records[record_index].request.volume = volume;

        if let Some(active_slot) = self.records[record_index].active_slot {
            if let Some(slot) = self.active.get_mut(usize::from(active_slot)) {
                slot.volume = volume;
            }
        }

        Ok(())
    }

    /// Marks an active source complete when its decoder reached terminal end.
    ///
    /// TODO(KA-M10-001): keep this decoder-lifecycle boundary available for
    /// compressed decoders with explicit terminal/error transitions.
    #[allow(dead_code)]
    pub(crate) fn complete_active_slot_if_decoder_ended(
        &mut self,
        active_slot: ActiveSourceSlotId,
        counters: &mut AudioCounters,
    ) -> AudioResult<Option<SourceLifecycleEvent>> {
        let Some(slot) = self.active.get(usize::from(active_slot.get())) else {
            return Err(AudioError::InvalidArgument);
        };
        let Some(decoder) = slot.decoder else {
            return Ok(None);
        };

        if decoder.is_ended() {
            self.complete_active_slot(active_slot, counters).map(Some)
        } else {
            Ok(None)
        }
    }

    /// Returns the decoder for an active source slot.
    ///
    /// TODO(KA-M10-001): retain for codec-specific decoder inspection tests.
    #[allow(dead_code)]
    pub(crate) fn active_decoder(
        &self,
        active_slot: ActiveSourceSlotId,
    ) -> AudioResult<Option<&DecoderState<'a>>> {
        self.active
            .get(usize::from(active_slot.get()))
            .map(|slot| slot.decoder.as_ref())
            .ok_or(AudioError::InvalidArgument)
    }

    /// Returns mutable decoder state for an active source slot.
    ///
    /// TODO(KA-M10-001): retain for codec-specific decoder state tests.
    #[allow(dead_code)]
    pub(crate) fn active_decoder_mut(
        &mut self,
        active_slot: ActiveSourceSlotId,
    ) -> AudioResult<Option<&mut DecoderState<'a>>> {
        self.active
            .get_mut(usize::from(active_slot.get()))
            .map(|slot| slot.decoder.as_mut())
            .ok_or(AudioError::InvalidArgument)
    }

    /// Drops a queued source by id.
    ///
    /// TODO(KA-M11-002): use this for bus/admission policies that drop queued work.
    #[allow(dead_code)]
    pub fn drop_queued(
        &mut self,
        source_id: SourceId,
        counters: &mut AudioCounters,
    ) -> AudioResult<SourceLifecycleEvent> {
        let record_index = self.validate_id(source_id)?;
        if self.records[record_index].state != Some(SourceState::Queued) {
            return Err(AudioError::InvalidArgument);
        }

        self.queue.remove(source_id);
        counters.queued_source_count = counters.queued_source_count.saturating_sub(1);
        counters.dropped_source_count = counters.dropped_source_count.saturating_add(1);
        self.records[record_index].state = Some(SourceState::Dropped);

        Ok(SourceLifecycleEvent {
            source_id,
            state: SourceState::Dropped,
            error: None,
        })
    }

    /// Marks a source stolen. v0 exposes the path without implementing stealing policy.
    ///
    /// TODO(KA-M1-004): connect this when priority-based source stealing exists.
    #[allow(dead_code)]
    pub fn mark_stolen(
        &mut self,
        source_id: SourceId,
        counters: &mut AudioCounters,
    ) -> AudioResult<SourceLifecycleEvent> {
        let record_index = self.validate_id(source_id)?;
        if self.records[record_index].state == Some(SourceState::Queued) {
            self.queue.remove(source_id);
            counters.queued_source_count = counters.queued_source_count.saturating_sub(1);
        }
        if self.records[record_index].state == Some(SourceState::Playing) {
            self.clear_active_for_record(record_index, counters);
        }

        counters.stolen_source_count = counters.stolen_source_count.saturating_add(1);
        self.records[record_index].state = Some(SourceState::Stolen);
        Ok(SourceLifecycleEvent {
            source_id,
            state: SourceState::Stolen,
            error: None,
        })
    }

    /// Marks a source failed with a logical error.
    ///
    /// TODO(KA-M10-001): connect decoder-specific malformed stream failures here.
    #[allow(dead_code)]
    pub fn mark_error(
        &mut self,
        source_id: SourceId,
        error: AudioError,
        counters: &mut AudioCounters,
    ) -> AudioResult<SourceLifecycleEvent> {
        let record_index = self.validate_id(source_id)?;
        if self.records[record_index].state == Some(SourceState::Queued) {
            self.queue.remove(source_id);
            counters.queued_source_count = counters.queued_source_count.saturating_sub(1);
        }
        if self.records[record_index].state == Some(SourceState::Playing) {
            self.clear_active_for_record(record_index, counters);
        }

        self.records[record_index].state = Some(SourceState::Error);
        Ok(SourceLifecycleEvent {
            source_id,
            state: SourceState::Error,
            error: Some(error),
        })
    }

    /// Returns the logical state for a non-stale source id.
    #[cfg(test)]
    pub fn state(&self, source_id: SourceId) -> AudioResult<Option<SourceState>> {
        let record_index = self.validate_id(source_id)?;
        Ok(self.records[record_index].state)
    }

    /// Releases a terminal source record so its slot can be reused.
    pub fn release(&mut self, source_id: SourceId) -> AudioResult<()> {
        let record_index = self.validate_id(source_id)?;
        match self.records[record_index].state {
            Some(SourceState::Completed)
            | Some(SourceState::Dropped)
            | Some(SourceState::Stopping)
            | Some(SourceState::Stolen)
            | Some(SourceState::Error) => {
                self.records[record_index].release();
                Ok(())
            }
            Some(SourceState::Queued) | Some(SourceState::Playing) => {
                Err(AudioError::InvalidArgument)
            }
            None => Err(AudioError::StaleSourceId),
        }
    }

    fn allocate_record(
        &mut self,
        state: SourceState,
        request: SourceRequest<'a>,
    ) -> AudioResult<u16> {
        for (index, record) in self.records.iter_mut().enumerate() {
            if record.state.is_none() {
                record.state = Some(state);
                record.request = request;
                return u16::try_from(index).map_err(|_| AudioError::InvalidArgument);
            }
        }

        Err(AudioError::AdmissionRejected)
    }

    fn release_record(&mut self, source_id: SourceId) -> AudioResult<()> {
        let record_index = self.validate_id(source_id)?;
        self.records[record_index].release();
        Ok(())
    }

    fn validate_id(&self, source_id: SourceId) -> AudioResult<usize> {
        let record_index = usize::from(source_id.slot());
        let Some(record) = self.records.get(record_index) else {
            return Err(AudioError::StaleSourceId);
        };

        if record.generation != source_id.generation || record.state.is_none() {
            return Err(AudioError::StaleSourceId);
        }

        Ok(record_index)
    }

    fn is_valid_in_state(&self, source_id: SourceId, state: SourceState) -> bool {
        self.validate_id(source_id)
            .map(|record_index| self.records[record_index].state == Some(state))
            .unwrap_or(false)
    }

    fn first_free_active_slot(&self) -> Option<u16> {
        self.active
            .iter()
            .take(usize::from(self.limits.max_sfx_sources))
            .position(|slot| slot.source_id.is_none())
            .and_then(|index| u16::try_from(index).ok())
    }

    fn clear_active_for_record(&mut self, record_index: usize, counters: &mut AudioCounters) {
        let active_slot = self.records[record_index].active_slot.take();
        if let Some(active_slot) = active_slot {
            if let Some(slot) = self.active.get_mut(usize::from(active_slot)) {
                let bus = slot.bus;
                slot.source_id = None;
                slot.decoder = None;
                slot.volume = MixerVolume::UNITY;
                slot.bus = SourceBus::Sfx;
                counters.decrement_active_bus(bus);
            }
            counters.active_source_count = counters.active_source_count.saturating_sub(1);
        }
    }

    pub(crate) fn active_mix_slot_mut(
        &mut self,
        active_slot: ActiveSourceSlotId,
    ) -> AudioResult<Option<ActiveMixSlot<'_, 'a>>> {
        let Some(slot) = self.active.get_mut(usize::from(active_slot.get())) else {
            return Err(AudioError::InvalidArgument);
        };
        if slot.source_id.is_none() {
            return Ok(None);
        }
        let Some(decoder) = slot.decoder.as_mut() else {
            return Ok(None);
        };

        Ok(Some(ActiveMixSlot {
            volume: slot.volume,
            bus: slot.bus,
            decoder,
        }))
    }

    pub(crate) fn active_slot_id_at(&self, index: usize) -> Option<ActiveSourceSlotId> {
        if index >= usize::from(self.limits.max_sfx_sources) {
            return None;
        }

        self.active.get(index).and_then(|slot| {
            slot.source_id
                .map(|_| ActiveSourceSlotId::new(index as u16))
        })
    }
}

pub(crate) struct ActiveMixSlot<'slot, 'clip> {
    pub(crate) volume: MixerVolume,
    pub(crate) bus: SourceBus,
    pub(crate) decoder: &'slot mut DecoderState<'clip>,
}

pub(crate) struct SourceLifecycleEvents<const MAX_EVENTS: usize> {
    items: [Option<SourceLifecycleEvent>; MAX_EVENTS],
    len: usize,
}

impl<const MAX_EVENTS: usize> SourceLifecycleEvents<MAX_EVENTS> {
    const fn new() -> Self {
        Self {
            items: [None; MAX_EVENTS],
            len: 0,
        }
    }

    fn push(&mut self, event: SourceLifecycleEvent) {
        if self.len < MAX_EVENTS {
            self.items[self.len] = Some(event);
            self.len += 1;
        }
    }

    pub(crate) const fn len(&self) -> usize {
        self.len
    }

    pub(crate) const fn events(&self) -> &[Option<SourceLifecycleEvent>; MAX_EVENTS] {
        &self.items
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SourceRecord<'a> {
    generation: SourceGeneration,
    state: Option<SourceState>,
    request: SourceRequest<'a>,
    active_slot: Option<u16>,
}

impl SourceRecord<'_> {
    const EMPTY: Self = Self {
        generation: SourceGeneration::INITIAL,
        state: None,
        request: SourceRequest {
            clip: None,
            sequence: None,
            poly_sequence: None,
            volume: MixerVolume::UNITY,
            bus: SourceBus::Sfx,
            priority: SourcePriority::Normal,
            owner: SourceOwner { scope: 0 },
        },
        active_slot: None,
    };

    fn release(&mut self) {
        self.generation = self.generation.next();
        self.state = None;
        self.request = SourceRequest::default();
        self.active_slot = None;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ActiveSourceSlot<'a> {
    source_id: Option<SourceId>,
    decoder: Option<DecoderState<'a>>,
    volume: MixerVolume,
    bus: SourceBus,
}

impl ActiveSourceSlot<'_> {
    const EMPTY: Self = Self {
        source_id: None,
        decoder: None,
        volume: MixerVolume::UNITY,
        bus: SourceBus::Sfx,
    };
}

#[derive(Debug)]
struct SourceQueue<const MAX_QUEUE: usize> {
    items: [Option<SourceId>; MAX_QUEUE],
    head: usize,
    len: usize,
}

impl<const MAX_QUEUE: usize> SourceQueue<MAX_QUEUE> {
    const fn new() -> Self {
        Self {
            items: [None; MAX_QUEUE],
            head: 0,
            len: 0,
        }
    }

    const fn len(&self) -> usize {
        self.len
    }

    /// Empties the ring in place (no replacement value allocated on the stack).
    fn clear(&mut self) {
        self.items.fill(None);
        self.head = 0;
        self.len = 0;
    }

    fn push(&mut self, source_id: SourceId) -> Result<(), ()> {
        if self.len == MAX_QUEUE {
            return Err(());
        }

        let index = (self.head + self.len) % MAX_QUEUE;
        self.items[index] = Some(source_id);
        self.len += 1;
        Ok(())
    }

    fn pop(&mut self) -> Option<SourceId> {
        if self.len == 0 {
            return None;
        }

        let item = self.items[self.head].take();
        self.head = (self.head + 1) % MAX_QUEUE;
        self.len -= 1;
        item
    }

    fn remove(&mut self, source_id: SourceId) -> bool {
        let original_len = self.len;
        let mut removed = false;
        let mut kept = [None; MAX_QUEUE];
        let mut kept_len = 0;

        for _ in 0..original_len {
            if let Some(item) = self.pop() {
                if item == source_id && !removed {
                    removed = true;
                } else {
                    kept[kept_len] = Some(item);
                    kept_len += 1;
                }
            }
        }

        self.items = kept;
        self.head = 0;
        self.len = kept_len;
        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestLifecycle<'a> = SourceLifecycle<'a, 4, 2, 2>;

    fn limits() -> AudioLimits {
        AudioLimits {
            max_sfx_sources: 2,
            source_queue_depth: 2,
            ..AudioLimits::default()
        }
    }

    fn lifecycle<'a>() -> TestLifecycle<'a> {
        TestLifecycle::new(limits()).unwrap()
    }

    fn enqueue(lifecycle: &mut TestLifecycle<'_>, counters: &mut AudioCounters) -> SourceId {
        lifecycle
            .enqueue(SourceRequest::default(), counters, DropPolicy::RejectNew)
            .unwrap()
    }

    #[test]
    fn source_id_keeps_slot_private_but_generation_visible() {
        let id = SourceId::new(3, SourceGeneration::INITIAL);

        assert_eq!(id.slot(), 3);
        assert_eq!(id.generation().get(), 1);
    }

    #[test]
    fn allocate_free_reallocate_bumps_generation() {
        let mut lifecycle = lifecycle();
        let mut counters = AudioCounters::default();
        let first = enqueue(&mut lifecycle, &mut counters);

        lifecycle.drop_queued(first, &mut counters).unwrap();
        lifecycle.release(first).unwrap();
        let second = enqueue(&mut lifecycle, &mut counters);

        assert_eq!(first.slot(), second.slot());
        assert_ne!(first.generation(), second.generation());
    }

    #[test]
    fn stale_id_is_rejected_after_reuse() {
        let mut lifecycle = lifecycle();
        let mut counters = AudioCounters::default();
        let stale = enqueue(&mut lifecycle, &mut counters);

        lifecycle.drop_queued(stale, &mut counters).unwrap();
        lifecycle.release(stale).unwrap();
        let _fresh = enqueue(&mut lifecycle, &mut counters);

        assert_eq!(
            lifecycle.stop(stale, &mut counters),
            Err(AudioError::StaleSourceId)
        );
        assert_eq!(lifecycle.state(stale), Err(AudioError::StaleSourceId));
    }

    #[test]
    fn queue_capacity_boundary_is_non_blocking() {
        let mut lifecycle = lifecycle();
        let mut counters = AudioCounters::default();

        let _first = enqueue(&mut lifecycle, &mut counters);
        let _second = enqueue(&mut lifecycle, &mut counters);
        let third = lifecycle.enqueue(
            SourceRequest::default(),
            &mut counters,
            DropPolicy::RejectNew,
        );

        assert_eq!(third, Err(AudioError::QueueFull));
        assert_eq!(counters.queue_full_count, 1);
        assert_eq!(counters.queued_source_count, 2);
    }

    #[test]
    fn fifo_queue_order_is_preserved() {
        let mut lifecycle = lifecycle();
        let mut counters = AudioCounters::default();
        let first = enqueue(&mut lifecycle, &mut counters);
        let second = enqueue(&mut lifecycle, &mut counters);

        assert_eq!(
            lifecycle.promote_next(&mut counters).unwrap().source_id,
            first
        );
        assert_eq!(
            lifecycle.promote_next(&mut counters).unwrap().source_id,
            second
        );
    }

    #[test]
    fn queued_source_promotes_to_active() {
        let mut lifecycle = lifecycle();
        let mut counters = AudioCounters::default();
        let source_id = enqueue(&mut lifecycle, &mut counters);

        let promoted = lifecycle.promote_next(&mut counters).unwrap();

        assert_eq!(promoted.source_id, source_id);
        assert_eq!(lifecycle.state(source_id), Ok(Some(SourceState::Playing)));
        assert_eq!(counters.queued_source_count, 0);
        assert_eq!(counters.active_source_count, 1);
    }

    #[test]
    fn stop_before_activation_transitions_to_stopping() {
        let mut lifecycle = lifecycle();
        let mut counters = AudioCounters::default();
        let source_id = enqueue(&mut lifecycle, &mut counters);

        let event = lifecycle.stop(source_id, &mut counters).unwrap();

        assert_eq!(event.state, SourceState::Stopping);
        assert_eq!(lifecycle.state(source_id), Ok(Some(SourceState::Stopping)));
        assert_eq!(counters.queued_source_count, 0);
        assert!(lifecycle.promote_next(&mut counters).is_none());
    }

    #[test]
    fn stop_while_playing_transitions_to_stopping() {
        let mut lifecycle = lifecycle();
        let mut counters = AudioCounters::default();
        let source_id = enqueue(&mut lifecycle, &mut counters);
        lifecycle.promote_next(&mut counters).unwrap();

        let event = lifecycle.stop(source_id, &mut counters).unwrap();

        assert_eq!(event.state, SourceState::Stopping);
        assert_eq!(lifecycle.state(source_id), Ok(Some(SourceState::Stopping)));
        assert_eq!(counters.active_source_count, 0);
    }

    #[test]
    fn completion_transition_is_represented() {
        let mut lifecycle = lifecycle();
        let mut counters = AudioCounters::default();
        let source_id = enqueue(&mut lifecycle, &mut counters);
        let promoted = lifecycle.promote_next(&mut counters).unwrap();

        let event = lifecycle
            .complete_active_slot(promoted.active_slot, &mut counters)
            .unwrap();

        assert_eq!(event.state, SourceState::Completed);
        assert_eq!(lifecycle.state(source_id), Ok(Some(SourceState::Completed)));
        assert_eq!(counters.active_source_count, 0);
    }

    #[test]
    fn queued_source_promotes_with_decoder() {
        let payload = [5, 0, 251, 255];
        let mut lifecycle = lifecycle();
        let mut counters = AudioCounters::default();
        let clip = ClipAsset::pcm16_mono(limits().sample_rate_hz, 2, &payload);
        let source_id = lifecycle
            .enqueue(
                SourceRequest {
                    clip: Some(clip),
                    ..SourceRequest::default()
                },
                &mut counters,
                DropPolicy::RejectNew,
            )
            .unwrap();

        let promoted = lifecycle.promote_next(&mut counters).unwrap();
        let decoder = lifecycle
            .active_decoder_mut(promoted.active_slot)
            .unwrap()
            .unwrap();

        assert_eq!(promoted.source_id, source_id);
        assert_eq!(decoder.next_sample(), crate::DecodeResult::Sample(5));
        assert_eq!(decoder.next_sample(), crate::DecodeResult::Sample(-5));
    }

    #[test]
    fn decoder_end_completes_active_source_through_hook() {
        let payload = [7, 0];
        let mut lifecycle = lifecycle();
        let mut counters = AudioCounters::default();
        let clip = ClipAsset::pcm16_mono(limits().sample_rate_hz, 1, &payload);
        let source_id = lifecycle
            .enqueue(
                SourceRequest {
                    clip: Some(clip),
                    ..SourceRequest::default()
                },
                &mut counters,
                DropPolicy::RejectNew,
            )
            .unwrap();
        let promoted = lifecycle.promote_next(&mut counters).unwrap();

        {
            let decoder = lifecycle
                .active_decoder_mut(promoted.active_slot)
                .unwrap()
                .unwrap();
            assert_eq!(decoder.next_sample(), crate::DecodeResult::Sample(7));
            assert_eq!(decoder.next_sample(), crate::DecodeResult::End);
        }

        let event = lifecycle
            .complete_active_slot_if_decoder_ended(promoted.active_slot, &mut counters)
            .unwrap()
            .unwrap();

        assert_eq!(event.state, SourceState::Completed);
        assert_eq!(lifecycle.state(source_id), Ok(Some(SourceState::Completed)));
        assert_eq!(counters.active_source_count, 0);
    }

    #[test]
    fn drop_new_policy_updates_drop_counter() {
        let mut lifecycle = lifecycle();
        let mut counters = AudioCounters::default();

        let _first = enqueue(&mut lifecycle, &mut counters);
        let _second = enqueue(&mut lifecycle, &mut counters);
        let result =
            lifecycle.enqueue(SourceRequest::default(), &mut counters, DropPolicy::DropNew);

        assert_eq!(result, Err(AudioError::AdmissionRejected));
        assert_eq!(counters.queue_full_count, 1);
        assert_eq!(counters.dropped_source_count, 1);
    }
}
