use crate::{
    AudioBackend, BackendError, BackendReport, BackendResult, BackendState, MixerBlock,
    DEFAULT_MIXER_BLOCK_FRAMES,
};

/// Default number of mixer blocks captured by [`SimulatorBackend`].
pub const DEFAULT_SIM_CAPTURE_BLOCKS: usize = 32;

/// Policy used when the simulator capture window is full.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CaptureFullPolicy {
    /// Keep accepting blocks and discard the oldest captured block.
    #[default]
    DropOldest,
    /// Reject the submit with [`BackendError::QueueFull`].
    RejectSubmit,
    /// Keep the existing capture window and accept later blocks without storing them.
    Truncate,
}

/// Deterministic no-output backend for desktop simulators and inspection tests.
///
/// The simulator implements the same [`AudioBackend`] boundary as other
/// backends. Submitted mixer blocks are copied into a bounded capture window
/// for tests and debug tools; no real audio output is performed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SimulatorBackend<
    const BLOCK_FRAMES: usize = DEFAULT_MIXER_BLOCK_FRAMES,
    const CAPTURE_BLOCKS: usize = DEFAULT_SIM_CAPTURE_BLOCKS,
> {
    state: BackendState,
    capture_policy: CaptureFullPolicy,
    captured_blocks: [Option<MixerBlock<BLOCK_FRAMES>>; CAPTURE_BLOCKS],
    captured_block_count: usize,
    submitted_block_count: u64,
    dropped_capture_block_count: u64,
    rejected_submit_count: u64,
    truncated_capture_block_count: u64,
    restart_count: u64,
}

/// Alias for simulator users that want to emphasize inspection-only behavior.
pub type InspectionBackend<
    const BLOCK_FRAMES: usize = DEFAULT_MIXER_BLOCK_FRAMES,
    const CAPTURE_BLOCKS: usize = DEFAULT_SIM_CAPTURE_BLOCKS,
> = SimulatorBackend<BLOCK_FRAMES, CAPTURE_BLOCKS>;

impl<const BLOCK_FRAMES: usize, const CAPTURE_BLOCKS: usize>
    SimulatorBackend<BLOCK_FRAMES, CAPTURE_BLOCKS>
{
    /// Creates a stopped simulator using [`CaptureFullPolicy::DropOldest`].
    pub const fn with_config() -> Self {
        Self::with_capture_policy(CaptureFullPolicy::DropOldest)
    }

    /// Creates a stopped simulator using the requested capture-full policy.
    pub const fn with_capture_policy(capture_policy: CaptureFullPolicy) -> Self {
        Self {
            state: BackendState::Stopped,
            capture_policy,
            captured_blocks: [None; CAPTURE_BLOCKS],
            captured_block_count: 0,
            submitted_block_count: 0,
            dropped_capture_block_count: 0,
            rejected_submit_count: 0,
            truncated_capture_block_count: 0,
            restart_count: 0,
        }
    }

    /// Returns the configured capture-full policy.
    pub const fn capture_policy(&self) -> CaptureFullPolicy {
        self.capture_policy
    }

    /// Returns the maximum number of captured blocks retained at once.
    pub const fn max_captured_blocks(&self) -> usize {
        CAPTURE_BLOCKS
    }

    /// Returns the number of currently retained captured blocks.
    pub const fn captured_block_count(&self) -> usize {
        self.captured_block_count
    }

    /// Returns the total number of successfully submitted blocks.
    pub const fn submitted_block_count(&self) -> u64 {
        self.submitted_block_count
    }

    /// Returns the number of blocks discarded by drop-oldest capture behavior.
    pub const fn dropped_capture_block_count(&self) -> u64 {
        self.dropped_capture_block_count
    }

    /// Returns the number of submissions rejected because capture was full.
    pub const fn rejected_submit_count(&self) -> u64 {
        self.rejected_submit_count
    }

    /// Returns the number of accepted blocks omitted by truncate capture behavior.
    pub const fn truncated_capture_block_count(&self) -> u64 {
        self.truncated_capture_block_count
    }

    /// Returns the number of starts or resumes reported by this simulator.
    pub const fn restart_count(&self) -> u64 {
        self.restart_count
    }

    /// Returns the retained captured block slots in chronological order.
    pub const fn captured_block_slots(&self) -> &[Option<MixerBlock<BLOCK_FRAMES>>] {
        &self.captured_blocks
    }

    /// Returns the latest retained mixer block.
    pub fn latest_block(&self) -> Option<&MixerBlock<BLOCK_FRAMES>> {
        if self.captured_block_count == 0 {
            return None;
        }

        self.captured_blocks[self.captured_block_count - 1].as_ref()
    }

    /// Clears retained captured blocks without changing lifecycle counters.
    pub fn clear_capture(&mut self) {
        self.captured_blocks.fill(None);
        self.captured_block_count = 0;
    }

    /// Copies captured samples into `out` in chronological block order.
    ///
    /// Returns the number of samples written.
    pub fn copy_captured_samples(&self, out: &mut [i16]) -> usize {
        let mut written = 0;
        for block in self.captured_blocks.iter().flatten() {
            for sample in block.as_pcm16_mono() {
                if written == out.len() {
                    return written;
                }
                out[written] = *sample;
                written += 1;
            }
        }
        written
    }

    /// Returns the number of retained captured samples.
    pub const fn captured_sample_count(&self) -> usize {
        self.captured_block_count * BLOCK_FRAMES
    }

    /// Returns the absolute peak level of retained captured samples.
    pub fn peak_level(&self) -> i16 {
        let mut peak = 0;
        for block in self.captured_blocks.iter().flatten() {
            for sample in block.as_pcm16_mono() {
                peak = peak.max(sample.saturating_abs());
            }
        }
        peak
    }

    /// Returns true when at least one retained sample is non-zero.
    pub fn has_non_silence(&self) -> bool {
        self.peak_level() != 0
    }

    fn capture_block(&mut self, block: MixerBlock<BLOCK_FRAMES>) -> BackendResult {
        if CAPTURE_BLOCKS == 0 {
            return match self.capture_policy {
                CaptureFullPolicy::DropOldest | CaptureFullPolicy::Truncate => {
                    if self.capture_policy == CaptureFullPolicy::Truncate {
                        self.truncated_capture_block_count =
                            self.truncated_capture_block_count.saturating_add(1);
                    } else {
                        self.dropped_capture_block_count =
                            self.dropped_capture_block_count.saturating_add(1);
                    }
                    Ok(BackendReport::submitted_block())
                }
                CaptureFullPolicy::RejectSubmit => {
                    self.rejected_submit_count = self.rejected_submit_count.saturating_add(1);
                    Err(BackendError::QueueFull)
                }
            };
        }

        if self.captured_block_count < CAPTURE_BLOCKS {
            self.captured_blocks[self.captured_block_count] = Some(block);
            self.captured_block_count += 1;
            return Ok(BackendReport::submitted_block());
        }

        match self.capture_policy {
            CaptureFullPolicy::DropOldest => {
                self.captured_blocks.rotate_left(1);
                self.captured_blocks[CAPTURE_BLOCKS - 1] = Some(block);
                self.dropped_capture_block_count =
                    self.dropped_capture_block_count.saturating_add(1);
                Ok(BackendReport::submitted_block())
            }
            CaptureFullPolicy::RejectSubmit => {
                self.rejected_submit_count = self.rejected_submit_count.saturating_add(1);
                Err(BackendError::QueueFull)
            }
            CaptureFullPolicy::Truncate => {
                self.truncated_capture_block_count =
                    self.truncated_capture_block_count.saturating_add(1);
                Ok(BackendReport::submitted_block())
            }
        }
    }
}

impl SimulatorBackend {
    /// Creates a stopped v0 default simulator backend.
    pub const fn new() -> Self {
        Self::with_config()
    }
}

impl<const BLOCK_FRAMES: usize, const CAPTURE_BLOCKS: usize> Default
    for SimulatorBackend<BLOCK_FRAMES, CAPTURE_BLOCKS>
{
    fn default() -> Self {
        Self::with_config()
    }
}

impl<const BLOCK_FRAMES: usize, const CAPTURE_BLOCKS: usize> AudioBackend<BLOCK_FRAMES>
    for SimulatorBackend<BLOCK_FRAMES, CAPTURE_BLOCKS>
{
    fn start(&mut self) -> BackendResult {
        self.state = BackendState::Running;
        self.restart_count = self.restart_count.saturating_add(1);
        Ok(BackendReport::backend_restart())
    }

    fn stop(&mut self) -> BackendResult {
        self.state = BackendState::Stopped;
        Ok(BackendReport::default())
    }

    fn submit_block(&mut self, block: &MixerBlock<BLOCK_FRAMES>) -> BackendResult {
        if self.state != BackendState::Running {
            return Err(BackendError::NotRunning);
        }

        let report = self.capture_block(*block)?;
        self.submitted_block_count = self.submitted_block_count.saturating_add(1);
        Ok(report)
    }

    fn suspend(&mut self) -> BackendResult {
        self.state = BackendState::Suspended;
        Ok(BackendReport::default())
    }

    fn resume(&mut self) -> BackendResult {
        self.state = BackendState::Running;
        self.restart_count = self.restart_count.saturating_add(1);
        Ok(BackendReport::backend_restart())
    }

    fn query_state(&self) -> BackendState {
        self.state
    }

    fn reset(&mut self) -> BackendResult {
        *self = Self::with_capture_policy(self.capture_policy);
        Ok(BackendReport::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AudioCounterSnapshot, AudioLimits, AudioPolicy, AudioService, ClipAsset, MixerVolume,
    };

    const TEST_BLOCK_FRAMES: usize = 4;

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

    fn payload(samples: &[i16]) -> Payload {
        Payload::from_samples(samples)
    }

    fn clip<'a>(bytes: &'a [u8], samples: u32) -> ClipAsset<'a> {
        ClipAsset::pcm16_mono(policy().limits.sample_rate_hz, samples, bytes)
    }

    #[test]
    fn simulator_backend_start_stop_state_transition() {
        let mut backend = SimulatorBackend::new();

        assert_eq!(backend.query_state(), BackendState::Stopped);
        assert_eq!(backend.start(), Ok(BackendReport::backend_restart()));
        assert_eq!(backend.query_state(), BackendState::Running);
        assert_eq!(backend.stop(), Ok(BackendReport::default()));
        assert_eq!(backend.query_state(), BackendState::Stopped);
    }

    #[test]
    fn simulator_backend_suspend_resume_state_transition() {
        let mut backend = SimulatorBackend::new();

        backend.start().unwrap();
        assert_eq!(backend.suspend(), Ok(BackendReport::default()));
        assert_eq!(backend.query_state(), BackendState::Suspended);
        assert_eq!(backend.resume(), Ok(BackendReport::backend_restart()));
        assert_eq!(backend.query_state(), BackendState::Running);
        assert_eq!(backend.restart_count(), 2);
    }

    #[test]
    fn submit_block_captures_block_and_samples() {
        let mut backend = SimulatorBackend::<TEST_BLOCK_FRAMES, 4>::with_config();
        let block = MixerBlock::new([7, -8, 9, -10]);
        let mut samples = [0; TEST_BLOCK_FRAMES];

        backend.start().unwrap();
        assert_eq!(
            backend.submit_block(&block),
            Ok(BackendReport::submitted_block())
        );

        assert_eq!(backend.submitted_block_count(), 1);
        assert_eq!(backend.captured_block_count(), 1);
        assert_eq!(backend.latest_block(), Some(&block));
        assert_eq!(
            backend.copy_captured_samples(&mut samples),
            TEST_BLOCK_FRAMES
        );
        assert_eq!(samples, [7, -8, 9, -10]);
        assert_eq!(backend.peak_level(), 10);
        assert!(backend.has_non_silence());
    }

    #[test]
    fn clear_capture_keeps_lifecycle_counters() {
        let mut backend = SimulatorBackend::<TEST_BLOCK_FRAMES, 4>::with_config();

        backend.start().unwrap();
        backend
            .submit_block(&MixerBlock::new([1, 0, 0, 0]))
            .unwrap();
        backend.clear_capture();

        assert_eq!(backend.captured_block_count(), 0);
        assert_eq!(backend.submitted_block_count(), 1);
        assert_eq!(backend.latest_block(), None);
        assert!(!backend.has_non_silence());
    }

    #[test]
    fn capture_limit_drops_oldest_by_default() {
        let mut backend = SimulatorBackend::<TEST_BLOCK_FRAMES, 2>::with_config();
        let mut samples = [0; TEST_BLOCK_FRAMES * 2];

        backend.start().unwrap();
        backend
            .submit_block(&MixerBlock::new([1, 1, 1, 1]))
            .unwrap();
        backend
            .submit_block(&MixerBlock::new([2, 2, 2, 2]))
            .unwrap();
        backend
            .submit_block(&MixerBlock::new([3, 3, 3, 3]))
            .unwrap();

        assert_eq!(backend.capture_policy(), CaptureFullPolicy::DropOldest);
        assert_eq!(backend.submitted_block_count(), 3);
        assert_eq!(backend.captured_block_count(), 2);
        assert_eq!(backend.dropped_capture_block_count(), 1);
        assert_eq!(backend.copy_captured_samples(&mut samples), 8);
        assert_eq!(samples, [2, 2, 2, 2, 3, 3, 3, 3]);
    }

    #[test]
    fn reject_submit_policy_returns_queue_full() {
        let mut backend = SimulatorBackend::<TEST_BLOCK_FRAMES, 1>::with_capture_policy(
            CaptureFullPolicy::RejectSubmit,
        );

        backend.start().unwrap();
        backend
            .submit_block(&MixerBlock::new([1, 1, 1, 1]))
            .unwrap();

        assert_eq!(
            backend.submit_block(&MixerBlock::new([2, 2, 2, 2])),
            Err(BackendError::QueueFull)
        );
        assert_eq!(backend.submitted_block_count(), 1);
        assert_eq!(backend.rejected_submit_count(), 1);
        assert_eq!(
            backend.latest_block().unwrap().as_pcm16_mono(),
            &[1, 1, 1, 1]
        );
    }

    #[test]
    fn truncate_policy_accepts_without_extending_capture() {
        let mut backend = SimulatorBackend::<TEST_BLOCK_FRAMES, 1>::with_capture_policy(
            CaptureFullPolicy::Truncate,
        );

        backend.start().unwrap();
        backend
            .submit_block(&MixerBlock::new([1, 1, 1, 1]))
            .unwrap();
        backend
            .submit_block(&MixerBlock::new([2, 2, 2, 2]))
            .unwrap();

        assert_eq!(backend.submitted_block_count(), 2);
        assert_eq!(backend.truncated_capture_block_count(), 1);
        assert_eq!(
            backend.latest_block().unwrap().as_pcm16_mono(),
            &[1, 1, 1, 1]
        );
    }

    #[test]
    fn audio_service_uses_simulator_backend_without_call_site_changes() {
        let mut service = AudioService::<_, TEST_BLOCK_FRAMES, 4, 2, 2, 4>::new(
            policy(),
            SimulatorBackend::<TEST_BLOCK_FRAMES, 4>::with_config(),
        )
        .unwrap();

        service.start().unwrap();
        service.tick().unwrap();

        assert_eq!(service.counter_snapshot().submitted_block_count, 1);
        assert_eq!(service.backend_state(), BackendState::Running);
    }

    #[test]
    fn non_silent_pcm16_clip_produces_non_silent_capture() {
        let bytes = payload(&[10, -20, 30, -40]);
        let mut service = AudioService::<_, TEST_BLOCK_FRAMES, 4, 2, 2, 4>::new(
            policy(),
            SimulatorBackend::<TEST_BLOCK_FRAMES, 4>::with_config(),
        )
        .unwrap();

        service.start().unwrap();
        service.play_clip(clip(bytes.as_slice(), 4)).unwrap();
        service
            .set_app_volume(MixerVolume::UNITY)
            .expect("unity app volume is valid");
        service.tick().unwrap();

        assert!(service.backend().has_non_silence());
        assert_eq!(
            service.backend().latest_block().unwrap().as_pcm16_mono(),
            &[10, -20, 30, -40]
        );
        assert_eq!(
            service.counter_snapshot(),
            AudioCounterSnapshot {
                submitted_block_count: 1,
                backend_restart_count: 1,
                ..AudioCounterSnapshot::default()
            }
        );
    }

    #[test]
    fn suspended_simulator_rejects_submit_until_resume() {
        let mut backend = SimulatorBackend::<TEST_BLOCK_FRAMES, 4>::with_config();
        let block = MixerBlock::silence();

        backend.start().unwrap();
        backend.suspend().unwrap();
        assert_eq!(backend.submit_block(&block), Err(BackendError::NotRunning));
        backend.resume().unwrap();
        assert_eq!(
            backend.submit_block(&block),
            Ok(BackendReport::submitted_block())
        );
    }

    #[test]
    fn public_simulator_api_names_do_not_expose_raw_backend_details() {
        let public_surface = [
            core::any::type_name::<SimulatorBackend>(),
            core::any::type_name::<InspectionBackend>(),
            core::any::type_name::<CaptureFullPolicy>(),
            "start stop suspend resume reset submit_block captured samples latest block clear_capture peak level non-silence",
        ];
        let forbidden = ["PWM", "PIO", "DMA", "timer", "raw buffer", "buffer pointer"];

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
}
