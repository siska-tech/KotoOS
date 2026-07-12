use crate::{
    backend::AudioBackend,
    counters::AudioCounters,
    event::AudioEventQueue,
    mixer::{Mixer, MixerBlock, MixerTiming, MixerVolume},
    source::{SourceBus, SourceLifecycle, SourcePriority, SourceRequest},
    AudioCounterSnapshot, AudioError, AudioEvent, AudioEventKind, AudioPolicy, AudioResult,
    BackendError, BackendReport, BackendState, ClipAsset, PolyphonicSequence, Sequence, SourceId,
    SourceOwner, DEFAULT_MIXER_BLOCK_FRAMES,
};

/// Default record capacity for the v0 audio service.
pub const DEFAULT_SERVICE_SOURCE_RECORDS: usize = 12;

/// Default active source capacity for the v0 audio service.
pub const DEFAULT_SERVICE_ACTIVE_SOURCES: usize = 4;

/// Default queued source capacity for the v0 audio service.
pub const DEFAULT_SERVICE_SOURCE_QUEUE: usize = 8;

/// Default event capacity for the v0 audio service.
pub const DEFAULT_SERVICE_EVENT_QUEUE: usize = 16;

/// Default concrete audio service shape for v0 policy limits.
pub type DefaultAudioService<'a, B> = AudioService<
    'a,
    B,
    DEFAULT_MIXER_BLOCK_FRAMES,
    DEFAULT_SERVICE_SOURCE_RECORDS,
    DEFAULT_SERVICE_ACTIVE_SOURCES,
    DEFAULT_SERVICE_SOURCE_QUEUE,
    DEFAULT_SERVICE_EVENT_QUEUE,
>;

/// High-level audio runtime integrating source lifecycle, decoding, mixing,
/// event delivery, counters, and an abstract backend.
#[derive(Debug)]
pub struct AudioService<
    'a,
    B,
    const BLOCK_FRAMES: usize = DEFAULT_MIXER_BLOCK_FRAMES,
    const MAX_RECORDS: usize = DEFAULT_SERVICE_SOURCE_RECORDS,
    const MAX_ACTIVE: usize = DEFAULT_SERVICE_ACTIVE_SOURCES,
    const MAX_QUEUE: usize = DEFAULT_SERVICE_SOURCE_QUEUE,
    const MAX_EVENTS: usize = DEFAULT_SERVICE_EVENT_QUEUE,
> where
    B: AudioBackend<BLOCK_FRAMES>,
{
    policy: AudioPolicy,
    lifecycle: SourceLifecycle<'a, MAX_RECORDS, MAX_ACTIVE, MAX_QUEUE>,
    mixer: Mixer<BLOCK_FRAMES>,
    backend: B,
    counters: AudioCounters,
    events: AudioEventQueue<MAX_EVENTS>,
}

impl<
        'a,
        B,
        const BLOCK_FRAMES: usize,
        const MAX_RECORDS: usize,
        const MAX_ACTIVE: usize,
        const MAX_QUEUE: usize,
        const MAX_EVENTS: usize,
    > AudioService<'a, B, BLOCK_FRAMES, MAX_RECORDS, MAX_ACTIVE, MAX_QUEUE, MAX_EVENTS>
where
    B: AudioBackend<BLOCK_FRAMES>,
{
    /// Creates a service from a policy and backend implementation.
    pub fn new(policy: AudioPolicy, backend: B) -> AudioResult<Self> {
        policy.validate()?;
        MixerBlock::<BLOCK_FRAMES>::validate_limits(policy.limits)?;
        if usize::from(policy.limits.event_queue_depth) > MAX_EVENTS {
            return Err(AudioError::InvalidArgument);
        }

        Ok(Self {
            policy,
            lifecycle: SourceLifecycle::new(policy.limits)?,
            mixer: Mixer::new(policy)?,
            backend,
            counters: AudioCounters::default(),
            events: AudioEventQueue::new(),
        })
    }

    /// Returns the active runtime policy.
    pub const fn policy(&self) -> AudioPolicy {
        self.policy
    }

    /// Starts the backend boundary and records backend restart hooks.
    pub fn start(&mut self) -> AudioResult<()> {
        let report = self.backend.start().map_err(|error| {
            self.counters.record_backend_error(error);
            AudioError::from(error)
        })?;
        self.record_backend_report(report);
        Ok(())
    }

    /// Stops the backend boundary without exposing backend implementation detail.
    pub fn stop_backend(&mut self) -> AudioResult<()> {
        let report = self.backend.stop().map_err(|error| {
            self.counters.record_backend_error(error);
            AudioError::from(error)
        })?;
        self.record_backend_report(report);
        Ok(())
    }

    /// Resets service-owned runtime state and clears backend-local transients.
    pub fn reset(&mut self) -> AudioResult<()> {
        let report = self.backend.reset().map_err(|error| {
            self.counters.record_backend_error(error);
            AudioError::from(error)
        })?;
        self.record_backend_report(report);
        // Reset the source table and mixer **in place**. Reassigning
        // `SourceLifecycle::new(..)` / `Mixer::new(..)` here materialized whole
        // replacement structs on the caller frame; once LTO inlined this into
        // the core1 audio worker, the ~5 KiB `SourceLifecycle` temporary tipped
        // the worker past its fixed stack and corrupted adjacent .bss on the
        // first `StopAll` (KOTO-0186). The in-place resets reuse the existing
        // storage and preserve the exact freshly-constructed state.
        self.lifecycle.reset();
        self.mixer.reset();
        self.events.clear();
        self.counters.active_source_count = 0;
        self.counters.active_bgm_source_count = 0;
        self.counters.active_sfx_source_count = 0;
        self.counters.queued_source_count = 0;
        Ok(())
    }

    /// Returns the current logical backend state.
    pub fn backend_state(&self) -> BackendState {
        self.backend.query_state()
    }

    /// Enqueues a PCM16 clip for non-blocking playback.
    pub fn play_clip(&mut self, clip: ClipAsset<'a>) -> AudioResult<SourceId> {
        self.play_clip_for_owner(clip, SourceOwner::default())
    }

    /// Enqueues an experimental static monophonic sequence for playback.
    pub fn play_sequence(&mut self, sequence: Sequence<'a>) -> AudioResult<SourceId> {
        self.play_sequence_for_owner(sequence, SourceOwner::default())
    }

    /// Enqueues an experimental fixed-capacity polyphonic sequence on the SFX bus.
    pub fn play_poly_sequence(
        &mut self,
        sequence: PolyphonicSequence<'a>,
    ) -> AudioResult<SourceId> {
        self.play_poly_sequence_for_owner(sequence, SourceOwner::default())
    }

    /// Enqueues an experimental fixed-capacity polyphonic BGM sequence.
    pub fn play_bgm_sequence(&mut self, sequence: PolyphonicSequence<'a>) -> AudioResult<SourceId> {
        self.play_bgm_sequence_for_owner(sequence, SourceOwner::default())
    }

    /// Enqueues an experimental static monophonic sequence with an owner tag.
    pub fn play_sequence_for_owner(
        &mut self,
        sequence: Sequence<'a>,
        owner: SourceOwner,
    ) -> AudioResult<SourceId> {
        sequence.validate(self.policy.limits)?;

        self.lifecycle.enqueue(
            SourceRequest {
                sequence: Some(sequence),
                volume: MixerVolume::new(self.policy.default_volume),
                bus: SourceBus::Sfx,
                priority: SourcePriority::High,
                owner,
                ..SourceRequest::default()
            },
            &mut self.counters,
            self.policy.drop_policy,
        )
    }

    /// Enqueues an experimental polyphonic sequence on the SFX bus with an owner tag.
    pub fn play_poly_sequence_for_owner(
        &mut self,
        sequence: PolyphonicSequence<'a>,
        owner: SourceOwner,
    ) -> AudioResult<SourceId> {
        self.enqueue_poly_sequence_for_owner(sequence, owner, SourceBus::Sfx, SourcePriority::High)
    }

    /// Replaces existing BGM and enqueues an experimental polyphonic BGM sequence.
    pub fn play_bgm_sequence_for_owner(
        &mut self,
        sequence: PolyphonicSequence<'a>,
        owner: SourceOwner,
    ) -> AudioResult<SourceId> {
        sequence.validate(self.policy.limits)?;

        let replaced = self.stop_bgm_sources_for_replacement()?;
        if replaced != 0 {
            self.counters.bgm_replaced_count = self
                .counters
                .bgm_replaced_count
                .saturating_add(u64::from(replaced));
            self.counters.bgm_stop_count = self
                .counters
                .bgm_stop_count
                .saturating_add(u64::from(replaced));
        }

        let source_id = self.enqueue_poly_sequence_for_owner(
            sequence,
            owner,
            SourceBus::Bgm,
            SourcePriority::Low,
        )?;
        self.counters.bgm_start_count = self.counters.bgm_start_count.saturating_add(1);
        Ok(source_id)
    }

    fn enqueue_poly_sequence_for_owner(
        &mut self,
        sequence: PolyphonicSequence<'a>,
        owner: SourceOwner,
        bus: SourceBus,
        priority: SourcePriority,
    ) -> AudioResult<SourceId> {
        sequence.validate(self.policy.limits)?;

        self.lifecycle.enqueue(
            SourceRequest {
                poly_sequence: Some(sequence),
                volume: MixerVolume::new(self.policy.default_volume),
                bus,
                priority,
                owner,
                ..SourceRequest::default()
            },
            &mut self.counters,
            self.policy.drop_policy,
        )
    }

    /// Enqueues a PCM16 clip with a future owner/app scope tag.
    pub fn play_clip_for_owner(
        &mut self,
        clip: ClipAsset<'a>,
        owner: SourceOwner,
    ) -> AudioResult<SourceId> {
        if let Err(error) = clip.validate(self.policy.limits) {
            if matches!(
                error,
                AudioError::MalformedAsset | AudioError::UnsupportedCodec
            ) {
                self.counters.malformed_asset_count =
                    self.counters.malformed_asset_count.saturating_add(1);
            }
            return Err(error);
        }

        self.lifecycle.enqueue(
            SourceRequest {
                clip: Some(clip),
                volume: MixerVolume::new(self.policy.default_volume),
                bus: SourceBus::Sfx,
                priority: SourcePriority::High,
                owner,
                ..SourceRequest::default()
            },
            &mut self.counters,
            self.policy.drop_policy,
        )
    }

    /// Advances queued sources, mixes one fixed block, and submits it to the backend.
    pub fn tick(&mut self) -> AudioResult<()> {
        self.tick_with_timing(MixerTiming::default())
    }

    /// Advances one service tick with deterministic timing diagnostics.
    pub(crate) fn tick_with_timing(&mut self, timing: MixerTiming) -> AudioResult<()> {
        while self.lifecycle.promote_next(&mut self.counters).is_some() {}

        let output = self
            .mixer
            .mix_with_timing(&mut self.lifecycle, &mut self.counters, timing)?;
        for event in output.completions().iter().take(output.completion_count()) {
            if let Some(event) = event {
                self.queue_event(*event);
                if let Some(source_id) = event.source_id {
                    if self.lifecycle.source_bus(source_id)? == SourceBus::Bgm {
                        self.counters.bgm_stop_count =
                            self.counters.bgm_stop_count.saturating_add(1);
                    }
                    self.lifecycle.release(source_id)?;
                }
            }
        }

        match self.backend.submit_block(&output.block) {
            Ok(report) => {
                self.record_backend_report(report);
                Ok(())
            }
            Err(error) => {
                self.counters.record_backend_error(error);
                self.queue_event(backend_error_event(error));
                Err(AudioError::from(error))
            }
        }
    }

    /// Polls one logical audio event in FIFO order.
    pub fn poll_audio_event(&mut self) -> Option<AudioEvent> {
        self.events.pop()
    }

    /// Stops a queued or active source.
    pub fn stop(&mut self, source_id: SourceId) -> AudioResult<()> {
        let bus = self.lifecycle.source_bus(source_id)?;
        let event = self.lifecycle.stop(source_id, &mut self.counters)?;
        self.queue_event(event.into());
        if bus == SourceBus::Bgm {
            self.counters.bgm_stop_count = self.counters.bgm_stop_count.saturating_add(1);
        }
        self.lifecycle.release(source_id)?;
        Ok(())
    }

    /// Stops queued and active BGM sources without stopping SFX clips/sequences.
    pub fn stop_bgm(&mut self) -> AudioResult<()> {
        let stopped = self.stop_bgm_sources_for_replacement()?;
        self.counters.bgm_stop_count = self
            .counters
            .bgm_stop_count
            .saturating_add(u64::from(stopped));
        Ok(())
    }

    /// Sets source-local logical volume for queued or active playback.
    pub fn set_source_volume(
        &mut self,
        source_id: SourceId,
        volume: MixerVolume,
    ) -> AudioResult<()> {
        self.validate_volume(volume)?;
        self.lifecycle.set_source_volume(source_id, volume)
    }

    /// Sets the app-scoped logical volume used by subsequent blocks.
    pub fn set_app_volume(&mut self, volume: MixerVolume) -> AudioResult<()> {
        self.mixer.set_app_volume(volume)
    }

    /// Sets the minimal BGM bus gain used by sequence sources.
    pub fn set_bgm_volume(&mut self, volume: MixerVolume) -> AudioResult<()> {
        self.mixer.set_bgm_volume(volume)
    }

    /// Sets the minimal SFX bus gain used by clip sources.
    pub fn set_sfx_volume(&mut self, volume: MixerVolume) -> AudioResult<()> {
        self.mixer.set_sfx_volume(volume)
    }

    /// Returns a public counter snapshot.
    pub const fn counter_snapshot(&self) -> AudioCounterSnapshot {
        self.counters.snapshot()
    }

    #[cfg(test)]
    pub(crate) const fn backend(&self) -> &B {
        &self.backend
    }

    fn validate_volume(&self, volume: MixerVolume) -> AudioResult<()> {
        if volume.get() < self.policy.min_volume || volume.get() > self.policy.max_volume {
            return Err(AudioError::InvalidArgument);
        }

        Ok(())
    }

    fn record_backend_report(&mut self, report: BackendReport) {
        self.counters.record_backend_report(report);
        if report.underruns != 0 {
            self.queue_event(AudioEvent::runtime(AudioEventKind::Underrun));
        }
        if report.submit_failures != 0 {
            self.queue_event(AudioEvent::runtime(AudioEventKind::BackendSubmitFailed));
        }
    }

    fn queue_event(&mut self, event: AudioEvent) {
        if self.events.push(event).is_err() {
            self.counters.event_queue_full_count =
                self.counters.event_queue_full_count.saturating_add(1);
        }
    }

    fn stop_bgm_sources_for_replacement(&mut self) -> AudioResult<u16> {
        let events = self
            .lifecycle
            .stop_all_on_bus(SourceBus::Bgm, &mut self.counters);
        let stopped = u16::try_from(events.len()).map_err(|_| AudioError::InvalidArgument)?;

        for event in events.events().iter().take(events.len()).flatten() {
            self.queue_event((*event).into());
            self.lifecycle.release(event.source_id)?;
        }

        Ok(stopped)
    }
}

fn backend_error_event(error: BackendError) -> AudioEvent {
    match error {
        BackendError::Underrun => AudioEvent::runtime(AudioEventKind::Underrun),
        BackendError::SubmitFailed | BackendError::QueueFull => {
            AudioEvent::runtime(AudioEventKind::BackendSubmitFailed)
        }
        BackendError::Unavailable | BackendError::NotRunning => {
            AudioEvent::runtime(AudioEventKind::Error(AudioError::from(error)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        backend::mock::MockBackend, parse_clip_asset, AssetPlacement, AudioEventKind,
        BackendResult, ClipLoop, CodecId, DropPolicy, LoopCount, PolyphonicSequence,
        PolyphonicSequenceVoice, SequenceEvent, SequenceInstrument, SequenceWaveform,
        PCM16_MONO_CHANNELS, SEQUENCE_REPEAT_INFINITE,
    };

    const TEST_BLOCK_FRAMES: usize = 4;
    type TestBackend = MockBackend<TEST_BLOCK_FRAMES, 8>;
    type TestService<'a> = AudioService<'a, TestBackend, TEST_BLOCK_FRAMES, 4, 2, 2, 4>;

    fn policy() -> AudioPolicy {
        AudioPolicy {
            limits: crate::AudioLimits {
                block_frames: TEST_BLOCK_FRAMES as u16,
                max_sfx_sources: 2,
                source_queue_depth: 2,
                event_queue_depth: 4,
                ..crate::AudioLimits::default()
            },
            ..AudioPolicy::default()
        }
    }

    fn service<'a>() -> TestService<'a> {
        AudioService::new(policy(), MockBackend::with_config()).unwrap()
    }

    fn payload(samples: &[i16]) -> Payload {
        Payload::from_samples(samples)
    }

    fn clip<'a>(bytes: &'a [u8], samples: u32) -> ClipAsset<'a> {
        ClipAsset::pcm16_mono(policy().limits.sample_rate_hz, samples, bytes)
    }

    fn square_instrument() -> [SequenceInstrument; 1] {
        [SequenceInstrument::new(SequenceWaveform::Square, 255)]
    }

    fn looping_clip<'a>(bytes: &'a [u8], samples: u32) -> ClipAsset<'a> {
        let mut clip = clip(bytes, samples);
        clip.loop_metadata = ClipLoop::Whole {
            count: LoopCount::Infinite,
        };
        clip
    }

    #[test]
    fn audio_service_constructs() {
        let service = service();

        assert_eq!(service.policy(), policy());
        assert_eq!(service.counter_snapshot(), AudioCounterSnapshot::default());
    }

    #[test]
    fn backend_start_stop_is_connected() {
        let mut service = service();

        service.start().unwrap();
        assert_eq!(service.backend_state(), BackendState::Running);
        assert_eq!(service.counter_snapshot().backend_restart_count, 1);
        service.stop_backend().unwrap();
        assert_eq!(service.backend_state(), BackendState::Stopped);
    }

    #[test]
    fn play_valid_pcm16_clip_returns_source_id() {
        let bytes = payload(&[1, 2, 3, 4]);
        let mut service = service();

        let source_id = service.play_clip(clip(bytes.as_slice(), 4)).unwrap();

        assert_eq!(source_id.generation().get(), 1);
        assert_eq!(service.counter_snapshot().queued_source_count, 1);
    }

    #[test]
    fn invalid_asset_increments_malformed_counter() {
        let bytes = payload(&[1, 2]);
        let mut service = service();

        assert_eq!(
            service.play_clip(clip(bytes.as_slice(), 3)),
            Err(AudioError::MalformedAsset)
        );
        assert_eq!(service.counter_snapshot().malformed_asset_count, 1);
    }

    #[test]
    fn unsupported_codec_is_immediate_result_and_counter() {
        let bytes = payload(&[1]);
        let mut bad = clip(bytes.as_slice(), 1);
        bad.codec = CodecId::Unsupported(99);
        let mut service = service();

        assert_eq!(service.play_clip(bad), Err(AudioError::UnsupportedCodec));
        assert_eq!(service.counter_snapshot().malformed_asset_count, 1);
    }

    #[test]
    fn queue_full_increments_queue_and_drop_counters() {
        let bytes = payload(&[1]);
        let mut service = AudioService::<_, TEST_BLOCK_FRAMES, 4, 1, 1, 4>::new(
            AudioPolicy {
                limits: crate::AudioLimits {
                    block_frames: TEST_BLOCK_FRAMES as u16,
                    max_sfx_sources: 1,
                    source_queue_depth: 1,
                    event_queue_depth: 4,
                    ..crate::AudioLimits::default()
                },
                drop_policy: DropPolicy::DropNew,
                ..AudioPolicy::default()
            },
            MockBackend::<TEST_BLOCK_FRAMES, 4>::with_config(),
        )
        .unwrap();

        service.play_clip(clip(bytes.as_slice(), 1)).unwrap();
        assert_eq!(
            service.play_clip(clip(bytes.as_slice(), 1)),
            Err(AudioError::AdmissionRejected)
        );

        let snapshot = service.counter_snapshot();
        assert_eq!(snapshot.queue_full_count, 1);
        assert_eq!(snapshot.dropped_source_count, 1);
    }

    #[test]
    fn tick_submits_non_silent_mixer_block_to_mock_backend() {
        let bytes = payload(&[10, -20, 30, -40]);
        let mut service = service();

        service.start().unwrap();
        service.play_clip(clip(bytes.as_slice(), 4)).unwrap();
        service.tick().unwrap();

        let block = service.backend.latest_block().unwrap();
        assert_eq!(block.as_pcm16_mono(), &[10, -20, 30, -40]);
        assert!(block.as_pcm16_mono().iter().any(|sample| *sample != 0));
        assert_eq!(service.counter_snapshot().submitted_block_count, 1);
    }

    #[test]
    fn audio_service_can_play_parsed_runtime_ready_pcm16_asset() {
        let raw = payload(&[10, -20, 30, -40]);
        let header = crate::ClipAssetHeader::from_clip(clip(raw.as_slice(), 4), 0)
            .unwrap()
            .encode();
        let mut bytes = [0u8; crate::CLIP_ASSET_HEADER_SIZE + 8];
        bytes[..crate::CLIP_ASSET_HEADER_SIZE].copy_from_slice(&header);
        bytes[crate::CLIP_ASSET_HEADER_SIZE..].copy_from_slice(raw.as_slice());
        let parsed = parse_clip_asset(&bytes, policy().limits).unwrap();
        let mut service = service();

        service.start().unwrap();
        service.play_clip(parsed).unwrap();
        service.tick().unwrap();

        assert_eq!(
            service.backend.latest_block().unwrap().as_pcm16_mono(),
            &[10, -20, 30, -40]
        );
    }

    #[test]
    fn completion_event_can_be_polled() {
        let bytes = payload(&[1, 2, 3, 4]);
        let mut service = service();

        service.start().unwrap();
        let source_id = service.play_clip(clip(bytes.as_slice(), 4)).unwrap();
        service.tick().unwrap();

        assert_eq!(
            service.poll_audio_event(),
            Some(AudioEvent::for_source(AudioEventKind::Completed, source_id))
        );
        let snapshot = service.counter_snapshot();
        assert_eq!(snapshot.active_source_count, 0);
        assert_eq!(snapshot.queued_source_count, 0);
    }

    #[test]
    fn simple_note_sequence_submits_non_silent_block() {
        let instruments = square_instrument();
        let events = [
            SequenceEvent::Note {
                pitch: 4,
                duration_ticks: 1,
                volume: 255,
                instrument_id: 0,
            },
            SequenceEvent::End,
        ];
        let mut service = service();

        service.start().unwrap();
        service
            .play_sequence(crate::Sequence::new(&events, &instruments, 4000))
            .unwrap();
        service.tick().unwrap();

        let block = service.backend.latest_block().unwrap();
        assert!(block.as_pcm16_mono().iter().any(|sample| *sample != 0));
    }

    #[test]
    fn rest_sequence_submits_silent_block() {
        let instruments = square_instrument();
        let events = [
            SequenceEvent::Rest { duration_ticks: 1 },
            SequenceEvent::End,
        ];
        let mut service = service();

        service.start().unwrap();
        service
            .play_sequence(crate::Sequence::new(&events, &instruments, 4000))
            .unwrap();
        service.tick().unwrap();

        assert_eq!(
            service.backend.latest_block().unwrap().as_pcm16_mono(),
            &[0, 0, 0, 0]
        );
    }

    #[test]
    fn sequence_completes_and_emits_completion_event() {
        let instruments = square_instrument();
        let events = [
            SequenceEvent::Rest { duration_ticks: 1 },
            SequenceEvent::End,
        ];
        let mut service = service();

        service.start().unwrap();
        let source_id = service
            .play_sequence(crate::Sequence::new(&events, &instruments, 4000))
            .unwrap();
        service.tick().unwrap();

        assert_eq!(
            service.poll_audio_event(),
            Some(AudioEvent::for_source(AudioEventKind::Completed, source_id))
        );
        assert_eq!(service.counter_snapshot().active_source_count, 0);
    }

    #[test]
    fn finite_polyphonic_sequence_completes_after_all_voices_end() {
        let instruments = square_instrument();
        let short_events = [
            SequenceEvent::Rest { duration_ticks: 1 },
            SequenceEvent::End,
        ];
        let long_events = [
            SequenceEvent::Rest { duration_ticks: 2 },
            SequenceEvent::End,
        ];
        let voices = [
            PolyphonicSequenceVoice::unity(crate::Sequence::new(&short_events, &instruments, 4000)),
            PolyphonicSequenceVoice::unity(crate::Sequence::new(&long_events, &instruments, 4000)),
        ];
        let mut service = service();

        service.start().unwrap();
        let source_id = service
            .play_poly_sequence(PolyphonicSequence::new(&voices))
            .unwrap();
        service.tick().unwrap();

        assert_eq!(service.poll_audio_event(), None);
        assert_eq!(service.counter_snapshot().active_source_count, 1);

        service.tick().unwrap();

        assert_eq!(
            service.poll_audio_event(),
            Some(AudioEvent::for_source(AudioEventKind::Completed, source_id))
        );
        assert_eq!(service.counter_snapshot().active_source_count, 0);
    }

    #[test]
    fn loop_voice_polyphonic_sequence_remains_active_until_stopped() {
        let instruments = square_instrument();
        let finite_events = [
            SequenceEvent::Rest { duration_ticks: 1 },
            SequenceEvent::End,
        ];
        let loop_events = [
            SequenceEvent::LoopStart,
            SequenceEvent::Rest { duration_ticks: 1 },
            SequenceEvent::LoopEnd {
                repeat_count: SEQUENCE_REPEAT_INFINITE,
            },
            SequenceEvent::End,
        ];
        let voices = [
            PolyphonicSequenceVoice::unity(crate::Sequence::new(
                &finite_events,
                &instruments,
                4000,
            )),
            PolyphonicSequenceVoice::unity(crate::Sequence::new(&loop_events, &instruments, 4000)),
        ];
        let mut service = service();

        service.start().unwrap();
        let source_id = service
            .play_bgm_sequence(PolyphonicSequence::new(&voices))
            .unwrap();
        service.tick().unwrap();
        service.tick().unwrap();

        assert_eq!(service.poll_audio_event(), None);
        assert_eq!(service.counter_snapshot().active_source_count, 1);

        service.stop(source_id).unwrap();
        assert_eq!(
            service.poll_audio_event(),
            Some(AudioEvent::for_source(AudioEventKind::Stopped, source_id))
        );
        assert_eq!(service.counter_snapshot().active_source_count, 0);
    }

    #[test]
    fn stop_sequence_emits_stopped_event_and_releases_source() {
        let instruments = square_instrument();
        let events = [
            SequenceEvent::Note {
                pitch: 4,
                duration_ticks: 4,
                volume: 255,
                instrument_id: 0,
            },
            SequenceEvent::End,
        ];
        let mut service = service();
        let source_id = service
            .play_sequence(crate::Sequence::new(&events, &instruments, 4000))
            .unwrap();

        service.stop(source_id).unwrap();

        assert_eq!(
            service.poll_audio_event(),
            Some(AudioEvent::for_source(AudioEventKind::Stopped, source_id))
        );
        assert_eq!(service.counter_snapshot().queued_source_count, 0);
        assert_eq!(service.stop(source_id), Err(AudioError::StaleSourceId));
    }

    #[test]
    fn infinite_loop_sequence_does_not_complete_until_stopped() {
        let instruments = square_instrument();
        let events = [
            SequenceEvent::LoopStart,
            SequenceEvent::Rest { duration_ticks: 1 },
            SequenceEvent::LoopEnd {
                repeat_count: SEQUENCE_REPEAT_INFINITE,
            },
            SequenceEvent::End,
        ];
        let mut service = service();

        service.start().unwrap();
        let source_id = service
            .play_sequence(crate::Sequence::new(&events, &instruments, 4000))
            .unwrap();
        service.tick().unwrap();
        service.tick().unwrap();

        assert_eq!(service.poll_audio_event(), None);
        assert_eq!(service.counter_snapshot().active_source_count, 1);

        service.stop(source_id).unwrap();
        assert_eq!(
            service.poll_audio_event(),
            Some(AudioEvent::for_source(AudioEventKind::Stopped, source_id))
        );
    }

    #[test]
    fn completed_source_cannot_be_stopped_again() {
        let bytes = payload(&[1, 2, 3, 4]);
        let mut service = service();

        service.start().unwrap();
        let source_id = service.play_clip(clip(bytes.as_slice(), 4)).unwrap();
        service.tick().unwrap();

        assert_eq!(service.stop(source_id), Err(AudioError::StaleSourceId));
        assert_eq!(
            service.poll_audio_event().unwrap().kind,
            AudioEventKind::Completed
        );
        assert_eq!(service.poll_audio_event(), None);
    }

    #[test]
    fn completed_source_id_is_stale_after_release() {
        let bytes = payload(&[1, 2, 3, 4]);
        let mut service = service();

        service.start().unwrap();
        let source_id = service.play_clip(clip(bytes.as_slice(), 4)).unwrap();
        service.tick().unwrap();

        assert_eq!(
            service.set_source_volume(source_id, MixerVolume::UNITY),
            Err(AudioError::StaleSourceId)
        );
    }

    #[test]
    fn repeated_short_clips_do_not_exhaust_source_records() {
        let bytes = payload(&[1]);
        let mut service = AudioService::<_, TEST_BLOCK_FRAMES, 2, 1, 1, 4>::new(
            AudioPolicy {
                limits: crate::AudioLimits {
                    block_frames: TEST_BLOCK_FRAMES as u16,
                    max_sfx_sources: 1,
                    source_queue_depth: 1,
                    event_queue_depth: 4,
                    ..crate::AudioLimits::default()
                },
                ..AudioPolicy::default()
            },
            MockBackend::<TEST_BLOCK_FRAMES, 4>::with_config(),
        )
        .unwrap();

        service.start().unwrap();
        for _ in 0..8 {
            service.play_clip(clip(bytes.as_slice(), 1)).unwrap();
            service.tick().unwrap();
            assert_eq!(
                service.poll_audio_event().unwrap().kind,
                AudioEventKind::Completed
            );
        }

        let snapshot = service.counter_snapshot();
        assert_eq!(snapshot.active_source_count, 0);
        assert_eq!(snapshot.queued_source_count, 0);
        assert_eq!(snapshot.queue_full_count, 0);
    }

    #[test]
    fn stop_before_activation_goes_through_service() {
        let bytes = payload(&[1]);
        let mut service = service();
        let source_id = service.play_clip(clip(bytes.as_slice(), 1)).unwrap();

        service.stop(source_id).unwrap();

        assert_eq!(
            service.poll_audio_event(),
            Some(AudioEvent::for_source(AudioEventKind::Stopped, source_id))
        );
        assert_eq!(service.counter_snapshot().queued_source_count, 0);
    }

    #[test]
    fn stop_while_playing_goes_through_service() {
        let bytes = payload(&[1, 2, 3, 4]);
        let mut service = service();
        let source_id = service.play_clip(clip(bytes.as_slice(), 4)).unwrap();
        service
            .lifecycle
            .promote_next(&mut service.counters)
            .unwrap();

        service.stop(source_id).unwrap();

        assert_eq!(
            service.poll_audio_event(),
            Some(AudioEvent::for_source(AudioEventKind::Stopped, source_id))
        );
        assert_eq!(service.counter_snapshot().active_source_count, 0);
    }

    #[test]
    fn stale_id_is_rejected_through_service() {
        let bytes = payload(&[1]);
        let mut service = service();
        let source_id = service.play_clip(clip(bytes.as_slice(), 1)).unwrap();
        service.stop(source_id).unwrap();

        assert_eq!(service.stop(source_id), Err(AudioError::StaleSourceId));
        assert_eq!(
            service.set_source_volume(source_id, MixerVolume::UNITY),
            Err(AudioError::StaleSourceId)
        );
    }

    #[test]
    fn source_and_app_volume_affect_output() {
        let bytes = payload(&[1024, -1024, 512, -512]);
        let mut service = service();
        service.start().unwrap();
        let source_id = service.play_clip(clip(bytes.as_slice(), 4)).unwrap();
        service
            .set_source_volume(source_id, MixerVolume::new(128))
            .unwrap();
        service.set_app_volume(MixerVolume::new(128)).unwrap();

        service.tick().unwrap();

        assert_eq!(
            service.backend.latest_block().unwrap().as_pcm16_mono(),
            &[256, -256, 128, -128]
        );
    }

    #[test]
    fn sfx_volume_affects_sfx_clip_output() {
        let bytes = payload(&[1024, -1024, 512, -512]);
        let mut service = service();
        service.start().unwrap();
        service.set_sfx_volume(MixerVolume::new(128)).unwrap();
        service.play_clip(clip(bytes.as_slice(), 4)).unwrap();

        service.tick().unwrap();

        assert_eq!(
            service.backend.latest_block().unwrap().as_pcm16_mono(),
            &[512, -512, 256, -256]
        );
    }

    #[test]
    fn sfx_volume_affects_sfx_sequence_output() {
        let instruments = square_instrument();
        let events = [
            SequenceEvent::Note {
                pitch: 4,
                duration_ticks: 2,
                volume: 255,
                instrument_id: 0,
            },
            SequenceEvent::End,
        ];
        let mut service = service();
        service.start().unwrap();
        service.set_sfx_volume(MixerVolume::SILENCE).unwrap();
        service
            .play_sequence(crate::Sequence::new(&events, &instruments, 4000))
            .unwrap();

        service.tick().unwrap();

        assert_eq!(
            service.backend.latest_block().unwrap().as_pcm16_mono(),
            &[0, 0, 0, 0]
        );
        assert_eq!(service.counter_snapshot().active_sfx_source_count, 1);
    }

    #[test]
    fn bgm_volume_affects_bgm_sequence_output() {
        let instruments = square_instrument();
        let events = [
            SequenceEvent::Note {
                pitch: 4,
                duration_ticks: 2,
                volume: 255,
                instrument_id: 0,
            },
            SequenceEvent::End,
        ];
        let voices = [PolyphonicSequenceVoice::unity(crate::Sequence::new(
            &events,
            &instruments,
            4000,
        ))];
        let mut service = service();
        service.start().unwrap();
        service.set_bgm_volume(MixerVolume::SILENCE).unwrap();
        service
            .play_bgm_sequence(PolyphonicSequence::new(&voices))
            .unwrap();

        service.tick().unwrap();

        assert_eq!(
            service.backend.latest_block().unwrap().as_pcm16_mono(),
            &[0, 0, 0, 0]
        );
        assert_eq!(service.counter_snapshot().active_bgm_source_count, 1);
    }

    #[test]
    fn bgm_and_sfx_volume_are_independent() {
        let bytes = payload(&[1000, 1000, 1000, 1000]);
        let instruments = square_instrument();
        let events = [
            SequenceEvent::Note {
                pitch: 4,
                duration_ticks: 2,
                volume: 255,
                instrument_id: 0,
            },
            SequenceEvent::End,
        ];
        let voices = [PolyphonicSequenceVoice::unity(crate::Sequence::new(
            &events,
            &instruments,
            4000,
        ))];
        let mut service = service();
        service.start().unwrap();
        service.set_bgm_volume(MixerVolume::SILENCE).unwrap();
        service.set_sfx_volume(MixerVolume::UNITY).unwrap();
        service
            .play_bgm_sequence(PolyphonicSequence::new(&voices))
            .unwrap();
        service.play_clip(clip(bytes.as_slice(), 4)).unwrap();

        service.tick().unwrap();

        assert_eq!(
            service.backend.latest_block().unwrap().as_pcm16_mono(),
            &[1000, 1000, 1000, 1000]
        );
        let snapshot = service.counter_snapshot();
        assert_eq!(snapshot.active_bgm_source_count, 1);
        assert_eq!(snapshot.active_sfx_source_count, 0);
    }

    #[test]
    fn play_bgm_sequence_starts_bgm_role_source() {
        let instruments = square_instrument();
        let events = [
            SequenceEvent::Rest { duration_ticks: 1 },
            SequenceEvent::End,
        ];
        let voices = [PolyphonicSequenceVoice::unity(crate::Sequence::new(
            &events,
            &instruments,
            4000,
        ))];
        let mut service = service();
        service.start().unwrap();

        service
            .play_bgm_sequence(PolyphonicSequence::new(&voices))
            .unwrap();
        service.tick().unwrap();

        let snapshot = service.counter_snapshot();
        assert_eq!(snapshot.bgm_start_count, 1);
        assert_eq!(snapshot.active_bgm_source_count, 0);
        assert_eq!(snapshot.bgm_stop_count, 1);
    }

    #[test]
    fn stop_bgm_stops_only_bgm_and_keeps_sfx_active() {
        let bytes = payload(&[10, -10]);
        let instruments = square_instrument();
        let loop_events = [
            SequenceEvent::LoopStart,
            SequenceEvent::Rest { duration_ticks: 1 },
            SequenceEvent::LoopEnd {
                repeat_count: SEQUENCE_REPEAT_INFINITE,
            },
            SequenceEvent::End,
        ];
        let voices = [PolyphonicSequenceVoice::unity(crate::Sequence::new(
            &loop_events,
            &instruments,
            4000,
        ))];
        let mut service = service();
        service.start().unwrap();
        service
            .play_bgm_sequence(PolyphonicSequence::new(&voices))
            .unwrap();
        let sfx_id = service
            .play_clip(looping_clip(bytes.as_slice(), 2))
            .unwrap();
        service.tick().unwrap();

        service.stop_bgm().unwrap();

        let snapshot = service.counter_snapshot();
        assert_eq!(snapshot.active_bgm_source_count, 0);
        assert_eq!(snapshot.active_sfx_source_count, 1);
        assert_eq!(snapshot.active_source_count, 1);
        assert_eq!(service.stop(sfx_id), Ok(()));
    }

    #[test]
    fn bgm_loop_remains_active_until_stop_bgm() {
        let instruments = square_instrument();
        let loop_events = [
            SequenceEvent::LoopStart,
            SequenceEvent::Rest { duration_ticks: 1 },
            SequenceEvent::LoopEnd {
                repeat_count: SEQUENCE_REPEAT_INFINITE,
            },
            SequenceEvent::End,
        ];
        let voices = [PolyphonicSequenceVoice::unity(crate::Sequence::new(
            &loop_events,
            &instruments,
            4000,
        ))];
        let mut service = service();
        service.start().unwrap();
        service
            .play_bgm_sequence(PolyphonicSequence::new(&voices))
            .unwrap();

        service.tick().unwrap();
        service.tick().unwrap();
        assert_eq!(service.counter_snapshot().active_bgm_source_count, 1);

        service.stop_bgm().unwrap();

        assert_eq!(service.counter_snapshot().active_bgm_source_count, 0);
        assert_eq!(service.counter_snapshot().bgm_stop_count, 1);
    }

    #[test]
    fn bgm_replacement_stops_existing_bgm() {
        let instruments = square_instrument();
        let loop_events = [
            SequenceEvent::LoopStart,
            SequenceEvent::Rest { duration_ticks: 1 },
            SequenceEvent::LoopEnd {
                repeat_count: SEQUENCE_REPEAT_INFINITE,
            },
            SequenceEvent::End,
        ];
        let first_voices = [PolyphonicSequenceVoice::unity(crate::Sequence::new(
            &loop_events,
            &instruments,
            4000,
        ))];
        let second_voices = [PolyphonicSequenceVoice::unity(crate::Sequence::new(
            &loop_events,
            &instruments,
            4000,
        ))];
        let mut service = service();
        service.start().unwrap();
        let first = service
            .play_bgm_sequence(PolyphonicSequence::new(&first_voices))
            .unwrap();
        service.tick().unwrap();

        let second = service
            .play_bgm_sequence(PolyphonicSequence::new(&second_voices))
            .unwrap();

        assert_ne!(first, second);
        assert_eq!(
            service.poll_audio_event(),
            Some(AudioEvent::for_source(AudioEventKind::Stopped, first))
        );
        let snapshot = service.counter_snapshot();
        assert_eq!(snapshot.bgm_start_count, 2);
        assert_eq!(snapshot.bgm_stop_count, 1);
        assert_eq!(snapshot.bgm_replaced_count, 1);
        assert_eq!(snapshot.active_bgm_source_count, 0);
        assert_eq!(snapshot.queued_source_count, 1);
    }

    #[test]
    fn reset_clears_sources_events_and_counters_and_stays_playable() {
        let bytes = payload(&[10, -20, 30, -40]);
        let instruments = square_instrument();
        let loop_events = [
            SequenceEvent::LoopStart,
            SequenceEvent::Rest { duration_ticks: 1 },
            SequenceEvent::LoopEnd {
                repeat_count: SEQUENCE_REPEAT_INFINITE,
            },
            SequenceEvent::End,
        ];
        let voices = [PolyphonicSequenceVoice::unity(crate::Sequence::new(
            &loop_events,
            &instruments,
            4000,
        ))];
        let mut service = service();
        service.start().unwrap();
        service
            .play_bgm_sequence(PolyphonicSequence::new(&voices))
            .unwrap();
        service.play_clip(clip(bytes.as_slice(), 4)).unwrap();
        service.tick().unwrap();
        assert_ne!(
            service.counter_snapshot().active_source_count
                + service.counter_snapshot().queued_source_count,
            0
        );

        service.reset().unwrap();

        // In-place reset must leave the exact freshly-constructed source state:
        // no active/queued sources, a drained event queue, and zeroed live-count
        // counters (parity with the former `SourceLifecycle::new`/`Mixer::new`).
        let snapshot = service.counter_snapshot();
        assert_eq!(snapshot.active_source_count, 0);
        assert_eq!(snapshot.active_bgm_source_count, 0);
        assert_eq!(snapshot.active_sfx_source_count, 0);
        assert_eq!(snapshot.queued_source_count, 0);
        assert_eq!(service.poll_audio_event(), None);

        // The service keeps mixing after an in-place reset.
        service.start().unwrap();
        service.play_clip(clip(bytes.as_slice(), 4)).unwrap();
        service.tick().unwrap();
        assert_eq!(
            service.backend.latest_block().unwrap().as_pcm16_mono(),
            &[10, -20, 30, -40]
        );
    }

    #[test]
    fn reset_restores_mixer_volumes_to_policy_defaults() {
        let bytes = payload(&[1024, -1024, 512, -512]);
        let mut service = service();
        service.start().unwrap();
        service.set_app_volume(MixerVolume::new(128)).unwrap();
        service.set_sfx_volume(MixerVolume::new(128)).unwrap();

        service.reset().unwrap();

        // Defaults are unity for this policy, so a unity-authored clip mixes
        // through unchanged once the in-place mixer reset restores them.
        service.start().unwrap();
        service.play_clip(clip(bytes.as_slice(), 4)).unwrap();
        service.tick().unwrap();
        assert_eq!(
            service.backend.latest_block().unwrap().as_pcm16_mono(),
            &[1024, -1024, 512, -512]
        );
    }

    #[test]
    fn backend_submit_failure_updates_counters_and_event() {
        let bytes = payload(&[1, 2, 3, 4]);
        let mut service = service();
        service.start().unwrap();
        service.play_clip(clip(bytes.as_slice(), 4)).unwrap();
        service.backend.fail_next_submit();

        assert_eq!(service.tick(), Err(AudioError::BackendUnavailable));
        let snapshot = service.counter_snapshot();
        assert_eq!(snapshot.backend_submit_failure_count, 1);
        assert_eq!(snapshot.underrun_count, 0);
        assert_eq!(
            service.poll_audio_event(),
            Some(AudioEvent::for_source(
                AudioEventKind::Completed,
                SourceId::new(0, crate::SourceGeneration::INITIAL),
            ))
        );
        assert_eq!(
            service.poll_audio_event(),
            Some(AudioEvent::runtime(AudioEventKind::BackendSubmitFailed))
        );
    }

    #[test]
    fn explicit_underrun_updates_counter_and_event() {
        let mut service = service();
        service.start().unwrap();
        service
            .backend
            .fail_next_submit_with(BackendError::Underrun);

        assert_eq!(service.tick(), Err(AudioError::BackendUnavailable));

        assert_eq!(service.counter_snapshot().underrun_count, 1);
        assert_eq!(
            service.poll_audio_event(),
            Some(AudioEvent::runtime(AudioEventKind::Underrun))
        );
    }

    #[test]
    fn backend_submit_failed_and_underrun_events_are_distinct() {
        let mut service = service();
        service.start().unwrap();
        service
            .backend
            .fail_next_submit_with(BackendError::QueueFull);

        assert_eq!(service.tick(), Err(AudioError::QueueFull));
        assert_eq!(
            service.poll_audio_event(),
            Some(AudioEvent::runtime(AudioEventKind::BackendSubmitFailed))
        );

        service.start().unwrap();
        service
            .backend
            .fail_next_submit_with(BackendError::Underrun);
        assert_eq!(service.tick(), Err(AudioError::BackendUnavailable));
        assert_eq!(
            service.poll_audio_event(),
            Some(AudioEvent::runtime(AudioEventKind::Underrun))
        );
    }

    #[test]
    fn event_queue_preserves_fifo_order() {
        let a = payload(&[1]);
        let b = payload(&[2]);
        let mut service = service();
        let first = service.play_clip(clip(a.as_slice(), 1)).unwrap();
        let second = service.play_clip(clip(b.as_slice(), 1)).unwrap();

        service.stop(first).unwrap();
        service.stop(second).unwrap();

        assert_eq!(
            service.poll_audio_event(),
            Some(AudioEvent::for_source(AudioEventKind::Stopped, first))
        );
        assert_eq!(
            service.poll_audio_event(),
            Some(AudioEvent::for_source(AudioEventKind::Stopped, second))
        );
    }

    #[test]
    fn event_queue_full_updates_counter() {
        let bytes = payload(&[1]);
        let mut service = AudioService::<_, TEST_BLOCK_FRAMES, 4, 2, 2, 1>::new(
            AudioPolicy {
                limits: crate::AudioLimits {
                    block_frames: TEST_BLOCK_FRAMES as u16,
                    max_sfx_sources: 2,
                    source_queue_depth: 2,
                    event_queue_depth: 1,
                    ..crate::AudioLimits::default()
                },
                ..AudioPolicy::default()
            },
            MockBackend::<TEST_BLOCK_FRAMES, 4>::with_config(),
        )
        .unwrap();
        let first = service.play_clip(clip(bytes.as_slice(), 1)).unwrap();
        let second = service.play_clip(clip(bytes.as_slice(), 1)).unwrap();

        service.stop(first).unwrap();
        service.stop(second).unwrap();

        assert_eq!(service.counter_snapshot().event_queue_full_count, 1);
        assert_eq!(
            service.poll_audio_event(),
            Some(AudioEvent::for_source(AudioEventKind::Stopped, first))
        );
    }

    #[test]
    fn backend_can_be_swapped_through_same_abstraction() {
        let mut service = AudioService::<_, TEST_BLOCK_FRAMES, 4, 2, 2, 4>::new(
            policy(),
            CountingBackend::default(),
        )
        .unwrap();

        service.start().unwrap();
        service.tick().unwrap();

        assert_eq!(service.counter_snapshot().submitted_block_count, 1);
        assert_eq!(service.backend_state(), BackendState::Running);
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct CountingBackend {
        state: BackendState,
        submitted: u64,
    }

    impl Default for CountingBackend {
        fn default() -> Self {
            Self {
                state: BackendState::Stopped,
                submitted: 0,
            }
        }
    }

    impl AudioBackend<TEST_BLOCK_FRAMES> for CountingBackend {
        fn start(&mut self) -> BackendResult {
            self.state = BackendState::Running;
            Ok(BackendReport::backend_restart())
        }

        fn stop(&mut self) -> BackendResult {
            self.state = BackendState::Stopped;
            Ok(BackendReport::default())
        }

        fn submit_block(&mut self, _block: &MixerBlock<TEST_BLOCK_FRAMES>) -> BackendResult {
            if self.state != BackendState::Running {
                return Err(BackendError::NotRunning);
            }
            self.submitted = self.submitted.saturating_add(1);
            Ok(BackendReport::submitted_block())
        }

        fn suspend(&mut self) -> BackendResult {
            self.state = BackendState::Suspended;
            Ok(BackendReport::default())
        }

        fn resume(&mut self) -> BackendResult {
            self.state = BackendState::Running;
            Ok(BackendReport::backend_restart())
        }

        fn query_state(&self) -> BackendState {
            self.state
        }
    }

    #[derive(Clone, Copy)]
    struct Payload {
        bytes: [u8; 16],
        len: usize,
    }

    impl Payload {
        fn from_samples(samples: &[i16]) -> Self {
            let mut bytes = [0; 16];
            let mut len = 0;
            for sample in samples {
                let encoded = sample.to_le_bytes();
                bytes[len] = encoded[0];
                bytes[len + 1] = encoded[1];
                len += 2;
            }

            Self { bytes, len }
        }

        fn as_slice(&self) -> &[u8] {
            &self.bytes[..self.len]
        }
    }

    #[test]
    fn public_service_api_names_do_not_expose_raw_backend_details() {
        let public_surface = [
            core::any::type_name::<TestService<'_>>(),
            core::any::type_name::<AudioCounterSnapshot>(),
            core::any::type_name::<AudioEvent>(),
            core::any::type_name::<AudioEventKind>(),
            "start stop_backend reset play_clip tick poll_audio_event stop stop_bgm set_source_volume set_app_volume counter_snapshot",
            "play_sequence play_poly_sequence play_bgm_sequence play_bgm_sequence_for_owner set_bgm_volume set_sfx_volume",
        ];
        let forbidden = ["PWM", "PIO", "DMA", "timer", "backend buffer", "raw buffer"];

        for item in public_surface {
            for term in forbidden {
                assert!(!item.contains(term));
            }
        }

        let clip = ClipAsset {
            codec: CodecId::Pcm16,
            sample_rate_hz: policy().limits.sample_rate_hz,
            channels: PCM16_MONO_CHANNELS,
            sample_count: 0,
            payload: &[],
            loop_metadata: crate::ClipLoop::None,
            placement: AssetPlacement::Resident,
        };
        assert_eq!(clip.validate(policy().limits), Ok(clip));
    }

    #[test]
    fn crate_root_public_exports_do_not_include_internal_building_blocks() {
        let lib = include_str!("lib.rs");
        let forbidden = [
            "pub use counters::{AudioCounterSnapshot, AudioCounters}",
            "AudioEventQueue",
            "Mixer, MixerBlock",
            "MixerOutput",
            "MixerTiming",
            "SourceLifecycle",
            "ActiveSourceSlotId",
            "SourceLifecycleEvent",
            "DecoderState",
            "Sldpcm4Decoder",
            "Sldpcm4TableId",
            "Sldpcm4LoopState",
            "MockBackend",
            "koto_audio_tools",
            "parse_wav",
            "Wav",
            "resample",
        ];

        for line in lib
            .lines()
            .filter(|line| line.trim_start().starts_with("pub use "))
        {
            for term in forbidden {
                assert!(!line.contains(term));
            }
        }
    }
}
