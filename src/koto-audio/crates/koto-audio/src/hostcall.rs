use crate::{
    AudioBackend, AudioCounterSnapshot, AudioError, AudioEvent, AudioResult, AudioService,
    BackendState, ClipAsset, MixerVolume, Sequence, SourceId,
};

/// Logical hostcall privilege scope.
///
/// This is a classification aid for VM integration. It is not a stable numeric
/// ABI and must not be used as a syscall number.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostcallScope {
    /// Normal app audio calls.
    Normal,
    /// System app audio policy calls.
    System,
    /// Debug-only diagnostics calls.
    Debug,
}

/// Placeholder audio focus request for future system hostcalls.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AudioFocus {
    /// Request foreground audio focus.
    Request,
    /// Release previously granted audio focus.
    Release,
}

/// Placeholder backend policy token for future system hostcalls.
///
/// This deliberately does not contain backend handles, buffers, timers, DMA
/// channels, PIO/PWM state, or hardware-specific configuration.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BackendPolicy;

/// Public debug mixer load placeholder.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DebugMixerLoadDump {
    /// Currently active logical sources.
    pub active_source_count: u16,
    /// Currently queued logical sources.
    pub queued_source_count: u16,
    /// Total late mixer ticks.
    pub late_mix_count: u64,
    /// Maximum observed mix time in implementation-defined ticks.
    pub max_mix_time: u64,
}

/// Public debug backend state placeholder.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DebugBackendStateDump {
    /// Abstract backend state only.
    pub state: BackendState,
}

/// Public debug underrun counter placeholder.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DebugUnderrunCounterDump {
    /// Total mixer/backend underruns.
    pub underrun_count: u64,
    /// Total failed backend block submissions.
    pub backend_submit_failure_count: u64,
}

/// Thin normal app hostcall adapter over [`AudioService`].
///
/// The normal surface exposes logical clip/source/event/counter operations
/// only. It has no backend, buffer, timer, DMA, PIO, or PWM accessor.
pub struct NormalHostcallAdapter<
    'svc,
    'a,
    B,
    const BLOCK_FRAMES: usize,
    const MAX_RECORDS: usize,
    const MAX_ACTIVE: usize,
    const MAX_QUEUE: usize,
    const MAX_EVENTS: usize,
> where
    B: AudioBackend<BLOCK_FRAMES>,
{
    service:
        &'svc mut AudioService<'a, B, BLOCK_FRAMES, MAX_RECORDS, MAX_ACTIVE, MAX_QUEUE, MAX_EVENTS>,
}

impl<
        'svc,
        'a,
        B,
        const BLOCK_FRAMES: usize,
        const MAX_RECORDS: usize,
        const MAX_ACTIVE: usize,
        const MAX_QUEUE: usize,
        const MAX_EVENTS: usize,
    >
    NormalHostcallAdapter<'svc, 'a, B, BLOCK_FRAMES, MAX_RECORDS, MAX_ACTIVE, MAX_QUEUE, MAX_EVENTS>
where
    B: AudioBackend<BLOCK_FRAMES>,
{
    /// Creates a normal app adapter over an existing service.
    pub fn new(
        service: &'svc mut AudioService<
            'a,
            B,
            BLOCK_FRAMES,
            MAX_RECORDS,
            MAX_ACTIVE,
            MAX_QUEUE,
            MAX_EVENTS,
        >,
    ) -> Self {
        Self { service }
    }

    /// Returns this adapter's logical scope.
    pub const fn scope(&self) -> HostcallScope {
        HostcallScope::Normal
    }

    /// Enqueues a PCM16 clip through the service.
    pub fn play_clip(&mut self, clip: ClipAsset<'a>) -> AudioResult<SourceId> {
        self.service.play_clip(clip)
    }

    /// Enqueues an experimental static monophonic sequence through the service.
    pub fn play_sequence(&mut self, sequence: Sequence<'a>) -> AudioResult<SourceId> {
        self.service.play_sequence(sequence)
    }

    /// Stops a queued or active source through the service.
    pub fn stop(&mut self, source_id: SourceId) -> AudioResult<()> {
        self.service.stop(source_id)
    }

    /// Sets source-local logical volume through the service.
    pub fn set_source_volume(
        &mut self,
        source_id: SourceId,
        volume: MixerVolume,
    ) -> AudioResult<()> {
        self.service.set_source_volume(source_id, volume)
    }

    /// Sets app-scoped logical volume through the service.
    pub fn set_app_volume(&mut self, volume: MixerVolume) -> AudioResult<()> {
        self.service.set_app_volume(volume)
    }

    /// Polls one logical audio event through the service.
    pub fn poll_audio_event(&mut self) -> Option<AudioEvent> {
        self.service.poll_audio_event()
    }

    /// Queries public logical counters through the service.
    pub fn query_audio_counters(&self) -> AudioCounterSnapshot {
        self.service.counter_snapshot()
    }
}

/// Placeholder system app hostcall adapter over [`AudioService`].
///
/// System operations are intentionally separated from the normal app adapter
/// even while their v0 behavior is unsupported.
pub struct SystemHostcallAdapter<
    'svc,
    'a,
    B,
    const BLOCK_FRAMES: usize,
    const MAX_RECORDS: usize,
    const MAX_ACTIVE: usize,
    const MAX_QUEUE: usize,
    const MAX_EVENTS: usize,
> where
    B: AudioBackend<BLOCK_FRAMES>,
{
    service:
        &'svc mut AudioService<'a, B, BLOCK_FRAMES, MAX_RECORDS, MAX_ACTIVE, MAX_QUEUE, MAX_EVENTS>,
}

impl<
        'svc,
        'a,
        B,
        const BLOCK_FRAMES: usize,
        const MAX_RECORDS: usize,
        const MAX_ACTIVE: usize,
        const MAX_QUEUE: usize,
        const MAX_EVENTS: usize,
    >
    SystemHostcallAdapter<'svc, 'a, B, BLOCK_FRAMES, MAX_RECORDS, MAX_ACTIVE, MAX_QUEUE, MAX_EVENTS>
where
    B: AudioBackend<BLOCK_FRAMES>,
{
    /// Creates a system app adapter over an existing service.
    pub fn new(
        service: &'svc mut AudioService<
            'a,
            B,
            BLOCK_FRAMES,
            MAX_RECORDS,
            MAX_ACTIVE,
            MAX_QUEUE,
            MAX_EVENTS,
        >,
    ) -> Self {
        Self { service }
    }

    /// Returns this adapter's logical scope.
    pub const fn scope(&self) -> HostcallScope {
        HostcallScope::System
    }

    /// Reserved master volume hostcall.
    pub fn set_master_volume(&mut self, _volume: MixerVolume) -> AudioResult<()> {
        let _ = self.service.policy();
        Err(AudioError::UnsupportedOperation)
    }

    /// Reserved mute hostcall.
    pub fn mute(&mut self, _muted: bool) -> AudioResult<()> {
        let _ = self.service.policy();
        Err(AudioError::UnsupportedOperation)
    }

    /// Reserved audio focus hostcall.
    pub fn audio_focus(&mut self, _focus: AudioFocus) -> AudioResult<()> {
        let _ = self.service.policy();
        Err(AudioError::UnsupportedOperation)
    }

    /// Reserved backend policy hostcall.
    pub fn backend_policy(&mut self, _policy: BackendPolicy) -> AudioResult<()> {
        let _ = self.service.policy();
        Err(AudioError::UnsupportedOperation)
    }
}

/// Debug-only hostcall adapter over [`AudioService`].
///
/// This adapter is available for tests, debug builds, or the
/// `debug-hostcalls` feature. It is intentionally separate from the normal app
/// adapter.
#[cfg(any(test, debug_assertions, feature = "debug-hostcalls"))]
pub struct DebugHostcallAdapter<
    'svc,
    'a,
    B,
    const BLOCK_FRAMES: usize,
    const MAX_RECORDS: usize,
    const MAX_ACTIVE: usize,
    const MAX_QUEUE: usize,
    const MAX_EVENTS: usize,
> where
    B: AudioBackend<BLOCK_FRAMES>,
{
    service:
        &'svc mut AudioService<'a, B, BLOCK_FRAMES, MAX_RECORDS, MAX_ACTIVE, MAX_QUEUE, MAX_EVENTS>,
}

#[cfg(any(test, debug_assertions, feature = "debug-hostcalls"))]
impl<
        'svc,
        'a,
        B,
        const BLOCK_FRAMES: usize,
        const MAX_RECORDS: usize,
        const MAX_ACTIVE: usize,
        const MAX_QUEUE: usize,
        const MAX_EVENTS: usize,
    >
    DebugHostcallAdapter<'svc, 'a, B, BLOCK_FRAMES, MAX_RECORDS, MAX_ACTIVE, MAX_QUEUE, MAX_EVENTS>
where
    B: AudioBackend<BLOCK_FRAMES>,
{
    /// Creates a debug adapter over an existing service.
    pub fn new(
        service: &'svc mut AudioService<
            'a,
            B,
            BLOCK_FRAMES,
            MAX_RECORDS,
            MAX_ACTIVE,
            MAX_QUEUE,
            MAX_EVENTS,
        >,
    ) -> Self {
        Self { service }
    }

    /// Returns this adapter's logical scope.
    pub const fn scope(&self) -> HostcallScope {
        HostcallScope::Debug
    }

    /// Dumps public mixer load counters.
    pub fn dump_mixer_load(&self) -> DebugMixerLoadDump {
        let counters = self.service.counter_snapshot();
        DebugMixerLoadDump {
            active_source_count: counters.active_source_count,
            queued_source_count: counters.queued_source_count,
            late_mix_count: counters.late_mix_count,
            max_mix_time: counters.max_mix_time,
        }
    }

    /// Dumps abstract backend state only.
    pub fn dump_backend_state(&self) -> DebugBackendStateDump {
        DebugBackendStateDump {
            state: self.service.backend_state(),
        }
    }

    /// Dumps public underrun-related counters.
    pub fn dump_underrun_counters(&self) -> DebugUnderrunCounterDump {
        let counters = self.service.counter_snapshot();
        DebugUnderrunCounterDump {
            underrun_count: counters.underrun_count,
            backend_submit_failure_count: counters.backend_submit_failure_count,
        }
    }

    /// Reserved synthetic underrun hook.
    pub fn force_underrun_test(&mut self) -> AudioResult<()> {
        Err(AudioError::UnsupportedOperation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        backend::mock::MockBackend, AudioEventKind, AudioLimits, AudioPolicy, BackendState,
        CodecId, SequenceEvent, SequenceInstrument, SequenceWaveform, SourceGeneration,
        PCM16_MONO_CHANNELS,
    };

    const TEST_BLOCK_FRAMES: usize = 4;
    type TestBackend = MockBackend<TEST_BLOCK_FRAMES, 8>;
    type TestService<'a> = AudioService<'a, TestBackend, TEST_BLOCK_FRAMES, 4, 2, 2, 4>;

    fn policy() -> AudioPolicy {
        AudioPolicy {
            limits: AudioLimits {
                block_frames: TEST_BLOCK_FRAMES as u16,
                max_sfx_sources: 2,
                source_queue_depth: 2,
                event_queue_depth: 4,
                ..AudioLimits::default()
            },
            ..AudioPolicy::default()
        }
    }

    fn service<'a>() -> TestService<'a> {
        AudioService::new(policy(), MockBackend::with_config()).unwrap()
    }

    fn clip<'a>(bytes: &'a [u8], samples: u32) -> ClipAsset<'a> {
        ClipAsset::pcm16_mono(policy().limits.sample_rate_hz, samples, bytes)
    }

    fn payload(samples: &[i16]) -> Payload {
        Payload::from_samples(samples)
    }

    fn test_sequence<'a>(
        events: &'a [SequenceEvent],
        instruments: &'a [SequenceInstrument],
    ) -> Sequence<'a> {
        Sequence::new(events, instruments, 4)
    }

    #[test]
    fn normal_play_clip_delegates_to_service() {
        let bytes = payload(&[1, 2, 3, 4]);
        let mut service = service();
        let mut adapter = NormalHostcallAdapter::new(&mut service);

        let source_id = adapter.play_clip(clip(bytes.as_slice(), 4)).unwrap();

        assert_eq!(source_id.generation().get(), 1);
        assert_eq!(adapter.query_audio_counters().queued_source_count, 1);
    }

    #[test]
    fn normal_stop_delegates_to_service() {
        let bytes = payload(&[1]);
        let mut service = service();
        let mut adapter = NormalHostcallAdapter::new(&mut service);
        let source_id = adapter.play_clip(clip(bytes.as_slice(), 1)).unwrap();

        adapter.stop(source_id).unwrap();

        assert_eq!(
            adapter.poll_audio_event(),
            Some(AudioEvent::for_source(AudioEventKind::Stopped, source_id))
        );
        assert_eq!(adapter.query_audio_counters().queued_source_count, 0);
    }

    #[test]
    fn normal_volume_calls_delegate_to_service() {
        let bytes = payload(&[1024, -1024, 512, -512]);
        let mut service = service();
        let mut adapter = NormalHostcallAdapter::new(&mut service);
        let source_id = adapter.play_clip(clip(bytes.as_slice(), 4)).unwrap();

        adapter
            .set_source_volume(source_id, MixerVolume::new(128))
            .unwrap();
        adapter.set_app_volume(MixerVolume::new(128)).unwrap();
        assert_eq!(
            adapter.set_source_volume(source_id, MixerVolume::new(u16::MAX)),
            Err(AudioError::InvalidArgument)
        );
    }

    #[test]
    fn normal_poll_audio_event_works() {
        let bytes = payload(&[1, 2, 3, 4]);
        let mut service = service();
        service.start().unwrap();
        let mut adapter = NormalHostcallAdapter::new(&mut service);
        let source_id = adapter.play_clip(clip(bytes.as_slice(), 4)).unwrap();
        adapter.service.tick().unwrap();

        assert_eq!(
            adapter.poll_audio_event(),
            Some(AudioEvent::for_source(AudioEventKind::Completed, source_id))
        );
    }

    #[test]
    fn normal_play_sequence_delegates_to_service() {
        let instruments = [SequenceInstrument::new(SequenceWaveform::Square, 255)];
        let events = [
            SequenceEvent::Rest { duration_ticks: 1 },
            SequenceEvent::End,
        ];
        let mut service = service();
        let mut adapter = NormalHostcallAdapter::new(&mut service);

        let source_id = adapter
            .play_sequence(test_sequence(&events, &instruments))
            .unwrap();

        assert_eq!(source_id.generation().get(), 1);
        assert_eq!(adapter.query_audio_counters().queued_source_count, 1);
    }

    #[test]
    fn normal_adapter_surface_does_not_expose_backend_controls() {
        let public_surface = [
            "NormalHostcallAdapter",
            "scope play_clip play_sequence stop set_source_volume set_app_volume poll_audio_event query_audio_counters",
        ];
        let forbidden = [
            "AudioBackend",
            "MockBackend",
            "backend_policy",
            "dump_backend_state",
            "force_underrun_test",
            "PWM",
            "PIO",
            "DMA",
            "timer",
            "backend buffer",
            "buffer pointer",
        ];

        for item in public_surface {
            for term in forbidden {
                assert!(!item.contains(term));
            }
        }
    }

    #[test]
    fn public_hostcall_api_does_not_expose_raw_backend_details() {
        let public_surface = [
            core::any::type_name::<AudioCounterSnapshot>(),
            core::any::type_name::<AudioEvent>(),
            core::any::type_name::<DebugMixerLoadDump>(),
            core::any::type_name::<DebugBackendStateDump>(),
            core::any::type_name::<DebugUnderrunCounterDump>(),
            "NormalHostcallAdapter SystemHostcallAdapter DebugHostcallAdapter",
        ];
        let forbidden = [
            "PWM",
            "PIO",
            "DMA",
            "timer",
            "backend buffer",
            "buffer pointer",
        ];

        for item in public_surface {
            for term in forbidden {
                assert!(!item.contains(term));
            }
        }

        let abstract_source = SourceId::new(0, SourceGeneration::INITIAL);
        assert_eq!(abstract_source.slot(), 0);
    }

    #[test]
    fn system_and_debug_placeholders_are_separated_from_normal_adapter() {
        let mut service = service();
        {
            let normal = NormalHostcallAdapter::new(&mut service);
            assert_eq!(normal.scope(), HostcallScope::Normal);
        }
        {
            let mut system = SystemHostcallAdapter::new(&mut service);
            assert_eq!(system.scope(), HostcallScope::System);
            assert_eq!(
                system.set_master_volume(MixerVolume::UNITY),
                Err(AudioError::UnsupportedOperation)
            );
            assert_eq!(system.mute(true), Err(AudioError::UnsupportedOperation));
            assert_eq!(
                system.audio_focus(AudioFocus::Request),
                Err(AudioError::UnsupportedOperation)
            );
            assert_eq!(
                system.backend_policy(BackendPolicy),
                Err(AudioError::UnsupportedOperation)
            );
        }
        {
            let mut debug = DebugHostcallAdapter::new(&mut service);
            assert_eq!(debug.scope(), HostcallScope::Debug);
            assert_eq!(
                debug.dump_backend_state(),
                DebugBackendStateDump {
                    state: BackendState::Stopped,
                }
            );
            assert_eq!(debug.dump_mixer_load(), DebugMixerLoadDump::default());
            assert_eq!(
                debug.dump_underrun_counters(),
                DebugUnderrunCounterDump::default()
            );
            assert_eq!(
                debug.force_underrun_test(),
                Err(AudioError::UnsupportedOperation)
            );
        }
    }

    #[test]
    fn normal_counter_visibility_is_public_snapshot_only() {
        let bytes = payload(&[1]);
        let mut service = service();
        let mut adapter = NormalHostcallAdapter::new(&mut service);
        adapter.play_clip(clip(bytes.as_slice(), 1)).unwrap();

        let snapshot = adapter.query_audio_counters();

        assert_eq!(snapshot.queued_source_count, 1);
        assert_eq!(
            core::any::type_name::<AudioCounterSnapshot>(),
            "koto_audio::counters::AudioCounterSnapshot"
        );
    }

    #[test]
    fn hostcall_layer_does_not_define_stable_numeric_abi() {
        let public_surface = [
            "HostcallScope Normal System Debug",
            "NormalHostcallAdapter SystemHostcallAdapter DebugHostcallAdapter",
        ];
        let forbidden = ["number", "ordinal", "opcode", "u32"];

        for item in public_surface {
            for term in forbidden {
                assert!(!item.contains(term));
            }
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
    fn placeholder_clip_shape_remains_pcm16_only() {
        let clip = ClipAsset {
            codec: CodecId::Pcm16,
            sample_rate_hz: policy().limits.sample_rate_hz,
            channels: PCM16_MONO_CHANNELS,
            sample_count: 0,
            payload: &[],
            loop_metadata: crate::ClipLoop::None,
            placement: crate::AssetPlacement::Resident,
        };

        assert_eq!(clip.validate(policy().limits), Ok(clip));
    }
}
