use crate::{
    AudioBackend, BackendError, BackendReport, BackendResult, BackendState, MixerBlock,
    DEFAULT_MIXER_BLOCK_FRAMES,
};

/// PicoCalc backend sample-rate candidates for real hardware experiments.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PicoCalcSampleRateCandidate {
    /// 16 kHz mono output experiment.
    Hz16000,
    /// 22.05 kHz mono output experiment.
    Hz22050,
}

impl PicoCalcSampleRateCandidate {
    /// Returns the candidate sample rate in hertz.
    pub const fn as_hz(self) -> u32 {
        match self {
            Self::Hz16000 => 16_000,
            Self::Hz22050 => 22_050,
        }
    }
}

/// PicoCalc backend buffer-depth candidates measured in mixer blocks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PicoCalcBufferDepthCandidate {
    /// Two backend blocks.
    Blocks2,
    /// Three backend blocks.
    Blocks3,
    /// Four backend blocks.
    Blocks4,
}

impl PicoCalcBufferDepthCandidate {
    /// Returns the candidate depth in mixer blocks.
    pub const fn as_blocks(self) -> u8 {
        match self {
            Self::Blocks2 => 2,
            Self::Blocks3 => 3,
            Self::Blocks4 => 4,
        }
    }
}

/// Policy for how often the backend should surface underrun diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PicoCalcUnderrunReportPolicy {
    /// Record every observed underrun.
    EveryOccurrence,
    /// Coalesce consecutive underruns until the backend is restarted or reset.
    CoalesceUntilRestart,
}

/// Experiment policy for the PicoCalc backend boundary.
///
/// Values are deliberately candidates rather than production commitments. The
/// structure avoids exposing device handles, register state, or backend-owned
/// storage to normal application code.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PicoCalcBackendConfig {
    /// Sample-rate candidate for this run.
    pub sample_rate: PicoCalcSampleRateCandidate,
    /// Logical mixer block size candidate in frames.
    pub block_frames: u16,
    /// Backend queue depth candidate in mixer blocks.
    pub buffer_depth: PicoCalcBufferDepthCandidate,
    /// Number of silent blocks submitted before live mixer data.
    pub silent_prefill_blocks: u8,
    /// Underrun reporting behavior for this experiment.
    pub underrun_report_policy: PicoCalcUnderrunReportPolicy,
}

impl PicoCalcBackendConfig {
    /// Creates the default v0 PicoCalc backend experiment policy.
    pub const fn new() -> Self {
        Self {
            sample_rate: PicoCalcSampleRateCandidate::Hz16000,
            block_frames: 128,
            buffer_depth: PicoCalcBufferDepthCandidate::Blocks3,
            silent_prefill_blocks: 2,
            underrun_report_policy: PicoCalcUnderrunReportPolicy::EveryOccurrence,
        }
    }

    /// Returns true when the block size is one of the planned candidates.
    pub const fn has_candidate_block_size(self) -> bool {
        self.block_frames == 128 || self.block_frames == 256
    }
}

impl Default for PicoCalcBackendConfig {
    fn default() -> Self {
        Self::new()
    }
}

/// PicoCalc backend measurement counters.
///
/// The latency field is a placeholder using implementation-defined ticks until
/// real hardware validation defines a stable measurement source.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PicoCalcBackendCounters {
    /// Number of successfully submitted mixer blocks.
    pub submitted_blocks: u64,
    /// Number of observed underruns.
    pub underruns: u64,
    /// Number of failed block submissions.
    pub submit_failures: u64,
    /// Maximum observed submit latency in implementation-defined ticks.
    pub max_submit_latency_ticks: u64,
    /// Number of starts or resumes.
    pub restart_resume_count: u64,
    /// Number of silent prefill blocks accepted by this backend.
    pub silent_prefill_blocks: u64,
}

impl PicoCalcBackendCounters {
    fn record_restart(&mut self) -> BackendReport {
        self.restart_resume_count = self.restart_resume_count.saturating_add(1);
        BackendReport::backend_restart()
    }

    fn record_submit(&mut self) -> BackendReport {
        self.submitted_blocks = self.submitted_blocks.saturating_add(1);
        BackendReport::submitted_block()
    }

    fn record_submit_failure(&mut self) -> BackendReport {
        self.submit_failures = self.submit_failures.saturating_add(1);
        BackendReport::submit_failure()
    }

    fn record_underrun(&mut self) -> BackendReport {
        self.underruns = self.underruns.saturating_add(1);
        BackendReport::underrun()
    }

    fn record_silence_fill(&mut self) -> BackendReport {
        self.silent_prefill_blocks = self.silent_prefill_blocks.saturating_add(1);
        BackendReport::silence_fill()
    }
}

/// PicoCalc backend state and measurement snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PicoCalcBackendSnapshot {
    /// Current abstract backend state.
    pub state: BackendState,
    /// Active experiment policy.
    pub config: PicoCalcBackendConfig,
    /// Current measurement counters.
    pub counters: PicoCalcBackendCounters,
}

/// PicoCalc/RP2040 audio backend experiment boundary.
///
/// This is intentionally a compile-time and measurement skeleton. It implements
/// [`AudioBackend`] without exposing hardware details, and it can be replaced by
/// a real device implementation inside this module without changing
/// [`crate::AudioService`] call sites.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PicoCalcBackend<const BLOCK_FRAMES: usize = DEFAULT_MIXER_BLOCK_FRAMES> {
    state: BackendState,
    config: PicoCalcBackendConfig,
    counters: PicoCalcBackendCounters,
    queued_blocks: u8,
    underrun_pending: bool,
}

impl<const BLOCK_FRAMES: usize> PicoCalcBackend<BLOCK_FRAMES> {
    /// Creates a stopped backend using the default experiment policy.
    pub const fn new() -> Self {
        Self::with_config(PicoCalcBackendConfig::new())
    }

    /// Creates a stopped backend using the provided experiment policy.
    pub const fn with_config(config: PicoCalcBackendConfig) -> Self {
        Self {
            state: BackendState::Stopped,
            config,
            counters: PicoCalcBackendCounters {
                submitted_blocks: 0,
                underruns: 0,
                submit_failures: 0,
                max_submit_latency_ticks: 0,
                restart_resume_count: 0,
                silent_prefill_blocks: 0,
            },
            queued_blocks: 0,
            underrun_pending: false,
        }
    }

    /// Returns the active experiment policy.
    pub const fn config(&self) -> PicoCalcBackendConfig {
        self.config
    }

    /// Returns the backend-local measurement counters.
    pub const fn counters(&self) -> PicoCalcBackendCounters {
        self.counters
    }

    /// Returns a snapshot of backend state, policy, and counters.
    pub const fn snapshot(&self) -> PicoCalcBackendSnapshot {
        PicoCalcBackendSnapshot {
            state: self.state,
            config: self.config,
            counters: self.counters,
        }
    }

    /// Records a submit latency observation in implementation-defined ticks.
    pub fn record_submit_latency_ticks(&mut self, ticks: u64) {
        self.counters.max_submit_latency_ticks = self.counters.max_submit_latency_ticks.max(ticks);
    }

    /// Records an underrun observation and returns the standard report payload.
    pub fn report_underrun(&mut self) -> BackendReport {
        self.state = BackendState::Underrun;
        if self.config.underrun_report_policy == PicoCalcUnderrunReportPolicy::CoalesceUntilRestart
            && self.underrun_pending
        {
            return BackendReport::default();
        }

        self.underrun_pending = true;
        self.counters.record_underrun()
    }

    /// Records a submit failure observation and returns the standard report payload.
    pub fn report_submit_failure(&mut self) -> BackendReport {
        self.state = BackendState::Error;
        self.counters.record_submit_failure()
    }

    fn prefill_silence(&mut self) -> BackendReport {
        let mut report = BackendReport::default();
        let target = self.config.silent_prefill_blocks;
        while self.queued_blocks < target
            && self.queued_blocks < self.config.buffer_depth.as_blocks()
        {
            self.queued_blocks = self.queued_blocks.saturating_add(1);
            let fill = self.counters.record_silence_fill();
            report.silence_fills = report.silence_fills.saturating_add(fill.silence_fills);
        }
        report
    }

    fn start_or_resume(&mut self) -> BackendReport {
        self.state = BackendState::Running;
        self.underrun_pending = false;
        let mut report = self.counters.record_restart();
        let prefill = self.prefill_silence();
        report.silence_fills = report.silence_fills.saturating_add(prefill.silence_fills);
        report
    }
}

impl<const BLOCK_FRAMES: usize> Default for PicoCalcBackend<BLOCK_FRAMES> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const BLOCK_FRAMES: usize> AudioBackend<BLOCK_FRAMES> for PicoCalcBackend<BLOCK_FRAMES> {
    fn start(&mut self) -> BackendResult {
        if !self.config.has_candidate_block_size()
            || usize::from(self.config.block_frames) != BLOCK_FRAMES
        {
            self.counters.record_submit_failure();
            self.state = BackendState::Unavailable;
            return Err(BackendError::Unavailable);
        }

        Ok(self.start_or_resume())
    }

    fn stop(&mut self) -> BackendResult {
        self.state = BackendState::Stopped;
        self.queued_blocks = 0;
        Ok(BackendReport::default())
    }

    fn submit_block(&mut self, _block: &MixerBlock<BLOCK_FRAMES>) -> BackendResult {
        if self.state != BackendState::Running && self.state != BackendState::Underrun {
            return Err(BackendError::NotRunning);
        }

        self.state = BackendState::Running;
        if self.queued_blocks < self.config.buffer_depth.as_blocks() {
            self.queued_blocks = self.queued_blocks.saturating_add(1);
        }
        Ok(self.counters.record_submit())
    }

    fn suspend(&mut self) -> BackendResult {
        self.state = BackendState::Suspended;
        Ok(BackendReport::default())
    }

    fn resume(&mut self) -> BackendResult {
        Ok(self.start_or_resume())
    }

    fn query_state(&self) -> BackendState {
        self.state
    }

    fn reset(&mut self) -> BackendResult {
        *self = Self::with_config(self.config);
        Ok(BackendReport::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::counters::AudioCounters;

    #[test]
    fn picocalc_backend_follows_audio_backend_boundary() {
        let mut backend = PicoCalcBackend::<128>::new();
        let block = MixerBlock::<128>::silence();

        let start = backend.start().unwrap();
        assert_eq!(backend.query_state(), BackendState::Running);
        assert_eq!(start.backend_restarts, 1);
        assert_eq!(start.silence_fills, 2);

        assert_eq!(
            backend.submit_block(&block),
            Ok(BackendReport::submitted_block())
        );
        assert_eq!(backend.counters().submitted_blocks, 1);

        assert_eq!(backend.suspend(), Ok(BackendReport::default()));
        assert_eq!(backend.query_state(), BackendState::Suspended);
        assert_eq!(backend.resume().unwrap().backend_restarts, 1);
        assert_eq!(backend.counters().restart_resume_count, 2);
    }

    #[test]
    fn measurement_hooks_connect_to_backend_report_counters() {
        let mut backend = PicoCalcBackend::<128>::new();
        let mut counters = AudioCounters::default();

        counters.record_backend_report(backend.start().unwrap());
        counters.record_backend_report(backend.report_underrun());
        counters.record_backend_report(backend.report_submit_failure());
        backend.record_submit_latency_ticks(42);

        assert_eq!(counters.backend_restart_count, 1);
        assert_eq!(counters.underrun_count, 1);
        assert_eq!(counters.backend_submit_failure_count, 1);
        assert_eq!(backend.snapshot().state, BackendState::Error);
        assert_eq!(backend.snapshot().counters.max_submit_latency_ticks, 42);
    }

    #[test]
    fn invalid_experiment_block_size_is_unavailable() {
        let mut backend = PicoCalcBackend::<64>::with_config(PicoCalcBackendConfig {
            block_frames: 64,
            ..PicoCalcBackendConfig::new()
        });

        assert_eq!(backend.start(), Err(BackendError::Unavailable));
        assert_eq!(backend.query_state(), BackendState::Unavailable);
    }

    #[test]
    fn public_picocalc_api_names_do_not_expose_raw_hardware_details() {
        let public_surface = [
            core::any::type_name::<PicoCalcBackend<128>>(),
            core::any::type_name::<PicoCalcBackendConfig>(),
            core::any::type_name::<PicoCalcBackendCounters>(),
            core::any::type_name::<PicoCalcBackendSnapshot>(),
            "sample rate block frames queue depth silent prefill underrun report latency state",
        ];
        let forbidden = ["PWM", "PIO", "DMA", "timer", "raw buffer", "buffer pointer"];

        for item in public_surface {
            for term in forbidden {
                assert!(!item.contains(term));
            }
        }
    }
}
