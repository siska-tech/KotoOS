use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SimError {
    Io,
    InvalidInputScript,
    InvalidManifest,
    InvalidRenderCommand,
    InvalidRuntime,
    RuntimeVerifyFailed,
    RuntimeExecutionFailed,
    /// The app's KBC heap request exceeds the manifest's declared `sram_work_bytes`
    /// device budget (per-app profile validation, KOTO-0096).
    AppExceedsMemoryBudget,
    MemoValidationFailed,
    PackageListFull,
}

/// Sentinel in a text-draw colour slot meaning "the colourless `draw_text` host
/// call was used; the frontend should fall back to its default app-text colour".
/// It is outside the value range a VM-supplied colour can hold (those arrive as
/// sign-extended `i16`), so it never collides with a real colour such as white.
pub const TEXT_COLOR_DEFAULT: i32 = i32::MIN;

pub const KOTORUNTIME_BYTECODE: &str = "kotoruntime-bytecode";
// The simulator's fixed VM dimensions and per-frame fuel derive from the one
// canonical profile, `RuntimeLimits::simulator_default()` (KOTO-0060): operand-stack
// slots, call depth, and the per-frame instruction budget. The app heap is *not*
// fixed — each app's VM gets a heap sized to its own KBC header request (per-app
// profile, KOTO-0096); `SIM_HEAP_CEILING` is the device ceiling an app may not
// exceed (and the verifier enforces it).
pub const SIM_PROFILE: koto_core::RuntimeLimits = koto_core::RuntimeLimits::simulator_default();
pub const SIM_FRAME_FUEL: u32 = SIM_PROFILE.frame_fuel;
pub const SIM_VM_STACK_SLOTS: usize = SIM_PROFILE.max_stack_slots as usize;
pub const SIM_VM_CALL_DEPTH: usize = SIM_PROFILE.max_call_depth as usize;
pub const SIM_HEAP_CEILING: usize = SIM_PROFILE.max_heap_bytes as usize;
pub const SIM_MAX_OPEN_FILES: usize = 8;
/// Frame cap for a no-script `run_app_scenario`, bounding apps that loop forever
/// without an exit so a direct headless launch always terminates.
pub const SIM_APP_IDLE_FRAME_CAP: usize = 1024;
/// Document capacity for the host-side editor backing the text-buffer host calls.
pub const MEMO_DOC_CAPACITY: usize = 1024;
/// System IME dictionary path the runtime host loads on the app's behalf. It is a
/// host-privileged read outside the per-app save-data sandbox. `skk_koto.skk` is
/// the original KotoOS dictionary (KOTO-0089); the firmware opens the same name
/// on the real SD card.
pub(super) const SKK_DICT_PATH: &str = "dict/skk_koto.skk";
pub(super) const MEMO_APP_ID: &str = "dev.koto.memo";
pub(super) const MEMO_FILE_PATH: &str = "memo.txt";
pub(super) const SAVE_DATA_ROOT: &str = "data";
pub(super) type SimMemoEditor = MemoEditor<MEMO_DOC_CAPACITY>;
/// SKK dictionary fixture used by tests to populate a temp sdcard `dict/`.
#[cfg(test)]
pub(super) const MEMO_VALIDATION_DICT: &[u8] =
    include_bytes!("../../../../harness/fixtures/skk_min.skk");
