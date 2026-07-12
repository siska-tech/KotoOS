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
    bind_interrupts, peripherals,
    pio::{InterruptHandler as PioInterruptHandler, Pio},
    uart::{Config as UartConfig, UartTx},
};
use embassy_time::{Instant, Timer};
use koto_core::psram::{PsramBlocks, PsramError, PSRAM_BLOCK_SIZE};
use koto_pico::psram::{PicoCalcPsram, PSRAM_CAPACITY};
use panic_halt as _;

bind_interrupts!(struct Irqs {
    PIO1_IRQ_0 => PioInterruptHandler<peripherals::PIO1>;
});

const TEST_BLOCK: u32 = 257;
const BANNER: &[u8] = concat!(
    "KotoOS KOTO-0069 psram-roundtrip-uart v",
    env!("CARGO_PKG_VERSION"),
    "\r\n"
)
.as_bytes();

#[embassy_executor::main(
    executor = "embassy_rp::executor::Executor",
    entry = "cortex_m_rt::entry"
)]
async fn main(_spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    // PicoCalc's mainboard Type-C connector exposes the RP2040's UART0 through
    // its USB-UART bridge. GP0 is TX; Tera Term should use 115200 8N1.
    let mut uart_config = UartConfig::default();
    uart_config.baudrate = 115_200;
    let mut uart = UartTx::new_blocking(p.UART0, p.PIN_0, uart_config);
    Timer::after_secs(2).await;
    let _ = uart.blocking_write(BANNER);
    let _ = uart.blocking_write(b"log=uart0 tx=GP0 baud=115200 format=8N1\r\n");

    let mut pio = Pio::new(p.PIO1, Irqs);
    let hal = PicoCalcPsram::new(
        &mut pio.common,
        pio.sm0,
        p.PIN_20,
        p.PIN_21,
        p.PIN_2,
        p.PIN_3,
    );
    let mut psram = PsramBlocks::try_new(hal, PSRAM_CAPACITY).unwrap();
    let mut line = LineBuffer::new();
    let _ = write!(
        line,
        "psram capacity={} block_size={} blocks={} test_block={} pio=PIO1 clock_hz=16625000\r\n",
        psram.capacity(),
        PSRAM_BLOCK_SIZE,
        psram.block_count(),
        TEST_BLOCK
    );
    write_line(&mut uart, &line);

    let mut source = [0u8; PSRAM_BLOCK_SIZE];
    for (index, byte) in source.iter_mut().enumerate() {
        *byte = (index as u8).wrapping_mul(73).wrapping_add(0x5a);
    }
    let mut destination = [0u8; PSRAM_BLOCK_SIZE];

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
