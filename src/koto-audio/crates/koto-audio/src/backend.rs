/// Deterministic backend implementations for tests and service bring-up.
#[cfg(test)]
pub mod mock;
#[cfg(any(feature = "picocalc-backend", target_arch = "arm"))]
pub mod picocalc;
pub mod sim;

use crate::MixerBlock;

/// Result type returned by mixer-facing backend operations.
pub type BackendResult = core::result::Result<BackendReport, BackendError>;

/// Logical counter/report payload returned by backend operations.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BackendReport {
    /// Number of backend starts, restarts, or resumes.
    pub backend_restarts: u64,
    /// Number of backend underruns.
    pub underruns: u64,
    /// Number of failed block submissions.
    pub submit_failures: u64,
    /// Number of successfully submitted mixer blocks.
    pub submitted_blocks: u64,
    /// Number of silence blocks or fills emitted by backend/service recovery.
    pub silence_fills: u64,
}

impl BackendReport {
    /// Creates a report for one backend restart/resume.
    pub const fn backend_restart() -> Self {
        Self {
            backend_restarts: 1,
            underruns: 0,
            submit_failures: 0,
            submitted_blocks: 0,
            silence_fills: 0,
        }
    }

    /// Creates a report for one backend underrun.
    pub const fn underrun() -> Self {
        Self {
            backend_restarts: 0,
            underruns: 1,
            submit_failures: 0,
            submitted_blocks: 0,
            silence_fills: 0,
        }
    }

    /// Creates a report for one failed block submission.
    pub const fn submit_failure() -> Self {
        Self {
            backend_restarts: 0,
            underruns: 0,
            submit_failures: 1,
            submitted_blocks: 0,
            silence_fills: 0,
        }
    }

    /// Creates a report for one successfully submitted mixer block.
    pub const fn submitted_block() -> Self {
        Self {
            backend_restarts: 0,
            underruns: 0,
            submit_failures: 0,
            submitted_blocks: 1,
            silence_fills: 0,
        }
    }

    /// Creates a report for one silence fill.
    pub const fn silence_fill() -> Self {
        Self {
            backend_restarts: 0,
            underruns: 0,
            submit_failures: 0,
            submitted_blocks: 0,
            silence_fills: 1,
        }
    }
}

/// Abstract backend running state.
///
/// This is a logical state only; it does not expose hardware registers,
/// buffers, DMA channels, timers, or device-specific error codes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackendState {
    /// Backend has not been started.
    Stopped,
    /// Backend is accepting logical audio blocks.
    Running,
    /// Backend is quiesced for power management.
    Suspended,
    /// Backend is unavailable to this build or platform.
    Unavailable,
    /// Backend observed an underrun and awaits service recovery.
    Underrun,
    /// Backend is in an abstract error state.
    Error,
}

/// Backend boundary errors mapped into [`crate::AudioError`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackendError {
    /// Backend cannot be used.
    Unavailable,
    /// Backend is not running.
    NotRunning,
    /// Backend-side bounded queue is full.
    QueueFull,
    /// Backend observed an underrun while servicing output.
    Underrun,
    /// Backend rejected a submitted mixer block.
    SubmitFailed,
}

/// Mixer-facing backend abstraction for fixed mono blocks.
///
/// This boundary intentionally hides hardware details such as output engines,
/// timing sources, and backend-owned buffers.
pub trait AudioBackend<const BLOCK_FRAMES: usize = { crate::DEFAULT_MIXER_BLOCK_FRAMES }> {
    /// Starts the backend boundary.
    fn start(&mut self) -> BackendResult;

    /// Stops the backend boundary.
    fn stop(&mut self) -> BackendResult;

    /// Submits one fixed mono mixer block.
    fn submit_block(&mut self, block: &MixerBlock<BLOCK_FRAMES>) -> BackendResult;

    /// Suspends the backend boundary for power management.
    fn suspend(&mut self) -> BackendResult;

    /// Resumes the backend boundary after suspension.
    fn resume(&mut self) -> BackendResult;

    /// Returns the current abstract backend state.
    fn query_state(&self) -> BackendState;

    /// Clears backend-local transient state.
    fn reset(&mut self) -> BackendResult {
        self.stop()
    }
}

/// Minimal no-output backend stub for compile-time integration.
///
/// TODO(KA-M3-002): replace this with a deterministic mock backend that records
/// submitted mixer blocks and injected underruns.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)] // TODO(KA-M3-002): grow into deterministic mock backend.
pub struct NullBackend {
    state: BackendState,
}

impl NullBackend {
    /// Creates a stopped no-output backend.
    #[allow(dead_code)] // TODO(KA-M3-002): used once backend tests are expanded.
    pub const fn new() -> Self {
        Self {
            state: BackendState::Stopped,
        }
    }
}

impl Default for NullBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioBackend for NullBackend {
    fn start(&mut self) -> BackendResult {
        self.state = BackendState::Running;
        Ok(BackendReport::backend_restart())
    }

    fn stop(&mut self) -> BackendResult {
        self.state = BackendState::Stopped;
        Ok(BackendReport::default())
    }

    fn submit_block(&mut self, _block: &MixerBlock) -> BackendResult {
        if self.state != BackendState::Running {
            return Err(BackendError::NotRunning);
        }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_backend_tracks_logical_state() {
        let mut backend = NullBackend::new();

        assert_eq!(backend.query_state(), BackendState::Stopped);
        assert_eq!(backend.start(), Ok(BackendReport::backend_restart()));
        assert_eq!(backend.query_state(), BackendState::Running);
        assert_eq!(backend.suspend(), Ok(BackendReport::default()));
        assert_eq!(backend.query_state(), BackendState::Suspended);
    }

    #[test]
    fn audio_backend_dummy_compile_test() {
        fn accepts_backend<B: AudioBackend>(backend: &mut B) {
            let block: MixerBlock = MixerBlock::silence();

            backend.start().unwrap();
            backend.submit_block(&block).unwrap();
        }

        accepts_backend(&mut NullBackend::new());
    }

    #[test]
    fn public_backend_api_does_not_expose_raw_hardware_detail_names() {
        let public_surface = [
            core::any::type_name::<dyn AudioBackend>(),
            core::any::type_name::<BackendState>(),
            core::any::type_name::<BackendError>(),
            core::any::type_name::<BackendReport>(),
            core::any::type_name::<MixerBlock>(),
            "Stopped Running Suspended Unavailable Underrun Error",
            "Unavailable NotRunning QueueFull Underrun SubmitFailed",
        ];
        let forbidden = ["PWM", "PIO", "DMA", "timer", "buffer pointer"];

        for item in public_surface {
            for term in forbidden {
                assert!(!item.contains(term));
            }
        }
    }
}
