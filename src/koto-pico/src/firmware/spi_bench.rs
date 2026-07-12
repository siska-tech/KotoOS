//! One-shot boot-time SPI present-path microbenches (KOTO-0174 re-investigation).
//!
//! Two open questions survived the KOTO-0174 close, both answerable with
//! observe-only device measurements:
//!
//! - **(a) `phase=179 spi-rate`** — the measured present DMA runs ~0.55 µs/px
//!   against a 62.5 MHz wire floor of ~0.38 µs/px (~1.45×). Timing
//!   `transfer_rgb666` at 1/2/4/8/16 rows (plus the `begin` window-setup
//!   prologue alone) decomposes that gap into a per-transfer fixed cost `a`
//!   and a per-byte rate `b` (`t ≈ a + b·bytes`). If `b` lands at the nominal
//!   0.128 µs/byte, the wire is fine and the gap is per-strip overhead
//!   (bigger strips / chained DMA recover it); if `b` itself is high, the SPI
//!   clock or inter-byte gaps are the problem.
//! - **(b) `phase=178 spi-overlap`** — the H-A pipeline was reverted because
//!   present wall time never dropped, i.e. the CPU work joined against
//!   `Spi::write().await` did not visibly run during the DMA. This bench
//!   reproduces the exact H-A shape in isolation: open the window
//!   (`begin_rgb666`, awaited to completion), then race one full-strip data
//!   DMA against calibrated CPU work under `embassy_futures::join`.
//!   `overlap_us = dma_us + cpu_us - join_us` ≈ `min(dma_us, cpu_us)` means
//!   embassy does yield during the data DMA (H-A failed for structural
//!   reasons and could be retried); ≈ 0 means the executor really is held
//!   for the duration (H-A is dead on this HAL, as concluded).
//!
//! Gated on `DiagClass::Gfx` (compile-time const, dead code in shipping
//! profiles), runs once at boot right before the pixel-blit diagnostic, and
//! paints only black strips into the top of the boot test screen, which the
//! first shell render overwrites moments later.

use core::fmt::Write;

use embassy_futures::join::join;
use embassy_rp::uart::UartTx;
use embassy_time::Instant;

use crate::dashboard::LineBuffer;
use crate::lcd::PicoCalcLcd;

use super::config::{DiagClass, DIAG_PROFILE, RASTER_STRIP_BYTES, RGB666_STRIP_BYTES};
use super::diag::{uart_log, uart_write_line};

/// Repetitions per measured point; means are reported.
const REPS: u32 = 8;

/// Read-only CPU work standing in for the pipeline's raster+convert: a
/// wrapping checksum over the RGB565 strip. `inline(never)` + the folded
/// result flowing into the UART line keep it from being optimized away. It
/// touches only `strip`, so it cannot alias the RGB666 bytes the DMA reads.
#[inline(never)]
fn checksum_passes(data: &[u8], passes: u32) -> u32 {
    let mut sum = 0u32;
    let mut pass = 0u32;
    while pass < passes {
        for chunk in data.chunks_exact(4) {
            let word = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            sum = sum.wrapping_add(word ^ pass).rotate_left(1);
        }
        pass += 1;
    }
    sum
}

/// Run both microbenches and emit their `phase=178`/`phase=179` lines.
/// Observe-only apart from black pixels in the boot test screen.
pub async fn run_spi_present_bench(
    lcd: &mut PicoCalcLcd<'_>,
    strip: &mut [u8; RASTER_STRIP_BYTES],
    scratch: &mut [u8; RGB666_STRIP_BYTES],
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
) {
    if !DIAG_PROFILE.enables(DiagClass::Gfx) {
        return;
    }
    scratch.fill(0);
    let mut line = LineBuffer::new();

    // ---- (a) phase=179: fixed-vs-per-byte decomposition ----
    // The window-setup prologue alone (CASET/PASET/RAMWR + CS/DC + their
    // awaits): the per-transfer cost that is there even for zero data bytes.
    let mut begin_total_us = 0u64;
    for _ in 0..REPS {
        let started = Instant::now();
        if lcd.begin_rgb666(0, 0, 320, 16).await.is_err() {
            uart_log(uart, "phase=179 spi-rate error=begin\r\n");
            return;
        }
        begin_total_us += started.elapsed().as_micros();
        lcd.end_rgb666();
    }
    line.clear();
    let _ = write!(
        line,
        "phase=179 spi-rate begin reps={} mean_us={}\r\n",
        REPS,
        begin_total_us / u64::from(REPS),
    );
    uart_write_line(uart, &line);

    for rows in [1u16, 2, 4, 8, 16] {
        let bytes = 320 * rows as usize * 3;
        let mut total_us = 0u64;
        for _ in 0..REPS {
            let started = Instant::now();
            if lcd
                .transfer_rgb666(0, 0, 320, rows, &scratch[..bytes])
                .await
                .is_err()
            {
                uart_log(uart, "phase=179 spi-rate error=transfer\r\n");
                return;
            }
            total_us += started.elapsed().as_micros();
        }
        line.clear();
        let _ = write!(
            line,
            "phase=179 spi-rate rows={} bytes={} reps={} mean_us={}\r\n",
            rows,
            bytes,
            REPS,
            total_us / u64::from(REPS),
        );
        uart_write_line(uart, &line);
    }

    // ---- (b) phase=178: does the data DMA yield the CPU? ----
    // Data-phase DMA alone (window already open — the H-A shape).
    let bytes = RGB666_STRIP_BYTES;
    let mut dma_total_us = 0u64;
    for _ in 0..REPS {
        if lcd.begin_rgb666(0, 0, 320, 16).await.is_err() {
            uart_log(uart, "phase=178 spi-overlap error=begin\r\n");
            return;
        }
        let started = Instant::now();
        if lcd.write_rgb666_data(&scratch[..bytes]).await.is_err() {
            uart_log(uart, "phase=178 spi-overlap error=data\r\n");
            return;
        }
        dma_total_us += started.elapsed().as_micros();
        lcd.end_rgb666();
    }
    let dma_us = dma_total_us / u64::from(REPS);

    // Calibrate the CPU work to roughly one DMA's worth of time, so the
    // overlap (if any) is unmistakable in the means.
    let started = Instant::now();
    let mut sum = checksum_passes(&strip[..], 1);
    let one_pass_us = started.elapsed().as_micros().max(1);
    let passes = (dma_us / one_pass_us).clamp(1, 256) as u32;
    let mut cpu_total_us = 0u64;
    for _ in 0..REPS {
        let started = Instant::now();
        sum ^= checksum_passes(&strip[..], passes);
        cpu_total_us += started.elapsed().as_micros();
    }
    let cpu_us = cpu_total_us / u64::from(REPS);

    // The race: one poll starts the full-buffer data DMA and returns Pending;
    // the CPU future then runs its checksum to completion inside the same
    // join poll — exactly how H-A overlapped raster+convert under the
    // in-flight transfer. Wall time tells whether the DMA made progress
    // underneath.
    let mut join_total_us = 0u64;
    for _ in 0..REPS {
        if lcd.begin_rgb666(0, 0, 320, 16).await.is_err() {
            uart_log(uart, "phase=178 spi-overlap error=begin\r\n");
            return;
        }
        let started = Instant::now();
        let strip_ref: &[u8] = &strip[..];
        let (write_result, join_sum) = join(lcd.write_rgb666_data(&scratch[..bytes]), async {
            checksum_passes(strip_ref, passes)
        })
        .await;
        join_total_us += started.elapsed().as_micros();
        sum ^= join_sum;
        lcd.end_rgb666();
        if write_result.is_err() {
            uart_log(uart, "phase=178 spi-overlap error=join-data\r\n");
            return;
        }
    }
    let join_us = join_total_us / u64::from(REPS);

    line.clear();
    let _ = write!(
        line,
        "phase=178 spi-overlap bytes={} reps={} passes={} dma_us={} cpu_us={} join_us={} overlap_us={} sum=0x{:08x}\r\n",
        bytes,
        REPS,
        passes,
        dma_us,
        cpu_us,
        join_us,
        (dma_us + cpu_us).saturating_sub(join_us),
        sum,
    );
    uart_write_line(uart, &line);
}
