#![no_std]
#![no_main]

#[cfg(all(
    any(feature = "wifi_residency_probe", feature = "network_service"),
    feature = "board-picocalc-pico2w"
))]
use core::mem::MaybeUninit;
use core::{fmt::Write, ptr::addr_of_mut};

use embassy_executor::Spawner;
use embassy_rp::{
    bind_interrupts, dma,
    gpio::{Input, Level, Output, Pull},
    i2c::{Config as I2cConfig, I2c},
    multicore::Stack,
    peripherals,
    pio::{InterruptHandler as PioInterruptHandler, Pio},
    pwm::{Config as PwmConfig, Pwm},
    spi::{Config as SpiConfig, Spi},
    uart::{Config as UartConfig, UartTx},
};
use embassy_time::{Delay, Instant, Timer};
use embedded_hal_bus::spi::ExclusiveDevice;
use embedded_sdmmc::{SdCard, ShortFileName, VolumeManager};
use koto_core::psram::PsramBlocks;
use koto_core::shell::{MemoryStatus, SaveStatus, StorageStatus};
#[cfg(all(
    feature = "network_service",
    any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"),
    not(feature = "wifi_residency_probe")
))]
use koto_core::unix_to_shell_clock;
use koto_core::{
    ui_input::push_input_state, BitmapFont, BootSplash, BootStep, BootStepStatus, ConfigService,
    KotoConfigAction, KotoConfigUi, PowerState, ShellAction, CONFIG_FORMAT_MAX_BYTES, MAX_PACKAGES,
};
#[cfg(all(
    feature = "network_service",
    any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"),
    not(feature = "wifi_residency_probe")
))]
use koto_core::{
    ConfigCapability, KotoConfigWifiUi, NetworkError, NetworkService, OperationState, SubmitResult,
    WifiConfigInputs, WifiIntent, WifiPageState, SHELL_CLOCK_RECT,
};
#[cfg(any(
    feature = "wifi_residency_probe",
    all(
        feature = "network_service",
        any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w")
    )
))]
use koto_pico::board::PicoWRadioResources;
use koto_pico::firmware::app_host::{
    AppStaticLayer, DeviceHostSkk, DeviceRuntimeHost, ManifestFetchResident, StaticLayerShadow,
};
use koto_pico::firmware::app_render::present_pixel_diagnostic;
use koto_pico::firmware::app_runtime::{read_event, run_device_app};
#[cfg(all(
    feature = "network_service",
    feature = "board-picocalc-picow",
    not(feature = "wifi_residency_probe")
))]
use koto_pico::firmware::audio::WifiResidencyArena;
use koto_pico::firmware::audio::{PicoAudioBackend, AUDIO_CORE1_STACK_BYTES};
#[cfg(any(
    feature = "wifi_residency_probe",
    all(feature = "network_service", feature = "board-picocalc-picow")
))]
use koto_pico::firmware::audio_residency::ResidencyState;
#[cfg(all(
    feature = "network_service",
    any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"),
    not(feature = "wifi_residency_probe")
))]
use koto_pico::firmware::config::publish_firmware_time;
use koto_pico::firmware::config::{
    FirmwareClock, CODE_WINDOW_TILES, CODE_WINDOW_TOTAL_BYTES, KICON_BYTES, MANIFEST_LFN_BYTES,
    MAX_DEVICE_HEAP_BYTES, MAX_EVENTS_PER_FRAME, POWER_POLL_MS, RASTER_STRIP_BYTES,
    RGB666_STRIP_BYTES, SD_ACQUIRE_SPI_HZ, SHELL_SWAP_PSRAM_ADDR, SYSTEM_STATUS_RECT,
};
use koto_pico::firmware::config_render::{paint_config, paint_config_rect};
#[cfg(all(
    feature = "network_service",
    any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"),
    not(feature = "wifi_residency_probe")
))]
use koto_pico::firmware::config_render::{paint_config_wifi, paint_config_wifi_rect};
use koto_pico::firmware::config_store::{load_system_config, save_system_config};
use koto_pico::firmware::diag::{log_paint_metrics, uart_log, uart_write_line};
use koto_pico::firmware::power::poll_power_state;
use koto_pico::firmware::resident::{shell_value_bytes, ShellCodeResident};
#[cfg(all(
    feature = "network_service",
    any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"),
    not(feature = "wifi_residency_probe")
))]
use koto_pico::firmware::secret_store::load_wifi_secret_store;
use koto_pico::firmware::shell_prefs::{apply_shell_prefs, save_shell_prefs};
use koto_pico::firmware::shell_render::{
    paint_selection_change, paint_shell, paint_shell_rect_metrics,
};
use koto_pico::firmware::spi_bench::run_spi_present_bench;
use koto_pico::firmware::splash_render::{paint_splash, paint_splash_step};
use koto_pico::firmware::stack_canary;
use koto_pico::firmware::storage::{fill_fallback, initialize_sd_card, load_packages};
#[cfg(all(
    feature = "network_service",
    any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"),
    not(feature = "wifi_residency_probe")
))]
use koto_pico::firmware::wifi_residency::cyw43_network_future;
#[cfg(all(
    feature = "wifi_residency_probe",
    not(feature = "wifi_stream_soak_probe")
))]
use koto_pico::firmware::wifi_residency::WifiRuntimeArena;
#[cfg(all(
    any(feature = "wifi_residency_probe", feature = "network_service"),
    feature = "board-picocalc-pico2w"
))]
use koto_pico::firmware::wifi_residency::WIFI_RESIDENCY_BYTES;
#[cfg(all(
    feature = "wifi_residency_probe",
    not(feature = "wifi_stream_soak_probe")
))]
use koto_pico::firmware::wifi_residency::{cyw43_lifecycle_future, wifi_spi_telemetry};
#[cfg(any(
    feature = "wifi_residency_probe",
    all(
        feature = "network_service",
        any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w")
    )
))]
use koto_pico::firmware::wifi_residency::{wifi_lifecycle_phase, WifiLifecyclePhase, WifiRuntime};
#[cfg(feature = "wifi_stream_soak_probe")]
use koto_pico::firmware::{stream_soak, wifi_residency::cyw43_soak_future};
use koto_pico::{
    board::{BOARD_ID, MCU_ID},
    dashboard::LineBuffer,
    firmware::FirmwareInput,
    keyboard::{
        HeldKeys, FIFO_CAPACITY, FRAME_PERIOD_MS, KEY_F1, KEY_F2, KEY_F3, KEY_F4, KEY_F5,
        KEY_STATE_PRESSED,
    },
    lcd::{PicoCalcLcd, ILI9488_SPI},
    psram::PSRAM_CAPACITY,
};
use koto_ui::{EventBuffer, UiRect};
// Default config uses the extracted `koto-psram` crate adapter. The legacy
// in-tree `PicoCalcPsram` base is still used by the `legacy_psram` escape hatch
// and by the `psram_dma_read_code_window` experiment wrapper.
#[cfg(feature = "psram_qpi_safe_read_code_window")]
use koto_pico::psram::PicoCalcQpiPsram;
#[cfg(all(
    not(feature = "psram_qpi_safe_read_code_window"),
    any(feature = "psram_dma_read_code_window", feature = "legacy_psram")
))]
use koto_pico::psram::{FirmwarePsramHal, PicoCalcPsram};
#[cfg(all(
    not(feature = "psram_qpi_safe_read_code_window"),
    not(feature = "psram_dma_read_code_window"),
    not(feature = "legacy_psram")
))]
use koto_pico::psram_ext::KotoPsram;
use static_cell::ConstStaticCell;

// The fast CodeWindow refill (KOTO-0153) reserves DMA_CH1 for `koto-psram`'s
// validated RX-DMA read, so its interrupt handler is bound alongside the LCD
// SPI's DMA_CH0 handler. Both share DMA_IRQ_0; the fast read arms CH1 with
// `irq_quiet`, so the extra handler never fires in either build.
#[cfg(not(feature = "psram_fast_code_window"))]
bind_interrupts!(struct Irqs {
    DMA_IRQ_0 => dma::InterruptHandler<peripherals::DMA_CH0>, dma::InterruptHandler<peripherals::DMA_CH2>, dma::InterruptHandler<peripherals::DMA_CH3>;
    PIO0_IRQ_0 => PioInterruptHandler<peripherals::PIO0>;
    PIO1_IRQ_0 => PioInterruptHandler<peripherals::PIO1>;
});

#[cfg(feature = "psram_fast_code_window")]
bind_interrupts!(struct Irqs {
    DMA_IRQ_0 => dma::InterruptHandler<peripherals::DMA_CH0>, dma::InterruptHandler<peripherals::DMA_CH1>, dma::InterruptHandler<peripherals::DMA_CH2>, dma::InterruptHandler<peripherals::DMA_CH3>;
    PIO0_IRQ_0 => PioInterruptHandler<peripherals::PIO0>;
    PIO1_IRQ_0 => PioInterruptHandler<peripherals::PIO1>;
});

const FONT_BYTES: &[u8] = include_bytes!("../../../../assets/fonts/mplus12.kfont");

// Raw UART0 MMIO writers shared by the fault/panic reporters below. The
// normal driver cannot be borrowed in a fault context, and a panicking core
// must not take any lock.
mod boot_diag {
    #[cfg(feature = "mcu-rp2040")]
    const UART0_BASE: usize = 0x4003_4000;
    #[cfg(feature = "mcu-rp235xa")]
    const UART0_BASE: usize = 0x4007_0000;
    const UARTDR: *mut u32 = UART0_BASE as *mut u32;
    const UARTFR: *const u32 = (UART0_BASE + 0x18) as *const u32;
    pub const SIO_CPUID: *const u32 = 0xd000_0000 as *const u32;

    pub fn put(byte: u8) {
        unsafe {
            while core::ptr::read_volatile(UARTFR) & (1 << 5) != 0 {}
            core::ptr::write_volatile(UARTDR, u32::from(byte));
        }
    }

    pub fn put_hex(value: u32) {
        for shift in (0..8u32).rev() {
            let nibble = ((value >> (shift * 4)) & 0xf) as u8;
            put(if nibble < 10 {
                b'0' + nibble
            } else {
                b'a' + nibble - 10
            });
        }
    }

    pub fn put_dec(value: u32) {
        let mut digits = [0u8; 10];
        let mut length = 0usize;
        let mut rest = value;
        loop {
            digits[length] = b'0' + (rest % 10) as u8;
            length += 1;
            rest /= 10;
            if rest == 0 {
                break;
            }
        }
        for index in (0..length).rev() {
            put(digits[index]);
        }
    }

    pub fn put_str(text: &[u8]) {
        for &byte in text {
            put(byte);
        }
    }
}

/// A HardFault otherwise lands in the silent cortex-m-rt default loop,
/// indistinguishable over UART from a livelock (KOTO-0178/KOTO-0251 boot
/// diagnosis). Prints the faulting core and return addresses through raw
/// UART0 MMIO, then parks. The vector table is shared, so a CPU1 fault
/// reports core=1 here.
#[cortex_m_rt::exception]
unsafe fn HardFault(frame: &cortex_m_rt::ExceptionFrame) -> ! {
    boot_diag::put_str(b"phase=91 hardfault core=");
    boot_diag::put_hex(unsafe { core::ptr::read_volatile(boot_diag::SIO_CPUID) });
    boot_diag::put_str(b" pc=");
    boot_diag::put_hex(frame.pc());
    boot_diag::put_str(b" lr=");
    boot_diag::put_hex(frame.lr());
    boot_diag::put_str(b" xpsr=");
    boot_diag::put_hex(frame.xpsr());
    // Stacked-frame address ~= SP at fault. On core0 a value at or below
    // `_stack_end` (top of .bss, which holds the audio worker control state)
    // is a main-stack overflow, not a wild pointer (KOTO-0245 TLS handshake).
    boot_diag::put_str(b" sp=");
    boot_diag::put_hex(core::ptr::from_ref(frame) as u32);
    boot_diag::put_str(b" stack_end=");
    boot_diag::put_hex(core::ptr::addr_of!(__ebss_marker) as u32);
    boot_diag::put_str(b"\r\n");
    loop {
        cortex_m::asm::wfi();
    }
}

extern "C" {
    /// Linker symbol at the top of `.bss` / bottom of the main stack. A stacked
    /// fault frame at or below this address indicates core0 stack overflow.
    #[link_name = "__ebss"]
    static __ebss_marker: u8;
}

/// Replaces `panic-halt` for the product binary: a panic previously halted in
/// a silent loop, indistinguishable over UART from a hang (KOTO-0178 lesson,
/// applied during the KOTO-0251 boot diagnosis). Prints core and panic
/// location through raw UART0 MMIO, then parks like `panic-halt` did.
#[panic_handler]
fn panic(info: &core::panic::PanicInfo<'_>) -> ! {
    boot_diag::put_str(b"phase=91 panic core=");
    boot_diag::put_hex(unsafe { core::ptr::read_volatile(boot_diag::SIO_CPUID) });
    if let Some(location) = info.location() {
        boot_diag::put_str(b" at ");
        boot_diag::put_str(location.file().as_bytes());
        boot_diag::put(b':');
        boot_diag::put_dec(location.line());
    }
    boot_diag::put_str(b"\r\n");
    loop {
        cortex_m::asm::wfi();
    }
}

// The static buffers stay in the binary; the firmware modules borrow them as
// mutable references so all working set sizing remains visible at the entry point.
//
// All of these are `ConstStaticCell` — const-initialized in place, handed out
// with `take()` — NOT `StaticCell` + a runtime `init(value)`. `init(value)`
// materializes the whole value in the *caller's stack frame* before copying it
// into the cell, and because the caller is the async main task, every one of
// those temporaries was carved into the main future's poll frame: KOTO-0172
// measured that prologue at 39,916 B, reserved on *every* poll of the main
// task, and it was the single largest term of the measured 68,588 B stack peak
// (phase=176). Const initializers live in the ELF image instead (all-zero ones
// stay in `.bss`; non-zero ones move to `.data` and are copied up by
// cortex-m-rt at reset with no stack involved).
static RASTER_STRIP: ConstStaticCell<[u8; RASTER_STRIP_BYTES]> =
    ConstStaticCell::new([0; RASTER_STRIP_BYTES]);
static RGB666_STRIP: ConstStaticCell<[u8; RGB666_STRIP_BYTES]> =
    ConstStaticCell::new([0; RGB666_STRIP_BYTES]);
// SD catalog scan buffers live in static storage so the loader runs in a small
// call frame instead of a multi-kilobyte stack frame (KOTO-0121).
static MANIFEST_NAMES: ConstStaticCell<[Option<ShortFileName>; MAX_PACKAGES]> =
    ConstStaticCell::new([const { None }; MAX_PACKAGES]);
static MANIFEST_FETCH: ConstStaticCell<ManifestFetchResident> =
    ConstStaticCell::new(ManifestFetchResident::new());
static CONFIG_BYTES: ConstStaticCell<[u8; CONFIG_FORMAT_MAX_BYTES]> =
    ConstStaticCell::new([0; CONFIG_FORMAT_MAX_BYTES]);
static SYSTEM_CONFIG: ConstStaticCell<ConfigService> = ConstStaticCell::new(ConfigService::new());
static MANIFEST_LFN: ConstStaticCell<[u8; MANIFEST_LFN_BYTES]> =
    ConstStaticCell::new([0; MANIFEST_LFN_BYTES]);
// One bounded, sequential `.kicon` read at a time (KOTO-0122).
static KICON_SCRATCH: ConstStaticCell<[u8; KICON_BYTES]> = ConstStaticCell::new([0; KICON_BYTES]);
// The portable shell model (~28 KiB) and app code cache (two resident 16 KiB
// tiles on RP2040) cannot be active at the same time. One tagged slot holds
// either value; launch snapshots the shell to reserved PSRAM before activating
// the code window. This keeps the two-tile MRU/LRU performance without paying
// for both objects in SRAM simultaneously.
static SHELL_CODE_RESIDENT: ConstStaticCell<ShellCodeResident<CODE_WINDOW_TOTAL_BYTES>> =
    ConstStaticCell::new(ShellCodeResident::new());
static APP_HEAP: ConstStaticCell<[u8; MAX_DEVICE_HEAP_BYTES]> =
    ConstStaticCell::new([0; MAX_DEVICE_HEAP_BYTES]);
// Current + previous frame app draw-command lists. Held here (not as locals in the
// async run loop) so these two ~30 KiB buffers stay out of the embassy main-task
// future, which was ~128 KiB largely because of them (KOTO-0134).
static APP_DRAW: ConstStaticCell<[DeviceRuntimeHost; 2]> =
    ConstStaticCell::new([DeviceRuntimeHost::new(), DeviceRuntimeHost::new()]);
// Retained KotoUI app session. Kept out of the async main future and reset on
// every app launch; mount validation copies no VM pointers into this storage.
static APP_UI_SESSION: ConstStaticCell<koto_core::UiSession> =
    ConstStaticCell::new(koto_core::UiSession::new());
// The single retained Game2D static/background layer (KOTO-0136). Held in its own
// cell — NOT inside the `APP_DRAW` pair — because it is retained app-session
// state, not a positional-diff target, so it needs no current/previous copy. (The
// first cut put it inside both `DeviceRuntimeHost`s, doubling its ~6 KiB and cutting
// boot-stack headroom enough to hang boot after `phase=146 battery`.)
static APP_STATIC: ConstStaticCell<AppStaticLayer> = ConstStaticCell::new(AppStaticLayer::new());
// Fingerprint shadow of the last *applied* static layer (GFX-0013): ~1 KiB of
// per-command content hashes + clipped footprints, so a mid-session rebuild can
// be diffed instead of always forcing a whole-surface repaint. Retained session
// state in its own cell per the KOTO-0136/KOTO-0148 ownership rule (never
// inside the double-buffered APP_DRAW pair).
static APP_STATIC_SHADOW: ConstStaticCell<StaticLayerShadow> =
    ConstStaticCell::new(StaticLayerShadow::new());
// SKK dictionary index + scan window for the running app session (KOTO-0252).
// Held here so the ~2.8 KiB bulk stays out of the app-session future: the
// future's size is paid again as poll-frame staging slots in `run_device_app`,
// which sat under the session-long `phase=176 at=app` stack peak.
static APP_HOST_SKK: ConstStaticCell<DeviceHostSkk> = ConstStaticCell::new(DeviceHostSkk::new());
static mut AUDIO_CORE1_STACK: Stack<AUDIO_CORE1_STACK_BYTES> = Stack::new();
#[cfg(all(feature = "wifi_residency_probe", feature = "board-picocalc-pico2w"))]
static WIFI_CONCURRENT_ARENA: ConstStaticCell<[MaybeUninit<u8>; WIFI_RESIDENCY_BYTES]> =
    ConstStaticCell::new([MaybeUninit::uninit(); WIFI_RESIDENCY_BYTES]);
#[cfg(all(feature = "network_service", feature = "board-picocalc-pico2w"))]
static NETWORK_SERVICE_ARENA: ConstStaticCell<[MaybeUninit<u8>; WIFI_RESIDENCY_BYTES]> =
    ConstStaticCell::new([MaybeUninit::uninit(); WIFI_RESIDENCY_BYTES]);

#[cfg(all(
    feature = "wifi_residency_probe",
    not(any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"))
))]
compile_error!("wifi_residency_probe is supported only by Pico W board profiles");

#[cfg(all(
    feature = "wifi_stream_soak_probe",
    not(feature = "board-picocalc-picow")
))]
compile_error!("wifi_stream_soak_probe requires the Pico W board profile");

/// Installs, polls, and tears down one arena-owned CYW43 lifecycle. Returns
/// the recovered arena plus whether the radio reached `RadioReady` (packet
/// staging included); `None` means the shutdown/join boundary itself failed.
#[cfg(all(
    feature = "wifi_residency_probe",
    not(feature = "wifi_stream_soak_probe")
))]
async fn run_wifi_lifecycle<A: WifiRuntimeArena>(
    arena: A,
    resources: PicoWRadioResources,
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
) -> Option<(A, bool)> {
    uart_log(uart, "phase=227 wifi-residency power-profile low_ms=20\r\n");
    #[cfg(feature = "wifi_pio_sequential_probe")]
    uart_log(
        uart,
        "phase=227 wifi-residency transport=sequential-tx-rx\r\n",
    );
    #[cfg(not(feature = "wifi_pio_sequential_probe"))]
    uart_log(
        uart,
        "phase=227 wifi-residency transport=concurrent-tx-rx\r\n",
    );
    let mut runtime = match WifiRuntime::try_new(arena, |state, _fetch_mailbox, _tls_session| {
        cyw43_lifecycle_future(state, resources, Irqs, Irqs, Irqs)
    }) {
        Ok(runtime) => runtime,
        Err((_, arena)) => {
            uart_log(uart, "phase=227 wifi-residency install-failed\r\n");
            return Some((arena, false));
        }
    };
    let mut line = LineBuffer::new();
    let _ = write!(
        line,
        "phase=227 wifi-residency future-bytes={}\r\n",
        runtime.future_bytes()
    );
    uart_write_line(uart, &line);
    let init_started = Instant::now();
    let mut radio_ready = false;
    loop {
        runtime.service().await;
        if wifi_lifecycle_phase() == WifiLifecyclePhase::RadioReady {
            radio_ready = true;
            uart_log(uart, "phase=227 wifi-residency radio-ready\r\n");
            uart_log(
                uart,
                "phase=227 wifi-residency packet-tx staged=5 recycled-min=1\r\n",
            );
            break;
        }
        if init_started.elapsed().as_millis() >= 10_000 {
            let telemetry = wifi_spi_telemetry();
            let mut line = LineBuffer::new();
            let _ = write!(
                line,
                "phase=227 wifi-residency radio-ready-timeout state={:?} polls={} spi_reads={} spi_writes={} spi_status=0x{:08x} spi_word=0x{:08x} pwr_highs={} pwr_latch={} pwr_input={}\r\n",
                wifi_lifecycle_phase(),
                runtime.polls(),
                telemetry.reads,
                telemetry.writes,
                telemetry.last_status,
                telemetry.last_word,
                telemetry.power_highs,
                telemetry.power_latch_high,
                telemetry.power_input_high
            );
            uart_write_line(uart, &line);
            let mut line = LineBuffer::new();
            let _ = write!(
                line,
                "phase=227 wifi-residency pio gpio_in=0x{:08x} funcs=0x{:08x} ctrl=0x{:08x} fstat=0x{:08x} fdebug=0x{:08x} padout=0x{:08x} padoe=0x{:08x} sm0_addr={}\r\n",
                telemetry.gpio_in,
                telemetry.pin_funcs,
                telemetry.pio_ctrl,
                telemetry.pio_fstat,
                telemetry.pio_fdebug,
                telemetry.pio_padout,
                telemetry.pio_padoe,
                telemetry.pio_sm0_addr
            );
            uart_write_line(uart, &line);
            break;
        }
        Timer::after_millis(1).await;
    }

    let arena = match runtime.shutdown() {
        Ok(arena) => arena,
        Err(_) => {
            uart_log(uart, "phase=227 wifi-residency shutdown-failed\r\n");
            return None;
        }
    };
    if wifi_lifecycle_phase() != WifiLifecyclePhase::Offline {
        uart_log(uart, "phase=227 wifi-residency power-down-failed\r\n");
        return None;
    }
    Some((arena, radio_ready))
}

/// KOTO-0227 acceptance soak: 100 physical `FullAudio -> WifiStreamAudio ->
/// FullAudio` round trips. Every trip reinstalls the concrete CYW43 future in
/// the borrowed 36 KiB arena, requires packet-buffer recycling before
/// `RadioReady`, and must reconstruct rich audio before the next trip starts.
#[cfg(all(
    feature = "wifi_residency_probe",
    feature = "board-picocalc-picow",
    not(feature = "wifi_stream_soak_probe")
))]
async fn run_wifi_residency_probe(
    audio: &mut PicoAudioBackend,
    resources: PicoWRadioResources,
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
) {
    const RESIDENCY_ROUND_TRIPS: u32 = 100;
    let mut ok_trips = 0u32;
    let mut radio_failures = 0u32;
    let mut aborted = false;
    for trip in 1..=RESIDENCY_ROUND_TRIPS {
        let mut line = LineBuffer::new();
        let _ = write!(
            line,
            "phase=227 wifi-residency trip={}/{}\r\n",
            trip, RESIDENCY_ROUND_TRIPS
        );
        uart_write_line(uart, &line);
        // SAFETY: on the first trip the canonical resources have never been
        // used; on later trips the previous lifecycle future was dropped and
        // joined inside `WifiRuntime::shutdown` (a `None`/aborted trip never
        // reaches this point again) and `RadioPowerOutput` forced GP23 low,
        // so no live user of PIO0, DMA_CH2/DMA_CH3, or the radio pins remains
        // when this alias is constructed.
        let trip_resources = unsafe { resources.clone_for_probe() };
        match run_wifi_residency_round_trip(audio, trip_resources, uart).await {
            Some(true) => ok_trips += 1,
            Some(false) => radio_failures += 1,
            None => {
                aborted = true;
                let mut line = LineBuffer::new();
                let _ = write!(
                    line,
                    "phase=227 wifi-residency soak-aborted trip={}\r\n",
                    trip
                );
                uart_write_line(uart, &line);
                break;
            }
        }
    }
    let stats = audio.stats();
    let mut line = LineBuffer::new();
    let _ = write!(
        line,
        "phase=227 wifi-residency soak trips={} ok={} radio_failures={} aborted={} transition_failures={} arena_guard_failures={}\r\n",
        RESIDENCY_ROUND_TRIPS,
        ok_trips,
        radio_failures,
        u8::from(aborted),
        stats.transition_failures,
        stats.arena_guard_failures
    );
    uart_write_line(uart, &line);
}

/// One bounded `FullAudio -> WifiStreamAudio -> FullAudio` transition.
/// Returns `Some(true)` when the radio reached `RadioReady` and rich audio
/// was reconstructed, `Some(false)` when a bounded radio/quiesce timeout was
/// recovered cleanly, and `None` when recovery itself failed and the soak
/// must stop (the arena or peripherals can no longer be proven released).
#[cfg(all(
    feature = "wifi_residency_probe",
    feature = "board-picocalc-picow",
    not(feature = "wifi_stream_soak_probe")
))]
async fn run_wifi_residency_round_trip(
    audio: &mut PicoAudioBackend,
    resources: PicoWRadioResources,
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
) -> Option<bool> {
    uart_log(uart, "phase=227 wifi-residency quiesce-start\r\n");
    if audio.begin_wifi_quiesce().is_err() {
        uart_log(uart, "phase=227 wifi-residency quiesce-rejected\r\n");
        return None;
    }
    let quiesce_started = Instant::now();
    let mut quiesce_timed_out = false;
    let arena = loop {
        audio.service();
        if let Ok(arena) = audio.activate_wifi_stream_audio() {
            break arena;
        }
        if !quiesce_timed_out && quiesce_started.elapsed().as_millis() >= 2_000 {
            uart_log(uart, "phase=227 wifi-residency audio-offline-timeout\r\n");
            quiesce_timed_out = true;
        }
        Timer::after_millis(1).await;
    };
    if quiesce_timed_out {
        return restore_full_audio(audio, arena, uart)
            .await
            .then_some(false);
    }
    let Some((arena, radio_ready)) = run_wifi_lifecycle(arena, resources, uart).await else {
        return None;
    };
    restore_full_audio(audio, arena, uart)
        .await
        .then_some(radio_ready)
}

#[cfg(all(feature = "wifi_residency_probe", feature = "board-picocalc-pico2w"))]
async fn run_wifi_residency_probe(
    audio: &mut PicoAudioBackend,
    resources: PicoWRadioResources,
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
) {
    uart_log(uart, "phase=227 wifi-concurrent start audio=full\r\n");
    if audio.residency_state() != ResidencyState::FullAudio {
        uart_log(uart, "phase=227 wifi-concurrent audio-not-full\r\n");
        return;
    }
    let heartbeat_before = audio.stats().worker_heartbeat;
    let arena = WIFI_CONCURRENT_ARENA.take();
    let Some((_arena, radio_ready)) = run_wifi_lifecycle(arena, resources, uart).await else {
        return;
    };
    if !radio_ready {
        uart_log(uart, "phase=227 wifi-concurrent radio-not-ready\r\n");
        return;
    }
    let audio_stats = audio.stats();
    let mut line = LineBuffer::new();
    let _ = write!(
        line,
        "phase=227 wifi-concurrent audio-heartbeat before={} after={}\r\n",
        heartbeat_before, audio_stats.worker_heartbeat
    );
    uart_write_line(uart, &line);
    if audio_stats.residency_state != ResidencyState::FullAudio {
        uart_log(uart, "phase=227 wifi-concurrent audio-state-changed\r\n");
        return;
    }
    if audio_stats.worker_heartbeat == heartbeat_before {
        uart_log(
            uart,
            "phase=227 wifi-concurrent audio-heartbeat-stalled\r\n",
        );
        return;
    }
    uart_log(
        uart,
        "phase=227 wifi-concurrent concurrent-ok audio=full\r\n",
    );
}

/// Reverse transition back to `FullAudio`. Returns `true` only after the CPU1
/// online acknowledgement is observed; `false` means the reverse boundary
/// failed and the arena state can no longer be trusted for another trip.
#[cfg(all(
    feature = "board-picocalc-picow",
    any(feature = "wifi_residency_probe", feature = "network_service")
))]
async fn restore_full_audio(
    audio: &mut PicoAudioBackend,
    arena: koto_pico::firmware::audio::WifiResidencyArena,
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
) -> bool {
    let Ok(arena) = audio.begin_full_audio_quiesce(arena) else {
        uart_log(uart, "phase=227 wifi-residency reverse-rejected\r\n");
        return false;
    };
    if audio.complete_wifi_quiesce(arena).is_err() {
        uart_log(uart, "phase=227 wifi-residency reconstruct-failed\r\n");
        return false;
    }
    let online_started = Instant::now();
    while audio.residency_state() != ResidencyState::FullAudio {
        audio.service();
        if online_started.elapsed().as_millis() >= 2_000 {
            uart_log(uart, "phase=227 wifi-residency audio-online-timeout\r\n");
            return false;
        }
        Timer::after_millis(1).await;
    }
    uart_log(uart, "phase=227 wifi-residency round-trip-ok\r\n");
    true
}

/// KOTO-0227 five-minute product-path soak entry: one held
/// `FullAudio -> WifiStreamAudio` transition wrapping the SD-driven
/// alternating PCM16/SLDPCM4 stream half in [`stream_soak::run`], then the
/// proven shutdown/power-down/reconstruction boundary and one summary line.
#[cfg(feature = "wifi_stream_soak_probe")]
async fn run_wifi_stream_soak<D>(
    audio: &mut PicoAudioBackend,
    resources: PicoWRadioResources,
    volume_mgr: &VolumeManager<D, FirmwareClock>,
    lfn_storage: &mut [u8],
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
) where
    D: embedded_sdmmc::BlockDevice,
    D::Error: core::fmt::Debug,
{
    uart_log(uart, "phase=227 stream-soak quiesce-start\r\n");
    if audio.begin_wifi_quiesce().is_err() {
        uart_log(uart, "phase=227 stream-soak quiesce-rejected\r\n");
        return;
    }
    let quiesce_started = Instant::now();
    let mut quiesce_timed_out = false;
    let arena = loop {
        audio.service();
        if let Ok(arena) = audio.activate_wifi_stream_audio() {
            break arena;
        }
        if !quiesce_timed_out && quiesce_started.elapsed().as_millis() >= 2_000 {
            uart_log(uart, "phase=227 stream-soak audio-offline-timeout\r\n");
            quiesce_timed_out = true;
        }
        Timer::after_millis(1).await;
    };
    if quiesce_timed_out {
        let _ = restore_full_audio(audio, arena, uart).await;
        return;
    }
    let mut runtime = match WifiRuntime::try_new(arena, |state, _fetch_mailbox, _tls_session| {
        cyw43_soak_future(state, resources, Irqs, Irqs, Irqs)
    }) {
        Ok(runtime) => runtime,
        Err((_, arena)) => {
            uart_log(uart, "phase=227 stream-soak install-failed\r\n");
            let _ = restore_full_audio(audio, arena, uart).await;
            return;
        }
    };
    let report = stream_soak::run(audio, &mut runtime, volume_mgr, lfn_storage, uart).await;
    let arena = match runtime.shutdown() {
        Ok(arena) => arena,
        Err(_) => {
            uart_log(uart, "phase=227 stream-soak shutdown-failed\r\n");
            return;
        }
    };
    if wifi_lifecycle_phase() != WifiLifecyclePhase::Offline {
        uart_log(uart, "phase=227 stream-soak power-down-failed\r\n");
        return;
    }
    let restored = restore_full_audio(audio, arena, uart).await;
    let stats = audio.stats();
    let mut line = LineBuffer::new();
    let _ = write!(
        line,
        "phase=227 stream-soak done result={:?} elapsed_ms={} pcm16_passes={} sld4_passes={} refills={} samples_submitted={} underruns={} drops={} tx_frames={} transition_failures={} arena_guard_failures={} audio_restored={}\r\n",
        report.result,
        report.elapsed_ms,
        report.pcm16_passes,
        report.sld4_passes,
        report.refills,
        report.samples_submitted,
        report.underruns,
        report.drops,
        report.tx_frames,
        stats.transition_failures,
        stats.arena_guard_failures,
        u8::from(restored)
    );
    uart_write_line(uart, &line);
}

/// Reads one pressed PicoCalc key and maps it to a `WifiKey`. Returns `None`
/// for released/idle/unmapped events.
#[cfg(all(
    feature = "network_service",
    any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"),
    not(feature = "wifi_residency_probe")
))]
fn read_wifi_key(
    keyboard: &mut I2c<'_, peripherals::I2C1, embassy_rp::i2c::Blocking>,
    held: &mut HeldKeys,
) -> Option<koto_core::net_ui::WifiKey> {
    use koto_core::net_ui::WifiKey;
    let event = koto_pico::firmware::app_runtime::read_event(keyboard).ok()?;
    held.apply(event);
    if event.state != KEY_STATE_PRESSED {
        return None;
    }
    Some(match event.key {
        0xb5 => WifiKey::Up,
        0xb6 => WifiKey::Down,
        0xb4 => WifiKey::Left,
        0xb7 => WifiKey::Right,
        0x0a => WifiKey::Enter,
        0xb1 => WifiKey::Esc,
        0x08 => WifiKey::Backspace,
        0x09 if held.as_slice().contains(&0xa2) || held.as_slice().contains(&0xa3) => {
            WifiKey::Previous
        }
        0x09 => WifiKey::Next,
        c if (0x20..=0x7e).contains(&c) => WifiKey::Char(c),
        _ => return None,
    })
}

/// Prints a redacted one-screen summary of the page. Never logs SSID or
/// credential bytes: only lengths, RSSI, security flag, and the fixed error.
#[cfg(all(feature = "network_service", feature = "board-picocalc-pico2w"))]
#[allow(dead_code)]
fn render_wifi_page(
    page: &koto_core::net_ui::WifiPageController,
    snapshot: &koto_core::NetworkSnapshot,
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
) {
    use koto_core::net::Security;
    use koto_core::net_ui::WifiPageState;

    let mut line = LineBuffer::new();
    let _ = write!(line, "phase=241 wifi page={:?}\r\n", page.state());
    uart_write_line(uart, &line);
    match page.state() {
        WifiPageState::Disabled => {
            uart_log(uart, "phase=241 wifi   Enter=enable, Esc=exit\r\n");
        }
        WifiPageState::Results => {
            for (index, row) in page.rows().enumerate() {
                let mut line = LineBuffer::new();
                let marker = if index == usize::from(page.selected()) {
                    '>'
                } else {
                    ' '
                };
                let secured = u8::from(matches!(row.security, Security::Wpa2PersonalAes));
                let _ = write!(
                    line,
                    "phase=241 wifi  {}[{}] ssid_len={} rssi={} secured={}\r\n",
                    marker,
                    index,
                    row.ssid.len(),
                    row.rssi_dbm,
                    secured
                );
                uart_write_line(uart, &line);
            }
            uart_log(uart, "phase=241 wifi   Up/Down, Enter=choose, R=rescan\r\n");
        }
        WifiPageState::CredentialEntry => {
            let mut line = LineBuffer::new();
            let _ = write!(
                line,
                "phase=241 wifi   password len={} (type, Enter=connect, Esc=back)\r\n",
                page.credential_len()
            );
            uart_write_line(uart, &line);
        }
        WifiPageState::Connected => {
            uart_log(
                uart,
                "phase=241 wifi   Enter=disconnect, F=forget, Esc=exit\r\n",
            );
        }
        WifiPageState::Failed => {
            let mut line = LineBuffer::new();
            let _ = write!(
                line,
                "phase=241 wifi   err={:?} (Enter=retry, Esc=back)\r\n",
                snapshot.last_error
            );
            uart_write_line(uart, &line);
        }
        _ => {}
    }
}

/// Starts the Pico 2 W concurrent product runtime without enabling the radio.
/// Failure leaves the normal offline Shell and language settings available.
#[cfg(all(feature = "network_service", feature = "board-picocalc-pico2w"))]
async fn start_product_network_runtime(
    resources: PicoWRadioResources,
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
) -> Option<WifiRuntime<&'static mut [MaybeUninit<u8>; WIFI_RESIDENCY_BYTES]>> {
    let arena = NETWORK_SERVICE_ARENA.take();
    let mut runtime = match WifiRuntime::try_new(arena, |state, fetch_mailbox, tls_session| {
        cyw43_network_future(
            state,
            fetch_mailbox,
            tls_session,
            resources,
            Irqs,
            Irqs,
            Irqs,
        )
    }) {
        Ok(runtime) => runtime,
        Err(_) => {
            uart_log(uart, "phase=243 wifi-product install-failed\r\n");
            return None;
        }
    };

    let bringup_started = Instant::now();
    loop {
        for _ in 0..64 {
            runtime.service().await;
            embassy_futures::yield_now().await;
        }
        if wifi_lifecycle_phase() == WifiLifecyclePhase::DriverReady {
            uart_log(uart, "phase=243 wifi-product driver-ready\r\n");
            return Some(runtime);
        }
        if bringup_started.elapsed().as_millis() >= 15_000 {
            uart_log(uart, "phase=243 wifi-product driver-timeout\r\n");
            let _ = runtime.shutdown();
            return None;
        }
    }
}

/// Retained UART-only service probe. Product KotoConfig instead uses
/// `start_product_network_runtime` and the LCD page from the normal Shell path.
#[cfg(all(feature = "network_service", feature = "board-picocalc-pico2w"))]
#[allow(dead_code)]
async fn run_network_service_probe(
    resources: PicoWRadioResources,
    keyboard: &mut I2c<'_, peripherals::I2C1, embassy_rp::i2c::Blocking>,
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
) {
    use koto_core::net::{CredentialView, NetworkService, OperationState};
    use koto_core::net_ui::{WifiIntent, WifiPageController, WifiPageState};
    use koto_pico::firmware::network::{dhcp_status, service_network, Cyw43WifiHal};

    uart_log(uart, "phase=241 wifi start\r\n");
    let arena = NETWORK_SERVICE_ARENA.take();
    let mut runtime = match WifiRuntime::try_new(arena, |state, fetch_mailbox, tls_session| {
        cyw43_network_future(
            state,
            fetch_mailbox,
            tls_session,
            resources,
            Irqs,
            Irqs,
            Irqs,
        )
    }) {
        Ok(runtime) => runtime,
        Err(_) => {
            uart_log(uart, "phase=241 wifi install-failed\r\n");
            return;
        }
    };
    let mut line = LineBuffer::new();
    let _ = write!(
        line,
        "phase=241 wifi future-bytes={}\r\n",
        runtime.future_bytes()
    );
    uart_write_line(uart, &line);

    // Pump the radio future hard until the CYW43 firmware upload finishes
    // (`DriverReady`) before starting the service clock, so the 10 s radio-enable
    // deadline covers only CLM init, not the ~231 KB firmware upload.
    let bringup_started = Instant::now();
    loop {
        for _ in 0..64 {
            runtime.service().await;
            embassy_futures::yield_now().await;
        }
        if wifi_lifecycle_phase() == WifiLifecyclePhase::DriverReady {
            uart_log(uart, "phase=241 wifi driver-ready\r\n");
            break;
        }
        if bringup_started.elapsed().as_millis() >= 15_000 {
            uart_log(uart, "phase=241 wifi driver-timeout\r\n");
            let _ = runtime.shutdown();
            return;
        }
    }

    let mut service = NetworkService::new();
    let mut page = WifiPageController::new();

    // Composite WIFI_CONFIG (KOTO-0224): the board declares a CYW43 transport,
    // the HAL reached DriverReady behind a resolved release region policy, the
    // NetworkService is alive for this lifecycle generation, and the (stub)
    // credential provider is ready. A board name or single bit alone never
    // promotes this; see koto_core::WifiConfigInputs.
    {
        use koto_core::net::RegulatoryRegion;
        use koto_pico::firmware::network::PRODUCT_REGION;

        let region = RegulatoryRegion::resolve(PRODUCT_REGION);
        let code = region.map(|r| r.code()).unwrap_or(*b"--");
        let inputs = koto_core::WifiConfigInputs {
            supported_transport: true,
            hal_initialized: region.is_ok(),
            network_service_generation: Some(service.generation()),
            lifecycle_generation: service.generation(),
            credential_provider_ready: true,
        };
        let mut line = LineBuffer::new();
        let _ = write!(
            line,
            "phase=241 wifi region={}{} wifi-config={}\r\n",
            code[0] as char,
            code[1] as char,
            u8::from(inputs.wifi_config())
        );
        uart_write_line(uart, &line);
    }

    let started = Instant::now();
    let mut last_render: Option<(WifiPageState, u8, u8, Option<_>)> = None;
    let mut dhcp_reported = false;
    let mut done = false;
    let mut wifi_held = HeldKeys::new();

    while !done {
        for _ in 0..64 {
            runtime.service().await;
            embassy_futures::yield_now().await;
        }
        let now_ms = started.elapsed().as_millis() as u64;
        service_network(&mut service, now_ms, 8);
        let snapshot = service.snapshot();

        let key = read_wifi_key(keyboard, &mut wifi_held);
        let intent = page.update(&snapshot, service.results().copied(), key);

        match intent {
            WifiIntent::None => {}
            WifiIntent::Exit => done = true,
            WifiIntent::EnableRadio => {
                let _ = service.set_radio(true);
            }
            WifiIntent::Scan => {
                let _ = service.scan();
            }
            WifiIntent::Connect {
                result_id,
                security,
            } => {
                let view = CredentialView {
                    security,
                    secret: page.credential(),
                };
                let _ = service.connect(result_id, view);
                page.clear_credential();
                uart_log(uart, "phase=241 wifi connect-submitted\r\n");
            }
            WifiIntent::Disconnect => {
                let _ = service.disconnect();
            }
            WifiIntent::Forget { profile_id } => {
                let _ = service.forget(profile_id);
            }
            WifiIntent::Cancel => {
                let mut hal = Cyw43WifiHal;
                let _ = service.cancel(snapshot.request_id, &mut hal);
            }
        }

        let signature = (
            page.state(),
            page.selected(),
            page.credential_len(),
            snapshot.last_error,
        );
        if last_render != Some(signature) {
            render_wifi_page(&page, &snapshot, uart);
            last_render = Some(signature);
        }

        // Report embassy-net DHCP config-up once after association.
        if matches!(page.state(), WifiPageState::Connected) {
            if !dhcp_reported {
                let (link_up, config_up, ip) = dhcp_status();
                if config_up {
                    let mut line = LineBuffer::new();
                    let _ = write!(
                        line,
                        "phase=241 wifi dhcp-up ip={}.{}.{}.{} link={}\r\n",
                        ip[0],
                        ip[1],
                        ip[2],
                        ip[3],
                        u8::from(link_up)
                    );
                    uart_write_line(uart, &line);
                    dhcp_reported = true;
                }
            }
        } else {
            dhcp_reported = false;
        }

        if started.elapsed().as_millis() >= 300_000 {
            uart_log(uart, "phase=241 wifi timeout\r\n");
            done = true;
        }
    }

    // Graceful teardown into the safe offline state: leave the AP, quiesce the
    // service (cancel active request, drain queues, advance generation), then
    // cancel/join the driver + network futures and power the radio down before
    // releasing the arena.
    uart_log(uart, "phase=241 wifi teardown\r\n");
    let _ = service.disconnect();
    let teardown_started = Instant::now();
    loop {
        for _ in 0..64 {
            runtime.service().await;
            embassy_futures::yield_now().await;
        }
        let now_ms = started.elapsed().as_millis() as u64;
        service_network(&mut service, now_ms, 8);
        if !matches!(
            service.snapshot().state,
            OperationState::Connected | OperationState::Connecting | OperationState::Disconnecting
        ) {
            break;
        }
        if teardown_started.elapsed().as_millis() >= 5_000 {
            uart_log(uart, "phase=241 wifi leave-timeout\r\n");
            break;
        }
    }
    service.quiesce_offline();
    page.clear_credential();
    match runtime.shutdown() {
        Ok(_) => uart_log(uart, "phase=241 wifi shutdown-ok\r\n"),
        Err(_) => uart_log(uart, "phase=241 wifi shutdown-failed\r\n"),
    }
    if wifi_lifecycle_phase() == WifiLifecyclePhase::Offline {
        uart_log(uart, "phase=241 wifi offline-ok\r\n");
    } else {
        uart_log(uart, "phase=241 wifi offline-failed\r\n");
    }
}

/// KOTO-0251 Pico W radio-enable transition phases. On RP2040 the radio cannot
/// come up until rich audio has quiesced, so radio enable from the Wi-Fi page
/// is an asynchronous transition pumped once per page frame with input live;
/// Esc cancels through the same KOTO-0227 recovery boundaries.
#[cfg(all(
    feature = "network_service",
    feature = "board-picocalc-picow",
    not(feature = "wifi_residency_probe")
))]
enum PicowWifiEnable {
    Idle,
    /// `begin_wifi_quiesce` accepted; waiting for the CPU1 offline
    /// acknowledgement to yield the switchable arena.
    Quiescing {
        started: Instant,
        timed_out: bool,
    },
    /// The arena-owned network lifecycle is installed; waiting for the CYW43
    /// firmware upload to reach `DriverReady` before the service clock starts.
    Bringup {
        started: Instant,
    },
}

/// KOTO-0227 proven audio-offline bound; past it the enable recovers to
/// `FullAudio` instead of continuing.
#[cfg(all(
    feature = "network_service",
    feature = "board-picocalc-picow",
    not(feature = "wifi_residency_probe")
))]
const PICOW_QUIESCE_TIMEOUT_MS: u64 = 2_000;
/// Hard bound on waiting for the CPU1 offline acknowledgement. Past it the
/// arena cannot be proven released, so the radio latches unavailable.
#[cfg(all(
    feature = "network_service",
    feature = "board-picocalc-picow",
    not(feature = "wifi_residency_probe")
))]
const PICOW_ARENA_CLAIM_HARD_LIMIT_MS: u64 = 10_000;
/// CYW43 firmware-upload bound, matching the Pico 2 W product bring-up.
#[cfg(all(
    feature = "network_service",
    feature = "board-picocalc-picow",
    not(feature = "wifi_residency_probe")
))]
const PICOW_DRIVER_BRINGUP_TIMEOUT_MS: u64 = 15_000;

/// KOTO-0251 reverse transition: quiesces the service, cancels/joins the
/// arena-owned runner (dropping it forces GP23 low), and rebuilds rich audio
/// through the proven KOTO-0227 boundary. When a boundary cannot be proven the
/// radio latches unavailable so no later enable reuses the arena; boot, Shell,
/// and offline app launch continue either way.
#[cfg(all(
    feature = "network_service",
    feature = "board-picocalc-picow",
    not(feature = "wifi_residency_probe")
))]
async fn picow_teardown_network_runtime(
    runtime: WifiRuntime<WifiResidencyArena>,
    network_service: &mut NetworkService,
    audio: &mut PicoAudioBackend,
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
) -> bool {
    // Cancel any in-flight operation and advance the lifecycle generation so
    // stale completions from the dying runtime are rejected (KOTO-0224).
    network_service.quiesce_offline();
    koto_pico::firmware::network::reset_radio_mailbox();
    koto_pico::firmware::network::clear_sntp_utc();
    let arena = match runtime.shutdown() {
        Ok(arena) => arena,
        Err(_) => {
            uart_log(uart, "phase=251 wifi-picow shutdown-failed\r\n");
            koto_pico::firmware::network::mark_radio_unavailable();
            return false;
        }
    };
    if wifi_lifecycle_phase() != WifiLifecyclePhase::Offline {
        uart_log(uart, "phase=251 wifi-picow power-down-failed\r\n");
        koto_pico::firmware::network::mark_radio_unavailable();
        return false;
    }
    if restore_full_audio(audio, arena, uart).await {
        uart_log(
            uart,
            "phase=251 wifi-picow radio-off full-audio-restored\r\n",
        );
        true
    } else {
        koto_pico::firmware::network::mark_radio_unavailable();
        false
    }
}

/// Pumps one frame of the KOTO-0251 radio-enable transition. Each phase
/// advances at most one bounded step so Wi-Fi page input stays live.
#[cfg(all(
    feature = "network_service",
    feature = "board-picocalc-picow",
    not(feature = "wifi_residency_probe")
))]
async fn picow_service_wifi_enable(
    enable: &mut PicowWifiEnable,
    network_runtime: &mut Option<WifiRuntime<WifiResidencyArena>>,
    network_service: &mut NetworkService,
    audio: &mut PicoAudioBackend,
    radio_resources: &PicoWRadioResources,
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
) {
    match enable {
        PicowWifiEnable::Idle => {}
        PicowWifiEnable::Quiescing { started, timed_out } => {
            audio.service();
            if let Ok(arena) = audio.activate_wifi_stream_audio() {
                if *timed_out {
                    // The quiesce exceeded its bound; recover to FullAudio
                    // instead of enabling on an unproven timeline.
                    if !restore_full_audio(audio, arena, uart).await {
                        koto_pico::firmware::network::mark_radio_unavailable();
                    }
                    *enable = PicowWifiEnable::Idle;
                    return;
                }
                // SAFETY: either no lifecycle was ever installed or the
                // previous one was cancelled/joined in
                // `picow_teardown_network_runtime` (arena returned, GP23
                // forced low, phase `Offline`), so no user of PIO0,
                // DMA_CH2/DMA_CH3, or the radio pins remains alive.
                let resources = unsafe { radio_resources.clone_for_enable_cycle() };
                match WifiRuntime::try_new(arena, |state, fetch_mailbox, tls_session| {
                    cyw43_network_future(
                        state,
                        fetch_mailbox,
                        tls_session,
                        resources,
                        Irqs,
                        Irqs,
                        Irqs,
                    )
                }) {
                    Ok(runtime) => {
                        // KOTO-0245 hardware admission check: the lifecycle
                        // future (grown by the HTTPS TlsFetchState) must fit
                        // the arena's future region.
                        let mut line = LineBuffer::new();
                        let _ = write!(
                            line,
                            "phase=251 wifi-picow lifecycle-installed future-bytes={} region={}\r\n",
                            runtime.future_bytes(),
                            runtime.future_region_bytes(),
                        );
                        uart_write_line(uart, &line);
                        *network_runtime = Some(runtime);
                        *enable = PicowWifiEnable::Bringup {
                            started: Instant::now(),
                        };
                    }
                    Err((_, arena)) => {
                        uart_log(uart, "phase=251 wifi-picow install-failed\r\n");
                        if !restore_full_audio(audio, arena, uart).await {
                            koto_pico::firmware::network::mark_radio_unavailable();
                        }
                        *enable = PicowWifiEnable::Idle;
                    }
                }
            } else if started.elapsed().as_millis() >= PICOW_ARENA_CLAIM_HARD_LIMIT_MS {
                // The CPU1 offline acknowledgement never arrived; the arena
                // cannot be proven released, so fail closed without reuse.
                uart_log(uart, "phase=251 wifi-picow quiesce-stalled\r\n");
                koto_pico::firmware::network::mark_radio_unavailable();
                *enable = PicowWifiEnable::Idle;
            } else if !*timed_out && started.elapsed().as_millis() >= PICOW_QUIESCE_TIMEOUT_MS {
                uart_log(uart, "phase=251 wifi-picow quiesce-timeout\r\n");
                *timed_out = true;
            }
        }
        PicowWifiEnable::Bringup { started } => {
            if wifi_lifecycle_phase() == WifiLifecyclePhase::DriverReady {
                uart_log(uart, "phase=251 wifi-picow driver-ready\r\n");
                // The service clock starts only now, so the 10 s radio-enable
                // deadline covers CLM init, not the ~231 KB firmware upload.
                if !matches!(network_service.set_radio(true), SubmitResult::Accepted(_)) {
                    uart_log(uart, "phase=251 wifi-picow radio-enable-rejected\r\n");
                }
                *enable = PicowWifiEnable::Idle;
            } else if started.elapsed().as_millis() >= PICOW_DRIVER_BRINGUP_TIMEOUT_MS {
                uart_log(uart, "phase=251 wifi-picow driver-timeout\r\n");
                if let Some(runtime) = network_runtime.take() {
                    let _ =
                        picow_teardown_network_runtime(runtime, network_service, audio, uart).await;
                }
                *enable = PicowWifiEnable::Idle;
            }
        }
    }
}

/// Cancels an in-flight enable transition (page escape/exit). `Quiescing`
/// waits for the arena at the proven claim boundary before rebuilding rich
/// audio; `Bringup` tears the freshly installed lifecycle down completely.
#[cfg(all(
    feature = "network_service",
    feature = "board-picocalc-picow",
    not(feature = "wifi_residency_probe")
))]
async fn picow_cancel_wifi_enable(
    enable: &mut PicowWifiEnable,
    network_runtime: &mut Option<WifiRuntime<WifiResidencyArena>>,
    network_service: &mut NetworkService,
    audio: &mut PicoAudioBackend,
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
) {
    match core::mem::replace(enable, PicowWifiEnable::Idle) {
        PicowWifiEnable::Idle => {}
        PicowWifiEnable::Quiescing { started, .. } => {
            uart_log(uart, "phase=251 wifi-picow enable-cancelled\r\n");
            loop {
                audio.service();
                if let Ok(arena) = audio.activate_wifi_stream_audio() {
                    if !restore_full_audio(audio, arena, uart).await {
                        koto_pico::firmware::network::mark_radio_unavailable();
                    }
                    break;
                }
                if started.elapsed().as_millis() >= PICOW_ARENA_CLAIM_HARD_LIMIT_MS {
                    uart_log(uart, "phase=251 wifi-picow quiesce-stalled\r\n");
                    koto_pico::firmware::network::mark_radio_unavailable();
                    break;
                }
                Timer::after_millis(1).await;
            }
        }
        PicowWifiEnable::Bringup { .. } => {
            uart_log(uart, "phase=251 wifi-picow enable-cancelled\r\n");
            if let Some(runtime) = network_runtime.take() {
                let _ = picow_teardown_network_runtime(runtime, network_service, audio, uart).await;
            }
        }
    }
}

#[embassy_executor::main(
    executor = "embassy_rp::executor::Executor",
    entry = "cortex_m_rt::entry"
)]
async fn main(_spawner: Spawner) {
    // KOTO-0170 Stage 0: paint the free RAM gap before anything with a deep
    // call tree runs, so the phase=176 low-water scans measure the whole boot.
    stack_canary::paint();
    let p = koto_pico::board::split_peripherals(embassy_rp::init(Default::default()));
    #[cfg(any(
        feature = "wifi_residency_probe",
        all(
            feature = "network_service",
            any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"),
            not(feature = "wifi_residency_probe")
        )
    ))]
    let radio_resources = PicoWRadioResources {
        pio: p.radio_pio,
        power: p.radio_power,
        data: p.radio_data,
        cs: p.radio_cs,
        clock: p.radio_clock,
        dma_tx: p.radio_dma,
        dma_rx: p.radio_dma_rx,
    };

    let mut uart_config = UartConfig::default();
    uart_config.baudrate = 115_200;
    let mut uart = UartTx::new_blocking(p.uart, p.uart_tx, uart_config);
    uart_log(
        &mut uart,
        "KotoOS firmware diagnostics KOTO-0119\r\nphase=10 uart-ready baud=115200 format=8N1\r\nphase=10 fw-tag=k0150-flashprobe-v1\r\n",
    );
    let mut board_line = LineBuffer::new();
    let _ = write!(board_line, "phase=10 board={} mcu={}\r\n", BOARD_ID, MCU_ID);
    uart_write_line(&mut uart, &board_line);
    // The mainboard UART bridge may already be enumerated while the terminal
    // opens after reset. Repeat the banner long enough for it to be observed.
    for _ in 0..6 {
        Timer::after_millis(500).await;
        uart_log(&mut uart, "phase=10 uart-ready\r\n");
        uart_log(&mut uart, "phase=10 fw-tag=k0150-flashprobe-v1\r\n");
    }

    // KOTO-0251 boot-bracket markers: the stretch from here to the first PSRAM
    // line had no UART output, so a silent hang (a panic included, before the
    // phase=91 reporters existed) could not be localized on hardware. One
    // line per peripheral bring-up step.
    uart_log(&mut uart, "phase=10 boot-mark lcd-spi\r\n");
    let mut spi_config = SpiConfig::default();
    spi_config.frequency = ILI9488_SPI.spi_hz;
    let spi = Spi::new_txonly(
        p.lcd_spi, p.lcd_sck, p.lcd_mosi, p.dma_ch0, Irqs, spi_config,
    );
    let mut lcd = PicoCalcLcd::new(
        spi,
        Output::new(p.lcd_cs, Level::High),
        Output::new(p.lcd_dc, Level::High),
        Output::new(p.lcd_reset, Level::High),
        &ILI9488_SPI,
    );

    uart_log(&mut uart, "phase=10 boot-mark keyboard-i2c\r\n");
    let mut i2c_config = I2cConfig::default();
    i2c_config.frequency = 100_000;
    // Use the same blocking STM32 bridge path validated by KOTO-0067 and
    // KOTO-0115. The async read path worked for the keyboard FIFO but failed
    // repeatedly on the slower battery-register response.
    let mut keyboard =
        I2c::new_blocking(p.keyboard_i2c, p.keyboard_sda, p.keyboard_scl, i2c_config);
    let mut audio_pwm = PwmConfig::default();
    audio_pwm.divider = 32u8.into();
    audio_pwm.top = 1_000;
    audio_pwm.compare_a = 500;
    audio_pwm.compare_b = 500;
    let audio_pwm = Pwm::new_output_ab(p.audio_pwm, p.audio_a, p.audio_b, audio_pwm);
    uart_log(&mut uart, "phase=10 boot-mark audio-core1-spawn\r\n");
    let mut audio = PicoAudioBackend::spawn_cpu1(
        p.core1,
        unsafe { &mut *addr_of_mut!(AUDIO_CORE1_STACK) },
        audio_pwm,
    );
    uart_log(&mut uart, "phase=10 boot-mark audio-core1-ok\r\n");
    #[cfg(all(
        feature = "wifi_residency_probe",
        not(feature = "wifi_stream_soak_probe")
    ))]
    run_wifi_residency_probe(&mut audio, radio_resources, &mut uart).await;
    #[cfg(all(
        feature = "network_service",
        feature = "board-picocalc-pico2w",
        not(feature = "wifi_residency_probe")
    ))]
    let mut network_runtime = start_product_network_runtime(radio_resources, &mut uart).await;
    // KOTO-0251: RP2040 has no boot-time network runtime. The switchable
    // residency installs one on demand from the Wi-Fi page radio enable and
    // tears it down on radio-off/fault, so the offline boot path is identical
    // to the offline artifact.
    #[cfg(all(
        feature = "network_service",
        feature = "board-picocalc-picow",
        not(feature = "wifi_residency_probe")
    ))]
    let mut network_runtime: Option<WifiRuntime<WifiResidencyArena>> = None;
    #[cfg(all(
        feature = "network_service",
        any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"),
        not(feature = "wifi_residency_probe")
    ))]
    let mut network_service = NetworkService::new();
    #[cfg(all(
        feature = "network_service",
        any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"),
        not(feature = "wifi_residency_probe")
    ))]
    let network_started = Instant::now();
    #[cfg(not(all(
        feature = "network_service",
        any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"),
        not(feature = "wifi_residency_probe")
    )))]
    let mut app_background = koto_pico::firmware::app_runtime::NoopAppBackgroundService;
    // PicoCalc SD detect is active-low on GP22. Removal is supported as a
    // fail-safe transition; reinsertion requires reboot/remount.
    uart_log(&mut uart, "phase=10 boot-mark sd-spi\r\n");
    let sd_detect = Input::new(p.sd_detect, Pull::Up);

    let mut sd_spi_config = SpiConfig::default();
    sd_spi_config.frequency = SD_ACQUIRE_SPI_HZ;
    let sd_spi = Spi::new_blocking(p.sd_spi, p.sd_sck, p.sd_mosi, p.sd_miso, sd_spi_config);
    let sd_device = ExclusiveDevice::new(sd_spi, Output::new(p.sd_cs, Level::High), Delay).unwrap();
    let sdcard = SdCard::new(sd_device, Delay);

    // Bring up the PicoCalc PSRAM (PIO1, GP20/21/2/3) so large apps can stage
    // their code off-SRAM and run through a small SRAM window (KOTO-0127). It is
    // optional for boot: if it cannot be created the launcher remains usable,
    // but app launch is disabled because the shell/code resident overlay needs
    // PSRAM to preserve the shell while the code window is active.
    //
    // The default `koto-psram` path consumes `pio1.common`/`pio1.sm0` by value,
    // so it does not need `mut`; the qpi_safe/dma/legacy paths borrow
    // `&mut pio1.common` and do.
    #[cfg_attr(
        all(
            not(feature = "psram_qpi_safe_read_code_window"),
            not(feature = "psram_dma_read_code_window"),
            not(feature = "legacy_psram")
        ),
        allow(unused_mut)
    )]
    let mut pio1 = Pio::new(p.psram_pio, Irqs);
    uart_log(&mut uart, "phase=10 boot-mark psram-init\r\n");
    #[cfg(feature = "psram_qpi_safe_read_code_window")]
    let mut psram = match PicoCalcQpiPsram::new(
        &mut pio1.common,
        pio1.sm0,
        p.psram_cs,
        p.psram_sck,
        p.psram_sio0,
        p.psram_sio1,
        p.psram_sio2,
        p.psram_sio3,
    ) {
        Ok(hal_base) => match koto_pico::psram::QpiCodeWindowPsram::new(hal_base) {
            Ok(hal) => PsramBlocks::try_new(hal, PSRAM_CAPACITY).ok(),
            Err(_) => None,
        },
        Err(_) => None,
    };

    // Legacy in-tree backend: the `legacy_psram` escape hatch, plus the
    // `psram_dma_read_code_window` experiment which wraps the same base HAL.
    #[cfg(all(
        not(feature = "psram_qpi_safe_read_code_window"),
        any(feature = "psram_dma_read_code_window", feature = "legacy_psram")
    ))]
    let mut psram = {
        let psram_hal_base = PicoCalcPsram::new(
            &mut pio1.common,
            pio1.sm0,
            p.psram_cs,
            p.psram_sck,
            p.psram_sio0,
            p.psram_sio1,
        );
        let identity = psram_hal_base.identity();
        let state = psram_hal_base.diag_state();
        let mut line = LineBuffer::new();
        let _ = write!(
            line,
            "phase=17 psram-device-id backend=picocalc-pio1 raw={:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x} manufacturer=0x{:02x} kgd=0x{:02x} density=0x{:02x} valid={} sys_hz={} pio_sm_hz={} serial_hz={} clkdiv={}.{} forced_fallback={}\r\n",
            identity.raw[0],
            identity.raw[1],
            identity.raw[2],
            identity.raw[3],
            identity.raw[4],
            identity.raw[5],
            identity.raw[6],
            identity.raw[7],
            identity.manufacturer,
            identity.known_good_die,
            identity.density,
            identity.is_aps6404_8m(),
            embassy_rp::clocks::clk_sys_freq(),
            state.sm_hz,
            state.sm_hz / u32::from(state.cycles_per_bit),
            state.clkdiv,
            state.clkdiv_frac,
            cfg!(feature = "force_psram_fallback"),
        );
        uart_write_line(&mut uart, &line);
        #[cfg(feature = "psram_dma_read_code_window")]
        let psram_hal: FirmwarePsramHal<'_> =
            koto_pico::psram::DmaCodeWindowPsram::new(psram_hal_base);
        #[cfg(not(feature = "psram_dma_read_code_window"))]
        let psram_hal: FirmwarePsramHal<'_> = psram_hal_base;
        PsramBlocks::try_new(psram_hal, PSRAM_CAPACITY).ok()
    };

    // Default config: bring up PSRAM through the extracted `koto-psram` crate
    // adapter (safe QPI profile). The crate consumes the PIO1 `common` handle by
    // value; only `common` + `sm0` are used here, so the move is safe.
    //
    // The fast CodeWindow refill drives PAC DMA channel 1; reserve `DMA_CH1`
    // here so embassy ownership matches the channel `koto-psram` manipulates.
    // `psram_fast_code_window` implies the default backend (enforced in
    // `psram_ext`), so this binding only exists in that build.
    #[cfg(feature = "psram_fast_code_window")]
    let psram_rx_dma = dma::Channel::new(p.psram_rx_dma, Irqs);
    #[cfg(all(
        not(feature = "psram_qpi_safe_read_code_window"),
        not(feature = "psram_dma_read_code_window"),
        not(feature = "legacy_psram")
    ))]
    let mut psram = match KotoPsram::new(
        pio1.common,
        pio1.sm0,
        p.psram_cs,
        p.psram_sck,
        p.psram_sio0,
        p.psram_sio1,
        p.psram_sio2,
        p.psram_sio3,
        #[cfg(feature = "psram_fast_code_window")]
        psram_rx_dma,
    ) {
        Ok(hal) => {
            if let Some(id) = hal.device_id() {
                let mut line = LineBuffer::new();
                let _ = write!(
                    line,
                    "phase=17 psram-device-id id=0x{:02x}{:02x}{:02x}\r\n",
                    id.raw[0], id.raw[1], id.raw[2]
                );
                uart_write_line(&mut uart, &line);
            }
            PsramBlocks::try_new(hal, PSRAM_CAPACITY).ok()
        }
        Err(_) => None,
    };
    if psram.is_some() {
        uart_log(&mut uart, "phase=16 psram-ready capacity=8388608\r\n");
    } else {
        uart_log(
            &mut uart,
            "phase=198 psram-unavailable fallback=shell-only\r\n",
        );
    }
    // Boot PCM diagnostic (KOTO-0165: rate follows the worker's 16 kHz output;
    // the square period doubles so the diag tone pitch stays put).
    const PCM_RATE: u32 = 16_000;
    const PCM_DIAG_FRAMES: usize = (PCM_RATE as usize * 160) / 1000;
    // Computed at compile time so the 5 KiB tone table reads from flash
    // instead of being built in the main task's poll frame (KOTO-0172).
    static PCM_DIAG: [i16; PCM_DIAG_FRAMES] = {
        let mut samples = [0i16; PCM_DIAG_FRAMES];
        let mut i = 0;
        while i < PCM_DIAG_FRAMES {
            samples[i] = if (i / 18) % 2 == 0 { 12_000 } else { -12_000 };
            i += 1;
        }
        samples
    };
    uart_log(
        &mut uart,
        "phase=171 audio_pcm_diag start backend=cpu1_pwm_koto_audio_gp26_gp27 sample_rate=16000 frames=2560\r\n",
    );
    let submit_result = audio.submit_pcm_mono_i16(PCM_RATE, &PCM_DIAG);
    let (accepted, diag_ok) = match submit_result {
        Ok(v) => (v.max(0) as u32, true),
        Err(_) => (0, false),
    };
    for _ in 0..accepted.saturating_add(24) {
        Timer::after_micros(63).await;
    }
    let mut audio_line = LineBuffer::new();
    let stats = audio.stats();
    let _ = write!(
        audio_line,
        "phase=171 audio_pcm_diag done backend={} sample_rate={} frames={} samples_submitted={} samples_played={} drops={} underruns={} result={}\r\n",
        audio.backend_name(),
        PCM_RATE,
        PCM_DIAG_FRAMES,
        stats.samples_submitted,
        stats.samples_played,
        stats.drops,
        stats.underruns,
        if diag_ok { "ok" } else { "error" },
    );
    uart_write_line(&mut uart, &audio_line);

    uart_log(&mut uart, "phase=11 lcd-init-start\r\n");
    if lcd.init().await.is_err() {
        uart_log(&mut uart, "phase=90 lcd-init-error\r\n");
        loop {
            Timer::after_secs(1).await;
        }
    }
    uart_log(&mut uart, "phase=12 lcd-init-ok\r\n");
    let font = BitmapFont::from_bytes(FONT_BYTES).unwrap();
    let raster_strip = RASTER_STRIP.take();
    let rgb666_strip = RGB666_STRIP.take();
    let app_draw = APP_DRAW.take();
    let app_ui_session = APP_UI_SESSION.take();
    let app_static = APP_STATIC.take();
    let app_static_shadow = APP_STATIC_SHADOW.take();
    let app_host_skk = APP_HOST_SKK.take();
    // One-shot panel diagnostics run *before* the splash so it stays clean.
    //
    // KOTO-0174 re-investigation (a)/(b): SPI present-path microbench
    // (`phase=179 spi-rate` fixed-vs-per-byte decomposition, `phase=178
    // spi-overlap` DMA/CPU join race). Gated on DiagClass::Gfx — compiles to
    // nothing in the shipping profile; in a Gfx build it borrows the idle
    // present strips for ~100 ms before the first paint.
    run_spi_present_bench(&mut lcd, raster_strip, rgb666_strip, &mut uart).await;
    // Pixel-blit bring-up (KOTO-0129): drive a known 16x16 RGB565 tile through
    // the real `draw_pixels` command + present path before any app runs, so the
    // blit pipeline can be confirmed on hardware in isolation from game logic.
    // The splash paints over it immediately; the UART phase marks it.
    present_pixel_diagnostic(
        &mut lcd,
        &font,
        &mut app_draw[0],
        app_static,
        raster_strip,
        rgb666_strip,
        &mut uart,
    )
    .await;
    // Boot splash (KOTO-0181), replacing the KOTO-0119 solid-blue panel test:
    // the splash's own full-surface paint is the LCD bring-up proof now, and it
    // carries the identity moment. Steps already decided by this point (kernel
    // bring-up, PSRAM, audio PCM diag) resolve up front; the rest resolve as
    // the real init milestones below complete, so the splash adds only its
    // paint time (no cosmetic delays) to boot.
    let mut splash = BootSplash::new();
    splash.resolve(BootStep::Kernel, BootStepStatus::Ok);
    splash.resolve(
        BootStep::Memory,
        if psram.is_some() {
            BootStepStatus::Ok
        } else {
            BootStepStatus::Failed("sram fallback")
        },
    );
    splash.resolve(
        BootStep::Audio,
        if diag_ok {
            BootStepStatus::Ok
        } else {
            BootStepStatus::Failed("pcm error")
        },
    );
    uart_log(&mut uart, "phase=18 splash-render-start\r\n");
    paint_splash(
        &mut lcd,
        &splash,
        &font,
        raster_strip,
        rgb666_strip,
        &mut uart,
    )
    .await;
    uart_log(&mut uart, "phase=18 splash-render-ok\r\n");
    // Reintegrate the validated SD package catalog (KOTO-0121). The loader fills
    // a caller-owned list and uses static scan buffers, so it runs in a small
    // call frame rather than the multi-kilobyte frame that stalled boot during
    // KOTO-0119 raster integration.
    uart_log(&mut uart, "phase=13 sd-scan-start\r\n");
    let names = MANIFEST_NAMES.take();
    let manifest_fetch = MANIFEST_FETCH.take();
    let config_bytes = CONFIG_BYTES.take();
    let system_config = SYSTEM_CONFIG.take();
    let lfn_storage = MANIFEST_LFN.take();
    let kicon = KICON_SCRATCH.take();
    let shell_code_resident = SHELL_CODE_RESIDENT.take();
    let app_heap = APP_HEAP.take();
    let active_sd_hz = initialize_sd_card(&sdcard, &mut uart);
    let volume_mgr = VolumeManager::new(sdcard, FirmwareClock);
    #[cfg(feature = "wifi_stream_soak_probe")]
    if active_sd_hz.is_some() {
        run_wifi_stream_soak(
            &mut audio,
            radio_resources,
            &volume_mgr,
            &mut lfn_storage[..],
            &mut uart,
        )
        .await;
    } else {
        uart_log(&mut uart, "phase=227 stream-soak sd-unavailable\r\n");
    }
    #[cfg(all(
        feature = "network_service",
        any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"),
        not(feature = "wifi_residency_probe")
    ))]
    let mut wifi_secrets = load_wifi_secret_store(&volume_mgr);
    // The catalog is loaded *in place* inside the resident shell (KOTO-0172):
    // no `PackageList`-sized local and no `ShellState::new(packages)` move, so
    // the ~28 KiB slot they cost the main task's poll frame is gone.
    let mut shell = shell_code_resident.shell_mut().unwrap();
    let (storage_status, package_count) = shell.reload_packages(|packages| {
        let status = if active_sd_hz.is_some() {
            load_packages(
                &volume_mgr,
                packages,
                names,
                manifest_fetch.scratch(),
                lfn_storage,
                kicon,
                &mut uart,
            )
        } else {
            fill_fallback(packages, "dev.koto.storage-unavailable", "SD unavailable");
            StorageStatus::Absent
        };
        (status, packages.len())
    });
    let mut line = LineBuffer::new();
    let _ = write!(
        line,
        "phase=14 catalog-ready packages={}\r\n",
        package_count
    );
    uart_write_line(&mut uart, &line);
    splash.resolve(
        BootStep::Storage,
        if storage_status == StorageStatus::Present {
            BootStepStatus::Ok
        } else {
            BootStepStatus::Failed("no sd")
        },
    );
    paint_splash_step(
        &mut lcd,
        &splash,
        &font,
        raster_strip,
        rgb666_strip,
        BootStep::Storage,
        &mut uart,
    )
    .await;

    shell.set_storage_status(storage_status);
    // Prefer the portable shell's full-width layout on RP2040. This keeps
    // normal navigation to two dirty tiles plus the status strip; X/Escape can
    // still open the shared details pane when needed.
    shell.set_detail_pane_visible(false);
    if storage_status == StorageStatus::Present {
        apply_shell_prefs(&volume_mgr, shell, manifest_fetch.scratch(), &mut uart);
        *system_config = load_system_config(&volume_mgr, config_bytes, &mut uart);
    }
    shell.apply_config_snapshot(system_config.snapshot());
    let initial_power = poll_power_state(&mut keyboard, &mut uart);
    if let Some(power) = initial_power {
        shell.set_power_state(power);
    } else {
        shell.set_power_state(PowerState::unsupported());
    }
    // The first keyboard-bridge poll doubles as the input milestone: the
    // battery registers ride the same STM32 I2C bridge as the key FIFO, so a
    // failed first poll means the input path is down too (KOTO-0067/0115).
    splash.resolve(
        BootStep::Input,
        if initial_power.is_some() {
            BootStepStatus::Ok
        } else {
            BootStepStatus::Failed("no bridge")
        },
    );
    paint_splash_step(
        &mut lcd,
        &splash,
        &font,
        raster_strip,
        rgb666_strip,
        BootStep::Input,
        &mut uart,
    )
    .await;
    let mut held = HeldKeys::new();
    let mut input = FirmwareInput::new();
    // Shell model is loaded and about to paint: the last checklist step
    // resolves and the progress bar completes just before the shell replaces
    // the splash. On a clean boot this state is visible only momentarily; a
    // failed step earns a short hold so the `[ng]` line can actually be read
    // on the panel rather than only in UART.
    splash.resolve(BootStep::Shell, BootStepStatus::Ok);
    paint_splash_step(
        &mut lcd,
        &splash,
        &font,
        raster_strip,
        rgb666_strip,
        BootStep::Shell,
        &mut uart,
    )
    .await;
    if splash.any_failed() {
        Timer::after_millis(1500).await;
    }
    uart_log(&mut uart, "phase=20 first-redraw-start\r\n");
    // Post-`phase=146 battery` boot markers (KOTO-0136 regression triage): bracket
    // the first shell paint and the main-loop entry so a hang after battery can be
    // localized over UART. `paint_shell` fuses compose+transfer per strip, so the
    // render and present are bracketed together (`render-start` .. `render-ok`); a
    // missing `render-ok` means the hang is in the shell paint, a missing
    // `loop-enter` means it is between paint and the loop, and `loop-enter` present
    // means the shell is live.
    uart_log(&mut uart, "phase=21 shell-render-start\r\n");
    let first_started = Instant::now();
    let metrics = paint_shell(
        &mut lcd,
        shell,
        &font,
        raster_strip,
        rgb666_strip,
        &mut uart,
    )
    .await;
    uart_log(&mut uart, "phase=22 shell-render-ok shell-present-ok\r\n");
    log_paint_metrics(
        &mut uart,
        &mut line,
        "phase=30 ready first",
        metrics,
        first_started,
    );

    let mut input_count = 0u32;
    let mut heartbeat = Instant::now();
    let mut power_poll = Instant::now();
    let mut card_present = sd_detect.is_low();
    // KOTO-0170 Stage 0: one self-describing region line, then the boot-init
    // peak. Later emits (shell cadence / app cadence / app exit) only report a
    // *new* observation point; the low-water mark itself is session-monotonic.
    stack_canary::emit_region(&mut uart);
    stack_canary::emit_peak(&mut uart, "boot");
    let mut stack_scan_beats = 0u32;
    uart_log(&mut uart, "phase=23 shell-loop-enter\r\n");
    loop {
        let frame_start = Instant::now();
        #[cfg(all(
            feature = "network_service",
            any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"),
            not(feature = "wifi_residency_probe")
        ))]
        {
            // KOTO-0251: a completed arena-owned network future is a fatal
            // runner fault; land the safe Offline state and rebuild rich audio.
            #[cfg(feature = "board-picocalc-picow")]
            if network_runtime
                .as_ref()
                .is_some_and(|runtime| !runtime.is_active())
            {
                uart_log(&mut uart, "phase=251 wifi-picow runner-fault\r\n");
                if let Some(runtime) = network_runtime.take() {
                    let _ = picow_teardown_network_runtime(
                        runtime,
                        &mut network_service,
                        &mut audio,
                        &mut uart,
                    )
                    .await;
                }
            }
            if let Some(runtime) = network_runtime.as_mut() {
                koto_pico::firmware::network::publish_sntp_server(
                    system_config.sntp_server().hostname(),
                );
                for _ in 0..8 {
                    runtime.service().await;
                    embassy_futures::yield_now().await;
                }
                koto_pico::firmware::network::service_network_with_credentials(
                    &mut network_service,
                    network_started.elapsed().as_millis() as u64,
                    8,
                    &mut wifi_secrets,
                );
            }
            let network_snapshot = network_service.snapshot();
            let connected =
                network_runtime.is_some() && network_snapshot.state == OperationState::Connected;
            let rssi_dbm = network_snapshot.connected_result_id.and_then(|result_id| {
                network_service
                    .results()
                    .find(|result| result.result_id == result_id)
                    .map(|result| result.rssi_dbm)
            });
            if shell.set_wifi_connection(connected, rssi_dbm) {
                let metrics = paint_shell_rect_metrics(
                    &mut lcd,
                    shell,
                    &font,
                    raster_strip,
                    rgb666_strip,
                    SYSTEM_STATUS_RECT,
                    &mut uart,
                )
                .await;
                log_paint_metrics(
                    &mut uart,
                    &mut line,
                    "phase=244 wifi-signal",
                    metrics,
                    frame_start,
                );
            }
        }
        #[cfg(all(
            feature = "network_service",
            any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"),
            not(feature = "wifi_residency_probe")
        ))]
        if let Some((utc_seconds, _generation)) = koto_pico::firmware::network::sntp_utc_seconds() {
            let local_seconds =
                utc_seconds.saturating_add(i64::from(system_config.utc_offset().minutes()) * 60);
            if let Some(clock) = unix_to_shell_clock(local_seconds) {
                publish_firmware_time(local_seconds);
                if shell.set_clock_if_minute_changed(clock) {
                    let metrics = paint_shell_rect_metrics(
                        &mut lcd,
                        shell,
                        &font,
                        raster_strip,
                        rgb666_strip,
                        SHELL_CLOCK_RECT,
                        &mut uart,
                    )
                    .await;
                    log_paint_metrics(
                        &mut uart,
                        &mut line,
                        "phase=244 clock-minute",
                        metrics,
                        frame_start,
                    );
                }
            }
        }
        let detected_present = sd_detect.is_low();
        if detected_present != card_present {
            card_present = detected_present;
            if card_present {
                shell.set_storage_status(StorageStatus::Unknown);
                uart_log(
                    &mut uart,
                    "phase=149 sd-inserted status=unknown remount=reboot-required\r\n",
                );
            } else {
                shell.set_storage_status(StorageStatus::Absent);
                shell.set_save_status(SaveStatus::Unknown);
                uart_log(
                    &mut uart,
                    "phase=148 sd-removed writes=disabled launches=disabled\r\n",
                );
            }
            let status_started = Instant::now();
            let metrics = paint_shell_rect_metrics(
                &mut lcd,
                shell,
                &font,
                raster_strip,
                rgb666_strip,
                SYSTEM_STATUS_RECT,
                &mut uart,
            )
            .await;
            log_paint_metrics(
                &mut uart,
                &mut line,
                "phase=30 ready storage-status",
                metrics,
                status_started,
            );
        }
        let mut latest = None;
        let mut command_key = None;
        for _ in 0..MAX_EVENTS_PER_FRAME.min(FIFO_CAPACITY) {
            match read_event(&mut keyboard) {
                Ok(event) if event.is_empty() => break,
                Ok(event) => {
                    if event.state == KEY_STATE_PRESSED {
                        // Reveal raw keycodes so command-key bindings can be
                        // confirmed on hardware (KOTO-0123).
                        line.clear();
                        let _ = write!(line, "phase=41 key-pressed code=0x{:02x}\r\n", event.key);
                        uart_write_line(&mut uart, &line);
                        if matches!(event.key, KEY_F1 | KEY_F2 | KEY_F3 | KEY_F4 | KEY_F5) {
                            command_key = Some(event.key);
                        }
                    }
                    held.apply(event);
                    latest = Some(event);
                    input_count = input_count.wrapping_add(1);
                }
                Err(()) => break,
            }
        }

        let previous = shell.selected_index();
        let previous_pane = shell.detail_pane_visible();
        let previous_system_view = shell.system_view_visible();
        // Refresh the memory snapshot for the system view (KOTO-0182) before any
        // paint. The canary scan is a sub-millisecond linear read and
        // `audio.stats()` is a critical-section snapshot, so this is cheap enough
        // to do every interaction and keeps the overlay live when open.
        let mem_peak = stack_canary::scan();
        let core1_free = match audio.stats().core1_stack_free_min {
            u32::MAX => None,
            free => Some(free as usize),
        };
        shell.set_memory_status(MemoryStatus {
            sram_total: stack_canary::SRAM_TOTAL,
            sram_static_used: Some(stack_canary::static_used()),
            sram_free_min: mem_peak.map(|peak| peak.free_min),
            stack_peak_used: mem_peak.map(|peak| peak.used),
            core1_stack_free_min: core1_free,
            app_heap_total: Some(app_heap.len()),
            app_heap_last_used: None,
            psram_total: PSRAM_CAPACITY as usize,
            psram_present: true,
            code_window_slots: CODE_WINDOW_TILES as u8,
        });
        // Total interaction latency spans state update, raster, and transfer.
        let interaction_started = Instant::now();
        // Command-bar actions use the shared shell state machine, mirroring
        // KotoSim's F2/F3/F4 bindings (KOTO-0123). They relayout the launcher, so
        // a command always forces a full redraw below. F5 toggles the system
        // status overlay (KOTO-0182): a view change, not a persisted preference.
        let command_action = match command_key {
            Some(KEY_F1) => shell.activate_command(koto_core::ShellCommandId::Settings),
            Some(KEY_F2) => {
                shell.activate_command(koto_core::ShellCommandId::Favorite);
                ShellAction::None
            }
            Some(KEY_F3) => {
                shell.activate_command(koto_core::ShellCommandId::Sort);
                ShellAction::None
            }
            Some(KEY_F4) => {
                shell.activate_command(koto_core::ShellCommandId::Category);
                ShellAction::None
            }
            Some(KEY_F5) => {
                shell.activate_command(koto_core::ShellCommandId::System);
                ShellAction::None
            }
            _ => ShellAction::None,
        };
        let command_acted = matches!(command_key, Some(KEY_F2 | KEY_F3 | KEY_F4));
        let input_action = shell.update(&input.sample(&held, latest));
        let action = if command_action == ShellAction::None {
            input_action
        } else {
            command_action
        };
        if action == ShellAction::OpenConfig {
            uart_log(&mut uart, "phase=348 config-open\r\n");
            held = HeldKeys::new();
            input = FirmwareInput::new();
            #[cfg(all(
                feature = "network_service",
                any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"),
                not(feature = "wifi_residency_probe")
            ))]
            let mut config_ui = {
                #[cfg(feature = "board-picocalc-pico2w")]
                let inputs = WifiConfigInputs {
                    supported_transport: true,
                    hal_initialized: network_runtime.is_some()
                        && wifi_lifecycle_phase() == WifiLifecyclePhase::DriverReady,
                    network_service_generation: network_runtime
                        .as_ref()
                        .map(|_| network_service.generation()),
                    lifecycle_generation: network_service.generation(),
                    credential_provider_ready: active_sd_hz.is_some()
                        && card_present
                        && wifi_secrets.available(),
                };
                // KOTO-0251: on RP2040 the driver is brought up on demand
                // behind the same release region policy (the radio cannot come
                // up until rich audio quiesces). WIFI_HAL therefore requires
                // the resolved region policy, and — whenever the switchable
                // runtime is live — an actually held `DriverReady`; a runtime
                // that failed bring-up withholds the capability.
                #[cfg(feature = "board-picocalc-picow")]
                let inputs = {
                    use koto_core::net::RegulatoryRegion;
                    use koto_pico::firmware::network::PRODUCT_REGION;

                    let region_ok = RegulatoryRegion::resolve(PRODUCT_REGION).is_ok();
                    WifiConfigInputs {
                        supported_transport: true,
                        hal_initialized: region_ok
                            && (network_runtime.is_none()
                                || wifi_lifecycle_phase() == WifiLifecyclePhase::DriverReady),
                        network_service_generation: Some(network_service.generation()),
                        lifecycle_generation: network_service.generation(),
                        credential_provider_ready: active_sd_hz.is_some()
                            && card_present
                            && wifi_secrets.available(),
                    }
                };
                let capabilities = if inputs.wifi_config() {
                    ConfigCapability::LOCALE_CONFIG.union(ConfigCapability::WIFI_CONFIG)
                } else {
                    ConfigCapability::LOCALE_CONFIG
                };
                KotoConfigUi::new_with_capabilities(system_config, capabilities)
            };
            #[cfg(not(all(
                feature = "network_service",
                any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"),
                not(feature = "wifi_residency_probe")
            )))]
            let mut config_ui = KotoConfigUi::new(system_config);
            let _ = paint_config(
                &mut lcd,
                &config_ui,
                &font,
                raster_strip,
                rgb666_strip,
                &mut uart,
            )
            .await;

            'config: loop {
                Timer::after_millis(FRAME_PERIOD_MS).await;
                let mut config_latest = None;
                let mut config_exit_requested = false;
                for _ in 0..MAX_EVENTS_PER_FRAME.min(FIFO_CAPACITY) {
                    match read_event(&mut keyboard) {
                        Ok(event) if event.is_empty() => break,
                        Ok(event) => {
                            config_exit_requested |= event.state == KEY_STATE_PRESSED
                                && koto_core::keymap::is_config_exit_key(event.key);
                            held.apply(event);
                            config_latest = Some(event);
                        }
                        Err(()) => break,
                    }
                }
                if config_exit_requested {
                    break 'config;
                }
                let state = input.sample(&held, config_latest);
                let mut events = EventBuffer::<8>::new();
                let _ = push_input_state(&state, &koto_core::Buttons::default(), &mut events);
                for event in events.iter() {
                    let config_action = config_ui.handle_event(event, system_config);
                    if config_action == KotoConfigAction::Exit {
                        break 'config;
                    }
                    #[cfg(all(
                        feature = "network_service",
                        any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"),
                        not(feature = "wifi_residency_probe")
                    ))]
                    if config_action == KotoConfigAction::OpenWifi {
                        let mut wifi_ui = KotoConfigWifiUi::new(
                            system_config.locale(),
                            network_service.snapshot(),
                        );
                        let mut wifi_held = HeldKeys::new();
                        #[cfg(feature = "board-picocalc-picow")]
                        let mut picow_enable = PicowWifiEnable::Idle;
                        let _ = paint_config_wifi(
                            &mut lcd,
                            &wifi_ui,
                            &font,
                            raster_strip,
                            rgb666_strip,
                            &mut uart,
                        )
                        .await;

                        'wifi: loop {
                            Timer::after_millis(FRAME_PERIOD_MS).await;
                            #[cfg(feature = "board-picocalc-pico2w")]
                            {
                                let Some(runtime) = network_runtime.as_mut() else {
                                    break 'wifi;
                                };
                                for _ in 0..8 {
                                    runtime.service().await;
                                    embassy_futures::yield_now().await;
                                }
                            }
                            // KOTO-0251: the page stays open with the radio off
                            // (runtime `None`); the enable transition is pumped
                            // per frame so input stays live throughout.
                            #[cfg(feature = "board-picocalc-picow")]
                            {
                                if network_runtime
                                    .as_ref()
                                    .is_some_and(|runtime| !runtime.is_active())
                                {
                                    uart_log(&mut uart, "phase=251 wifi-picow runner-fault\r\n");
                                    if let Some(runtime) = network_runtime.take() {
                                        let _ = picow_teardown_network_runtime(
                                            runtime,
                                            &mut network_service,
                                            &mut audio,
                                            &mut uart,
                                        )
                                        .await;
                                    }
                                }
                                if let Some(runtime) = network_runtime.as_mut() {
                                    for _ in 0..8 {
                                        runtime.service().await;
                                        embassy_futures::yield_now().await;
                                    }
                                }
                                picow_service_wifi_enable(
                                    &mut picow_enable,
                                    &mut network_runtime,
                                    &mut network_service,
                                    &mut audio,
                                    &radio_resources,
                                    &mut uart,
                                )
                                .await;
                            }
                            koto_pico::firmware::network::service_network_with_credentials(
                                &mut network_service,
                                network_started.elapsed().as_millis() as u64,
                                8,
                                &mut wifi_secrets,
                            );

                            let snapshot = network_service.snapshot();
                            if snapshot.state == OperationState::Connected
                                && !wifi_secrets.staging_zeroized()
                            {
                                if wifi_secrets.commit().is_ok() {
                                    uart_log(
                                        &mut uart,
                                        "phase=243 wifi-product credential-commit-ok\r\n",
                                    );
                                } else {
                                    uart_log(
                                        &mut uart,
                                        "phase=243 wifi-product credential-commit-failed\r\n",
                                    );
                                }
                            } else if snapshot.state == OperationState::RadioUnavailable
                                || snapshot.last_error == Some(NetworkError::AuthenticationFailed)
                            {
                                wifi_secrets.cancel_staged();
                            }

                            let key = read_wifi_key(&mut keyboard, &mut wifi_held);
                            let state_before_update = wifi_ui.state();
                            let mut intent =
                                wifi_ui.update(snapshot, network_service.results().copied(), key);
                            if intent == WifiIntent::None
                                && state_before_update == WifiPageState::Results
                                && wifi_ui.state() == WifiPageState::CredentialEntry
                            {
                                if let Some(result) = network_service
                                    .results()
                                    .nth(usize::from(wifi_ui.selected()))
                                    .copied()
                                {
                                    if wifi_secrets
                                        .credential_view_for(&result.ssid, result.security)
                                        .is_some()
                                    {
                                        intent = wifi_ui
                                            .begin_saved_connect(result.result_id, result.security);
                                    }
                                }
                            }
                            if intent == WifiIntent::Exit {
                                break 'wifi;
                            }

                            let submitted = match intent {
                                WifiIntent::None | WifiIntent::Exit => false,
                                WifiIntent::EnableRadio => {
                                    #[cfg(feature = "board-picocalc-pico2w")]
                                    let submitted = matches!(
                                        network_service.set_radio(true),
                                        SubmitResult::Accepted(_)
                                    );
                                    // KOTO-0251: with the runtime live the
                                    // radio hardware is already up (re-enable
                                    // after disconnect) and the service takes
                                    // it directly; otherwise start the
                                    // asynchronous residency transition — the
                                    // service accepts set_radio only after
                                    // `DriverReady`.
                                    #[cfg(feature = "board-picocalc-picow")]
                                    let submitted = if network_runtime.is_some() {
                                        matches!(
                                            network_service.set_radio(true),
                                            SubmitResult::Accepted(_)
                                        )
                                    } else {
                                        if matches!(picow_enable, PicowWifiEnable::Idle) {
                                            match audio.begin_wifi_quiesce() {
                                                Ok(_) => {
                                                    uart_log(
                                                        &mut uart,
                                                        "phase=251 wifi-picow quiesce-start\r\n",
                                                    );
                                                    picow_enable = PicowWifiEnable::Quiescing {
                                                        started: Instant::now(),
                                                        timed_out: false,
                                                    };
                                                }
                                                Err(_) => uart_log(
                                                    &mut uart,
                                                    "phase=251 wifi-picow quiesce-rejected\r\n",
                                                ),
                                            }
                                        }
                                        false
                                    };
                                    submitted
                                }
                                WifiIntent::Scan => {
                                    matches!(network_service.scan(), SubmitResult::Accepted(_))
                                }
                                WifiIntent::Connect {
                                    result_id,
                                    security,
                                } => {
                                    let result = network_service
                                        .results()
                                        .find(|result| result.result_id == result_id)
                                        .copied();
                                    if let Some(result) = result {
                                        let has_provider_credential = wifi_secrets
                                            .staged_credential_view()
                                            .or_else(|| {
                                                wifi_secrets.credential_view_for(
                                                    &result.ssid,
                                                    result.security,
                                                )
                                            })
                                            .is_some();
                                        if !wifi_ui.credential().is_empty()
                                            || !has_provider_credential
                                        {
                                            let _ = wifi_secrets.stage(
                                                &result.ssid,
                                                security,
                                                wifi_ui.credential(),
                                            );
                                        }
                                        let accepted = wifi_secrets
                                            .staged_credential_view()
                                            .or_else(|| {
                                                wifi_secrets.credential_view_for(
                                                    &result.ssid,
                                                    result.security,
                                                )
                                            })
                                            .is_some_and(|view| {
                                                matches!(
                                                    network_service.connect(result_id, view),
                                                    SubmitResult::Accepted(_)
                                                )
                                            });
                                        if !accepted {
                                            wifi_secrets.cancel_staged();
                                        }
                                        accepted
                                    } else {
                                        false
                                    }
                                }
                                WifiIntent::Disconnect => matches!(
                                    network_service.disconnect(),
                                    SubmitResult::Accepted(_)
                                ),
                                WifiIntent::Forget { profile_id } => {
                                    let durable_profile_id = network_service
                                        .results()
                                        .find(|result| result.result_id == profile_id)
                                        .and_then(|result| {
                                            wifi_secrets.find_by_ssid(&result.ssid, result.security)
                                        })
                                        .map(|profile| profile.profile_id);
                                    durable_profile_id.is_some_and(|profile_id| {
                                        matches!(
                                            network_service.forget(profile_id),
                                            SubmitResult::Accepted(_)
                                        )
                                    })
                                }
                                WifiIntent::Cancel => {
                                    let mut hal = koto_pico::firmware::network::Cyw43WifiHal;
                                    wifi_secrets.cancel_staged();
                                    matches!(
                                        network_service.cancel(snapshot.request_id, &mut hal),
                                        SubmitResult::Accepted(_)
                                    )
                                }
                            };
                            if submitted {
                                wifi_ui.submission_complete(intent);
                            }

                            let mut damage = [UiRect::EMPTY; 8];
                            let mut damage_len = 0usize;
                            for rect in wifi_ui.damaged_rects() {
                                if damage_len < damage.len() {
                                    damage[damage_len] = rect;
                                    damage_len += 1;
                                }
                            }
                            for rect in damage[..damage_len].iter().copied() {
                                let _ = paint_config_wifi_rect(
                                    &mut lcd,
                                    &wifi_ui,
                                    &font,
                                    raster_strip,
                                    rgb666_strip,
                                    rect,
                                    &mut uart,
                                )
                                .await;
                            }
                            wifi_ui.clear_damage();
                        }

                        // KOTO-0251 residency policy: Wi-Fi residency persists
                        // after leaving the page only while associated (so
                        // network time and future app networking operate);
                        // leaving unassociated powers the radio down and
                        // rebuilds rich audio. An in-flight enable is
                        // cancelled through the proven recovery boundaries.
                        #[cfg(feature = "board-picocalc-picow")]
                        {
                            picow_cancel_wifi_enable(
                                &mut picow_enable,
                                &mut network_runtime,
                                &mut network_service,
                                &mut audio,
                                &mut uart,
                            )
                            .await;
                            if network_service.snapshot().state != OperationState::Connected {
                                if let Some(runtime) = network_runtime.take() {
                                    uart_log(
                                        &mut uart,
                                        "phase=251 wifi-picow radio-off-on-exit\r\n",
                                    );
                                    let _ = picow_teardown_network_runtime(
                                        runtime,
                                        &mut network_service,
                                        &mut audio,
                                        &mut uart,
                                    )
                                    .await;
                                }
                            } else {
                                uart_log(&mut uart, "phase=251 wifi-picow residency-persists\r\n");
                            }
                        }
                        wifi_secrets.cancel_staged();
                        wifi_ui.reset();
                        held = HeldKeys::new();
                        input = FirmwareInput::new();
                        let _ = paint_config(
                            &mut lcd,
                            &config_ui,
                            &font,
                            raster_strip,
                            rgb666_strip,
                            &mut uart,
                        )
                        .await;
                    }
                    if matches!(config_action, KotoConfigAction::LocaleChanged(_)) {
                        shell.apply_config_snapshot(system_config.snapshot());
                    }
                    if matches!(
                        config_action,
                        KotoConfigAction::LocaleChanged(_)
                            | KotoConfigAction::UtcOffsetChanged(_)
                            | KotoConfigAction::SntpServerChanged(_)
                    ) {
                        if card_present {
                            let _ = save_system_config(
                                &volume_mgr,
                                system_config,
                                config_bytes,
                                &mut uart,
                            );
                        }
                    }
                    let mut damage = [UiRect::EMPTY; 8];
                    let mut damage_len = 0usize;
                    for rect in config_ui.damaged_rects() {
                        if damage_len < damage.len() {
                            damage[damage_len] = rect;
                            damage_len += 1;
                        }
                    }
                    for rect in damage[..damage_len].iter().copied() {
                        let _ = paint_config_rect(
                            &mut lcd,
                            &config_ui,
                            &font,
                            raster_strip,
                            rgb666_strip,
                            rect,
                            &mut uart,
                        )
                        .await;
                    }
                    config_ui.clear_damage();
                }
            }
            uart_log(&mut uart, "phase=349 config-exit\r\n");
            held = HeldKeys::new();
            input = FirmwareInput::new();
            let _ = paint_shell(
                &mut lcd,
                shell,
                &font,
                raster_strip,
                rgb666_strip,
                &mut uart,
            )
            .await;
            heartbeat = Instant::now();
            power_poll = Instant::now();
            continue;
        }
        if let ShellAction::Launch(package) = action {
            line.clear();
            let _ = write!(
                line,
                "phase=150 launch-request app={}\r\n",
                package.app_id()
            );
            uart_write_line(&mut uart, &line);
            if shell.storage_status() != StorageStatus::Present || !card_present {
                uart_log(
                    &mut uart,
                    "phase=263 launch-storage-unavailable remount=reboot-required\r\n",
                );
                continue;
            }
            if shell.battery_is_low() {
                uart_log(&mut uart, "phase=151 low-battery-before-launch\r\n");
            }
            // Shell UI and app execution are mutually exclusive. Preserve the
            // launcher at the reserved top of PSRAM, then reuse its SRAM slot as
            // the two-tile code cache instead of keeping both resident.
            let Some(psram_blocks) = psram.as_mut() else {
                uart_log(
                    &mut uart,
                    "phase=255 launch-memory-budget-error reason=shell-swap-requires-psram\r\n",
                );
                continue;
            };
            if psram_blocks
                .write(SHELL_SWAP_PSRAM_ADDR, shell_value_bytes(shell))
                .is_err()
            {
                uart_log(&mut uart, "phase=264 shell-swap-write-error\r\n");
                shell = shell_code_resident.shell_mut().unwrap();
                continue;
            }
            let code_window = shell_code_resident.begin_code().unwrap();
            run_device_app(
                &volume_mgr,
                package,
                system_config.snapshot(),
                app_ui_session,
                &mut psram,
                code_window,
                app_heap,
                manifest_fetch,
                #[cfg(all(
                    feature = "network_service",
                    any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"),
                    not(feature = "wifi_residency_probe")
                ))]
                &mut network_runtime,
                #[cfg(not(all(
                    feature = "network_service",
                    any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"),
                    not(feature = "wifi_residency_probe")
                )))]
                &mut app_background,
                lfn_storage,
                &mut keyboard,
                &mut lcd,
                &font,
                raster_strip,
                rgb666_strip,
                app_draw,
                app_static,
                app_static_shadow,
                &mut audio,
                app_host_skk,
                &mut uart,
            )
            .await;
            if !shell_code_resident.restore_shell_with(|dst| {
                psram
                    .as_mut()
                    .is_some_and(|blocks| blocks.read(SHELL_SWAP_PSRAM_ADDR, dst).is_ok())
            }) {
                uart_log(&mut uart, "phase=265 shell-swap-read-error fatal=1\r\n");
                loop {
                    Timer::after_secs(1).await;
                }
            }
            shell = shell_code_resident.shell_mut().unwrap();
            // KOTO-0170 Stage 0: the app session (VM + present path) is the
            // deepest expected user of the main stack, so sample right after it.
            stack_canary::emit_peak(&mut uart, "app-exit");
            held = HeldKeys::new();
            input = FirmwareInput::new();
            let repaint_started = Instant::now();
            let metrics = paint_shell(
                &mut lcd,
                shell,
                &font,
                raster_strip,
                rgb666_strip,
                &mut uart,
            )
            .await;
            log_paint_metrics(
                &mut uart,
                &mut line,
                "phase=30 ready app-return",
                metrics,
                repaint_started,
            );
            heartbeat = Instant::now();
            power_poll = Instant::now();
            continue;
        }
        let preferences_changed = command_acted || shell.detail_pane_visible() != previous_pane;
        // Opening/closing the system overlay (F5, or confirm/cancel dismissing it
        // inside `update`) swaps the whole body, so it needs a full redraw — but
        // it is not a persisted preference, so it stays out of the save path.
        let view_changed = shell.system_view_visible() != previous_system_view;
        if preferences_changed && shell.storage_status() == StorageStatus::Present && card_present {
            if shell.battery_is_low() {
                uart_log(&mut uart, "phase=144 low-battery-before-write\r\n");
            }
            if save_shell_prefs(&volume_mgr, shell, manifest_fetch.scratch(), &mut uart) {
                shell.set_save_status(SaveStatus::Saved);
            } else {
                shell.set_save_status(SaveStatus::Unsaved);
            }
        }
        // Device rendering currently presents the final state immediately.
        // KotoSim retains the multi-frame visual animations.
        for _ in 0..8 {
            shell.advance_feedback();
        }
        if preferences_changed || view_changed {
            uart_log(&mut uart, "phase=40 full-redraw\r\n");
            let metrics = paint_shell(
                &mut lcd,
                shell,
                &font,
                raster_strip,
                rgb666_strip,
                &mut uart,
            )
            .await;
            log_paint_metrics(
                &mut uart,
                &mut line,
                "phase=30 ready full",
                metrics,
                interaction_started,
            );
        } else if shell.selected_index() != previous {
            line.clear();
            let _ = write!(
                line,
                "phase=40 dirty-redraw-start selected={} input_count={}\r\n",
                shell.selected_index(),
                input_count
            );
            uart_write_line(&mut uart, &line);
            let metrics = paint_selection_change(
                &mut lcd,
                shell,
                &font,
                previous,
                raster_strip,
                rgb666_strip,
                &mut uart,
            )
            .await;
            log_paint_metrics(
                &mut uart,
                &mut line,
                "phase=30 ready dirty",
                metrics,
                interaction_started,
            );
        }
        if power_poll.elapsed().as_millis() >= POWER_POLL_MS {
            let previous_power = shell.power_state();
            // A transient I2C failure keeps the last displayed state. This
            // avoids turning a previously valid battery reading into
            // "unsupported" merely because one shared-bus poll failed.
            let current_power =
                poll_power_state(&mut keyboard, &mut uart).unwrap_or(previous_power);
            let changed = current_power != previous_power;
            line.clear();
            let _ = write!(
                line,
                "phase=145 power-poll changed={} state={:?}\r\n",
                changed, current_power
            );
            uart_write_line(&mut uart, &line);
            if changed {
                shell.set_power_state(current_power);
                let status_started = Instant::now();
                let metrics = paint_shell_rect_metrics(
                    &mut lcd,
                    shell,
                    &font,
                    raster_strip,
                    rgb666_strip,
                    SYSTEM_STATUS_RECT,
                    &mut uart,
                )
                .await;
                log_paint_metrics(
                    &mut uart,
                    &mut line,
                    "phase=30 ready status",
                    metrics,
                    status_started,
                );
            }
            power_poll = Instant::now();
        }
        if heartbeat.elapsed().as_millis() >= 1_000 {
            line.clear();
            let _ = write!(
                line,
                "phase=30 heartbeat selected={} input_count={}\r\n",
                shell.selected_index(),
                input_count
            );
            uart_write_line(&mut uart, &line);
            heartbeat = Instant::now();
            // KOTO-0170 Stage 0: sparse shell-side stack sample (every ~30 s of
            // shell time) so a session that ends browsing still lands its peak.
            stack_scan_beats = stack_scan_beats.wrapping_add(1);
            if stack_scan_beats.is_multiple_of(30) {
                stack_canary::emit_peak(&mut uart, "shell");
            }
        }

        let elapsed = frame_start.elapsed().as_millis();
        if elapsed < FRAME_PERIOD_MS {
            Timer::after_millis(FRAME_PERIOD_MS - elapsed).await;
        }
    }
}
