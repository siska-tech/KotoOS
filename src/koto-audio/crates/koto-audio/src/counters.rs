use crate::{source::SourceBus, BackendError, BackendReport};

/// Snapshot of logical audio diagnostics visible to normal/runtime callers.
///
/// Raw backend hardware state is intentionally excluded.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AudioCounters {
    /// Currently active logical sources.
    pub active_source_count: u16,
    /// Currently active BGM bus sources.
    pub active_bgm_source_count: u16,
    /// Currently active SFX bus sources.
    pub active_sfx_source_count: u16,
    /// Currently queued logical sources.
    pub queued_source_count: u16,
    /// Total admitted BGM sources.
    pub bgm_start_count: u64,
    /// Total stopped BGM sources, including replacement stops.
    pub bgm_stop_count: u64,
    /// Total BGM sources stopped by replacement policy.
    pub bgm_replaced_count: u64,
    /// Total dropped sources.
    pub dropped_source_count: u64,
    /// Total stolen sources.
    pub stolen_source_count: u64,
    /// Total mixer/backend underruns.
    pub underrun_count: u64,
    /// Total late mixer ticks.
    pub late_mix_count: u64,
    /// Maximum observed mix time in implementation-defined ticks.
    ///
    /// TODO(KA-M4-004): define the tick unit with the mixer timing source.
    pub max_mix_time_ticks: u64,
    /// Total backend starts/restarts/resumes.
    pub backend_restart_count: u64,
    /// Total failed backend block submissions.
    pub backend_submit_failure_count: u64,
    /// Total blocks submitted to the backend boundary.
    pub submitted_block_count: u64,
    /// Total silence fills emitted for recovery or empty output.
    pub silence_fill_count: u64,
    /// Total source queue full admissions.
    pub queue_full_count: u64,
    /// Total malformed asset validation failures.
    pub malformed_asset_count: u64,
    /// Total event queue drops.
    ///
    /// TODO(KA-M5-004): increment when the bounded event queue is implemented.
    pub event_queue_full_count: u64,
}

impl AudioCounters {
    /// Returns a normal public snapshot without backend-owned state.
    pub const fn snapshot(self) -> AudioCounterSnapshot {
        AudioCounterSnapshot {
            active_source_count: self.active_source_count,
            active_bgm_source_count: self.active_bgm_source_count,
            active_sfx_source_count: self.active_sfx_source_count,
            queued_source_count: self.queued_source_count,
            bgm_start_count: self.bgm_start_count,
            bgm_stop_count: self.bgm_stop_count,
            bgm_replaced_count: self.bgm_replaced_count,
            dropped_source_count: self.dropped_source_count,
            stolen_source_count: self.stolen_source_count,
            underrun_count: self.underrun_count,
            late_mix_count: self.late_mix_count,
            max_mix_time: self.max_mix_time_ticks,
            backend_restart_count: self.backend_restart_count,
            queue_full_count: self.queue_full_count,
            malformed_asset_count: self.malformed_asset_count,
            backend_submit_failure_count: self.backend_submit_failure_count,
            submitted_block_count: self.submitted_block_count,
            event_queue_full_count: self.event_queue_full_count,
        }
    }

    /// Applies a backend report to this counter snapshot.
    pub fn record_backend_report(&mut self, report: BackendReport) {
        self.backend_restart_count = self
            .backend_restart_count
            .saturating_add(report.backend_restarts);
        self.underrun_count = self.underrun_count.saturating_add(report.underruns);
        self.backend_submit_failure_count = self
            .backend_submit_failure_count
            .saturating_add(report.submit_failures);
        self.submitted_block_count = self
            .submitted_block_count
            .saturating_add(report.submitted_blocks);
        self.silence_fill_count = self.silence_fill_count.saturating_add(report.silence_fills);
    }

    /// Applies a backend error to counters when the operation returned `Err`.
    pub fn record_backend_error(&mut self, error: BackendError) {
        match error {
            BackendError::Underrun => self.record_backend_report(BackendReport::underrun()),
            BackendError::SubmitFailed => {
                self.record_backend_report(BackendReport::submit_failure());
            }
            BackendError::QueueFull => {
                self.record_backend_report(BackendReport::submit_failure());
            }
            BackendError::Unavailable | BackendError::NotRunning => {}
        }
    }

    pub(crate) fn increment_active_bus(&mut self, bus: SourceBus) {
        match bus {
            SourceBus::Bgm => {
                self.active_bgm_source_count = self.active_bgm_source_count.saturating_add(1);
            }
            SourceBus::Sfx => {
                self.active_sfx_source_count = self.active_sfx_source_count.saturating_add(1);
            }
        }
    }

    pub(crate) fn decrement_active_bus(&mut self, bus: SourceBus) {
        match bus {
            SourceBus::Bgm => {
                self.active_bgm_source_count = self.active_bgm_source_count.saturating_sub(1);
            }
            SourceBus::Sfx => {
                self.active_sfx_source_count = self.active_sfx_source_count.saturating_sub(1);
            }
        }
    }
}

/// Stable public snapshot of logical audio counters.
///
/// This intentionally excludes raw backend buffers, hardware state, and
/// backend-owned implementation detail.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AudioCounterSnapshot {
    /// Currently active logical sources.
    pub active_source_count: u16,
    /// Currently active BGM bus sources.
    pub active_bgm_source_count: u16,
    /// Currently active SFX bus sources.
    pub active_sfx_source_count: u16,
    /// Currently queued logical sources.
    pub queued_source_count: u16,
    /// Total admitted BGM sources.
    pub bgm_start_count: u64,
    /// Total stopped BGM sources, including replacement stops.
    pub bgm_stop_count: u64,
    /// Total BGM sources stopped by replacement policy.
    pub bgm_replaced_count: u64,
    /// Total dropped sources.
    pub dropped_source_count: u64,
    /// Total stolen sources.
    pub stolen_source_count: u64,
    /// Total mixer/backend underruns.
    pub underrun_count: u64,
    /// Total late mixer ticks.
    pub late_mix_count: u64,
    /// Maximum observed mix time in implementation-defined ticks.
    pub max_mix_time: u64,
    /// Total backend starts/restarts/resumes.
    pub backend_restart_count: u64,
    /// Total source queue full admissions.
    pub queue_full_count: u64,
    /// Total malformed asset validation failures.
    pub malformed_asset_count: u64,
    /// Total failed backend block submissions.
    pub backend_submit_failure_count: u64,
    /// Total blocks submitted to the backend boundary.
    pub submitted_block_count: u64,
    /// Total event queue drops.
    pub event_queue_full_count: u64,
}
