/// A per-app memory and frame-fuel budget report (KOTO-0101). Every `*_peak` is a
/// session high-water mark; the paired `*_cap`/`*_request`/`*_budget` is the bound
/// it must stay under. SRAM-resident VM state (stack/locals/heap) is distinguished
/// from the per-frame fuel budget and from host-owned working sets (draw lists,
/// file handles) whose pixel/PCM bytes never live in the VM heap.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppBudgetReport {
    pub app_id: String,
    pub frames: usize,
    pub stack_slots_peak: u16,
    pub stack_slots_cap: u16,
    pub call_depth_peak: u16,
    pub call_depth_cap: u16,
    pub local_slots_peak: u16,
    pub local_slots_cap: u16,
    /// Highest heap byte the VM addressed (bytes in use).
    pub heap_bytes_peak: u32,
    /// The program's KBC heap request (the heap it was given).
    pub heap_request: u32,
    /// The manifest's declared per-app SRAM working budget (device ceiling), if any.
    pub heap_budget: Option<u32>,
    pub frame_fuel_peak: u32,
    pub frame_fuel_cap: u32,
    /// Slowest observed host wall-clock frame. Informational rather than a
    /// deterministic CI gate; fuel remains the reproducible execution bound.
    pub frame_time_us_peak: u64,
    pub host_calls_per_frame_peak: u32,
    /// Fixed host-owned retained KotoUI session state.
    pub ui_session_sram_bytes: usize,
    /// Highest retained KotoUI rectangle + text command count.
    pub ui_render_commands_peak: usize,
    pub open_files_peak: usize,
    pub open_files_cap: usize,
    pub draw_rects_peak: usize,
    pub draw_pixels_peak: usize,
    pub text_draws_peak: usize,
    pub audio_events_peak: usize,
}

/// Render an [`AppBudgetReport`] as one parseable `key=value` line. All fields
/// except observational `frame_time_us_peak` are deterministic for a scenario.
/// `heap_budget` is `none` when the manifest declares no SRAM working budget.
pub fn describe_app_budget_report(report: &AppBudgetReport) -> String {
    let heap_budget = match report.heap_budget {
        Some(bytes) => bytes.to_string(),
        None => "none".to_string(),
    };
    format!(
        "budget app={} frames={} \
         stack_peak={} stack_cap={} call_peak={} call_cap={} \
         local_peak={} local_cap={} heap_peak={} heap_request={} heap_budget={} \
         fuel_peak={} fuel_cap={} frame_time_us_peak={} host_calls_peak={} \
         ui_session_sram={} ui_render_commands_peak={} \
         open_files_peak={} open_files_cap={} draw_rects_peak={} draw_pixels_peak={} \
         text_draws_peak={} audio_events_peak={}",
        report.app_id,
        report.frames,
        report.stack_slots_peak,
        report.stack_slots_cap,
        report.call_depth_peak,
        report.call_depth_cap,
        report.local_slots_peak,
        report.local_slots_cap,
        report.heap_bytes_peak,
        report.heap_request,
        heap_budget,
        report.frame_fuel_peak,
        report.frame_fuel_cap,
        report.frame_time_us_peak,
        report.host_calls_per_frame_peak,
        report.ui_session_sram_bytes,
        report.ui_render_commands_peak,
        report.open_files_peak,
        report.open_files_cap,
        report.draw_rects_peak,
        report.draw_pixels_peak,
        report.text_draws_peak,
        report.audio_events_peak,
    )
}
