//! Stage, verify, and run a launching app's bytecode (KOTO-0127), plus the
//! keyboard FIFO read and per-frame input snapshot the app session consumes.

use core::{cell::UnsafeCell, fmt::Write};

use embassy_rp::i2c::I2c;
use embassy_rp::peripherals;
use embassy_rp::uart::UartTx;
use embassy_time::{block_for, Duration, Instant, Timer};
use embedded_sdmmc::{BlockDevice, LfnBuffer, Mode, ShortFileName, VolumeIdx, VolumeManager};
#[cfg(feature = "psram_qpi_code_window_counters")]
use koto_core::psram::PsramCodeWindowDebugState;
use koto_core::psram::{PsramBlocks, PsramCodeWindow};
#[cfg(all(
    feature = "psram_dma_read_code_window",
    feature = "psram_dma_read_code_window_diag"
))]
use koto_core::VerifyError;
use koto_core::{
    BytecodeSession, CodeSource, KbcHeader, PackageInfo, RuntimeLimits, SessionError, SliceCode,
    VmInputSnapshot, VmRunResult, KBC_HEADER_SIZE,
};
use koto_gfx::{BudgetObservation, FullRepaintReason, APP_DRAW_BUDGET};

use crate::dashboard::LineBuffer;
use crate::firmware::app_host::{
    AppDrawCommand, AppStaticLayer, DeviceHost, DeviceRuntimeHost, StaticLayerShadow,
};
// DIAG-0001 Stage 2: `phase=161` command sample and `phase=154/155/162` draw
// usage/overflow are now routed through `DIAG_PROFILE` at their emit sites (the
// old `psram_qpi_code_window_prod_profile` cfg no longer gates any log), so these
// formatters import unconditionally — a disabled profile branch is const-folded
// away at the call site instead.
use crate::firmware::app_render::log_command_sample;
use crate::firmware::audio::{PicoAudioBackend, RUNTIME_CUE_IMAGE_CAPACITY};
use crate::firmware::config::{
    DiagClass, FirmwareClock, CODE_WINDOW_TOTAL_BYTES, DEVICE_CODE_CEILING, DEVICE_FRAME_FUEL,
    DEVICE_VM_CALL_DEPTH, DEVICE_VM_STACK_SLOTS, DIAG_PROFILE, KEYBOARD_REGISTER_SETTLE_US,
    MANIFEST_LFN_BYTES, MAX_APP_DRAW_COMMANDS, MAX_DEVICE_HEAP_BYTES, MAX_EVENTS_PER_FRAME,
    RASTER_STRIP_BYTES, RGB666_STRIP_BYTES,
};
use crate::firmware::diag::{
    log_app_budget_observation, log_app_cmdshift_correlation, log_app_cmdshift_probe,
    log_app_coalesce_pressure, log_app_draw_overflow, log_app_draw_usage, log_app_frame_metrics,
    log_app_present_cost, log_app_static_rebuild, log_app_vm_cost, log_code_window_fetch,
    log_dirty_rect_geometry, log_key_event, on_cadence, uart_log, uart_write_line, PaintMetrics,
};
use crate::firmware::display_service::{DisplayService, PresentRequest};
use crate::firmware::stack_canary;
use crate::keyboard::{
    HeldKeys, KeyEvent, FIFO_CAPACITY, FIFO_REGISTER, FRAME_PERIOD_MS, KEY_F3, KEY_STATE_HOLD,
    KEY_STATE_PRESSED,
};
use crate::lcd::{PicoCalcLcd, Rgb888};
use crate::pins::KeyboardPins;
#[cfg(all(
    feature = "psram_dma_read_code_window",
    feature = "psram_dma_read_code_window_diag"
))]
use crate::psram::dma_code_window_read_trace_snapshot;
#[cfg(all(
    feature = "psram_dma_read_code_window",
    feature = "psram_dma_read_code_window_diag"
))]
use crate::psram::DmaCodeWindowPsram;
#[cfg(all(
    feature = "psram_dma_read_code_window",
    feature = "psram_dma_read_code_window_diag"
))]
use crate::psram::DmaReadTraceMode;
use crate::psram::FirmwarePsramHal;
#[cfg(all(
    feature = "psram_qpi_code_window_verbose",
    feature = "psram_qpi_backend_v2"
))]
use crate::psram::PsramMode;
#[cfg(feature = "psram_qpi_code_window_counters")]
use crate::psram::QpiCodeWindowPsram;
use crate::psram::PSRAM_FAST_READ_DUMMY_CYCLES;
#[cfg(all(
    feature = "psram_fast_code_window",
    not(all(
        feature = "psram_qpi_safe_read_code_window",
        feature = "psram_qpi_backend_v2"
    ))
))]
use crate::psram::PSRAM_PIO_SYS_HZ;
#[cfg(all(
    feature = "psram_qpi_code_window_counters",
    not(feature = "psram_qpi_backend_v2")
))]
use crate::psram::PSRAM_PROD_READ_CHUNK_BYTES;
#[cfg(feature = "psram_qpi_safe_read_code_window")]
use crate::psram::PSRAM_QPI_SAFE_READ_CHUNK_BYTES;
#[cfg(all(
    feature = "psram_qpi_safe_read_code_window",
    not(feature = "psram_qpi_backend_v2")
))]
use crate::psram::PSRAM_QPI_SAFE_READ_SM_HZ;
#[cfg(all(
    not(feature = "psram_qpi_safe_read_code_window"),
    not(feature = "psram_fast_code_window")
))]
use crate::psram::{PSRAM_PIO_SM_HZ, PSRAM_PROD_READ_CHUNK_BYTES};
#[cfg(all(
    feature = "psram_qpi_safe_read_code_window",
    feature = "psram_qpi_backend_v2"
))]
use crate::psram::{PSRAM_PIO_SYS_HZ, PSRAM_QPI_V2_READ_CLOCK_DIVIDER};
#[cfg(feature = "psram_fast_code_window")]
use crate::psram_ext::{
    koto_psram_fast_code_window_snapshot, KotoPsramFastReadMode, PSRAM_FAST_CODE_WINDOW_CHUNK_BYTES,
};

const AUDIO_CUE_CACHE_ENTRIES: usize = 16;

struct AudioLoadScratch {
    image: [u8; RUNTIME_CUE_IMAGE_CAPACITY],
}

impl AudioLoadScratch {
    const fn new() -> Self {
        Self {
            image: [0; RUNTIME_CUE_IMAGE_CAPACITY],
        }
    }
}

/// CPU0 is the sole accessor. Keeping this large scratch outside the embassy
/// future avoids adding ~22 KiB to the main task and does not mask interrupts
/// during SD I/O or KMML compilation.
struct Cpu0AudioScratch(UnsafeCell<AudioLoadScratch>);
unsafe impl Sync for Cpu0AudioScratch {}
static AUDIO_LOAD_SCRATCH: Cpu0AudioScratch =
    Cpu0AudioScratch(UnsafeCell::new(AudioLoadScratch::new()));

#[derive(Clone, Copy)]
struct AudioCueCacheEntry {
    key: u64,
    address: u32,
    len: u16,
    bgm: bool,
    used: bool,
}

impl AudioCueCacheEntry {
    const EMPTY: Self = Self {
        key: 0,
        address: 0,
        len: 0,
        bgm: false,
        used: false,
    };
}

#[cfg(all(
    feature = "psram_dma_read_code_window",
    feature = "psram_dma_read_code_window_diag"
))]
fn code_window_read_mode_label() -> &'static str {
    match dma_code_window_read_trace_snapshot().last_mode {
        DmaReadTraceMode::Dma => "sm1_cpu_tx_rx_dma_serial",
        DmaReadTraceMode::DmaFallback => "sm1_dma_fallback_prod16",
        DmaReadTraceMode::PhaseEdgeFudge => "sm0_pio_cpu_clkdiv3_prod16",
        DmaReadTraceMode::Legacy => "prod16_serial_pio_cpu",
        DmaReadTraceMode::None => "prod16_serial_pio_cpu",
    }
}

#[cfg(all(
    feature = "psram_dma_read_code_window",
    feature = "psram_dma_read_code_window_diag"
))]
fn code_window_read_chunk_bytes() -> usize {
    match dma_code_window_read_trace_snapshot().last_mode {
        DmaReadTraceMode::Dma => PSRAM_PROD_READ_CHUNK_BYTES,
        DmaReadTraceMode::DmaFallback => PSRAM_PROD_READ_CHUNK_BYTES,
        DmaReadTraceMode::PhaseEdgeFudge => PSRAM_PROD_READ_CHUNK_BYTES,
        DmaReadTraceMode::Legacy => PSRAM_PROD_READ_CHUNK_BYTES,
        DmaReadTraceMode::None => PSRAM_PROD_READ_CHUNK_BYTES,
    }
}

#[cfg(all(
    feature = "psram_dma_read_code_window",
    not(feature = "psram_dma_read_code_window_diag")
))]
fn code_window_read_chunk_bytes() -> usize {
    PSRAM_PROD_READ_CHUNK_BYTES
}

#[cfg(feature = "psram_qpi_safe_read_code_window")]
fn code_window_read_chunk_bytes() -> usize {
    PSRAM_QPI_SAFE_READ_CHUNK_BYTES
}

#[cfg(all(
    feature = "psram_qpi_safe_read_code_window",
    feature = "psram_qpi_backend_v2"
))]
fn code_window_read_sm_hz() -> u32 {
    PSRAM_PIO_SYS_HZ / PSRAM_QPI_V2_READ_CLOCK_DIVIDER
}

#[cfg(all(
    not(feature = "psram_dma_read_code_window"),
    not(feature = "psram_qpi_safe_read_code_window"),
    not(feature = "psram_fast_code_window")
))]
fn code_window_read_chunk_bytes() -> usize {
    PSRAM_PROD_READ_CHUNK_BYTES
}

#[cfg(all(
    not(feature = "psram_dma_read_code_window"),
    not(feature = "psram_qpi_safe_read_code_window"),
    feature = "psram_fast_code_window"
))]
fn code_window_read_chunk_bytes() -> usize {
    // Fast refills read in this chunk size; a safe fallback re-reads at the safe
    // chunk size, but the dominant path is the fast chunk.
    PSRAM_FAST_CODE_WINDOW_CHUNK_BYTES
}

#[cfg(all(
    feature = "psram_dma_read_code_window",
    feature = "psram_dma_read_code_window_diag"
))]
fn code_window_read_sm_hz() -> u32 {
    match dma_code_window_read_trace_snapshot().last_mode {
        DmaReadTraceMode::Dma => PSRAM_PIO_SM_HZ,
        DmaReadTraceMode::DmaFallback => PSRAM_PIO_SM_HZ,
        DmaReadTraceMode::PhaseEdgeFudge => PSRAM_PIO_SM_HZ,
        DmaReadTraceMode::Legacy => PSRAM_PIO_SM_HZ,
        DmaReadTraceMode::None => PSRAM_PIO_SM_HZ,
    }
}

#[cfg(all(
    feature = "psram_dma_read_code_window",
    not(feature = "psram_dma_read_code_window_diag")
))]
fn code_window_read_sm_hz() -> u32 {
    PSRAM_PIO_SM_HZ
}

#[cfg(all(
    feature = "psram_qpi_safe_read_code_window",
    not(feature = "psram_qpi_backend_v2")
))]
fn code_window_read_sm_hz() -> u32 {
    PSRAM_QPI_SAFE_READ_SM_HZ
}

#[cfg(all(
    not(feature = "psram_dma_read_code_window"),
    not(feature = "psram_qpi_safe_read_code_window"),
    not(feature = "psram_fast_code_window")
))]
fn code_window_read_sm_hz() -> u32 {
    PSRAM_PIO_SM_HZ
}

#[cfg(all(
    not(feature = "psram_dma_read_code_window"),
    not(feature = "psram_qpi_safe_read_code_window"),
    feature = "psram_fast_code_window"
))]
fn code_window_read_sm_hz() -> u32 {
    // FastFallingClkdiv2 clocks the PSRAM SM at sys / read_clkdiv (2.0).
    PSRAM_PIO_SYS_HZ / 2
}

#[cfg(feature = "psram_dma_read_code_window")]
fn code_window_log_tag() -> &'static str {
    "pio3v1"
}

#[cfg(feature = "psram_qpi_safe_read_code_window")]
fn code_window_log_tag() -> &'static str {
    #[cfg(feature = "psram_qpi_backend_v2")]
    {
        return "qpi-v2-r8w2-c120";
    }

    #[cfg(not(feature = "psram_qpi_backend_v2"))]
    {
        "qpi6rw120v1"
    }
}

#[cfg(all(
    not(feature = "psram_dma_read_code_window"),
    not(feature = "psram_qpi_safe_read_code_window"),
    feature = "legacy_psram"
))]
fn code_window_log_tag() -> &'static str {
    "legacy"
}

#[cfg(all(
    not(feature = "psram_dma_read_code_window"),
    not(feature = "psram_qpi_safe_read_code_window"),
    not(feature = "legacy_psram")
))]
fn code_window_log_tag() -> &'static str {
    "koto_psram"
}

#[cfg(all(
    feature = "psram_dma_read_code_window",
    not(feature = "psram_dma_read_code_window_diag")
))]
fn code_window_read_mode_label() -> &'static str {
    "sm0_pio_cpu_clkdiv3_prod16"
}

#[cfg(feature = "psram_qpi_safe_read_code_window")]
fn code_window_read_mode_label() -> &'static str {
    #[cfg(feature = "psram_qpi_backend_v2")]
    {
        return "qpi_v2_r8";
    }

    #[cfg(not(feature = "psram_qpi_backend_v2"))]
    {
        "qpi_rw_clkdiv6_chunk120"
    }
}

#[cfg(all(
    feature = "psram_qpi_code_window_verbose",
    feature = "psram_qpi_backend_v2"
))]
fn qpi_mode_label(mode: PsramMode) -> &'static str {
    match mode {
        PsramMode::Unknown => "unknown",
        PsramMode::QpiRw => "qpi_rw",
        PsramMode::QpiWriteOnly => "qpi_write_only",
        PsramMode::RecoverSerial => "recover_serial",
    }
}

#[cfg(all(
    not(feature = "psram_dma_read_code_window"),
    not(feature = "psram_qpi_safe_read_code_window"),
    feature = "legacy_psram"
))]
fn code_window_read_mode_label() -> &'static str {
    "prod16_serial_pio_cpu"
}

#[cfg(all(
    not(feature = "psram_dma_read_code_window"),
    not(feature = "psram_qpi_safe_read_code_window"),
    not(feature = "legacy_psram"),
    not(feature = "psram_fast_code_window")
))]
fn code_window_read_mode_label() -> &'static str {
    "koto_psram_safe"
}

#[cfg(all(
    not(feature = "psram_dma_read_code_window"),
    not(feature = "psram_qpi_safe_read_code_window"),
    not(feature = "legacy_psram"),
    feature = "psram_fast_code_window"
))]
fn code_window_read_mode_label() -> &'static str {
    // Report the read mode the most recent refill actually used.
    match koto_psram_fast_code_window_snapshot().last_mode {
        KotoPsramFastReadMode::FastClkdiv2 => "koto_psram_fast_clkdiv2",
        KotoPsramFastReadMode::SafeFallback => "koto_psram_safe",
        KotoPsramFastReadMode::None => "koto_psram_safe",
    }
}

/// Outcome of staging a launching app's code (KOTO-0127). Verification, heap
/// sizing, and the run loop are driven from this plus the resident header.
struct StagedApp {
    /// Total `.kbc` length on the SD card (used for the streaming range checks).
    file_len: usize,
    /// Code segment length in bytes (always a multiple of 4).
    code_size: usize,
    /// Code segment length in 4-byte words (the VM program-counter range).
    code_words: u32,
    /// `true` when the code was staged into PSRAM; `false` when it was loaded into
    /// the SRAM window directly (PSRAM-absent fallback).
    used_psram: bool,
    /// Base byte address where code is staged in PSRAM.
    code_base_addr: u32,
    /// Resolved FAT short name of the authoritative APPS/*.kpa archive.
    package_file: ShortFileName,
    #[cfg(all(
        feature = "psram_dma_read_code_window",
        feature = "psram_dma_read_code_window_diag"
    ))]
    /// First 64 bytes from the original code stream before PSRAM/SRAM staging.
    source_first64: [u8; 64],
    #[cfg(all(
        feature = "psram_dma_read_code_window",
        feature = "psram_dma_read_code_window_diag"
    ))]
    /// Valid length in `source_first64` (always up to 64).
    source_first64_len: usize,
}

#[cfg_attr(not(feature = "psram_qpi_code_window_counters"), allow(dead_code))]
#[derive(Default)]
struct QpiCodeWindowCounters {
    cw_refills: u32,
    cw_refill_us_total: u32,
    cw_refill_us_max: u32,
    cw_verify_fail_count: u32,
    cw_map_fail_count: u32,
    first_fail_frame: u32,
    first_fail_tile: u32,
    first_fail_off: u32,
    first_fail_exp: u8,
    first_fail_got: u8,
    has_first_fail: bool,
}

#[cfg_attr(not(feature = "psram_qpi_code_window_counters"), allow(dead_code))]
impl QpiCodeWindowCounters {
    fn record_refills(&mut self, refills: u32, refill_us_total: u32, refill_us_max: u32) {
        self.cw_refills = self.cw_refills.saturating_add(refills);
        self.cw_refill_us_total = self.cw_refill_us_total.saturating_add(refill_us_total);
        self.cw_refill_us_max = self.cw_refill_us_max.max(refill_us_max);
    }

    fn record_verify(&mut self, frame: u32, tile: u32, fail: Option<(usize, u8, u8)>) {
        if let Some((off, exp, got)) = fail {
            self.cw_verify_fail_count = self.cw_verify_fail_count.saturating_add(1);
            self.record_first_fail(frame, tile, off, exp, got);
        }
    }

    fn record_map(&mut self, frame: u32, tile: u32, fail: Option<(usize, u8, u8)>) {
        if let Some((off, exp, got)) = fail {
            self.cw_map_fail_count = self.cw_map_fail_count.saturating_add(1);
            self.record_first_fail(frame, tile, off, exp, got);
        }
    }

    fn record_first_fail(&mut self, frame: u32, tile: u32, off: usize, exp: u8, got: u8) {
        if self.has_first_fail {
            return;
        }
        self.has_first_fail = true;
        self.first_fail_frame = frame;
        self.first_fail_tile = tile;
        self.first_fail_off = off as u32;
        self.first_fail_exp = exp;
        self.first_fail_got = got;
    }

    fn log(&self, uart: &mut UartTx<'_, embassy_rp::uart::Blocking>, app_id: &str, reason: &str) {
        let mut line = LineBuffer::new();
        let _ = write!(
            line,
            "phase=166 cw-counters app={} reason={} cw_refills={} cw_refill_us_total={} cw_refill_us_max={} cw_verify_fail_count={} cw_map_fail_count={} first_fail_frame={} first_fail_tile={} first_fail_off={} first_fail_exp=0x{:02x} first_fail_got=0x{:02x}\r\n",
            app_id,
            reason,
            self.cw_refills,
            self.cw_refill_us_total,
            self.cw_refill_us_max,
            self.cw_verify_fail_count,
            self.cw_map_fail_count,
            if self.has_first_fail { self.first_fail_frame as i32 } else { -1 },
            if self.has_first_fail { self.first_fail_tile as i32 } else { -1 },
            if self.has_first_fail { self.first_fail_off as i32 } else { -1 },
            self.first_fail_exp,
            self.first_fail_got,
        );
        uart_write_line(uart, &line);
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn run_device_app<D>(
    volume_mgr: &VolumeManager<D, FirmwareClock>,
    package: PackageInfo,
    psram: &mut Option<PsramBlocks<FirmwarePsramHal<'_>>>,
    code_window: &mut [u8; CODE_WINDOW_TOTAL_BYTES],
    heap: &mut [u8; MAX_DEVICE_HEAP_BYTES],
    lfn_storage: &mut [u8; MANIFEST_LFN_BYTES],
    keyboard: &mut I2c<'_, peripherals::I2C1, embassy_rp::i2c::Blocking>,
    lcd: &mut PicoCalcLcd<'_>,
    font: &koto_core::BitmapFont<'_>,
    raster_strip: &mut [u8; RASTER_STRIP_BYTES],
    rgb666_strip: &mut [u8; RGB666_STRIP_BYTES],
    // The current/previous frame draw-command lists, owned by a binary StaticCell
    // so these two ~30 KiB buffers stay out of the main-task future (KOTO-0134).
    app_draw: &mut [DeviceRuntimeHost; 2],
    // The single retained static/background layer, owned by its own binary
    // StaticCell (NOT double-buffered with `app_draw`, KOTO-0136).
    static_layer: &mut AppStaticLayer,
    // Fingerprint shadow of the last applied static layer (GFX-0013), also in
    // its own binary StaticCell. Lets a mid-session rebuild be diffed against
    // what the panel already shows instead of forcing a whole-surface repaint.
    static_shadow: &mut StaticLayerShadow,
    audio: &mut PicoAudioBackend,
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
) where
    D: BlockDevice,
    D::Error: core::fmt::Debug,
{
    if package.runtime() != Some("kotoruntime-bytecode") {
        audio.stop();
        uart_log(uart, "phase=250 launch-unsupported-runtime\r\n");
        return;
    }
    let Some(entry) = package.entry() else {
        audio.stop();
        uart_log(uart, "phase=251 launch-missing-entry\r\n");
        return;
    };
    let Some(_file_name) = entry.rsplit('/').next() else {
        audio.stop();
        uart_log(uart, "phase=251 launch-missing-entry\r\n");
        return;
    };
    let mut header = [0u8; KBC_HEADER_SIZE];
    let Some(staged) = stage_app_code(
        volume_mgr,
        package.app_id(),
        entry,
        &mut header,
        psram.as_mut(),
        code_window,
        heap,
        lfn_storage,
        uart,
    ) else {
        return;
    };
    let limits = RuntimeLimits {
        max_stack_slots: DEVICE_VM_STACK_SLOTS as u16,
        max_call_depth: DEVICE_VM_CALL_DEPTH as u16,
        max_heap_bytes: MAX_DEVICE_HEAP_BYTES as u32,
        frame_fuel: DEVICE_FRAME_FUEL,
        treat_ret_as_exit: true,
    };
    // Run the verified program against whichever code source backs it: a PSRAM
    // window for staged code, or the SRAM window directly for the fallback. The
    // session contract and run loop are shared (`run_app_session`); only the
    // `CodeSource` differs (KOTO-0127).
    if staged.used_psram {
        let Some(psram) = psram.as_mut() else {
            uart_log(uart, "phase=255 launch-memory-budget-error\r\n");
            return;
        };
        #[cfg(all(
            feature = "psram_dma_read_code_window",
            feature = "psram_dma_read_code_window_diag"
        ))]
        log_launch_header_compare(
            psram,
            code_window,
            &staged.source_first64,
            staged.source_first64_len,
            staged.code_base_addr,
            staged.code_words,
            uart,
        );
        // The board profile selects the resident CodeWindow slot count: RP2040
        // retains two slots while RP2350A can spend its additional SRAM on a
        // three-slot working set without changing portable VM code.
        let mut code = PsramCodeWindow::new_with_slots(
            psram,
            code_window,
            staged.code_base_addr,
            staged.code_words,
            crate::firmware::config::CODE_WINDOW_TILES,
        );
        // Install a monotonic microsecond clock so the window times each PSRAM refill
        // (KOTO-0132 phase 1); the fallback SliceCode path never refills and reports 0.
        code.set_refill_clock(|| Instant::now().as_micros());
        run_app_session(
            volume_mgr,
            &package,
            &mut code,
            &header,
            staged.file_len,
            staged.code_size,
            staged.code_base_addr,
            staged.package_file.clone(),
            limits,
            heap,
            keyboard,
            lcd,
            font,
            raster_strip,
            rgb666_strip,
            app_draw,
            static_layer,
            static_shadow,
            audio,
            uart,
        )
        .await;
    } else {
        let mut code = SliceCode::new(&code_window[..staged.code_size], 0);
        run_app_session(
            volume_mgr,
            &package,
            &mut code,
            &header,
            staged.file_len,
            staged.code_size,
            staged.code_base_addr,
            staged.package_file.clone(),
            limits,
            heap,
            keyboard,
            lcd,
            font,
            raster_strip,
            rgb666_strip,
            app_draw,
            static_layer,
            static_shadow,
            audio,
            uart,
        )
        .await;
    }
}

/// Verify a staged program from its resident header plus a [`CodeSource`], then run
/// its cooperative frame loop to clean exit or graceful failure (KOTO-0127). The
/// `code` source is the only platform-specific piece: PicoCalc passes a PSRAM
/// window, but this body is identical to KotoSim's session loop.
#[allow(clippy::too_many_arguments)]
async fn run_app_session<C, D>(
    volume_mgr: &VolumeManager<D, FirmwareClock>,
    package: &PackageInfo,
    code: &mut C,
    header: &[u8],
    file_len: usize,
    staged_code_size: usize,
    #[cfg_attr(
        not(feature = "psram_qpi_code_window_counters"),
        allow(unused_variables)
    )]
    staged_code_base_addr: u32,
    package_file: ShortFileName,
    limits: RuntimeLimits,
    heap: &mut [u8; MAX_DEVICE_HEAP_BYTES],
    keyboard: &mut I2c<'_, peripherals::I2C1, embassy_rp::i2c::Blocking>,
    lcd: &mut PicoCalcLcd<'_>,
    font: &koto_core::BitmapFont<'_>,
    raster_strip: &mut [u8; RASTER_STRIP_BYTES],
    rgb666_strip: &mut [u8; RGB666_STRIP_BYTES],
    app_draw: &mut [DeviceRuntimeHost; 2],
    static_layer: &mut AppStaticLayer,
    static_shadow: &mut StaticLayerShadow,
    audio: &mut PicoAudioBackend,
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
) where
    C: CodeSource + CodeWindowVerifyExt,
    D: BlockDevice,
    D::Error: core::fmt::Debug,
{
    let mut session =
        match BytecodeSession::<DEVICE_VM_STACK_SLOTS, DEVICE_VM_CALL_DEPTH>::new_streaming(
            header,
            file_len,
            code,
            limits,
            DEVICE_FRAME_FUEL,
        ) {
            Ok(session) => session,
            Err(SessionError::Verify(error)) => {
                let mut line = LineBuffer::new();
                let _ = write!(line, "phase=254 launch-verify-error error={:?}\r\n", error);
                uart_write_line(uart, &line);
                audio.stop();
                #[cfg(all(
                    feature = "psram_dma_read_code_window",
                    feature = "psram_dma_read_code_window_diag"
                ))]
                if error == VerifyError::BadBytecodeSize {
                    log_bad_bytecode_size_trace(
                        code,
                        header,
                        file_len,
                        staged_code_size,
                        _staged_code_base_addr,
                        uart,
                    );
                }
                return;
            }
            Err(SessionError::Vm(error)) => {
                let mut line = LineBuffer::new();
                let _ = write!(line, "phase=256 launch-vm-init-error error={:?}\r\n", error);
                uart_write_line(uart, &line);
                audio.stop();
                return;
            }
        };
    let heap_len = session.program().header().max_heap_bytes as usize;
    if heap_len > heap.len()
        || package
            .sram_work_bytes()
            .is_some_and(|budget| session.program().header().max_heap_bytes > budget)
    {
        uart_log(uart, "phase=255 launch-memory-budget-error\r\n");
        return;
    }
    // The const heap image (KOTO-0139) was already copied into heap[0..rodata_size]
    // by `stage_app_code` while the file was open; zero only the rest so the table
    // bake never runs as VM code. rodata_size <= max_heap_bytes == heap_len.
    let rodata_size = (session.program().header().rodata_size as usize).min(heap_len);
    heap[rodata_size..heap_len].fill(0);
    // Split the shared StaticCell pair into the current frame's list (owned by the
    // host) and the retained previous frame's list (KOTO-0134). Both buffers may
    // hold stale commands from a prior launch; `DeviceHost::new` clears the
    // current one, and the previous one is cleared here.
    let (current_slot, previous_slot) = app_draw.split_at_mut(1);
    let previous_draw = &mut previous_slot[0];
    previous_draw.clear_frame();
    let mut host = DeviceHost::new(
        volume_mgr,
        package.app_id(),
        &heap[..heap_len],
        &mut current_slot[0],
        static_layer,
        audio,
        package_file,
    );
    let mut audio_cache = [AudioCueCacheEntry::EMPTY; AUDIO_CUE_CACHE_ENTRIES];
    let mut audio_next_address =
        align_psram_address(staged_code_base_addr.saturating_add(staged_code_size as u32));
    host.load_skk();
    if !host.diag.as_bytes().is_empty() {
        uart_write_line(uart, &host.diag);
        host.diag.clear();
    }
    let mut has_previous_draw = false;
    // GFX-0013: the shadow may hold the previous session's static layer; this
    // session has applied nothing yet, so there is no trusted baseline to diff a
    // rebuild against until the first present captures one.
    static_shadow.invalidate();
    // One-shot latch for BUG-GFX-0012: after the app rebuilds its retained static layer,
    // the *next* present must be a full repaint even though a previous frame exists, because
    // the rebuild frame composited the new chrome but may have overdrawn it (e.g. KotoSnake
    // still paints its title over the freshly built HUD on the crossing-into-play frame), so
    // the GFX-0010/0011 incremental rescue would otherwise never reveal it. The bit is carried
    // from one present to the next by `force_full_repaint_after_static_rebuild`.
    let mut static_reveal_latched = false;
    // The display-service seam (GFX-0005): the frame loop issues present requests to
    // it instead of inlining the compose/flush. Stateless today; held across the
    // session so a future overlay/damage registry has a home without another
    // call-site change.
    let mut display_service = DisplayService::new();
    let mut held = HeldKeys::new();
    let mut frame = 0u32;
    // Latches the first time a frame fills the command list, so an overflow is
    // surfaced once rather than spamming every busy frame (KOTO-0129). The
    // one-shot `phase=162 app-draw-overflow` notice it gates is always-on
    // (DIAG-0001 §2 event-only), so this latch is now unconditional too.
    let mut draw_overflow_logged = false;
    // Session high-water of the per-frame draw-command count, and the running count
    // of frames that hit the cap (tail dropped). These ride the throttled
    // `phase=160` line so the KOTO-0134 160-cap can be judged against real usage.
    let mut draw_peak = 0usize;
    let mut overflow_count = 0u32;
    // Cumulative count of frames this session that rebuilt the Game2D static layer
    // (GFX-0009 Stage-0). Rides the throttled `phase=160` line as `static_rebuilds=`
    // so a hardware run can tell a one-shot-per-gameplay-entry rebuild (healthy) from
    // a per-frame rebuild (an accidental silent full repaint). Observe-only.
    let mut static_rebuilds = 0u32;
    // Latches the first mid-session static rebuild, so the off-cadence `phase=170`
    // notice is surfaced once; the periodic sample still catches a recurring rebuild
    // without flooding UART.
    let mut static_rebuild_logged = false;
    // Latches the first frame the observe-only budget would have degraded/rejected
    // any command (GFX-0006B), so first pressure is surfaced once off-cadence rather
    // than every busy frame; the periodic `phase=168` line still samples regardless.
    let mut budget_pressure_logged = false;
    // Latches the first frame whose full repaint was attributed to a CommandCountShift
    // (GFX-0006C pre-work), so the budget-correlation `phase=169` line is surfaced once
    // off-cadence rather than every shift frame; the periodic sample still catches later
    // shifts that land on the cadence.
    let mut cmdshift_corr_logged = false;
    // Latches the first frame whose present path recorded a coalesce-before-decide
    // contrast (GFX-0010 Stage 1B: a surviving RectsExceeded/AreaExceeded full repaint or
    // a frame converted back to incremental), so the `phase=171` line is surfaced once
    // off-cadence rather than every escalating frame; the periodic sample still catches
    // later ones that land on the cadence.
    let mut coalesce_pressure_logged = false;
    #[cfg(feature = "psram_qpi_code_window_counters")]
    let mut cw_counters = QpiCodeWindowCounters::default();
    #[cfg(all(
        feature = "psram_dma_read_code_window",
        feature = "psram_dma_read_code_window_diag"
    ))]
    let mut last_dma_fallbacks = dma_code_window_read_trace_snapshot().dma_fallbacks;
    #[cfg(not(feature = "psram_qpi_code_window_prod_profile"))]
    {
        let mut line = LineBuffer::new();
        let _ = write!(
            line,
            "phase=151 app-budget heap_request={} heap_actual={} heap_ceiling={} stack={} locals=48\r\n",
            session.program().header().max_heap_bytes,
            heap_len,
            MAX_DEVICE_HEAP_BYTES,
            DEVICE_VM_STACK_SLOTS,
        );
        uart_write_line(uart, &line);
    }
    #[cfg(all(
        feature = "psram_dma_read_code_window",
        feature = "psram_dma_read_code_window_diag",
        not(feature = "psram_qpi_code_window_prod_profile")
    ))]
    {
        let trace = dma_code_window_read_trace_snapshot();
        let mut line = LineBuffer::new();
        let _ = write!(
            line,
            "phase=331 cw-dma-read-experiment app={} app_launch_result=started dma_successes={} dma_fallbacks={} gameplay_result=running\r\n",
            package.app_id(),
            trace.dma_successes,
            trace.dma_fallbacks,
        );
        uart_write_line(uart, &line);
    }
    uart_log(uart, "phase=152 app-started\r\n");
    // Apps start with a clean surface. Incremental-drawing samples may only
    // repaint a narrow band and must never inherit KotoShell pixels.
    if lcd.fill(Rgb888::BLACK).await.is_err() {
        host.close_package_stream();
        audio.stop();
        uart_log(uart, "phase=258 app-clear-error\r\n");
        return;
    }

    loop {
        let frame_started = Instant::now();
        let mut latest = None;
        let mut f3_pressed = false;
        for _ in 0..MAX_EVENTS_PER_FRAME.min(FIFO_CAPACITY) {
            match read_event(keyboard) {
                Ok(event) if event.is_empty() => break,
                Ok(event) => {
                    // Raw bridge codes on UART (KOTO-0177): press/release only,
                    // HOLD repeats would flood at the bridge's repeat rate.
                    if event.state != KEY_STATE_HOLD {
                        let has = |key| held.as_slice().contains(&key);
                        let mut line = LineBuffer::new();
                        log_key_event(
                            uart,
                            &mut line,
                            frame,
                            event.state,
                            event.key,
                            has(0xa2) || has(0xa3),
                        );
                    }
                    // F3 is consumed by the host (wrap toggle) and never reaches
                    // the VM, matching the sim's session.toggle_wrap() path.
                    if event.state == KEY_STATE_PRESSED && event.key == KEY_F3 {
                        f3_pressed = true;
                    } else {
                        held.apply(event);
                        latest = Some(event);
                    }
                }
                Err(()) => break,
            }
        }
        if f3_pressed {
            host.toggle_wrap();
        }
        let snapshot = app_input_snapshot(&held, latest);
        host.service_audio();
        host.clear_frame();
        // Zero the code-window fetch counters so `refills`/`code_tiles` on this
        // frame's phase=160 line reflect only this frame's PSRAM thrash (KOTO-0134).
        code.reset_fetch_metrics();
        let vm_started = Instant::now();
        let result = session.step_frame_with(code, &mut host, snapshot, &mut heap[..heap_len]);
        let vm_us = vm_started.elapsed().as_micros();
        let host_calls = session.last_frame_host_calls();
        let code_refills = code.fetch_refills();
        let code_tiles = code.fetch_distinct_tiles();
        // Per-frame PSRAM refill timing (KOTO-0132 phase 1): total/worst microseconds
        // and bytes the window refills cost this frame, read alongside the counts.
        let cw_refill_us = code.cw_refill_us_total();
        let cw_refill_max_us = code.cw_refill_us_max();
        let cw_bytes = code.cw_refill_bytes();
        service_pending_audio_asset(
            &mut host,
            code,
            &mut audio_cache,
            &mut audio_next_address,
            uart,
        );
        #[cfg(feature = "psram_qpi_code_window_counters")]
        cw_counters.record_refills(code_refills, cw_refill_us, cw_refill_max_us);
        // Drain any UART diagnostic buffered by a host call (e.g. asset_load).
        if !host.diag.as_bytes().is_empty() {
            uart_write_line(uart, &host.diag);
            host.diag.clear();
        }
        match result {
            Ok(VmRunResult::Yielded) | Ok(VmRunResult::FuelExhausted) => {}
            Ok(VmRunResult::Exited(code)) => {
                #[cfg(feature = "psram_qpi_code_window_counters")]
                cw_counters.log(uart, package.app_id(), "app_exit");
                let mut line = LineBuffer::new();
                let _ = write!(line, "phase=153 app-exited code={}\r\n", code);
                uart_write_line(uart, &line);
                host.close_package_stream();
                audio.stop();
                #[cfg(feature = "psram_dma_read_code_window")]
                {
                    let mut line = LineBuffer::new();
                    let app_id = package.app_id();
                    let is_kotoblocks = app_id.contains("blocks");
                    let _ = write!(
                        line,
                        "phase=332 cw-dma-read-experiment app={} app_launch_result=exited code={} kotoblocks_gameplay_result={}\r\n",
                        app_id,
                        code,
                        if is_kotoblocks {
                            if code == 0 { "pass" } else { "fail" }
                        } else {
                            "n/a"
                        }
                    );
                    uart_write_line(uart, &line);
                }
                return;
            }
            Err(error) => {
                #[cfg(feature = "psram_qpi_code_window_counters")]
                code.log_qpi_code_window_verify(
                    volume_mgr,
                    package,
                    header,
                    staged_code_base_addr,
                    frame,
                    &mut cw_counters,
                    uart,
                );
                #[cfg(feature = "psram_qpi_code_window_counters")]
                cw_counters.log(uart, package.app_id(), "vm_error");
                let mut line = LineBuffer::new();
                let _ = write!(
                    line,
                    "phase=257 app-vm-error app={} pc={} error={:?} heap_len={}\r\n",
                    package.app_id(),
                    session.pc(),
                    error,
                    heap_len,
                );
                uart_write_line(uart, &line);
                #[cfg(feature = "psram_dma_read_code_window")]
                {
                    let mut line = LineBuffer::new();
                    let _ = write!(
                        line,
                        "phase=332 cw-dma-read-experiment app={} app_launch_result=vm_error kotoblocks_gameplay_result=fail\r\n",
                        package.app_id()
                    );
                    uart_write_line(uart, &line);
                }
                host.close_package_stream();
                return;
            }
        }
        // Read the Game2D static-layer state for diagnostics (KOTO-0136):
        // `static_cmds` is the retained chrome command count and `static_rebuilt`
        // flags the frame the app rebuilt it (the one full repaint). `rebuilt` is
        // set by `game2d_static_begin` during this frame's VM step and reset by the
        // next `clear_frame`, so it is read here, after the step and before present.
        let static_cmds = host.static_layer.len;
        let static_rebuilt = host.static_layer.rebuilt;
        // Accumulate the static-rebuild count for the GFX-0009 Stage-0 diagnostic
        // (read here, where `rebuilt` reflects a `game2d_static_begin` issued this
        // frame, before `clear_frame` resets it). Observe-only — no present change.
        if static_rebuilt {
            static_rebuilds = static_rebuilds.saturating_add(1);
        }
        // GFX-0013: align the just-rebuilt static layer against the fingerprint
        // shadow of the last *applied* one and plan how the rebuild presents:
        //   identical -> nothing to repaint (Stage 1 — the panel already shows it);
        //   bounded   -> the unmatched commands' old∪new union rects ride the
        //                delta working set like every other layer (Stage 2);
        //   otherwise -> the whole-surface StaticRebuild repaint + BUG-GFX-0012
        //                reveal latch, exactly as pre-GFX-0013 (the escape hatch).
        // The first paint has no baseline (`!has_previous_draw`) and stays on the
        // full path, matching the phase=170 frame-1 suppression.
        let mut static_align = None;
        let mut static_damage = [koto_gfx::Rect {
            x: 0,
            y: 0,
            w: 0,
            h: 0,
        }; koto_gfx::STATIC_DAMAGE_CAP];
        let mut static_damage_len = 0usize;
        let mut static_damage_px = 0u32;
        if static_rebuilt && has_previous_draw {
            // The buffer is sized to the alignment window, so a bounded diff can
            // never overflow it (≤ one union rect per unmatched slot).
            let mut damage_overflow = false;
            static_align = Some(koto_gfx::collect_static_rebuild_dirty(
                static_shadow,
                &host.static_layer.commands[..host.static_layer.len],
                320,
                320,
                koto_gfx::STATIC_DAMAGE_CAP,
                &mut static_damage,
                &mut static_damage_len,
                &mut static_damage_px,
                &mut damage_overflow,
            ));
        }
        let identical_rebuild = matches!(
            static_align,
            Some(koto_gfx::StaticRebuildAlignment::Identical)
        );
        // BUG-GFX-0012 ambiguity guard: when the static layer supplies the retained
        // base, an immediate full-screen fill in either diffed frame is invisible to
        // both the base-change check (one shared layer instance on both sides) and
        // the immediate command diff (base fills are skipped as clear-to-base) — the
        // exact overdraw that hid KotoSnake's freshly built chrome. A rebuild in
        // that state is ambiguous, so it falls back to the latch path rather than
        // trusting bounded damage. Steady gameplay immediate lists carry no
        // full-screen fill (the base lives in the static layer), so the recurring
        // per-action rebuilds this issue targets are unaffected.
        let base_overdraw_hazard = {
            let has_base = |commands: &[AppDrawCommand]| {
                commands
                    .iter()
                    .any(|command| koto_gfx::is_full_screen_base(*command, 320, 320))
            };
            has_base(&host.static_layer.commands[..host.static_layer.len])
                && (has_base(&previous_draw.commands[..previous_draw.len])
                    || has_base(&host.draw.commands[..host.draw.len]))
        };
        let bounded_rebuild = matches!(
            static_align,
            Some(koto_gfx::StaticRebuildAlignment::Bounded { .. })
        ) && !base_overdraw_hazard;
        // The fallback: this frame rebuilt the layer and the diff could not
        // positively bound the change (first paint, no shadow, wide relayout, or
        // the hazard above) — whole-surface repaint now, reveal latch armed below.
        let full_rebuild = static_rebuilt && !identical_rebuild && !bounded_rebuild;
        // Damage collected for a rebuild the plan rejected (hazard) must not feed
        // the delta; keep the collected counts for the phase=170 line regardless.
        let (static_would_rects, static_would_px) = (static_damage_len, static_damage_px);
        if !bounded_rebuild {
            static_damage_len = 0;
        }
        let mut metrics = PaintMetrics::default();
        // Present when the immediate list/board changed, on the first frame, or when
        // the static layer was just rebuilt (its change is not in `*host.draw`). This
        // is the present *trigger* (R11): the frame loop decides *whether* to present
        // and then issues a present request to the display service, which owns *how*
        // (delta vs first-frame full build) and the compose/flush itself (GFX-0005).
        // Previous immediate command-list length, captured before the present swap below
        // overwrites `previous_draw`. This is the count the delta diff compares against
        // (`command_count_changed = previous.len != current.len`), so it is what the
        // CommandCountShift correlation line reports as `prev_cmds` (GFX-0006C pre-work).
        let prev_immediate_len = previous_draw.len;
        // BUG-GFX-0012 one-shot latch: force the present *after* a static rebuild to be a full
        // repaint (`force_full_present`), and carry the latch bit to the next present. The
        // latch itself forces a present too, so a reveal frame that changed nothing else still
        // flushes the retained chrome to GRAM.
        // GFX-0013: only the fallback path arms the reveal latch. An identical
        // rebuild changed nothing on the panel, and a bounded rebuild's damage
        // covers every unmatched command's old∪new footprint, so this present
        // already puts the rebuilt content into GRAM — there is no hidden chrome
        // for a later frame to reveal (the overdraw case is excluded by the
        // hazard guard above and takes the latch).
        let (force_full_present, next_static_reveal_latch) =
            koto_gfx::force_full_repaint_after_static_rebuild(full_rebuild, static_reveal_latched);
        if !has_previous_draw
            || *host.draw != *previous_draw
            || full_rebuild
            || static_damage_len > 0
            || force_full_present
        {
            let request = PresentRequest {
                current: host.draw,
                previous: previous_draw,
                static_layer: host.static_layer,
                heap: &heap[..heap_len],
                has_previous: has_previous_draw,
                force_full: force_full_present,
                static_rebuild_full: full_rebuild,
                static_damage: &static_damage[..static_damage_len],
            };
            let result = display_service
                .present(request, lcd, font, raster_strip, rgb666_strip, &mut metrics)
                .await;
            if result.is_err() {
                uart_log(uart, "phase=258 app-draw-error\r\n");
                host.close_package_stream();
                return;
            }
            *previous_draw = *host.draw;
            has_previous_draw = true;
            // Arm the latch for the next present iff this frame rebuilt the static layer.
            static_reveal_latched = next_static_reveal_latch;
        }
        // GFX-0013: refresh the shadow to the static layer as now applied. Its
        // content only changes on a rebuild frame (begin/push during the VM step),
        // and the session baseline is the first successful present (the shadow was
        // invalidated at session start). Present errors return above, so a capture
        // here always reflects pixels that reached GRAM.
        if static_rebuilt || !static_shadow.is_valid() {
            static_shadow.capture(
                &host.static_layer.commands[..host.static_layer.len],
                320,
                320,
            );
        }
        // Whole-frame work time (input + VM + paint), captured before the frame-
        // pacing sleep so the fps estimate reflects achievable throughput, not the
        // deliberate 16 ms cadence (KOTO-0131).
        let work_us = frame_started.elapsed().as_micros();
        frame = frame.wrapping_add(1);
        // First-frame draw-pattern audit (KOTO-0131): dump a bounded sample of the
        // command list (geometry + clipped rects, full-screen-base flag) for the
        // first few frames so the static-clear / tile / text mix is visible.
        // verbose-only (DIAG-0001 §2): the first-frame command dump is bring-up
        // firehose, off under Perf and back under Verbose. `phase=160` already carries
        // the per-frame headline a perf run needs.
        if DIAG_PROFILE.enables(DiagClass::Verbose) && frame <= 3 {
            log_command_sample(uart, host.draw, host.static_layer, frame);
        }
        // always-on (DIAG-0001 §2, deliberately not profile-gated): sparse in-app
        // stack sample (every 600 frames ≈ 10 s) so a long session's deepest VM /
        // present frames land a `phase=176` observation while the app still runs.
        // The scan is a linear read of the untouched gap (sub-millisecond); the
        // canary is the permanent tripwire for future `.bss` growth (KOTO-0170).
        if frame.is_multiple_of(600) {
            stack_canary::emit_peak(uart, "app");
        }
        // verbose-only (DIAG-0001 §2): the heartbeat is redundant with `phase=160`
        // (which already carries frame/fps/fuel-adjacent state), so it is off under
        // Perf and returns under Verbose.
        if DIAG_PROFILE.enables(DiagClass::Verbose) && frame.is_multiple_of(60) {
            let mut line = LineBuffer::new();
            let _ = write!(
                line,
                "phase=154 app-heartbeat frame={} pc={} fuel={}\r\n",
                frame,
                session.pc(),
                session.last_frame_fuel()
            );
            uart_write_line(uart, &line);
        }
        // Surface the app's draw-command budget usage on the same throttled
        // cadence (first frame, then every 60), plus the first frame that hits
        // the cap, so KOTO-0129 hardware bring-up can read blit counts and spot
        // a budget overflow without per-frame spam.
        let draw_overflow = host.draw.is_full();
        // Always computed: the one-shot `phase=162` overflow notice below is
        // always-on (DIAG-0001 §2 event-only), so its first-overflow edge is too.
        let newly_overflowed = draw_overflow && !draw_overflow_logged;
        let counts = host.draw.command_counts();
        draw_peak = draw_peak.max(host.draw.len);
        if draw_overflow {
            overflow_count = overflow_count.saturating_add(1);
        }
        // verbose-only (DIAG-0001 §2): the periodic draw-usage line is redundant with
        // `phase=160`'s peak/ovf fields for a perf run, so it is off under Perf and
        // returns under Verbose. The overflow *event* (`phase=162`) stays always-on below.
        if DIAG_PROFILE.enables(DiagClass::Verbose)
            && (frame == 1 || frame.is_multiple_of(60) || newly_overflowed)
        {
            let mut line = LineBuffer::new();
            log_app_draw_usage(
                uart,
                &mut line,
                frame,
                host.draw.len,
                MAX_APP_DRAW_COMMANDS,
                counts.rect,
                counts.text,
                counts.pixels,
                draw_overflow,
            );
        }
        // always-on (DIAG-0001 §2 event-only): a dropped-tail overflow is a discrete
        // fault worth surfacing under every profile, so this is not profile-gated.
        if newly_overflowed {
            // Dedicated one-shot drop notice (KOTO-0134): the cap was hit and tail
            // commands were dropped this frame. Latched so a sustained overflow
            // does not flood UART; the running total rides `phase=160` as `ovf=`.
            let mut line = LineBuffer::new();
            log_app_draw_overflow(
                uart,
                &mut line,
                frame,
                host.draw.len,
                MAX_APP_DRAW_COMMANDS,
                draw_peak,
            );
            draw_overflow_logged = true;
        }
        // Observe-only immediate-overlay budget (GFX-0006B observe mode): dry-run this
        // frame's finished immediate command list through `APP_DRAW_BUDGET` to record
        // what the budget *would* admit/degrade/reject, classifying each command with
        // the generic, app-agnostic `koto_gfx::classify_command`. Nothing is gated —
        // every command was already drawn this frame — so visuals are unchanged; this
        // only measures pressure. Budget enforcement is NOT enabled (see GFX-0006).
        let observation = BudgetObservation::observe(
            &APP_DRAW_BUDGET,
            host.draw.commands[..host.draw.len].iter(),
        );
        let newly_pressured = observation.has_pressure() && !budget_pressure_logged;
        // Perf-default, but thinned (DIAG-0001): the observe-only budget verdict emits on
        // frame 1 and the first frame pressure appears (`newly_pressured`), which already
        // catches the onset — the every-cadence sample is dropped under Perf and returns
        // only under Verbose, so a Verbose build reproduces today's frame-1 + every-C +
        // on-pressure emit set as an A/B baseline.
        if frame == 1
            || newly_pressured
            || (DIAG_PROFILE.enables(DiagClass::Verbose)
                && frame.is_multiple_of(DIAG_PROFILE.sample_period()))
        {
            let mut line = LineBuffer::new();
            log_app_budget_observation(uart, &mut line, package.app_id(), frame, &observation);
            // Latch only the off-cadence first-pressure emit, so sustained pressure
            // rides the periodic sample rather than spamming a line every frame.
            if newly_pressured {
                budget_pressure_logged = true;
            }
        }
        // CommandCountShift edit-region diagnostics (GFX-0011 Stage 1): when this frame's
        // full repaint was attributed to a shift in the immediate command count, report the
        // aligned diff's edit-region *shape* (`phase=169`) and the real coalesce-before-decide
        // contrast (`phase=174`) — two short lines rather than one over-long line the hardware
        // UART truncated. Post-Stage-1 the wide region is collected on the live path, so these
        // lines classify why the frame stayed a full repaint (truncated / wide-area); a rescued
        // count shift is incremental and reported on `phase=171`. Budget/class correlation lives
        // on `phase=168` (GFX-0008 established these frames are not a budget population).
        // Low-volume: caller-gated to CommandCountShift frames, one-shot plus throttled sample.
        let is_cmd_shift =
            metrics.full_repaint_reason() == Some(FullRepaintReason::CommandCountShift);
        let newly_cmd_shift = is_cmd_shift && !cmdshift_corr_logged;
        // gfx-debug (DIAG-0001): the count-shift shape (`phase=169`) + coalesce probe
        // (`phase=174`) only matter while investigating a repaint decision, so they are
        // off under Perf and return under Gfx / Verbose.
        if DIAG_PROFILE.enables(DiagClass::Gfx)
            && is_cmd_shift
            && (newly_cmd_shift || frame.is_multiple_of(DIAG_PROFILE.sample_period()))
        {
            let shift = metrics.command_shift();
            let mut line = LineBuffer::new();
            log_app_cmdshift_correlation(
                uart,
                &mut line,
                package.app_id(),
                frame,
                prev_immediate_len,
                host.draw.len,
                shift,
            );
            // The coalesce-contrast measurements on a separate sparse line so neither truncates.
            if let Some((_, pressure)) = shift {
                log_app_cmdshift_probe(uart, &mut line, package.app_id(), frame, &pressure);
            }
            if newly_cmd_shift {
                cmdshift_corr_logged = true;
            }
        }
        // Coalesce-before-decide contrast (GFX-0010 Stage 1B): the present path now
        // batch-coalesces the full raw dirty set *before* the policy decides, so a
        // coalescible frame stays incremental instead of escalating on the raw rect count.
        // It records a coalesce-pressure contrast when the post-coalesce decision is
        // interesting — a surviving RectsExceeded/AreaExceeded full repaint, or a frame the
        // reorder converted back to incremental. Here we only emit it. Low-volume:
        // one-shot on the first recorded frame plus the throttled sample.
        let has_coalesce_pressure = metrics.coalesce_pressure().is_some();
        let newly_coalesce_pressure = has_coalesce_pressure && !coalesce_pressure_logged;
        // gfx-debug (DIAG-0001): the coalesce-before-decide contrast (`phase=171`) is a
        // repaint-decision investigation line — off under Perf, back under Gfx / Verbose.
        if DIAG_PROFILE.enables(DiagClass::Gfx)
            && has_coalesce_pressure
            && (newly_coalesce_pressure || frame.is_multiple_of(DIAG_PROFILE.sample_period()))
        {
            if let Some(pressure) = metrics.coalesce_pressure() {
                let mut line = LineBuffer::new();
                log_app_coalesce_pressure(uart, &mut line, package.app_id(), frame, &pressure);
            }
            if newly_coalesce_pressure {
                coalesce_pressure_logged = true;
            }
        }
        // Static-rebuild notice (GFX-0009 Stage-0, observe-only): surface a frame that
        // rebuilt the Game2D static layer so a hardware run can confirm the rebuild is
        // one-shot per gameplay entry (the title->gameplay transition) rather than
        // recurring. The first paint (frame 1) is suppressed — it is attributed
        // StaticRebuild for a different reason (no previous frame) and is not the
        // recurring-rebuild signal. Low-volume: one-shot on the first mid-session
        // rebuild plus the throttled sample, so even a buggy every-frame rebuild is
        // visible without flooding UART. The running total rides `phase=160` as
        // `static_rebuilds=`.
        let mid_session_rebuild = static_rebuilt && frame > 1;
        let newly_static_rebuild = mid_session_rebuild && !static_rebuild_logged;
        // `phase=170` is event-only (a mid-session rebuild is rare and latched one-shot),
        // so Stage 1 leaves it emitting under every profile; only its cadence source moves
        // off the PSRAM feature onto `sample_period()` (DIAG-0001). Class-gating it is a
        // later step, out of Stage 1 scope.
        if mid_session_rebuild
            && (newly_static_rebuild || frame.is_multiple_of(DIAG_PROFILE.sample_period()))
        {
            let mut line = LineBuffer::new();
            log_app_static_rebuild(
                uart,
                &mut line,
                package.app_id(),
                frame,
                static_rebuilds,
                static_cmds,
                static_align,
                static_would_rects,
                static_would_px,
                if identical_rebuild {
                    "skip"
                } else if bounded_rebuild {
                    "bounded"
                } else {
                    "full"
                },
            );
            if newly_static_rebuild {
                static_rebuild_logged = true;
            }
        }
        // Per-frame render-performance line on a tighter cadence than the heartbeat
        // (first frame, then every `sample_period()`) so a hardware run yields regular
        // samples to triage the slowdown — VM vs raster vs transfer, dirty pixels/rects,
        // host calls, and whether the delta fell back to a full repaint (KOTO-0131).
        // `phase=160` is the regression detector and stays enabled under Perf; only the
        // per-cadence *companion* lines below are class-gated (DIAG-0001).
        if on_cadence(frame) {
            let mut line = LineBuffer::new();
            log_app_frame_metrics(
                uart,
                &mut line,
                package.app_id(),
                frame,
                vm_us,
                metrics,
                host_calls,
                counts.rect,
                counts.text,
                counts.pixels,
                draw_peak,
                overflow_count,
                code_refills,
                code_tiles,
                static_cmds,
                static_rebuilt,
                static_rebuilds,
                work_us,
            );
            // Steady-frame vm_us attribution inputs (KOTO-0169 Stage 0), on the
            // same cadence as `phase=160` but as its own sparse line so the
            // headline's bytes stay identical: executed instructions (`ops=`,
            // Stage 0a), hostcall wall time (`host_us=`, Stage 0b), and the
            // already-metered refill cost, all read from this frame's step.
            log_app_vm_cost(
                uart,
                &mut line,
                package.app_id(),
                frame,
                vm_us,
                session.last_frame_fuel(),
                host.last_frame_host_us(),
                cw_refill_us,
                code_refills,
            );
            // Present-cost breakdown (KOTO-0174 Stage 0): split `raster_us` into
            // the base-clear pass and the command-stack composite so a device run
            // confirms the host attribution (clear is a minority; per-command
            // glyph/fill paint is the bulk). gfx-debug only — an investigation
            // line, off under Perf/Audio like the other present diagnostics.
            if DIAG_PROFILE.enables(DiagClass::Gfx) {
                log_app_present_cost(
                    uart,
                    &mut line,
                    package.app_id(),
                    frame,
                    metrics,
                    counts.rect,
                    counts.text,
                    counts.pixels,
                    static_cmds,
                );
            }
            // Dirty-rect fragmentation geometry (KOTO-0159) on the same throttled
            // cadence, only when the incremental delta path ran (a full-repaint or
            // idle frame leaves the geometry zero). This is the line that confirms
            // on hardware whether a slow event frame is per-rect raster overhead
            // (many scattered rects) rather than transfer area.
            // Gated on `!full_repaint()` as well as `rects_pre > 0`: the full-repaint
            // path now records its pre-coalesce fragmentation for the `phase=169`
            // correlation line (above), so this incremental-only line keeps its prior
            // behaviour and does not start firing on full-repaint frames.
            // gfx-debug (DIAG-0001): off under Perf, back under Gfx / Verbose. This
            // replaces the old `psram_qpi_code_window_prod_profile` cfg gate so verbosity
            // no longer rides PSRAM backend selection.
            if DIAG_PROFILE.enables(DiagClass::Gfx)
                && !metrics.full_repaint()
                && metrics.dirty_geometry().rects_pre > 0
            {
                log_dirty_rect_geometry(
                    uart,
                    &mut line,
                    package.app_id(),
                    frame,
                    metrics.dirty_geometry(),
                );
            }
            // Code-window refill histogram + top transitions (KOTO-0136 triage):
            // classify a high `refills=` as a hot-tile ping-pong vs a many-tile walk.
            // codewindow-debug (DIAG-0001): the headline `refills=`/`code_tiles=` already
            // ride `phase=160`, so this detail line is off under Perf and returns under
            // CodeWindow / Verbose.
            if DIAG_PROFILE.enables(DiagClass::CodeWindow) {
                log_code_window_fetch(
                    uart,
                    &mut line,
                    frame,
                    code_refills,
                    code_tiles,
                    cw_refill_us,
                    cw_refill_max_us,
                    cw_bytes,
                    code_window_read_mode_label(),
                    code_window_read_chunk_bytes(),
                    code_window_read_sm_hz(),
                    PSRAM_FAST_READ_DUMMY_CYCLES,
                    code.tile_refills(),
                    code.tile_transitions(),
                    code_window_log_tag(),
                );
            }
            // Opt-in fast CodeWindow refill summary (KOTO-0153): whether the last
            // refill used the validated FastFallingClkdiv2 read, plus running
            // fast/fallback counts and the last fallback reason.
            // codewindow-debug (DIAG-0001): fast-read success/fallback counters — a
            // strict add-on to `phase=163`, off under Perf, back under CodeWindow /
            // Verbose. The `psram_fast_code_window` feature still gates the read path
            // itself; the class gate only decides whether the summary transmits.
            #[cfg(feature = "psram_fast_code_window")]
            if DIAG_PROFILE.enables(DiagClass::CodeWindow) {
                let fast = koto_psram_fast_code_window_snapshot();
                line.clear();
                let _ = write!(
                    line,
                    "phase=167 cw-fast-clkdiv2 frame={} read_mode={} cw_tag=koto_psram chunk={} fast_success_count={} fast_fallback_count={} fallback_reason={}\r\n",
                    frame,
                    code_window_read_mode_label(),
                    code_window_read_chunk_bytes(),
                    fast.fast_success_count,
                    fast.fast_fallback_count,
                    fast.last_fallback_reason,
                );
                uart_write_line(uart, &line);
            }
            #[cfg(feature = "psram_qpi_code_window_counters")]
            if code_refills > 0 {
                code.log_qpi_code_window_verify(
                    volume_mgr,
                    package,
                    header,
                    staged_code_base_addr,
                    frame,
                    &mut cw_counters,
                    uart,
                );
            }
            #[cfg(all(
                feature = "psram_dma_read_code_window",
                feature = "psram_dma_read_code_window_diag"
            ))]
            {
                let trace = dma_code_window_read_trace_snapshot();
                if trace.dma_fallbacks > last_dma_fallbacks {
                    let mut line = LineBuffer::new();
                    let _ = write!(
                        line,
                        "phase=334 cw-dma-read-fallback app={} dma_fallbacks={} dma_error_code={} addr={} len={}\r\n",
                        package.app_id(),
                        trace.dma_fallbacks,
                        trace.last_dma_error,
                        trace.last_addr,
                        trace.last_len
                    );
                    uart_write_line(uart, &line);
                    last_dma_fallbacks = trace.dma_fallbacks;
                }
            }
            #[cfg(all(
                feature = "psram_dma_read_code_window",
                feature = "psram_dma_read_code_window_diag"
            ))]
            {
                let app_id = package.app_id();
                if frame == 1 || frame.is_multiple_of(120) {
                    let trace = dma_code_window_read_trace_snapshot();
                    let cw_effective_milli_mb_s = if cw_refill_us == 0 {
                        0u64
                    } else {
                        (cw_bytes as u64)
                            .saturating_mul(1_000_000)
                            .saturating_mul(1000)
                            .saturating_div(cw_refill_us as u64)
                            .saturating_div(1_000_000)
                    };
                    line.clear();
                    let _ = write!(
                        line,
                        "phase=333 cw-dma-read-experiment app={} frame={} cw_refill_us={} cw_bytes={} cw_effective_mb_s={}.{:03} dma_successes={} dma_fallbacks={} gameplay_result={}\r\n",
                        app_id,
                        frame,
                        cw_refill_us,
                        cw_bytes,
                        cw_effective_milli_mb_s / 1000,
                        cw_effective_milli_mb_s % 1000,
                        trace.dma_successes,
                        trace.dma_fallbacks,
                        if app_id.contains("blocks") { "running" } else { "n/a" }
                    );
                    uart_write_line(uart, &line);
                    line.clear();
                }
            }
            // audio-debug (DIAG-0001): the 17-field aggregate audio summary is only
            // meaningful while investigating drops/underruns — off under Perf, back under
            // Audio / Verbose. Replaces the old `psram_qpi_code_window_prod_profile` cfg
            // gate so audio verbosity no longer rides PSRAM backend selection.
            if DIAG_PROFILE.enables(DiagClass::Audio) {
                let audio_stats = host.audio_stats();
                line.clear();
                let _ = write!(
                    line,
                    "phase=173 audio-summary frame={} audio_events={} samples_submitted={} samples_played={} drops={} underruns={} unsupported_count={} buffer_level={} buffer_capacity={} command_drops={} bgm_starts={} bgm_stops={} bgm_voices={} sfx_voices={} mixer_saturations={} worker_late={} worker_max_jitter_us={} worker_heartbeat={} core1_stack_free_min={}\r\n",
                    frame,
                    audio_stats.audio_events,
                    audio_stats.samples_submitted,
                    audio_stats.samples_played,
                    audio_stats.drops,
                    audio_stats.underruns,
                    audio_stats.unsupported_count,
                    audio_stats.buffer_level,
                    audio_stats.buffer_capacity,
                    audio_stats.command_drops,
                    audio_stats.bgm_starts,
                    audio_stats.bgm_stops,
                    audio_stats.active_bgm_voices,
                    audio_stats.active_sfx_voices,
                    audio_stats.mixer_saturations,
                    audio_stats.worker_late,
                    audio_stats.worker_max_jitter_us,
                    audio_stats.worker_heartbeat,
                    audio_stats.core1_stack_free_min,
                );
                uart_write_line(uart, &line);
            }
        }
        let elapsed = frame_started.elapsed().as_millis();
        if elapsed < FRAME_PERIOD_MS {
            let sleep_budget_us = (FRAME_PERIOD_MS - elapsed).saturating_mul(1000);
            let sleep_started = Instant::now();
            while sleep_started.elapsed().as_micros() < sleep_budget_us {
                host.service_audio();
                Timer::after_micros(125).await;
            }
        }
    }
}

#[cfg(any(
    all(
        feature = "psram_dma_read_code_window",
        feature = "psram_dma_read_code_window_diag"
    ),
    feature = "psram_qpi_code_window_counters"
))]
fn first_mismatch(expected: &[u8], actual: &[u8]) -> Option<(usize, u8, u8)> {
    expected
        .iter()
        .zip(actual.iter())
        .position(|(e, a)| e != a)
        .map(|off| (off, expected[off], actual[off]))
}

#[cfg(any(
    all(
        feature = "psram_dma_read_code_window",
        feature = "psram_dma_read_code_window_diag"
    ),
    feature = "psram_qpi_code_window_counters"
))]
#[cfg_attr(
    all(
        feature = "psram_qpi_code_window_counters",
        not(feature = "psram_qpi_code_window_verbose")
    ),
    allow(dead_code)
)]
fn write_hex_bytes(line: &mut LineBuffer, bytes: &[u8]) {
    for byte in bytes {
        let _ = write!(line, "{:02x}", byte);
    }
}

#[cfg(feature = "psram_qpi_code_window_counters")]
fn copy_around_fail(dst: &mut [u8; 16], bytes: &[u8], fail_off: usize) {
    dst.fill(0);
    if bytes.is_empty() {
        return;
    }
    let start = fail_off
        .saturating_sub(8)
        .min(bytes.len().saturating_sub(1));
    let end = (start + dst.len()).min(bytes.len());
    let len = end.saturating_sub(start);
    dst[..len].copy_from_slice(&bytes[start..end]);
}

fn align_psram_address(address: u32) -> u32 {
    address.saturating_add(255) & !255
}

fn audio_asset_key(path: &str, bgm: bool) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in path.bytes().chain(core::iter::once(u8::from(bgm))) {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn service_pending_audio_asset<C, D>(
    host: &mut DeviceHost<'_, D>,
    code: &mut C,
    cache: &mut [AudioCueCacheEntry; AUDIO_CUE_CACHE_ENTRIES],
    next_address: &mut u32,
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
) where
    C: CodeWindowVerifyExt,
    D: BlockDevice,
    D::Error: core::fmt::Debug,
{
    let Some(request) = host.take_audio_asset_request() else {
        return;
    };
    if request.path().ends_with(".kacl") {
        match host.start_streaming_audio_asset(request.path(), request.bgm) {
            Ok(true) => {
                log_audio_asset_stage(uart, "streaming", request.path());
                return;
            }
            Ok(false) => {}
            Err(_) => {
                host.audio_asset_diag("stream-start", request.path());
                return;
            }
        }
    }
    let key = audio_asset_key(request.path(), request.bgm);
    // Safety: this routine runs only on CPU0, once between VM frames. CPU1
    // receives a copy through AudioShared and never touches this scratch.
    let scratch = unsafe { &mut *AUDIO_LOAD_SCRATCH.0.get() };

    if let Some(entry) = cache.iter().find(|entry| entry.used && entry.key == key) {
        let len = usize::from(entry.len);
        log_audio_asset_stage(uart, "cache-read", request.path());
        if code.asset_read(entry.address, &mut scratch.image[..len])
            && host.play_loaded_audio_image(&scratch.image[..len], entry.bgm)
        {
            log_audio_asset_stage(uart, "queued", request.path());
            return;
        }
        host.audio_asset_diag("psram-read", request.path());
        return;
    }

    let image_len = match host.read_audio_asset(request.path(), &mut scratch.image) {
        Ok(len) if len <= scratch.image.len() => len,
        _ => {
            host.audio_asset_diag("sd-read", request.path());
            return;
        }
    };
    let image = &mut scratch.image;
    let magic = image.get(..4);
    if image_len < 12 || !matches!(magic, Some(b"KAQ1") | Some(b"KACL")) {
        host.audio_asset_diag("koto-audio-image", request.path());
        return;
    }
    let Some(capacity) = code.asset_capacity() else {
        host.audio_asset_diag("no-psram", request.path());
        return;
    };
    let end = next_address.saturating_add(image_len as u32);
    log_audio_asset_stage(uart, "psram-write", request.path());
    if end > capacity || !code.asset_write(*next_address, &image[..image_len]) {
        host.audio_asset_diag("psram-write", request.path());
        return;
    }
    let Some(slot) = cache.iter_mut().find(|entry| !entry.used) else {
        host.audio_asset_diag("cache-full", request.path());
        return;
    };
    *slot = AudioCueCacheEntry {
        key,
        address: *next_address,
        len: image_len as u16,
        bgm: request.bgm,
        used: true,
    };
    *next_address = align_psram_address(end);

    // Read back through the same copy-only PSRAM API used on cache hits. This
    // makes PSRAM, not the compile scratch, the authoritative playback image.
    log_audio_asset_stage(uart, "psram-read", request.path());
    if !code.asset_read(slot.address, &mut image[..image_len])
        || !host.play_loaded_audio_image(&image[..image_len], request.bgm)
    {
        host.audio_asset_diag("audio-queue", request.path());
    } else {
        log_audio_asset_stage(uart, "queued", request.path());
    }
}

fn log_audio_asset_stage(
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
    stage: &str,
    path: &str,
) {
    let mut line = LineBuffer::new();
    let _ = write!(
        line,
        "phase=158 audio-runtime-stage stage={} path={}\r\n",
        stage, path
    );
    uart_write_line(uart, &line);
}

trait CodeWindowVerifyExt {
    fn asset_capacity(&self) -> Option<u32> {
        None
    }

    fn asset_read(&mut self, _address: u32, _dst: &mut [u8]) -> bool {
        false
    }

    fn asset_write(&mut self, _address: u32, _src: &[u8]) -> bool {
        false
    }

    #[cfg_attr(not(feature = "psram_qpi_code_window_counters"), allow(dead_code))]
    fn log_qpi_code_window_verify<D: BlockDevice>(
        &mut self,
        volume_mgr: &VolumeManager<D, FirmwareClock>,
        package: &PackageInfo,
        header: &[u8],
        staged_code_base_addr: u32,
        frame: u32,
        counters: &mut QpiCodeWindowCounters,
        uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
    ) where
        D::Error: core::fmt::Debug,
    {
        let _ = (
            volume_mgr,
            package,
            header,
            staged_code_base_addr,
            frame,
            counters,
            uart,
        );
    }
}

impl CodeWindowVerifyExt for SliceCode<'_> {}

impl<'a, 'b> CodeWindowVerifyExt for PsramCodeWindow<'a, FirmwarePsramHal<'b>> {
    fn asset_capacity(&self) -> Option<u32> {
        Some(self.psram_capacity())
    }

    fn asset_read(&mut self, address: u32, dst: &mut [u8]) -> bool {
        self.psram_read(address, dst).is_ok()
    }

    fn asset_write(&mut self, address: u32, src: &[u8]) -> bool {
        self.psram_write(address, src).is_ok()
    }

    #[cfg_attr(
        all(
            feature = "psram_qpi_code_window_counters",
            not(feature = "psram_qpi_code_window_verbose")
        ),
        allow(unused_variables, unused_assignments)
    )]
    fn log_qpi_code_window_verify<D: BlockDevice>(
        &mut self,
        volume_mgr: &VolumeManager<D, FirmwareClock>,
        package: &PackageInfo,
        header: &[u8],
        staged_code_base_addr: u32,
        frame: u32,
        counters: &mut QpiCodeWindowCounters,
        uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
    ) where
        D::Error: core::fmt::Debug,
    {
        #[cfg(not(feature = "psram_qpi_code_window_counters"))]
        {
            let _ = (
                volume_mgr,
                package,
                header,
                staged_code_base_addr,
                frame,
                counters,
                uart,
            );
            return;
        }
        #[cfg(feature = "psram_qpi_code_window_counters")]
        {
            let Some(parsed) = KbcHeader::parse(header).ok() else {
                return;
            };
            let Some(entry) = package.entry() else {
                return;
            };
            let Some(file_name) = entry.rsplit('/').next() else {
                return;
            };
            let state: PsramCodeWindowDebugState = self.debug_state();
            let window_len = self.current_window_bytes().len();
            if window_len == 0 || state.window_capacity_words == 0 {
                return;
            }

            let Ok(volume) = volume_mgr.open_volume(VolumeIdx(0)) else {
                return;
            };
            let Ok(root) = volume.open_root_dir() else {
                return;
            };
            let Ok(bytecode_dir) = root.open_dir("BYTECODE") else {
                return;
            };
            let mut short = None;
            let short_target = ShortFileName::create_from_str(file_name).ok();
            let mut lfn_storage = [0u8; MANIFEST_LFN_BYTES];
            let mut lfn = LfnBuffer::new(&mut lfn_storage);
            if bytecode_dir
                .iterate_dir_lfn(&mut lfn, |entry, long_name| {
                    if short.is_none()
                        && !entry.attributes.is_directory()
                        && (long_name.is_some_and(|name| name.eq_ignore_ascii_case(file_name))
                            || short_target.as_ref() == Some(&entry.name))
                    {
                        short = Some(entry.name.clone());
                    }
                })
                .is_err()
            {
                return;
            }
            let Some(short) = short else {
                return;
            };
            let Ok(file) = bytecode_dir.open_file_in_dir(&short, Mode::ReadOnly) else {
                return;
            };

            let psram_addr = state
                .base_addr
                .wrapping_add(state.window_base_word.saturating_mul(4));
            let app_code_off = psram_addr.saturating_sub(staged_code_base_addr) as usize;
            let tile = state.window_base_word / state.window_capacity_words;

            #[cfg(feature = "psram_qpi_backend_v2")]
            {
                let mut qpi_first16 = [0u8; 16];
                let mut qpi_cmp_ok = true;
                let mut qpi_read_ok = true;
                let mut qpi_fail = None;
                let mut qpi_around_refill = [0u8; 16];
                let mut qpi_around_verify = [0u8; 16];
                let mut map_ok = true;
                let mut map_read_ok = true;
                let mut map_fail = None;
                let mut expected_around = [0u8; 16];
                let mut current_around = [0u8; 16];
                let mut qpi_chunk = [0u8; PSRAM_QPI_SAFE_READ_CHUNK_BYTES];
                let mut source_chunk = [0u8; PSRAM_QPI_SAFE_READ_CHUNK_BYTES];
                let file = file;
                if file
                    .seek_from_start((parsed.code_offset as usize + app_code_off) as u32)
                    .is_err()
                {
                    map_read_ok = false;
                }

                let mut offset = 0usize;
                while offset < window_len {
                    let len = (window_len - offset).min(PSRAM_QPI_SAFE_READ_CHUNK_BYTES);
                    let qpi_res = {
                        let backend: &mut QpiCodeWindowPsram<'_> = self.psram_mut().backend_mut();
                        backend
                            .read_qpi_for_verify(psram_addr + offset as u32, &mut qpi_chunk[..len])
                    };
                    if qpi_res.is_err() {
                        qpi_read_ok = false;
                        qpi_cmp_ok = false;
                        qpi_fail = Some((offset, 0, 0));
                        break;
                    }

                    if offset < 16 {
                        let copy = (16 - offset).min(len);
                        qpi_first16[offset..offset + copy].copy_from_slice(&qpi_chunk[..copy]);
                    }

                    let refill_chunk = &self.current_window_bytes()[offset..offset + len];
                    if qpi_fail.is_none() {
                        if let Some((local_off, exp, got)) =
                            first_mismatch(&qpi_chunk[..len], refill_chunk)
                        {
                            qpi_cmp_ok = false;
                            qpi_fail = Some((offset + local_off, exp, got));
                            copy_around_fail(&mut qpi_around_verify, &qpi_chunk[..len], local_off);
                            copy_around_fail(&mut qpi_around_refill, refill_chunk, local_off);
                        }
                    }

                    if map_read_ok {
                        let mut got = 0usize;
                        while got < len {
                            match file.read(&mut source_chunk[got..len]) {
                                Ok(0) => break,
                                Ok(count) => got += count,
                                Err(_) => {
                                    map_read_ok = false;
                                    map_ok = false;
                                    break;
                                }
                            }
                        }
                        if got != len {
                            map_read_ok = false;
                            map_ok = false;
                            if map_fail.is_none() {
                                map_fail = Some((offset + got, 0, 0));
                            }
                        } else if map_fail.is_none() {
                            if let Some((local_off, exp, got)) =
                                first_mismatch(&source_chunk[..len], refill_chunk)
                            {
                                map_ok = false;
                                map_fail = Some((offset + local_off, exp, got));
                                copy_around_fail(
                                    &mut expected_around,
                                    &source_chunk[..len],
                                    local_off,
                                );
                                copy_around_fail(&mut current_around, refill_chunk, local_off);
                            }
                        }
                    }
                    offset += len;
                }

                counters.record_verify(frame, tile, qpi_fail);
                counters.record_map(frame, tile, map_fail);

                #[cfg(feature = "psram_qpi_code_window_verbose")]
                {
                    let mut line = LineBuffer::new();
                    let (qpi_fail_off, qpi_exp, qpi_got) = qpi_fail
                        .map(|(o, e, g)| (o as i32, e, g))
                        .unwrap_or((-1, 0, 0));
                    let _ = write!(
                line,
                "phase=164 cw-verify frame={} tile={} psram_addr=0x{:08x} len={} read_mode=qpi_v2_r8 chunk={} cw_tag={} ok={} fail_off={} fail_chunk_index={} fail_chunk_off={} fail_exp=0x{:02x} fail_got=0x{:02x} first16=",
                frame,
                tile,
                psram_addr,
                window_len,
                code_window_read_chunk_bytes(),
                code_window_log_tag(),
                if qpi_read_ok && qpi_cmp_ok { 1 } else { 0 },
                qpi_fail_off,
                if qpi_fail_off >= 0 {
                    qpi_fail_off as usize / code_window_read_chunk_bytes()
                } else {
                    0
                },
                if qpi_fail_off >= 0 {
                    qpi_fail_off as usize % code_window_read_chunk_bytes()
                } else {
                    0
                },
                qpi_exp,
                qpi_got,
            );
                    write_hex_bytes(&mut line, &qpi_first16);
                    let _ = write!(line, " around_fail_current=");
                    write_hex_bytes(&mut line, &qpi_around_refill);
                    let _ = write!(line, " around_fail_verify=");
                    write_hex_bytes(&mut line, &qpi_around_verify);
                    let _ = write!(line, "\r\n");
                    uart_write_line(uart, &line);

                    line.clear();
                    let (map_fail_off, map_exp, map_got) = map_fail
                        .map(|(o, e, g)| (o as i32, e, g))
                        .unwrap_or((-1, 0, 0));
                    let _ = write!(
                line,
                "phase=165 cw-map-verify tile={} vm_pc_base={} app_code_off={} psram_addr=0x{:08x} len={} ok={} fail_off={} expected_source=app_bytecode fail_exp=0x{:02x} fail_got=0x{:02x} around_fail_expected=",
                tile,
                state.window_base_word,
                app_code_off,
                psram_addr,
                window_len,
                if map_read_ok && map_ok { 1 } else { 0 },
                map_fail_off,
                map_exp,
                map_got,
            );
                    write_hex_bytes(&mut line, &expected_around);
                    let _ = write!(line, " around_fail_current=");
                    write_hex_bytes(&mut line, &current_around);
                    let _ = write!(line, "\r\n");
                    uart_write_line(uart, &line);
                }
                return;
            }

            #[cfg(not(feature = "psram_qpi_backend_v2"))]
            {
                let mut serial_first16 = [0u8; 16];
                let mut qpi_first16 = [0u8; 16];
                let mut serial_cmp_ok = true;
                let mut serial_read_ok = true;
                let mut serial_fail = None;
                let mut serial_around_qpi = [0u8; 16];
                let mut serial_around_ref = [0u8; 16];
                let mut map_ok = true;
                let mut map_read_ok = true;
                let mut map_fail = None;
                let mut expected_around = [0u8; 16];
                let mut current_around = [0u8; 16];
                let mut serial_chunk = [0u8; PSRAM_PROD_READ_CHUNK_BYTES];
                let mut source_chunk = [0u8; PSRAM_PROD_READ_CHUNK_BYTES];
                let file = file;
                if file
                    .seek_from_start((parsed.code_offset as usize + app_code_off) as u32)
                    .is_err()
                {
                    map_read_ok = false;
                }

                let mut offset = 0usize;
                while offset < window_len {
                    let len = (window_len - offset).min(PSRAM_PROD_READ_CHUNK_BYTES);
                    let serial_res = {
                        let backend: &mut QpiCodeWindowPsram<'_> = self.psram_mut().backend_mut();
                        backend.read_serial_for_verify(
                            psram_addr + offset as u32,
                            &mut serial_chunk[..len],
                        )
                    };
                    if serial_res.is_err() {
                        serial_read_ok = false;
                        serial_cmp_ok = false;
                        serial_fail = Some((offset, 0, 0));
                        break;
                    }
                    if offset < 16 {
                        let copy = (16 - offset).min(len);
                        serial_first16[offset..offset + copy]
                            .copy_from_slice(&serial_chunk[..copy]);
                        qpi_first16[offset..offset + copy]
                            .copy_from_slice(&self.current_window_bytes()[offset..offset + copy]);
                    }
                    let qpi_chunk = &self.current_window_bytes()[offset..offset + len];
                    if serial_fail.is_none() {
                        if let Some((local_off, exp, got)) =
                            first_mismatch(&serial_chunk[..len], qpi_chunk)
                        {
                            serial_cmp_ok = false;
                            serial_fail = Some((offset + local_off, exp, got));
                            copy_around_fail(
                                &mut serial_around_ref,
                                &serial_chunk[..len],
                                local_off,
                            );
                            copy_around_fail(&mut serial_around_qpi, qpi_chunk, local_off);
                        }
                    }

                    if map_read_ok {
                        let mut got = 0usize;
                        while got < len {
                            match file.read(&mut source_chunk[got..len]) {
                                Ok(0) => break,
                                Ok(count) => got += count,
                                Err(_) => {
                                    map_read_ok = false;
                                    map_ok = false;
                                    break;
                                }
                            }
                        }
                        if got != len {
                            map_read_ok = false;
                            map_ok = false;
                            if map_fail.is_none() {
                                map_fail = Some((offset + got, 0, 0));
                            }
                        } else if map_fail.is_none() {
                            if let Some((local_off, exp, got)) =
                                first_mismatch(&source_chunk[..len], qpi_chunk)
                            {
                                map_ok = false;
                                map_fail = Some((offset + local_off, exp, got));
                                copy_around_fail(
                                    &mut expected_around,
                                    &source_chunk[..len],
                                    local_off,
                                );
                                copy_around_fail(&mut current_around, qpi_chunk, local_off);
                            }
                        }
                    }
                    offset += len;
                }

                counters.record_verify(frame, tile, serial_fail);
                counters.record_map(frame, tile, map_fail);

                #[cfg(feature = "psram_qpi_code_window_verbose")]
                {
                    let mut line = LineBuffer::new();
                    let (serial_fail_off, serial_exp, serial_got) = serial_fail
                        .map(|(o, e, g)| (o as i32, e, g))
                        .unwrap_or((-1, 0, 0));
                    let _ = write!(
            line,
            "phase=164 cw-verify frame={} tile={} psram_addr=0x{:08x} len={} qpi_read_mode={} serial_read_mode=serial_safe_pio_cpu chunk={} cw_tag={} ok={} fail_off={} fail_chunk_index={} fail_chunk_off={} fail_exp=0x{:02x} fail_got=0x{:02x} first16_qpi=",
            frame,
            tile,
            psram_addr,
            window_len,
            code_window_read_mode_label(),
            code_window_read_chunk_bytes(),
            code_window_log_tag(),
            if serial_read_ok && serial_cmp_ok { 1 } else { 0 },
            serial_fail_off,
            if serial_fail_off >= 0 {
                serial_fail_off as usize / code_window_read_chunk_bytes()
            } else {
                0
            },
            if serial_fail_off >= 0 {
                serial_fail_off as usize % code_window_read_chunk_bytes()
            } else {
                0
            },
            serial_exp,
            serial_got,
        );
                    write_hex_bytes(&mut line, &qpi_first16);
                    let _ = write!(line, " first16_serial=");
                    write_hex_bytes(&mut line, &serial_first16);
                    let _ = write!(line, " around_fail_qpi=");
                    write_hex_bytes(&mut line, &serial_around_qpi);
                    let _ = write!(line, " around_fail_serial=");
                    write_hex_bytes(&mut line, &serial_around_ref);
                    let _ = write!(line, "\r\n");
                    uart_write_line(uart, &line);

                    line.clear();
                    let (map_fail_off, map_exp, map_got) = map_fail
                        .map(|(o, e, g)| (o as i32, e, g))
                        .unwrap_or((-1, 0, 0));
                    let _ = write!(
            line,
            "phase=165 cw-map-verify tile={} vm_pc_base={} app_code_off={} psram_addr=0x{:08x} len={} ok={} fail_off={} expected_source=app_bytecode fail_exp=0x{:02x} fail_got=0x{:02x} around_fail_expected=",
            tile,
            state.window_base_word,
            app_code_off,
            psram_addr,
            window_len,
            if map_read_ok && map_ok { 1 } else { 0 },
            map_fail_off,
            map_exp,
            map_got,
        );
                    write_hex_bytes(&mut line, &expected_around);
                    let _ = write!(line, " around_fail_current=");
                    write_hex_bytes(&mut line, &current_around);
                    let _ = write!(line, "\r\n");
                    uart_write_line(uart, &line);
                }
            }
        }
    }
}

#[cfg(all(
    feature = "psram_dma_read_code_window",
    feature = "psram_dma_read_code_window_diag"
))]
fn trace_mode_label(mode: DmaReadTraceMode) -> &'static str {
    match mode {
        DmaReadTraceMode::None => "none",
        DmaReadTraceMode::Legacy => "legacy",
        DmaReadTraceMode::Dma => "dma",
        DmaReadTraceMode::DmaFallback => "dma_fallback",
        DmaReadTraceMode::PhaseEdgeFudge => "phase_edge_fudge",
    }
}

#[cfg(all(
    feature = "psram_dma_read_code_window",
    feature = "psram_dma_read_code_window_diag"
))]
fn log_launch_header_compare(
    psram: &mut PsramBlocks<FirmwarePsramHal<'_>>,
    code_window: &mut [u8; CODE_WINDOW_TOTAL_BYTES],
    source_first64: &[u8; 64],
    source_first64_len: usize,
    code_base_addr: u32,
    code_words: u32,
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
) {
    let compare_len = source_first64_len
        .min(64)
        .min((code_words as usize).saturating_mul(4));
    if compare_len == 0 {
        return;
    }

    let mut legacy = [0u8; 64];
    let mut dma = [0u8; 64];
    let mut cw = [0u8; 64];

    let (legacy_res, dma_res) = {
        let backend: &mut DmaCodeWindowPsram<'_> = psram.backend_mut();
        (
            backend.read_legacy_for_diag(code_base_addr, &mut legacy[..compare_len]),
            backend.read_dma_for_diag(code_base_addr, &mut dma[..compare_len]),
        )
    };

    let mut cw_ok = true;
    let cw_state = {
        let mut code = PsramCodeWindow::new(psram, code_window, code_base_addr, code_words);
        for word_index in 0..compare_len.div_ceil(4) {
            let Some(word) = code.word(word_index as u32) else {
                cw_ok = false;
                break;
            };
            let start = word_index * 4;
            let copy = (compare_len - start).min(4);
            cw[start..start + copy].copy_from_slice(&word[..copy]);
        }
        code.debug_state()
    };

    let mut line = LineBuffer::new();
    let expected = &source_first64[..compare_len];

    let _ = write!(
        line,
        "phase=334 launch-header-compare source=original len={} first64=",
        compare_len
    );
    write_hex_bytes(&mut line, expected);
    let _ = write!(line, "\r\n");
    uart_write_line(uart, &line);
    line.clear();

    match legacy_res {
        Ok(()) => {
            let mismatch = first_mismatch(expected, &legacy[..compare_len]);
            let _ = write!(
                line,
                "phase=334 launch-header-compare source=psram_legacy first64="
            );
            write_hex_bytes(&mut line, &legacy[..compare_len]);
            if let Some((off, exp, act)) = mismatch {
                let _ = write!(
                    line,
                    " mismatch_off={} expected={:02x} actual={:02x}\r\n",
                    off, exp, act
                );
            } else {
                let _ = write!(line, " mismatch_off=none\r\n");
            }
        }
        Err(err) => {
            let _ = write!(
                line,
                "phase=334 launch-header-compare source=psram_legacy error={:?}\r\n",
                err
            );
        }
    }
    uart_write_line(uart, &line);
    line.clear();

    match dma_res {
        Ok(()) => {
            let mismatch = first_mismatch(expected, &dma[..compare_len]);
            let _ = write!(
                line,
                "phase=334 launch-header-compare source=psram_dma first64="
            );
            write_hex_bytes(&mut line, &dma[..compare_len]);
            if let Some((off, exp, act)) = mismatch {
                let _ = write!(
                    line,
                    " mismatch_off={} expected={:02x} actual={:02x}\r\n",
                    off, exp, act
                );
            } else {
                let _ = write!(line, " mismatch_off=none\r\n");
            }
        }
        Err(err) => {
            let _ = write!(
                line,
                "phase=334 launch-header-compare source=psram_dma error={:?}\r\n",
                err
            );
        }
    }
    uart_write_line(uart, &line);
    line.clear();

    if cw_ok {
        let mismatch = first_mismatch(expected, &cw[..compare_len]);
        let _ = write!(
            line,
            "phase=334 launch-header-compare source=code_window first64="
        );
        write_hex_bytes(&mut line, &cw[..compare_len]);
        if let Some((off, exp, act)) = mismatch {
            let _ = write!(
                line,
                " mismatch_off={} expected={:02x} actual={:02x}\r\n",
                off, exp, act
            );
        } else {
            let _ = write!(line, " mismatch_off=none\r\n");
        }
    } else {
        let _ = write!(
            line,
            "phase=334 launch-header-compare source=code_window error=CodeWindowReadFailed\r\n"
        );
    }
    uart_write_line(uart, &line);
    line.clear();

    let trace = dma_code_window_read_trace_snapshot();
    let window_base_addr = cw_state
        .base_addr
        .wrapping_add(cw_state.window_base_word.saturating_mul(4));
    let window_end_addr =
        window_base_addr.wrapping_add(cw_state.window_len_words.saturating_mul(4));
    let _ = write!(
        line,
        "phase=335 code-window-trace app_base={} staged_code_words={} verifier_logical_addr={} verifier_read_len={} code_window_base_addr={} code_window_end_addr={} offset_in_window={} physical_psram_addr={} copy_src_offset={} copy_len={} read_mode={} dma_attempts={} dma_successes={} dma_fallbacks={} dma_error_code={}\r\n",
        code_base_addr,
        code_words,
        code_base_addr,
        32,
        window_base_addr,
        window_end_addr,
        0,
        trace.last_addr,
        0,
        4,
        trace_mode_label(trace.last_mode),
        trace.dma_attempts,
        trace.dma_successes,
        trace.dma_fallbacks,
        trace.last_dma_error,
    );
    uart_write_line(uart, &line);
}

#[cfg(all(
    feature = "psram_dma_read_code_window",
    feature = "psram_dma_read_code_window_diag"
))]
fn log_bad_bytecode_size_trace<C: CodeSource>(
    code: &mut C,
    header: &[u8],
    file_len: usize,
    staged_code_size: usize,
    staged_code_base_addr: u32,
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
) {
    let parsed = KbcHeader::parse(header).ok();
    let decoded_bytecode_size = parsed.map(|h| h.bytecode_size as usize).unwrap_or(0);
    let header_len = header.len().min(32);
    let mut header32 = [0u8; 32];
    header32[..header_len].copy_from_slice(&header[..header_len]);

    let mut probe32 = [0u8; 32];
    let mut probe_ok = true;
    for word_index in 0..8usize {
        let Some(word) = code.word(word_index as u32) else {
            probe_ok = false;
            break;
        };
        let start = word_index * 4;
        probe32[start..start + 4].copy_from_slice(&word);
    }

    let mut line = LineBuffer::new();
    let _ = write!(
        line,
        "phase=336 launch-bad-bytecode-size-trace expected_code_size={} stored_len={} decoded_bytecode_size={} magic=",
        staged_code_size,
        file_len,
        decoded_bytecode_size
    );
    write_hex_bytes(&mut line, &header32[..4]);
    let _ = write!(
        line,
        " host_abi_major={} host_abi_minor={} verifier_read_addr={} verifier_read_len={} source=CodeSource\r\n",
        parsed.map(|h| h.host_abi_major).unwrap_or(0),
        parsed.map(|h| h.host_abi_minor).unwrap_or(0),
        staged_code_base_addr,
        32,
    );
    uart_write_line(uart, &line);
    line.clear();

    let _ = write!(line, "phase=336 launch-bad-bytecode-size-trace header32=");
    write_hex_bytes(&mut line, &header32[..header_len]);
    let _ = write!(line, "\r\n");
    uart_write_line(uart, &line);
    line.clear();

    let _ = write!(
        line,
        "phase=336 launch-bad-bytecode-size-trace verifier_probe32_read={} probe32=",
        probe_ok
    );
    write_hex_bytes(&mut line, &probe32);
    let _ = write!(line, "\r\n");
    uart_write_line(uart, &line);
}

/// Resolve a `BYTECODE/<file>` program, read its resident header into `header`,
/// budget-gate it, and stage its code segment for execution (KOTO-0127). With
/// PSRAM the code is streamed SD->`code_window` scratch->PSRAM (base 0); without
/// PSRAM it is read into `code_window` directly and capped at the window size.
/// While the file is open, the const heap image (KOTO-0139) is read from the
/// `rodata` segment into `heap[0..rodata_size]`; `run_app_session` zeroes the rest.
/// Every failure path logs a UART diagnostic and returns `None` so the caller
/// returns to a usable Shell without a device reset.
#[allow(clippy::too_many_arguments)]
fn stage_app_code<D>(
    volume_mgr: &VolumeManager<D, FirmwareClock>,
    #[cfg_attr(
        not(feature = "psram_qpi_code_window_verbose"),
        allow(unused_variables)
    )]
    app_id: &str,
    entry_path: &str,
    header: &mut [u8; KBC_HEADER_SIZE],
    psram: Option<&mut PsramBlocks<FirmwarePsramHal<'_>>>,
    code_window: &mut [u8; CODE_WINDOW_TOTAL_BYTES],
    heap: &mut [u8; MAX_DEVICE_HEAP_BYTES],
    lfn_storage: &mut [u8; MANIFEST_LFN_BYTES],
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
) -> Option<StagedApp>
where
    D: BlockDevice,
    D::Error: core::fmt::Debug,
{
    #[cfg(all(
        feature = "psram_dma_read_code_window",
        feature = "psram_dma_read_code_window_diag"
    ))]
    let mut source_first64 = [0u8; 64];
    #[cfg(all(
        feature = "psram_dma_read_code_window",
        feature = "psram_dma_read_code_window_diag"
    ))]
    let mut source_first64_len = 0usize;
    let code_base_addr = 0u32;

    let Ok(volume) = volume_mgr.open_volume(VolumeIdx(0)) else {
        uart_log(uart, "phase=259 launch-volume-open-error\r\n");
        return None;
    };
    let Ok(root) = volume.open_root_dir() else {
        uart_log(uart, "phase=260 launch-root-open-error\r\n");
        return None;
    };
    let Ok(apps_dir) = root.open_dir("APPS") else {
        uart_log(uart, "phase=261 launch-bytecode-dir-open-error\r\n");
        return None;
    };
    let file_name = entry_path.rsplit('/').next().unwrap_or(entry_path);
    let stem = file_name.strip_suffix(".kbc").unwrap_or(file_name);
    let mut package_name = LineBuffer::new();
    let _ = write!(package_name, "{}.kpa", stem);
    let package_name = core::str::from_utf8(package_name.as_bytes()).unwrap_or("");
    let mut short = None;
    let short_target = ShortFileName::create_from_str(package_name).ok();
    let mut lfn = LfnBuffer::new(lfn_storage);
    if apps_dir
        .iterate_dir_lfn(&mut lfn, |entry, long_name| {
            if short.is_none()
                && !entry.attributes.is_directory()
                && (long_name.is_some_and(|name| name.eq_ignore_ascii_case(package_name))
                    || short_target.as_ref() == Some(&entry.name))
            {
                short = Some(entry.name.clone());
            }
        })
        .is_err()
    {
        uart_log(uart, "phase=252 launch-bytecode-list-error\r\n");
        return None;
    }
    let Some(short) = short else {
        uart_log(uart, "phase=252 launch-bytecode-missing\r\n");
        return None;
    };
    let Ok(file) = apps_dir.open_file_in_dir(&short, Mode::ReadOnly) else {
        uart_log(uart, "phase=262 launch-bytecode-file-open-error\r\n");
        return None;
    };
    let package_file = short.clone();
    let read_exact = |dst: &mut [u8]| -> Result<(), ()> {
        let mut total = 0usize;
        while total < dst.len() {
            match file.read(&mut dst[total..]) {
                Ok(0) => return Err(()),
                Ok(count) => total += count,
                Err(_) => return Err(()),
            }
        }
        Ok(())
    };
    let mut kpa_header = [0u8; 64];
    if read_exact(&mut kpa_header).is_err() || &kpa_header[..4] != b"KPA1" {
        uart_log(uart, "phase=254 launch-package-invalid\r\n");
        return None;
    }
    let entry_count = u32::from_le_bytes(kpa_header[16..20].try_into().unwrap_or([0; 4]));
    let table_offset = u32::from_le_bytes(kpa_header[20..24].try_into().unwrap_or([0; 4]));
    let strings_offset = u32::from_le_bytes(kpa_header[24..28].try_into().unwrap_or([0; 4]));
    let mut record = [0u8; 64];
    let mut path_buf = [0u8; 96];
    let mut asset_range = None;
    for index in 0..entry_count {
        if file
            .seek_from_start(table_offset.saturating_add(index.saturating_mul(64)))
            .is_err()
            || read_exact(&mut record).is_err()
        {
            return None;
        }
        let path_offset = u32::from_le_bytes(record[0..4].try_into().unwrap_or([0; 4]));
        let path_len = u32::from_le_bytes(record[4..8].try_into().unwrap_or([0; 4])) as usize;
        if path_len > path_buf.len() {
            continue;
        }
        if file
            .seek_from_start(strings_offset.saturating_add(path_offset))
            .is_err()
            || read_exact(&mut path_buf[..path_len]).is_err()
        {
            return None;
        }
        if path_buf[..path_len] == *entry_path.as_bytes() {
            asset_range = Some((
                u32::from_le_bytes(record[16..20].try_into().unwrap_or([0; 4])),
                u32::from_le_bytes(record[20..24].try_into().unwrap_or([0; 4])),
            ));
            break;
        }
    }
    let Some((asset_base, asset_size)) = asset_range else {
        uart_log(uart, "phase=252 launch-bytecode-missing\r\n");
        return None;
    };
    let file_len = asset_size as usize;
    if file_len < KBC_HEADER_SIZE {
        uart_log(uart, "phase=254 launch-bytecode-truncated\r\n");
        return None;
    }

    if file.seek_from_start(asset_base).is_err() {
        return None;
    }
    // Read the resident header first; everything else is driven from it.
    let mut read = 0usize;
    while read < KBC_HEADER_SIZE {
        match file.read(&mut header[read..]) {
            Ok(0) => break,
            Ok(count) => read += count,
            Err(_) => {
                uart_log(uart, "phase=252 launch-bytecode-read-error\r\n");
                return None;
            }
        }
    }
    if read < KBC_HEADER_SIZE {
        uart_log(uart, "phase=254 launch-bytecode-truncated\r\n");
        return None;
    }
    let parsed = match KbcHeader::parse(header) {
        Ok(parsed) => parsed,
        Err(error) => {
            let mut line = LineBuffer::new();
            let _ = write!(line, "phase=254 launch-header-error error={:?}\r\n", error);
            uart_write_line(uart, &line);
            return None;
        }
    };
    let code_offset = parsed.code_offset as usize;
    let code_size = parsed.code_size as usize;
    if code_size == 0
        || code_size > DEVICE_CODE_CEILING
        || code_offset
            .checked_add(code_size)
            .is_none_or(|end| end > file_len)
    {
        let mut line = LineBuffer::new();
        let _ = write!(
            line,
            "phase=253 launch-bytecode-oversize code_size={} ceiling={}\r\n",
            code_size, DEVICE_CODE_CEILING
        );
        uart_write_line(uart, &line);
        return None;
    }
    if parsed.max_heap_bytes as usize > MAX_DEVICE_HEAP_BYTES {
        uart_log(uart, "phase=255 launch-memory-budget-error\r\n");
        return None;
    }
    // Const heap image (KOTO-0139): copy the rodata segment into the bottom of the
    // heap while the file is open. rodata is *not* staged into PSRAM/SRAM with the
    // code; it lives only on the SD card, so it is read straight into the heap here.
    // `run_app_session` then zeroes only heap[rodata_size..]. The bounds mirror the
    // verifier's: rodata must fit the heap request and lie within the file.
    let rodata_offset = parsed.rodata_offset as usize;
    let rodata_size = parsed.rodata_size as usize;
    if rodata_size > 0 {
        if rodata_size > parsed.max_heap_bytes as usize
            || rodata_offset < KBC_HEADER_SIZE
            || rodata_offset
                .checked_add(rodata_size)
                .is_none_or(|end| end > file_len)
        {
            uart_log(uart, "phase=255 launch-rodata-range-error\r\n");
            return None;
        }
        if file
            .seek_from_start(asset_base.saturating_add(rodata_offset as u32))
            .is_err()
        {
            uart_log(uart, "phase=252 launch-bytecode-seek-error\r\n");
            return None;
        }
        let mut got = 0usize;
        while got < rodata_size {
            match file.read(&mut heap[got..rodata_size]) {
                Ok(0) => break,
                Ok(count) => got += count,
                Err(_) => {
                    uart_log(uart, "phase=252 launch-bytecode-read-error\r\n");
                    return None;
                }
            }
        }
        if got != rodata_size {
            uart_log(uart, "phase=254 launch-bytecode-truncated\r\n");
            return None;
        }
    }
    if file
        .seek_from_start(asset_base.saturating_add(code_offset as u32))
        .is_err()
    {
        uart_log(uart, "phase=252 launch-bytecode-seek-error\r\n");
        return None;
    }
    let code_words = (code_size / 4) as u32;

    match psram {
        Some(psram) => {
            #[cfg(feature = "psram_qpi_code_window_verbose")]
            let mut verify_qpi_first16 = [0u8; 16];
            #[cfg(feature = "psram_qpi_code_window_verbose")]
            let mut verify_qpi_first16_len = 0usize;
            #[cfg(feature = "psram_qpi_code_window_verbose")]
            let mut verify_qpi_ok = true;
            #[cfg(feature = "psram_qpi_code_window_verbose")]
            let mut verify_qpi_fail = None;
            #[cfg(feature = "psram_qpi_code_window_verbose")]
            let mut verify_qpi_around = [0u8; 16];
            #[cfg(all(
                feature = "psram_qpi_code_window_verbose",
                not(feature = "psram_qpi_backend_v2")
            ))]
            let mut verify_legacy_first16 = [0u8; 16];
            #[cfg(all(
                feature = "psram_qpi_code_window_verbose",
                not(feature = "psram_qpi_backend_v2")
            ))]
            let mut verify_legacy_first16_len = 0usize;
            #[cfg(all(
                feature = "psram_qpi_code_window_verbose",
                not(feature = "psram_qpi_backend_v2")
            ))]
            let mut verify_legacy_ok = true;
            #[cfg(all(
                feature = "psram_qpi_code_window_verbose",
                not(feature = "psram_qpi_backend_v2")
            ))]
            let mut verify_legacy_fail = None;
            #[cfg(all(
                feature = "psram_qpi_code_window_verbose",
                not(feature = "psram_qpi_backend_v2")
            ))]
            let mut verify_legacy_around = [0u8; 16];
            // Stream the code into PSRAM at base 0 in `code_window`-sized chunks.
            let mut staged = 0usize;
            while staged < code_size {
                let want = (code_size - staged).min(code_window.len());
                let mut got = 0usize;
                while got < want {
                    match file.read(&mut code_window[got..want]) {
                        Ok(0) => break,
                        Ok(count) => got += count,
                        Err(_) => {
                            uart_log(uart, "phase=252 launch-bytecode-read-error\r\n");
                            return None;
                        }
                    }
                }
                if got == 0 {
                    break;
                }
                #[cfg(all(
                    feature = "psram_dma_read_code_window",
                    feature = "psram_dma_read_code_window_diag"
                ))]
                if source_first64_len < source_first64.len() {
                    let remaining = source_first64.len() - source_first64_len;
                    let copy = remaining.min(got);
                    source_first64[source_first64_len..source_first64_len + copy]
                        .copy_from_slice(&code_window[..copy]);
                    source_first64_len += copy;
                }
                #[cfg(feature = "psram_qpi_code_window_verbose")]
                {
                    let mut line = LineBuffer::new();
                    #[cfg(feature = "psram_qpi_backend_v2")]
                    {
                        let (mode_before, write_ok, mode_after) = {
                            let backend: &mut QpiCodeWindowPsram<'_> = psram.backend_mut();
                            let mode_before = backend.mode_for_diag();
                            let write_ok = backend
                                .write_qpi_stage_chunk(staged as u32, &code_window[..got])
                                .is_ok();
                            let mode_after = backend.mode_for_diag();
                            (mode_before, write_ok, mode_after)
                        };
                        let first16_len = got.min(16);
                        let _ = write!(
                            line,
                            "phase=257 app-stage-write app={} psram_addr=0x{:08x} src_off={} len={} write_mode=qpi_v2_w2 mode_before={} mode_after={} chunk={} src_first16=",
                            app_id,
                            staged as u32,
                            staged,
                            got,
                            qpi_mode_label(mode_before),
                            qpi_mode_label(mode_after),
                            PSRAM_QPI_SAFE_READ_CHUNK_BYTES,
                        );
                        write_hex_bytes(&mut line, &code_window[..first16_len]);
                        let _ = write!(line, "\r\n");
                        uart_write_line(uart, &line);
                        if !write_ok {
                            uart_log(uart, "phase=199 launch-psram-stage-error\r\n");
                            return None;
                        }
                    }

                    #[cfg(not(feature = "psram_qpi_backend_v2"))]
                    {
                        let mode_ok = {
                            let backend: &mut QpiCodeWindowPsram<'_> = psram.backend_mut();
                            backend.begin_legacy_stage_mode().is_ok()
                        };
                        let _ = write!(
                            line,
                            "phase=256 psram-mode step=before_legacy_write_exit_qpi ok={}\r\n",
                            if mode_ok { 1 } else { 0 }
                        );
                        uart_write_line(uart, &line);
                        if !mode_ok {
                            uart_log(uart, "phase=199 launch-psram-stage-error\r\n");
                            return None;
                        }
                        line.clear();
                        let first16_len = got.min(16);
                        let _ = write!(
                        line,
                        "phase=257 app-stage-write app={} psram_addr=0x{:08x} src_off={} len={} write_mode=legacy_spi_bitbang psram_mode=serial src_first16=",
                        app_id,
                        staged as u32,
                        staged,
                        got,
                    );
                        write_hex_bytes(&mut line, &code_window[..first16_len]);
                        let _ = write!(line, "\r\n");
                        uart_write_line(uart, &line);
                    }
                }
                #[cfg(feature = "psram_qpi_code_window_verbose")]
                #[cfg(feature = "psram_qpi_backend_v2")]
                if verify_qpi_fail.is_none() {
                    let mut verify_qpi_buf = [0u8; PSRAM_QPI_SAFE_READ_CHUNK_BYTES];
                    let mut local_off = 0usize;
                    while local_off < got {
                        let len = (got - local_off).min(PSRAM_QPI_SAFE_READ_CHUNK_BYTES);
                        let qpi_read_ok = {
                            let backend: &mut QpiCodeWindowPsram<'_> = psram.backend_mut();
                            backend
                                .read_qpi_stage_for_verify(
                                    staged as u32 + local_off as u32,
                                    &mut verify_qpi_buf[..len],
                                )
                                .is_ok()
                        };
                        if !qpi_read_ok {
                            verify_qpi_ok = false;
                            verify_qpi_fail = Some((staged + local_off, 0, 0));
                            break;
                        }
                        if verify_qpi_first16_len < verify_qpi_first16.len() {
                            let copy = (verify_qpi_first16.len() - verify_qpi_first16_len).min(len);
                            verify_qpi_first16
                                [verify_qpi_first16_len..verify_qpi_first16_len + copy]
                                .copy_from_slice(&verify_qpi_buf[..copy]);
                            verify_qpi_first16_len += copy;
                        }
                        if verify_qpi_fail.is_none() {
                            if let Some((mismatch, exp, got_byte)) = first_mismatch(
                                &code_window[local_off..local_off + len],
                                &verify_qpi_buf[..len],
                            ) {
                                verify_qpi_ok = false;
                                verify_qpi_fail =
                                    Some((staged + local_off + mismatch, exp, got_byte));
                                copy_around_fail(
                                    &mut verify_qpi_around,
                                    &verify_qpi_buf[..len],
                                    mismatch,
                                );
                            }
                        }
                        local_off += len;
                    }
                }

                #[cfg(feature = "psram_qpi_code_window_verbose")]
                #[cfg(not(feature = "psram_qpi_backend_v2"))]
                if verify_qpi_fail.is_none() || verify_legacy_fail.is_none() {
                    let mut verify_qpi_buf = [0u8; PSRAM_PROD_READ_CHUNK_BYTES];
                    let mut verify_legacy_buf = [0u8; PSRAM_PROD_READ_CHUNK_BYTES];
                    let legacy_write_ok = {
                        let backend: &mut QpiCodeWindowPsram<'_> = psram.backend_mut();
                        backend
                            .write_legacy_stage_chunk(staged as u32, &code_window[..got])
                            .is_ok()
                    };
                    if !legacy_write_ok {
                        verify_legacy_ok = false;
                        verify_legacy_fail = Some((staged, 0, 0));
                        uart_log(uart, "phase=199 launch-psram-stage-error\r\n");
                        return None;
                    }
                    let mut local_off = 0usize;
                    while local_off < got {
                        let len = (got - local_off).min(PSRAM_PROD_READ_CHUNK_BYTES);
                        let legacy_read_ok = {
                            let backend: &mut QpiCodeWindowPsram<'_> = psram.backend_mut();
                            backend
                                .read_legacy_same_mode_for_verify(
                                    staged as u32 + local_off as u32,
                                    &mut verify_legacy_buf[..len],
                                )
                                .is_ok()
                        };
                        if !legacy_read_ok {
                            verify_legacy_ok = false;
                            verify_legacy_fail = Some((staged + local_off, 0, 0));
                            break;
                        }
                        if verify_legacy_first16_len < verify_legacy_first16.len() {
                            let copy =
                                (verify_legacy_first16.len() - verify_legacy_first16_len).min(len);
                            verify_legacy_first16
                                [verify_legacy_first16_len..verify_legacy_first16_len + copy]
                                .copy_from_slice(&verify_legacy_buf[..copy]);
                            verify_legacy_first16_len += copy;
                        }
                        if verify_legacy_fail.is_none() {
                            if let Some((mismatch, exp, got_byte)) = first_mismatch(
                                &code_window[local_off..local_off + len],
                                &verify_legacy_buf[..len],
                            ) {
                                verify_legacy_ok = false;
                                verify_legacy_fail =
                                    Some((staged + local_off + mismatch, exp, got_byte));
                                copy_around_fail(
                                    &mut verify_legacy_around,
                                    &verify_legacy_buf[..len],
                                    mismatch,
                                );
                            }
                        }

                        local_off += len;
                    }

                    let reenter_ok = {
                        let backend: &mut QpiCodeWindowPsram<'_> = psram.backend_mut();
                        backend.finish_legacy_stage_mode().is_ok()
                    };
                    if !reenter_ok {
                        verify_qpi_ok = false;
                        verify_qpi_fail = Some((staged, 0, 0));
                        uart_log(uart, "phase=199 launch-psram-stage-error\r\n");
                        return None;
                    }

                    let mut local_off = 0usize;
                    while local_off < got {
                        let len = (got - local_off).min(PSRAM_PROD_READ_CHUNK_BYTES);

                        let qpi_read_ok = {
                            let backend: &mut QpiCodeWindowPsram<'_> = psram.backend_mut();
                            backend
                                .read_qpi_for_verify(
                                    staged as u32 + local_off as u32,
                                    &mut verify_qpi_buf[..len],
                                )
                                .is_ok()
                        };
                        if !qpi_read_ok {
                            verify_qpi_ok = false;
                            verify_qpi_fail = Some((staged + local_off, 0, 0));
                            break;
                        }
                        if verify_qpi_first16_len < verify_qpi_first16.len() {
                            let copy = (verify_qpi_first16.len() - verify_qpi_first16_len).min(len);
                            verify_qpi_first16
                                [verify_qpi_first16_len..verify_qpi_first16_len + copy]
                                .copy_from_slice(&verify_qpi_buf[..copy]);
                            verify_qpi_first16_len += copy;
                        }
                        if verify_qpi_fail.is_none() {
                            if let Some((mismatch, exp, got_byte)) = first_mismatch(
                                &code_window[local_off..local_off + len],
                                &verify_qpi_buf[..len],
                            ) {
                                verify_qpi_ok = false;
                                verify_qpi_fail =
                                    Some((staged + local_off + mismatch, exp, got_byte));
                                copy_around_fail(
                                    &mut verify_qpi_around,
                                    &verify_qpi_buf[..len],
                                    mismatch,
                                );
                            }
                        }
                        local_off += len;
                    }
                }
                #[cfg(not(feature = "psram_qpi_code_window_verbose"))]
                if psram.write(staged as u32, &code_window[..got]).is_err() {
                    uart_log(uart, "phase=199 launch-psram-stage-error\r\n");
                    return None;
                }
                staged += got;
            }
            if staged != code_size {
                uart_log(uart, "phase=252 launch-bytecode-truncated\r\n");
                return None;
            }
            #[cfg(feature = "psram_qpi_code_window_verbose")]
            {
                let mut line = LineBuffer::new();
                let (qpi_fail_off, qpi_fail_exp, qpi_fail_got) = verify_qpi_fail
                    .map(|(off, exp, got)| (off as i32, exp, got))
                    .unwrap_or((-1, 0, 0));
                let _ = write!(
                    line,
                    "phase=258 app-stage-verify app={} psram_addr=0x{:08x} len={} read_mode={} ok={} fail_off={} fail_exp=0x{:02x} fail_got=0x{:02x} first16=",
                    app_id,
                    code_base_addr,
                    code_size,
                    if cfg!(feature = "psram_qpi_backend_v2") {
                        "qpi_v2_r8"
                    } else {
                        "qpi_same_backend"
                    },
                    if verify_qpi_ok { 1 } else { 0 },
                    qpi_fail_off,
                    qpi_fail_exp,
                    qpi_fail_got,
                );
                write_hex_bytes(&mut line, &verify_qpi_first16[..verify_qpi_first16_len]);
                let _ = write!(line, " around_fail=");
                write_hex_bytes(&mut line, &verify_qpi_around);
                let _ = write!(line, "\r\n");
                uart_write_line(uart, &line);

                #[cfg(not(feature = "psram_qpi_backend_v2"))]
                {
                    line.clear();
                    let (legacy_fail_off, legacy_fail_exp, legacy_fail_got) = verify_legacy_fail
                        .map(|(off, exp, got)| (off as i32, exp, got))
                        .unwrap_or((-1, 0, 0));
                    let _ = write!(
                    line,
                    "phase=258 app-stage-verify app={} psram_addr=0x{:08x} len={} read_mode=legacy_same_mode ok={} fail_off={} fail_exp=0x{:02x} fail_got=0x{:02x} first16=",
                    app_id,
                    code_base_addr,
                    code_size,
                    if verify_legacy_ok { 1 } else { 0 },
                    legacy_fail_off,
                    legacy_fail_exp,
                    legacy_fail_got,
                );
                    write_hex_bytes(
                        &mut line,
                        &verify_legacy_first16[..verify_legacy_first16_len],
                    );
                    let _ = write!(line, " around_fail=");
                    write_hex_bytes(&mut line, &verify_legacy_around);
                    let _ = write!(line, "\r\n");
                    uart_write_line(uart, &line);
                }
            }
            let mut line = LineBuffer::new();
            let _ = write!(
                line,
                "phase=156 app-staged backing=psram code_size={} base_addr={}\r\n",
                code_size, code_base_addr
            );
            uart_write_line(uart, &line);
            Some(StagedApp {
                file_len,
                code_size,
                code_words,
                used_psram: true,
                code_base_addr,
                package_file,
                #[cfg(all(
                    feature = "psram_dma_read_code_window",
                    feature = "psram_dma_read_code_window_diag"
                ))]
                source_first64,
                #[cfg(all(
                    feature = "psram_dma_read_code_window",
                    feature = "psram_dma_read_code_window_diag"
                ))]
                source_first64_len,
            })
        }
        None => {
            // No PSRAM: the code must fit the SRAM window.
            if code_size > code_window.len() {
                let mut line = LineBuffer::new();
                let _ = write!(
                    line,
                    "phase=253 launch-bytecode-oversize code_size={} window={}\r\n",
                    code_size,
                    code_window.len()
                );
                uart_write_line(uart, &line);
                return None;
            }
            let mut got = 0usize;
            while got < code_size {
                match file.read(&mut code_window[got..code_size]) {
                    Ok(0) => break,
                    Ok(count) => got += count,
                    Err(_) => {
                        uart_log(uart, "phase=252 launch-bytecode-read-error\r\n");
                        return None;
                    }
                }
            }
            if got != code_size {
                uart_log(uart, "phase=252 launch-bytecode-truncated\r\n");
                return None;
            }
            #[cfg(all(
                feature = "psram_dma_read_code_window",
                feature = "psram_dma_read_code_window_diag"
            ))]
            {
                let copy = source_first64.len().min(code_size);
                source_first64[..copy].copy_from_slice(&code_window[..copy]);
                source_first64_len = copy;
            }
            let mut line = LineBuffer::new();
            let _ = write!(
                line,
                "phase=156 app-staged backing=sram code_size={} base_addr={}\r\n",
                code_size, code_base_addr
            );
            uart_write_line(uart, &line);
            Some(StagedApp {
                file_len,
                code_size,
                code_words,
                used_psram: false,
                code_base_addr,
                package_file,
                #[cfg(all(
                    feature = "psram_dma_read_code_window",
                    feature = "psram_dma_read_code_window_diag"
                ))]
                source_first64,
                #[cfg(all(
                    feature = "psram_dma_read_code_window",
                    feature = "psram_dma_read_code_window_diag"
                ))]
                source_first64_len,
            })
        }
    }
}

fn app_input_snapshot(held: &HeldKeys, latest: Option<KeyEvent>) -> VmInputSnapshot {
    let mut intent_bits = 0u32;
    let mut text_codepoint = 0u32;
    if let Some(event) = latest.filter(|event| event.state == KEY_STATE_PRESSED) {
        // Scan-code → intent/codepoint semantics are the host-tested contract
        // shared with KotoSim (koto-core `keymap`, KOTO-0177): EXIT comes only
        // from F10 (0x90, the bridge's shift-translated Shift+F5), Esc is
        // CANCEL, and `x` is a plain typed character.
        intent_bits = koto_core::keymap::intent_for_key(event.key);
        text_codepoint = koto_core::keymap::typed_codepoint_for_key(event.key);
    }
    VmInputSnapshot {
        held_bits: held_key_bits(held),
        pressed_bits: latest
            .filter(|event| event.state == KEY_STATE_PRESSED)
            .map(|_| held_key_bits(held))
            .unwrap_or(0),
        text_codepoint,
        intent_bits,
    }
}

fn held_key_bits(held: &HeldKeys) -> u32 {
    let has = |key| held.as_slice().contains(&key);
    u32::from(has(0xb5))
        | (u32::from(has(0xb6)) << 1)
        | (u32::from(has(0xb4)) << 2)
        | (u32::from(has(0xb7)) << 3)
        | (u32::from(has(b'z') || has(0x0a)) << 4)
        | (u32::from(has(b'x') || has(0xb1)) << 5)
}

// The unit error mirrors the pre-split private helper: a failed shared-bus poll
// simply means "no event this frame", which the caller treats identically.
#[allow(clippy::result_unit_err)]
pub fn read_event(
    keyboard: &mut I2c<'_, peripherals::I2C1, embassy_rp::i2c::Blocking>,
) -> Result<KeyEvent, ()> {
    keyboard
        .blocking_write(KeyboardPins::I2C_ADDRESS, &[FIFO_REGISTER])
        .map_err(|_| ())?;
    block_for(Duration::from_micros(KEYBOARD_REGISTER_SETTLE_US));
    let mut raw = [0u8; 2];
    keyboard
        .blocking_read(KeyboardPins::I2C_ADDRESS, &mut raw)
        .map_err(|_| ())?;
    Ok(KeyEvent::from_wire(raw))
}
