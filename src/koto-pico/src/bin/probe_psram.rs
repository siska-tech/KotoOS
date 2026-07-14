//! `probe_psram` — PSRAM round-trip probe (KOTO-0069).
//!
//! Writes a deterministic pattern to a PSRAM block over the PIO1 SPI path, reads
//! it back into a separate SRAM buffer, compares every byte, and checks that an
//! out-of-range block index is rejected. Timing and diagnostics are emitted over
//! UART0 (GP0, 115200 8N1). Validates the same PSRAM foundation the KOTO-0127
//! code-streaming launch path depends on.
//!
//! Not part of normal development: flash manually only to re-validate PSRAM. See
//! `docs/hardware/PICO_HARDWARE_LOG.md`.
#![no_std]
#![no_main]

use core::fmt::{self, Write};

use embassy_executor::Spawner;
use embassy_rp::{
    bind_interrupts,
    clocks::clk_sys_freq,
    pac::dma::vals::TreqSel,
    peripherals,
    pio::{InterruptHandler as PioInterruptHandler, Pio},
    uart::{Config as UartConfig, UartTx},
};
use embassy_time::{Instant, Timer};
use koto_core::{
    psram::{PsramBlocks, PsramCodeWindow, PsramError, PSRAM_BLOCK_SIZE},
    runtime::CodeSource,
};
use koto_pico::psram::{
    PicoCalcPsram, PSRAM_CAPACITY, PSRAM_PIO_CLOCK_DIVIDER, PSRAM_PIO_CYCLES_PER_BIT,
};
use panic_halt as _;

bind_interrupts!(struct Irqs {
    PIO1_IRQ_0 => PioInterruptHandler<peripherals::PIO1>;
});

const TEST_BLOCK: u32 = 257;
const LAST_BLOCK: u32 = PSRAM_CAPACITY / PSRAM_BLOCK_SIZE as u32 - 1;
const CODE_WINDOW_BASE: u32 = 1024 * PSRAM_BLOCK_SIZE as u32;
const BANNER: &[u8] = concat!(
    "KotoOS KOTO-0069 psram-roundtrip-uart v",
    env!("CARGO_PKG_VERSION"),
    " board=",
    env!("KOTO_BOARD_ID"),
    " mcu=",
    env!("KOTO_MCU_ID"),
    "\r\n"
)
.as_bytes();

#[embassy_executor::main(
    executor = "embassy_rp::executor::Executor",
    entry = "cortex_m_rt::entry"
)]
async fn main(_spawner: Spawner) {
    let p = koto_pico::board::split_peripherals(embassy_rp::init(Default::default()));

    // PicoCalc's mainboard Type-C connector exposes the RP2040's UART0 through
    // its USB-UART bridge. GP0 is TX; Tera Term should use 115200 8N1.
    let mut uart_config = UartConfig::default();
    uart_config.baudrate = 115_200;
    let mut uart = UartTx::new_blocking(p.uart, p.uart_tx, uart_config);
    let _ = uart.blocking_write(BANNER);
    let _ = uart.blocking_write(b"log=uart0 tx=GP0 baud=115200 format=8N1\r\n");
    for remaining in (1..=10).rev() {
        let mut countdown = LineBuffer::new();
        let _ = write!(countdown, "psram probe starts in {}s\r\n", remaining);
        write_line(&mut uart, &countdown);
        Timer::after_secs(1).await;
    }

    let mut pio = Pio::new(p.psram_pio, Irqs);
    let hal = PicoCalcPsram::new(
        &mut pio.common,
        pio.sm0,
        p.psram_cs,
        p.psram_sck,
        p.psram_sio0,
        p.psram_sio1,
    );
    let mut line = LineBuffer::new();
    let identity = hal.identity();
    let state = hal.diag_state();
    let identity_valid = identity.is_aps6404_8m();
    let system_hz = clk_sys_freq();
    let _ = write!(
        line,
        "psram identity raw={:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x} manufacturer=0x{:02x} kgd=0x{:02x} density=0x{:02x} valid={}\r\n",
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
        identity_valid,
    );
    write_line(&mut uart, &line);
    line.clear();
    let _ = write!(
        line,
        "psram timing sys_hz={} pio=PIO1 sm=0 clkdiv={}.{} sm_hz={} cycles_per_bit={} serial_hz={} dreq_rx={} dreq_tx={}\r\n",
        system_hz,
        state.clkdiv,
        state.clkdiv_frac,
        state.sm_hz,
        state.cycles_per_bit,
        state.sm_hz / u32::from(state.cycles_per_bit),
        TreqSel::PIO1_RX0 as u8,
        TreqSel::PIO1_TX0 as u8,
    );
    write_line(&mut uart, &line);

    if !identity_valid || cfg!(feature = "force_psram_fallback") {
        let _ = uart.blocking_write(
            b"psram fallback_gate=pass selected=sram-window reason=identity_or_forced_unavailable\r\n",
        );
        loop {
            Timer::after_secs(5).await;
            let _ = uart.blocking_write(BANNER);
            let _ = uart.blocking_write(b"KOTO-0205 fallback alive\r\n");
        }
    }

    let mut psram = PsramBlocks::try_new(hal, PSRAM_CAPACITY).unwrap();
    line.clear();
    let _ = write!(
        line,
        "psram capacity={} block_size={} blocks={} test_block={} pio=PIO1 configured_clkdiv={} expected_serial_hz={}\r\n",
        psram.capacity(),
        PSRAM_BLOCK_SIZE,
        psram.block_count(),
        TEST_BLOCK,
        PSRAM_PIO_CLOCK_DIVIDER,
        system_hz / PSRAM_PIO_CLOCK_DIVIDER / u32::from(PSRAM_PIO_CYCLES_PER_BIT),
    );
    write_line(&mut uart, &line);

    let mut source = [0u8; PSRAM_BLOCK_SIZE];
    let mut destination = [0u8; PSRAM_BLOCK_SIZE];
    for (index, byte) in source.iter_mut().enumerate() {
        *byte = (index as u8).wrapping_mul(29).wrapping_add(0xa5);
    }
    destination.fill(0);
    let boundary_write = psram.write_block(LAST_BLOCK, &source);
    let boundary_read = psram.read_block(LAST_BLOCK, &mut destination);
    let boundary_mismatch = source
        .iter()
        .zip(destination.iter())
        .position(|(expected, actual)| expected != actual);
    line.clear();
    let _ = write!(
        line,
        "psram boundary block={} address=0x{:06x} end=0x{:06x} write={:?} read={:?} verify={} mismatch={:?}\r\n",
        LAST_BLOCK,
        LAST_BLOCK * PSRAM_BLOCK_SIZE as u32,
        PSRAM_CAPACITY,
        boundary_write,
        boundary_read,
        if boundary_mismatch.is_none() { "pass" } else { "fail" },
        boundary_mismatch,
    );
    write_line(&mut uart, &line);

    let mut staged_code = [0u8; 32];
    for (index, byte) in staged_code.iter_mut().enumerate() {
        *byte = (index as u8).wrapping_mul(11).wrapping_add(7);
    }
    let stage_result = psram.write(CODE_WINDOW_BASE, &staged_code);
    let (code_window_pass, refills, refill_bytes, refill_us) = {
        let mut cache = [0u8; 16];
        let mut code = PsramCodeWindow::new_two_tile(
            &mut psram,
            &mut cache,
            CODE_WINDOW_BASE,
            (staged_code.len() / 4) as u32,
        );
        code.set_refill_clock(|| Instant::now().as_micros());
        let fetch_order = [0u32, 4, 1, 7, 0, 4];
        let mut pass = stage_result.is_ok();
        for index in fetch_order {
            let start = index as usize * 4;
            pass &= code.word(index) == staged_code[start..start + 4].try_into().ok();
        }
        (
            pass,
            code.fetch_refills(),
            code.cw_refill_bytes(),
            code.cw_refill_us_total(),
        )
    };
    line.clear();
    let _ = write!(
        line,
        "psram code_window stage={:?} verify={} shape=two_tile cache_bytes=16 code_bytes=32 refills={} refill_bytes={} refill_us={}\r\n",
        stage_result,
        if code_window_pass { "pass" } else { "fail" },
        refills,
        refill_bytes,
        refill_us,
    );
    write_line(&mut uart, &line);
    let _ = uart.blocking_write(
        b"psram fallback_gate=armed test=flash forced-fallback product artifact and confirm phase=198\r\n",
    );

    for (index, byte) in source.iter_mut().enumerate() {
        *byte = (index as u8).wrapping_mul(73).wrapping_add(0x5a);
    }
    destination.fill(0);

    let write_start = Instant::now();
    let write_result = psram.write_block(TEST_BLOCK, &source);
    let write_us = write_start.elapsed().as_micros();

    let read_start = Instant::now();
    let read_result = psram.read_block(TEST_BLOCK, &mut destination);
    let read_us = read_start.elapsed().as_micros();

    let mismatch = source
        .iter()
        .zip(destination.iter())
        .position(|(expected, actual)| expected != actual);
    let range_result = psram.read_block(psram.block_count(), &mut destination);

    line.clear();
    let _ = write!(
        line,
        "psram write={:?} write_us={} read={:?} read_us={} verify={} mismatch={:?}\r\n",
        write_result,
        write_us,
        read_result,
        read_us,
        if mismatch.is_none() { "pass" } else { "fail" },
        mismatch
    );
    write_line(&mut uart, &line);

    line.clear();
    let _ = line.write_str("psram expected=");
    write_hex(&mut line, &source[..16]);
    let _ = line.write_str(" actual=");
    write_hex(&mut line, &destination[..16]);
    let _ = line.write_str("\r\n");
    write_line(&mut uart, &line);

    line.clear();
    let _ = write!(
        line,
        "psram out_of_range={} expected={:?}\r\n",
        if range_result == Err(PsramError::OutOfRange) {
            "pass"
        } else {
            "fail"
        },
        PsramError::OutOfRange
    );
    write_line(&mut uart, &line);
    let _ = uart.blocking_write(b"KOTO-0069 awaiting observation\r\n");

    loop {
        Timer::after_secs(5).await;
        let _ = uart.blocking_write(BANNER);
        line.clear();
        let _ = write!(
            line,
            "psram write={:?} write_us={} read={:?} read_us={} verify={} mismatch={:?}\r\n",
            write_result,
            write_us,
            read_result,
            read_us,
            if mismatch.is_none() { "pass" } else { "fail" },
            mismatch
        );
        write_line(&mut uart, &line);
        line.clear();
        let _ = line.write_str("psram expected=");
        write_hex(&mut line, &source[..16]);
        let _ = line.write_str(" actual=");
        write_hex(&mut line, &destination[..16]);
        let _ = line.write_str("\r\n");
        write_line(&mut uart, &line);
        let _ = uart.blocking_write(b"KOTO-0069 alive\r\n");
    }
}

fn write_line(uart: &mut UartTx<'_, embassy_rp::uart::Blocking>, line: &LineBuffer) {
    let _ = uart.blocking_write(line.as_bytes());
}

fn write_hex(line: &mut LineBuffer, bytes: &[u8]) {
    for byte in bytes {
        let _ = write!(line, "{:02x}", byte);
    }
}

struct LineBuffer {
    bytes: [u8; 320],
    len: usize,
}

impl LineBuffer {
    const fn new() -> Self {
        Self {
            bytes: [0; 320],
            len: 0,
        }
    }

    fn clear(&mut self) {
        self.len = 0;
    }

    fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

impl fmt::Write for LineBuffer {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        if text.len() > self.bytes.len().saturating_sub(self.len) {
            return Err(fmt::Error);
        }
        self.bytes[self.len..self.len + text.len()].copy_from_slice(text.as_bytes());
        self.len += text.len();
        Ok(())
    }
}
