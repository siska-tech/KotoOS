use crate::{
    AudioBackend, BackendError, BackendReport, BackendResult, BackendState, MixerBlock,
    DEFAULT_MIXER_BLOCK_FRAMES,
};

/// Default number of submitted blocks retained by [`MockBackend`].
pub const DEFAULT_MOCK_RETAINED_BLOCKS: usize = 16;

/// Deterministic no-output backend for tests and service bring-up.
///
/// The mock records logical state transitions and submitted mixer blocks. It
/// never performs real audio output.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MockBackend<
    const BLOCK_FRAMES: usize = DEFAULT_MIXER_BLOCK_FRAMES,
    const RETAINED_BLOCKS: usize = DEFAULT_MOCK_RETAINED_BLOCKS,
> {
    state: BackendState,
    submitted_blocks: [Option<MixerBlock<BLOCK_FRAMES>>; RETAINED_BLOCKS],
    retained_block_count: usize,
    submitted_block_count: u64,
    underrun_count: u64,
    submit_failure_count: u64,
    restart_count: u64,
    fail_next_submit: Option<BackendError>,
}

impl<const BLOCK_FRAMES: usize, const RETAINED_BLOCKS: usize>
    MockBackend<BLOCK_FRAMES, RETAINED_BLOCKS>
{
    /// Creates a stopped mock backend with this type's configured limits.
    pub const fn with_config() -> Self {
        Self {
            state: BackendState::Stopped,
            submitted_blocks: [None; RETAINED_BLOCKS],
            retained_block_count: 0,
            submitted_block_count: 0,
            underrun_count: 0,
            submit_failure_count: 0,
            restart_count: 0,
            fail_next_submit: None,
        }
    }

    /// Returns the total number of successfully submitted blocks.
    pub const fn submitted_block_count(&self) -> u64 {
        self.submitted_block_count
    }

    /// Returns the total number of injected underruns.
    pub const fn underrun_count(&self) -> u64 {
        self.underrun_count
    }

    /// Returns the total number of simulated submit failures.
    pub const fn submit_failure_count(&self) -> u64 {
        self.submit_failure_count
    }

    /// Returns the number of starts or resumes reported by this mock.
    pub const fn restart_count(&self) -> u64 {
        self.restart_count
    }

    /// Returns the retained submitted block slots.
    pub const fn submitted_block_slots(&self) -> &[Option<MixerBlock<BLOCK_FRAMES>>] {
        &self.submitted_blocks
    }

    /// Returns the latest retained submitted block.
    pub fn latest_block(&self) -> Option<&MixerBlock<BLOCK_FRAMES>> {
        if self.retained_block_count == 0 {
            return None;
        }

        self.submitted_blocks[self.retained_block_count - 1].as_ref()
    }

    /// Injects a deterministic underrun and returns the report hook payload.
    pub fn inject_underrun(&mut self) -> BackendReport {
        self.state = BackendState::Underrun;
        self.underrun_count = self.underrun_count.saturating_add(1);
        BackendReport::underrun()
    }

    /// Makes the next block submission fail with a generic submit failure.
    pub fn fail_next_submit(&mut self) {
        self.fail_next_submit = Some(BackendError::SubmitFailed);
    }

    /// Makes the next block submission fail with a specific abstract error.
    pub fn fail_next_submit_with(&mut self, error: BackendError) {
        self.fail_next_submit = Some(error);
    }

    fn retain_block(&mut self, block: MixerBlock<BLOCK_FRAMES>) {
        if RETAINED_BLOCKS == 0 {
            return;
        }

        if self.retained_block_count < RETAINED_BLOCKS {
            self.submitted_blocks[self.retained_block_count] = Some(block);
            self.retained_block_count += 1;
            return;
        }

        self.submitted_blocks.rotate_left(1);
        self.submitted_blocks[RETAINED_BLOCKS - 1] = Some(block);
    }
}

impl MockBackend {
    /// Creates a stopped v0 default mock backend with an empty block log.
    pub const fn new() -> Self {
        Self::with_config()
    }
}

impl<const BLOCK_FRAMES: usize, const RETAINED_BLOCKS: usize> Default
    for MockBackend<BLOCK_FRAMES, RETAINED_BLOCKS>
{
    fn default() -> Self {
        Self::with_config()
    }
}

impl<const BLOCK_FRAMES: usize, const RETAINED_BLOCKS: usize> AudioBackend<BLOCK_FRAMES>
    for MockBackend<BLOCK_FRAMES, RETAINED_BLOCKS>
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
        if self.state != BackendState::Running && self.state != BackendState::Underrun {
            return Err(BackendError::NotRunning);
        }

        if let Some(error) = self.fail_next_submit.take() {
            self.submit_failure_count = self.submit_failure_count.saturating_add(1);
            self.state = BackendState::Error;
            return Err(error);
        }

        self.state = BackendState::Running;
        self.submitted_block_count = self.submitted_block_count.saturating_add(1);
        self.retain_block(*block);
        Ok(BackendReport::submitted_block())
    }

    fn query_state(&self) -> BackendState {
        self.state
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

    fn reset(&mut self) -> BackendResult {
        *self = Self::with_config();
        Ok(BackendReport::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{counters::AudioCounters, DEFAULT_MIXER_BLOCK_FRAMES};

    #[test]
    fn mock_backend_start_stop_state_transition() {
        let mut backend = MockBackend::new();

        assert_eq!(backend.query_state(), BackendState::Stopped);
        assert_eq!(backend.start(), Ok(BackendReport::backend_restart()));
        assert_eq!(backend.query_state(), BackendState::Running);
        assert_eq!(backend.stop(), Ok(BackendReport::default()));
        assert_eq!(backend.query_state(), BackendState::Stopped);
    }

    #[test]
    fn mock_backend_suspend_resume_state_transition() {
        let mut backend = MockBackend::new();

        backend.start().unwrap();
        assert_eq!(backend.suspend(), Ok(BackendReport::default()));
        assert_eq!(backend.query_state(), BackendState::Suspended);
        assert_eq!(backend.resume(), Ok(BackendReport::backend_restart()));
        assert_eq!(backend.query_state(), BackendState::Running);
    }

    #[test]
    fn submit_block_records_block() {
        let mut backend = MockBackend::new();
        let block = MixerBlock::new([7; DEFAULT_MIXER_BLOCK_FRAMES]);

        backend.start().unwrap();
        assert_eq!(
            backend.submit_block(&block),
            Ok(BackendReport::submitted_block())
        );

        assert_eq!(backend.submitted_block_count(), 1);
        assert_eq!(backend.latest_block(), Some(&block));
        assert_eq!(backend.submitted_block_slots()[0], Some(block));
    }

    #[test]
    fn manual_underrun_injection_reports_hook() {
        let mut backend = MockBackend::new();
        let mut counters = AudioCounters::default();

        backend.start().unwrap();
        let report = backend.inject_underrun();
        counters.record_backend_report(report);

        assert_eq!(backend.query_state(), BackendState::Underrun);
        assert_eq!(backend.underrun_count(), 1);
        assert_eq!(counters.underrun_count, 1);
    }

    #[test]
    fn submit_failure_simulation_reports_error() {
        let mut backend = MockBackend::new();
        let mut counters = AudioCounters::default();
        let block = MixerBlock::silence();

        backend.start().unwrap();
        backend.fail_next_submit();
        let result = backend.submit_block(&block);

        assert_eq!(result, Err(BackendError::SubmitFailed));
        counters.record_backend_error(BackendError::SubmitFailed);
        assert_eq!(backend.submit_failure_count(), 1);
        assert_eq!(counters.backend_submit_failure_count, 1);
    }

    #[test]
    fn backend_restart_counter_hook() {
        let mut backend = MockBackend::new();
        let mut counters = AudioCounters::default();

        counters.record_backend_report(backend.start().unwrap());
        counters.record_backend_report(backend.resume().unwrap());

        assert_eq!(backend.restart_count(), 2);
        assert_eq!(counters.backend_restart_count, 2);
    }
}
