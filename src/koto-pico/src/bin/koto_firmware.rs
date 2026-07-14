#![no_std]
#![no_main]

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
use koto_core::{
    BitmapFont, BootSplash, BootStep, BootStepStatus, PowerState, ShellAction, ShellState,
    MAX_PACKAGES,
};
use koto_pico::firmware::app_host::{AppStaticLayer, DeviceRuntimeHost, StaticLayerShadow};
use koto_pico::firmware::app_render::present_pixel_diagnostic;
use koto_pico::firmware::app_runtime::{read_event, run_device_app};
use koto_pico::firmware::audio::{PicoAudioBackend, AUDIO_CORE1_STACK_BYTES};
use koto_pico::firmware::config::{
    FirmwareClock, CODE_WINDOW_TILES, CODE_WINDOW_TOTAL_BYTES, KICON_BYTES, MANIFEST_LFN_BYTES,
    MAX_DEVICE_HEAP_BYTES, MAX_EVENTS_PER_FRAME, MAX_MANIFEST_BYTES, POWER_POLL_MS,
    RASTER_STRIP_BYTES, RGB666_STRIP_BYTES, SD_ACQUIRE_SPI_HZ, SYSTEM_STATUS_RECT,
};
use koto_pico::firmware::diag::{log_paint_metrics, uart_log, uart_write_line};
use koto_pico::firmware::power::poll_power_state;
use koto_pico::firmware::shell_prefs::{apply_shell_prefs, save_shell_prefs};
use koto_pico::firmware::shell_render::{
    paint_selection_change, paint_shell, paint_shell_rect_metrics,
};
use koto_pico::firmware::spi_bench::run_spi_present_bench;
use koto_pico::firmware::splash_render::{paint_splash, paint_splash_step};
use koto_pico::firmware::stack_canary;
use koto_pico::firmware::storage::{fill_fallback, initialize_sd_card, load_packages};
use koto_pico::{
    board::{BOARD_ID, MCU_ID},
    dashboard::LineBuffer,
    firmware::FirmwareInput,
    keyboard::{
        HeldKeys, FIFO_CAPACITY, FRAME_PERIOD_MS, KEY_F2, KEY_F3, KEY_F4, KEY_F5, KEY_STATE_PRESSED,
    },
    lcd::{PicoCalcLcd, ILI9488_SPI},
    psram::PSRAM_CAPACITY,
};
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
use panic_halt as _;
use static_cell::ConstStaticCell;

// The fast CodeWindow refill (KOTO-0153) reserves DMA_CH1 for `koto-psram`'s
// validated RX-DMA read, so its interrupt handler is bound alongside the LCD
// SPI's DMA_CH0 handler. Both share DMA_IRQ_0; the fast read arms CH1 with
// `irq_quiet`, so the extra handler never fires in either build.
#[cfg(not(feature = "psram_fast_code_window"))]
bind_interrupts!(struct Irqs {
    DMA_IRQ_0 => dma::InterruptHandler<peripherals::DMA_CH0>;
    PIO1_IRQ_0 => PioInterruptHandler<peripherals::PIO1>;
});

#[cfg(feature = "psram_fast_code_window")]
bind_interrupts!(struct Irqs {
    DMA_IRQ_0 => dma::InterruptHandler<peripherals::DMA_CH0>, dma::InterruptHandler<peripherals::DMA_CH1>;
    PIO1_IRQ_0 => PioInterruptHandler<peripherals::PIO1>;
});

const FONT_BYTES: &[u8] = include_bytes!("../../../../assets/fonts/mplus12.kfont");

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
static MANIFEST_BYTES: ConstStaticCell<[u8; MAX_MANIFEST_BYTES]> =
    ConstStaticCell::new([0; MAX_MANIFEST_BYTES]);
static MANIFEST_LFN: ConstStaticCell<[u8; MANIFEST_LFN_BYTES]> =
    ConstStaticCell::new([0; MANIFEST_LFN_BYTES]);
// One bounded, sequential `.kicon` read at a time (KOTO-0122).
static KICON_SCRATCH: ConstStaticCell<[u8; KICON_BYTES]> = ConstStaticCell::new([0; KICON_BYTES]);
// SRAM working set over PSRAM-resident code (KOTO-0127): two resident 16 KiB
// tiles with MRU/LRU replacement since KOTO-0173 (a deliberate +16 KiB spend
// against the KOTO-0172 margin), reused across launches.
static CODE_WINDOW: ConstStaticCell<[u8; CODE_WINDOW_TOTAL_BYTES]> =
    ConstStaticCell::new([0; CODE_WINDOW_TOTAL_BYTES]);
static APP_HEAP: ConstStaticCell<[u8; MAX_DEVICE_HEAP_BYTES]> =
    ConstStaticCell::new([0; MAX_DEVICE_HEAP_BYTES]);
// Current + previous frame app draw-command lists. Held here (not as locals in the
// async run loop) so these two ~30 KiB buffers stay out of the embassy main-task
// future, which was ~128 KiB largely because of them (KOTO-0134).
static APP_DRAW: ConstStaticCell<[DeviceRuntimeHost; 2]> =
    ConstStaticCell::new([DeviceRuntimeHost::new(), DeviceRuntimeHost::new()]);
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
// The portable shell model (catalog + view state; ~28 KiB, dominated by the
// 32-slot PackageList at 876 B/package). Const-initialized here for the same
// reason as the cells above: `ShellState::new(packages)` used to build the
// whole value — plus the PackageList moved into it — inside the main task's
// poll frame, and that single slot was ~28 KiB of the measured 39.9 KiB frame
// (KOTO-0172). The catalog is loaded in place via `ShellState::reload_packages`.
static SHELL: ConstStaticCell<ShellState> = ConstStaticCell::new(ShellState::empty());
static mut AUDIO_CORE1_STACK: Stack<AUDIO_CORE1_STACK_BYTES> = Stack::new();

#[embassy_executor::main(
    executor = "embassy_rp::executor::Executor",
    entry = "cortex_m_rt::entry"
)]
async fn main(_spawner: Spawner) {
    // KOTO-0170 Stage 0: paint the free RAM gap before anything with a deep
    // call tree runs, so the phase=176 low-water scans measure the whole boot.
    stack_canary::paint();
    let p = koto_pico::board::split_peripherals(embassy_rp::init(Default::default()));

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
    let mut audio = PicoAudioBackend::spawn_cpu1(
        p.core1,
        unsafe { &mut *addr_of_mut!(AUDIO_CORE1_STACK) },
        audio_pwm,
    );
    // PicoCalc SD detect is active-low on GP22. Removal is supported as a
    // fail-safe transition; reinsertion requires reboot/remount.
    let sd_detect = Input::new(p.sd_detect, Pull::Up);

    let mut sd_spi_config = SpiConfig::default();
    sd_spi_config.frequency = SD_ACQUIRE_SPI_HZ;
    let sd_spi = Spi::new_blocking(p.sd_spi, p.sd_sck, p.sd_mosi, p.sd_miso, sd_spi_config);
    let sd_device = ExclusiveDevice::new(sd_spi, Output::new(p.sd_cs, Level::High), Delay).unwrap();
    let sdcard = SdCard::new(sd_device, Delay);

    // Bring up the PicoCalc PSRAM (PIO1, GP20/21/2/3) so large apps can stage
    // their code off-SRAM and run through a small SRAM window (KOTO-0127). It is
    // optional: if it cannot be created the launch path falls back to running
    // small apps directly from the SRAM window.
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
            "phase=198 psram-unavailable fallback=sram-window\r\n",
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
    let app_static = APP_STATIC.take();
    let app_static_shadow = APP_STATIC_SHADOW.take();
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
    let manifest = MANIFEST_BYTES.take();
    let lfn_storage = MANIFEST_LFN.take();
    let kicon = KICON_SCRATCH.take();
    let code_window = CODE_WINDOW.take();
    let app_heap = APP_HEAP.take();
    let active_sd_hz = initialize_sd_card(&sdcard, &mut uart);
    let volume_mgr = VolumeManager::new(sdcard, FirmwareClock);
    // The catalog is loaded *in place* inside the static `SHELL` (KOTO-0172):
    // no `PackageList`-sized local and no `ShellState::new(packages)` move, so
    // the ~28 KiB slot they cost the main task's poll frame is gone.
    let shell = SHELL.take();
    let (storage_status, package_count) = shell.reload_packages(|packages| {
        let status = if active_sd_hz.is_some() {
            load_packages(
                &volume_mgr,
                packages,
                names,
                manifest,
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
        apply_shell_prefs(&volume_mgr, shell, manifest, &mut uart);
    }
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
                        if matches!(event.key, KEY_F2 | KEY_F3 | KEY_F4 | KEY_F5) {
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
        let command_acted = match command_key {
            Some(KEY_F2) => {
                shell.toggle_selected_favorite();
                true
            }
            Some(KEY_F3) => {
                shell.cycle_sort();
                true
            }
            Some(KEY_F4) => {
                shell.cycle_category();
                true
            }
            Some(KEY_F5) => {
                shell.toggle_system_view();
                false
            }
            _ => false,
        };
        let action = shell.update(&input.sample(&held, latest));
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
            run_device_app(
                &volume_mgr,
                package,
                &mut psram,
                code_window,
                app_heap,
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
                &mut uart,
            )
            .await;
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
            if save_shell_prefs(&volume_mgr, shell, manifest, &mut uart) {
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
