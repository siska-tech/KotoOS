use super::*;

/// A simulator-facing snapshot of a running bytecode app's VM and host state.
/// Counts and occupancy only: open file handles index the per-app sandbox and
/// draw counts are captured output, so nothing here exposes host paths.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InspectorReport {
    pub app_id: String,
    pub frame: usize,
    pub run_state: VmRunResult,
    pub pc: u32,
    pub frame_fuel_used: u32,
    /// The most recent `HOST_CALL` id, or `None` before any host call.
    pub last_host_call: Option<u8>,
    pub last_vm_error: Option<koto_core::VmError>,
    pub last_input: VmInputSnapshot,
    pub open_files: usize,
    pub draw_rects: usize,
    pub draw_pixels: usize,
    pub text_draws: usize,
    pub audio_events: usize,
}

impl InspectorReport {
    /// A short name for the last host call (`"<none>"` before any host call).
    pub fn last_host_call_name(&self) -> &'static str {
        match self.last_host_call {
            Some(id) => koto_core::runtime::host_call::name(id),
            None => "<none>",
        }
    }
}

/// Render an [`InspectorReport`] as one deterministic line, suitable for tests
/// and CLI output.
pub fn describe_inspector_report(report: &InspectorReport) -> String {
    let run_state = match report.run_state {
        VmRunResult::Yielded => "yielded".to_string(),
        VmRunResult::Exited(code) => format!("exited({code})"),
        VmRunResult::FuelExhausted => "fuel-exhausted".to_string(),
    };
    let error = match report.last_vm_error {
        Some(error) => format!("{error:?}"),
        None => "<none>".to_string(),
    };
    let input = report.last_input;
    format!(
        "inspect {} frame={} state={} pc={} fuel={} last_host_call={} error={} \
         input(held={:#x} pressed={:#x} char={} intent={:#x}) \
         open_files={} draw_rects={} draw_pixels={} text_draws={} audio_events={}",
        report.app_id,
        report.frame,
        run_state,
        report.pc,
        report.frame_fuel_used,
        report.last_host_call_name(),
        error,
        input.held_bits,
        input.pressed_bits,
        input.text_codepoint,
        input.intent_bits,
        report.open_files,
        report.draw_rects,
        report.draw_pixels,
        report.text_draws,
        report.audio_events,
    )
}
