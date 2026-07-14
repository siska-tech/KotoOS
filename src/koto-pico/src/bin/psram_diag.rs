//! `psram_diag` — KOTO-0132 phase 2 PSRAM correctness diagnostic.
//!
//! Device-only diagnostic: writes known patterns to a safe PSRAM block, reads
//! them back through the `PsramBlocks` path, and logs pass/fail plus the first
//! mismatch offset (if any). Patterns: incrementing byte, alternating 0x00/0xFF,
//! address-derived byte. Uses UART0 (GP0) for logs. Does not change PSRAM
//! transfer chunk sizes or implement DMA/QPI — it exercises the production
//! `PsramHal`/`PsramBlocks` path only.
#![no_std]
#![no_main]
#![cfg_attr(feature = "psram_qpi_backend_diag", allow(dead_code))]

use core::fmt::{self, Write};

use embassy_executor::Spawner;
use embassy_rp::{
    bind_interrupts, peripherals,
    pio::{InterruptHandler as PioInterruptHandler, Pio},
    uart::{Config as UartConfig, UartTx},
};
#[cfg(feature = "psram_qpi_backend_diag")]
use embassy_time::{block_for, Duration};
use embassy_time::{Instant, Timer};
use koto_core::hal::PsramHal;
use koto_core::psram::PSRAM_BLOCK_SIZE;
use koto_pico::psram::{
    PicoCalcPsram, PicoCalcPsramDiagState, PSRAM_CAPACITY, PSRAM_FAST_READ_DUMMY_CYCLES,
    PSRAM_PROD_READ_CHUNK_BYTES,
};
#[cfg(all(feature = "psram_qpi_backend_diag", feature = "psram_qpi_backend_v2"))]
use koto_pico::psram::{
    PicoCalcPsramQpiV2, PsramMode, PSRAM_QPI_V2_CHUNK_BYTES, PSRAM_QPI_V2_READ_CLOCK_DIVIDER,
    PSRAM_QPI_V2_WRITE_CLOCK_DIVIDER,
};
#[cfg(feature = "psram_qpi_backend_diag")]
use koto_pico::psram::{
    PicoCalcQpiPsram, PSRAM_QPI_CHUNK_BYTES, PSRAM_QPI_CLOCK_DIVIDER, PSRAM_QPI_DUMMY_NIBBLES,
    PSRAM_QPI_EXT_READ_MAX_CHUNK_BYTES, PSRAM_QPI_LARGE_CHUNK_BYTES, PSRAM_QPI_SM_HZ,
};
use panic_halt as _;
use static_cell::StaticCell;

#[cfg(feature = "psram_pio_word_diag")]
use embassy_rp::pio::{Config as PioConfig, FifoJoin, LoadedProgram, ShiftDirection, StateMachine};
#[cfg(feature = "psram_pio_word_diag")]
use embassy_rp::uart::Blocking;

// Diagnostic transfer modes. `prod16` is the only mode enabled by default.
#[allow(dead_code)]
enum TransferMode {
    Prod16,
    PioWord16,
    Dma24Reserved,
    Dma32Reserved,
    QpiCpu64,
    QpiRxDma64,
    QpiCpu120,
    QpiRxDma120,
}

const _DEFAULT_MODE: TransferMode = TransferMode::Prod16;

bind_interrupts!(struct Irqs {
    PIO1_IRQ_0 => PioInterruptHandler<peripherals::PIO1>;
});

const TEST_BLOCK: u32 = 257; // chosen scratch block well inside capacity
const PSRAM_BENCH_BASE_BLOCK: u32 = 512;
const PSRAM_BENCH_CHUNK_BYTES: usize = 4096;
const PSRAM_BENCH_TOTAL_BYTES: usize = 64 * 1024;
const PSRAM_BENCH_SIZES: [usize; 5] = [256, 1024, 4096, 16 * 1024, 64 * 1024];
const PSRAM_BENCH_SMALL_SIZES: [usize; 4] = [16, 32, 48, 64];
const PSRAM_BENCH_STABLE_CLKDIVS: [u32; 2] = [4, 3];
const PSRAM_BENCH_CLKDIV_FRAC_2P5_INT: u16 = 2;
const PSRAM_BENCH_CLKDIV_FRAC_2P5_FRAC: u8 = 128;
const PSRAM_BENCH_DUMMY_BITS_BASELINE: u8 = PSRAM_FAST_READ_DUMMY_CYCLES;
const PSRAM_BENCH_DUMMY_BITS_PLUS1: u8 = PSRAM_FAST_READ_DUMMY_CYCLES + 1;
const PSRAM_BENCH_CLKDIV2P5_DUMMY_SWEEP: [u8; 4] = [8, 9, 10, 12];
const PSRAM_BENCH_CLKDIV2P5_INTER_TX_DELAY_US: [u16; 2] = [0, 1];
const RUN_HAZARDOUS_BENCH_ROWS: bool = false;
#[cfg(feature = "psram_qpi_backend_diag")]
const PSRAM_QPI_CLKDIV_SWEEP: [u32; 6] = [16, 12, 10, 8, 6, 4];
#[cfg(feature = "psram_qpi_backend_diag")]
const RUN_QPI_EXT_READ_PROTOTYPE: bool = false;
#[cfg(feature = "psram_qpi_backend_diag")]
const PSRAM_QPI_EXT_READ_CLKDIVS: [u32; 4] = [16, 12, 10, 8];
#[cfg(feature = "psram_qpi_backend_diag")]
const PSRAM_QPI_EXT_READ_CHUNKS: [usize; 3] = [512, 1024, PSRAM_QPI_EXT_READ_MAX_CHUNK_BYTES];
#[cfg(feature = "psram_qpi_backend_diag")]
const PSRAM_QPI_STRESS_ITERATIONS: usize = 10;
#[cfg(feature = "psram_qpi_backend_diag")]
const PSRAM_QPI_STRESS_BASELINE_CLKDIV: u32 = 4;
#[cfg(feature = "psram_qpi_backend_diag")]
const PSRAM_QPI_STRESS_READ6_CLKDIV: u32 = 6;
#[cfg(feature = "psram_qpi_backend_diag")]
const PSRAM_QPI_STRESS_SAFE_READ_CLKDIV: u32 = 8;
#[cfg(feature = "psram_qpi_backend_diag")]
const PSRAM_QPI_STRESS_FAST_WRITE4_CLKDIV: u32 = 2;
#[cfg(feature = "psram_qpi_backend_diag")]
const PSRAM_QPI_STRESS_ADDRS: [u32; 5] = [
    0x0001_0100,
    0x0002_0000,
    0x0010_0000,
    0x0030_0000,
    0x0070_0000,
];
#[cfg(feature = "psram_qpi_backend_diag")]
const PSRAM_QPI_WRITE_READ_DELAYS_US: [u32; 7] = [0, 1, 2, 5, 10, 20, 50];
#[cfg(feature = "psram_qpi_backend_diag")]
const PSRAM_QPI_WRITE_READ_DELAYS_NONE: [u32; 1] = [0];
#[cfg(feature = "psram_qpi_backend_diag")]
const PSRAM_QPI_WRITE_DIAG_BASE_ADDR: u32 = 0x0004_0000;
#[cfg(feature = "psram_qpi_backend_diag")]
const PSRAM_QPI_WRITE_DIAG_READ_CLKDIV: u32 = 8;
#[cfg(feature = "psram_qpi_backend_diag")]
const PSRAM_QPI_WRITE_DIAG_RW_CLKDIVS: [u32; 3] = [8, 6, 4];
#[cfg(feature = "psram_qpi_backend_diag")]
const PSRAM_QPI_WRITE_DIAG_4WIRE_CLKDIVS: [u32; 4] = [8, 6, 4, 2];
#[cfg(feature = "psram_qpi_backend_diag")]
const PSRAM_QPI_WRITE_DIAG_SIZES: [usize; 8] = [1, 2, 4, 8, 16, 64, 120, 256];
#[cfg(all(feature = "psram_qpi_backend_diag", feature = "psram_qpi_backend_v2"))]
const PSRAM_QPI_V2_APP_STAGE_BYTES: usize = 27_004;
#[cfg(all(feature = "psram_qpi_backend_diag", feature = "psram_qpi_backend_v2"))]
const PSRAM_QPI_V2_STAGE_HEAD_BYTES: usize = 16 * 1024;
#[cfg(all(feature = "psram_qpi_backend_diag", feature = "psram_qpi_backend_v2"))]
const PSRAM_QPI_V2_STAGE_TAIL_BYTES: usize =
    PSRAM_QPI_V2_APP_STAGE_BYTES - PSRAM_QPI_V2_STAGE_HEAD_BYTES;
#[cfg(all(feature = "psram_qpi_backend_diag", feature = "psram_qpi_backend_v2"))]
const PSRAM_QPI_V2_TILE_BYTES: usize = 16 * 1024;
#[cfg(all(feature = "psram_qpi_backend_diag", feature = "psram_qpi_backend_v2"))]
const PSRAM_QPI_V2_TRANSITION_BYTES: usize = 1024;
static PSRAM_BENCH_SRC: StaticCell<[u8; PSRAM_BENCH_CHUNK_BYTES]> = StaticCell::new();
static PSRAM_BENCH_DST: StaticCell<[u8; PSRAM_BENCH_CHUNK_BYTES]> = StaticCell::new();
#[cfg(feature = "psram_pio_word_diag")]
const RUN_LEGACY_DMA_DIAGS: bool = false;
#[cfg(feature = "psram_pio_word_diag")]
const RUN_RETIRED_SM1_COMPARE: bool = false;
// Verbose read-validation suites are intentionally disabled by default to keep
// `psram_diag` smoke output concise; enable ad hoc when collecting deep traces.
#[cfg(feature = "psram_pio_word_diag")]
const RUN_READ_LADDER_DIAG: bool = false;
#[cfg(feature = "psram_pio_word_diag")]
const RUN_BOUNDARY_DIAG: bool = false;
#[cfg(feature = "psram_pio_word_diag")]
const RUN_SMALL_READ_DIAG: bool = false;
#[cfg(feature = "psram_pio_word_diag")]
const RUN_STRESS_DIAG: bool = false;
#[cfg(feature = "psram_diag_hazardous_bench")]
const RUN_PHASE_EDGE_EXPERIMENTS: bool = true;
#[cfg(feature = "psram_pio_word_diag")]
const VERIFIED_READ_LADDER_SIZES: [usize; 9] = [16, 32, 64, 128, 256, 512, 1024, 2048, 4096];
#[cfg(feature = "psram_pio_word_diag")]
const VERIFIED_BOUNDARY_READ_LEN: usize = 256;
#[cfg(feature = "psram_pio_word_diag")]
const VERIFIED_SMALL_READ_LENS: [usize; 10] = [1, 2, 3, 4, 5, 7, 8, 12, 15, 16];
#[cfg(feature = "psram_pio_word_diag")]
const VERIFIED_SMALL_READ_ADDRS: [u32; 5] = [0, 1, 15, 16, 257];
const BANNER: &[u8] = concat!(
    "KotoOS KOTO-0132 psram-diag v",
    env!("CARGO_PKG_VERSION"),
    " board=",
    env!("KOTO_BOARD_ID"),
    " mcu=",
    env!("KOTO_MCU_ID"),
    "\r\n"
)
.as_bytes();

#[cfg(feature = "psram_pio_word_diag")]
#[derive(Clone, Copy)]
enum ReadDiagPattern {
    Incrementing,
    AddressDerived,
}

#[cfg(feature = "psram_pio_word_diag")]
fn pattern_label(pattern: ReadDiagPattern) -> &'static str {
    match pattern {
        ReadDiagPattern::Incrementing => "incrementing",
        ReadDiagPattern::AddressDerived => "address_derived",
    }
}

#[cfg(feature = "psram_pio_word_diag")]
fn pattern_byte(pattern: ReadDiagPattern, base_address: u32, offset: usize) -> u8 {
    match pattern {
        ReadDiagPattern::Incrementing => offset as u8,
        ReadDiagPattern::AddressDerived => {
            let addr = base_address.wrapping_add(offset as u32);
            (addr.wrapping_mul(37).wrapping_add(13) & 0xff) as u8
        }
    }
}

fn psram_bench_byte(address: u32) -> u8 {
    (address.wrapping_mul(37).wrapping_add(13) & 0xff) as u8
}

fn fill_psram_bench_pattern(buf: &mut [u8], base_address: u32) {
    for (offset, byte) in buf.iter_mut().enumerate() {
        *byte = psram_bench_byte(base_address.wrapping_add(offset as u32));
    }
}

fn verify_psram_bench_pattern(buf: &[u8], base_address: u32) -> bool {
    buf.iter().enumerate().all(|(offset, actual)| {
        *actual == psram_bench_byte(base_address.wrapping_add(offset as u32))
    })
}

fn first_psram_bench_mismatch(buf: &[u8], base_address: u32) -> Option<(usize, u8, u8)> {
    for (offset, actual) in buf.iter().enumerate() {
        let expected = psram_bench_byte(base_address.wrapping_add(offset as u32));
        if *actual != expected {
            return Some((offset, expected, *actual));
        }
    }
    None
}

fn classify_verify_failure(mismatch_off: usize, expected: u8, actual: u8) -> &'static str {
    if mismatch_off == 0 && (expected >> 1) == actual {
        "verify_shift_r1"
    } else if mismatch_off == 0 && (actual >> 1) == expected {
        "verify_shift_l1"
    } else {
        "verify_fail"
    }
}

struct BenchResult {
    elapsed_us: u64,
    done_bytes: usize,
    ok: bool,
    fail_reason: &'static str,
    fail_offset: Option<usize>,
    fail_expected: u8,
    fail_actual: u8,
}

fn log_psram_bench(
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
    line: &mut LineBuffer,
    mode: &str,
    bytes: usize,
    done_bytes: usize,
    elapsed_us: u64,
    chunk: usize,
    txns_done: usize,
    dummy_bits: u8,
    inter_tx_delay_us: u16,
    state: PicoCalcPsramDiagState,
    ok: bool,
    fail_reason: &str,
    fail_offset: Option<usize>,
    fail_expected: u8,
    fail_actual: u8,
) {
    let milli_mb_s = if elapsed_us == 0 {
        0
    } else {
        (done_bytes as u64)
            .saturating_mul(1_000_000)
            .saturating_mul(1000)
            .saturating_div(elapsed_us)
            .saturating_div(1_000_000)
    };
    line.clear();
    let _ = write!(
        line,
        "phase=334 psram-bench mode={} bytes={} done_bytes={} elapsed_us={} mb_s={}.{:03} chunk={} txns_done={} txn_bytes={} clkdiv={}.{:03} sm_hz={} cycles_per_bit={} dummy={} inter_tx_delay_us={} rx_autopush={} rx_fjoin={} rx_thresh={} tx_autopull={} tx_fjoin={} tx_thresh={} input_sync_bypass={} flevel=0x{:08x} fstat=0x{:08x} fdebug=0x{:08x} ok={} fail={} fail_off={} fail_exp=0x{:02x} fail_got=0x{:02x}\r\n",
        mode,
        bytes,
        done_bytes,
        elapsed_us,
        milli_mb_s / 1000,
        milli_mb_s % 1000,
        chunk,
        txns_done,
        chunk,
        state.clkdiv,
        ((u32::from(state.clkdiv_frac) * 1000) / 256),
        state.sm_hz,
        state.cycles_per_bit,
        dummy_bits,
        inter_tx_delay_us,
        state.rx_autopush,
        state.rx_fjoin,
        state.rx_threshold,
        state.tx_autopull,
        state.tx_fjoin,
        state.tx_threshold,
        if state.qpi_input_sync_bypass { 1 } else { 0 },
        state.flevel,
        state.fstat,
        state.fdebug,
        if ok { 1 } else { 0 },
        fail_reason,
        fail_offset.map(|v| v as i32).unwrap_or(-1),
        fail_expected,
        fail_actual,
    );
    let _ = uart.blocking_write(line.as_bytes());
}

fn psram_bench_txns(bytes: usize) -> usize {
    bytes.div_ceil(PSRAM_PROD_READ_CHUNK_BYTES)
}

#[cfg(feature = "psram_diag_hazardous_bench")]
fn phase_edge_experiments_enabled() -> bool {
    RUN_PHASE_EDGE_EXPERIMENTS
}

fn hazardous_bench_enabled() -> bool {
    cfg!(feature = "psram_diag_hazardous_bench") && RUN_HAZARDOUS_BENCH_ROWS
}

#[cfg(feature = "psram_qpi_backend_diag")]
fn log_qpi_init(
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
    line: &mut LineBuffer,
    ok: bool,
) {
    line.clear();
    let _ = write!(
        line,
        "phase=340 psram-qpi-init pins=sio0-3:2,3,4,5 cs=20 sck=21 pio=PIO1 sm=0 clkdiv={} enter_qpi={} input_sync_bypass={}\r\n",
        PSRAM_QPI_CLOCK_DIVIDER,
        if ok { "ok" } else { "fail" },
        if ok { 1 } else { 0 }
    );
    let _ = uart.blocking_write(line.as_bytes());
}

#[cfg(feature = "psram_qpi_backend_diag")]
fn run_qpi_roundtrip(
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
    line: &mut LineBuffer,
    hal: &mut PicoCalcQpiPsram<'_>,
    src: &mut [u8; PSRAM_BLOCK_SIZE],
    dst: &mut [u8; PSRAM_BLOCK_SIZE],
) -> bool {
    let addr = TEST_BLOCK * PSRAM_BLOCK_SIZE as u32;
    fill_psram_bench_pattern(src, addr);
    dst.fill(0);

    let write_ok = hal.write(addr, src).is_ok();
    let read_ok = write_ok && hal.read(addr, dst).is_ok();
    let mismatch = if read_ok {
        src.iter()
            .zip(dst.iter())
            .position(|(expected, actual)| expected != actual)
    } else {
        Some(0)
    };
    let pass = write_ok && read_ok && mismatch.is_none();

    line.clear();
    let _ = write!(
        line,
        "phase=341 psram-qpi-roundtrip addr=0x{:08x} bytes={} write={} read={} verify={} mismatch={} first16=",
        addr,
        PSRAM_BLOCK_SIZE,
        if write_ok { "ok" } else { "fail" },
        if read_ok { "ok" } else { "fail" },
        if pass { "pass" } else { "fail" },
        mismatch.map(|v| v as i32).unwrap_or(-1)
    );
    write_hex(line, &dst[..16]);
    let _ = write!(line, "\r\n");
    let _ = uart.blocking_write(line.as_bytes());
    pass
}

#[cfg(feature = "psram_qpi_backend_diag")]
fn run_qpi_roundtrip_mode(
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
    line: &mut LineBuffer,
    hal: &mut PicoCalcQpiPsram<'_>,
    src: &mut [u8; PSRAM_BLOCK_SIZE],
    dst: &mut [u8; PSRAM_BLOCK_SIZE],
    mode: &str,
    chunk_bytes: usize,
    rx_dma: bool,
) -> bool {
    let addr = TEST_BLOCK * PSRAM_BLOCK_SIZE as u32;
    fill_psram_bench_pattern(src, addr);
    dst.fill(0);

    let write_ok = hal.write_for_diag(addr, src, chunk_bytes).is_ok();
    let read_ok = write_ok
        && if rx_dma {
            hal.read_rx_dma_for_diag(addr, dst, chunk_bytes).is_ok()
        } else {
            hal.read_cpu_for_diag(addr, dst, chunk_bytes).is_ok()
        };
    let mismatch = if read_ok {
        src.iter()
            .zip(dst.iter())
            .position(|(expected, actual)| expected != actual)
    } else {
        Some(0)
    };
    let pass = write_ok && read_ok && mismatch.is_none();

    line.clear();
    let _ = write!(
        line,
        "phase=341 psram-qpi-roundtrip mode={} addr=0x{:08x} bytes={} write={} read={} verify={} mismatch={} chunk={} rx_dma={} first16=",
        mode,
        addr,
        PSRAM_BLOCK_SIZE,
        if write_ok { "ok" } else { "fail" },
        if read_ok { "ok" } else { "fail" },
        if pass { "pass" } else { "fail" },
        mismatch.map(|v| v as i32).unwrap_or(-1),
        chunk_bytes,
        if rx_dma { 1 } else { 0 },
    );
    write_hex(line, &dst[..16]);
    let _ = write!(line, "\r\n");
    let _ = uart.blocking_write(line.as_bytes());
    pass
}

#[cfg(feature = "psram_qpi_backend_diag")]
fn prepare_qpi_bench_region(
    hal: &mut PicoCalcQpiPsram<'_>,
    src: &mut [u8; PSRAM_BENCH_CHUNK_BYTES],
    chunk_bytes: usize,
) -> bool {
    let base = PSRAM_BENCH_BASE_BLOCK * PSRAM_BLOCK_SIZE as u32;
    let mut offset = 0usize;
    while offset < PSRAM_BENCH_TOTAL_BYTES {
        let chunk_len = (PSRAM_BENCH_TOTAL_BYTES - offset).min(src.len());
        let addr = base + offset as u32;
        fill_psram_bench_pattern(&mut src[..chunk_len], addr);
        if hal
            .write_for_diag(addr, &src[..chunk_len], chunk_bytes)
            .is_err()
        {
            return false;
        }
        offset += chunk_len;
    }
    true
}

#[cfg(feature = "psram_qpi_backend_diag")]
fn bench_read_qpi_cpu(
    hal: &mut PicoCalcQpiPsram<'_>,
    dst: &mut [u8; PSRAM_BENCH_CHUNK_BYTES],
    bytes: usize,
    chunk_bytes: usize,
) -> BenchResult {
    let base = PSRAM_BENCH_BASE_BLOCK * PSRAM_BLOCK_SIZE as u32;
    let started = Instant::now();
    let mut offset = 0usize;
    let mut ok = true;
    let mut fail_reason = "none";
    let mut fail_offset = None;
    let mut fail_expected = 0;
    let mut fail_actual = 0;

    while offset < bytes {
        let chunk_len = (bytes - offset).min(dst.len());
        let addr = base + offset as u32;
        if hal
            .read_cpu_for_diag(addr, &mut dst[..chunk_len], chunk_bytes)
            .is_err()
        {
            ok = false;
            fail_reason = "read_err";
            fail_offset = Some(offset);
            break;
        }
        if !verify_psram_bench_pattern(&dst[..chunk_len], addr) {
            ok = false;
            if let Some((mismatch_off, expected, actual)) =
                first_psram_bench_mismatch(&dst[..chunk_len], addr)
            {
                fail_reason = classify_verify_failure(mismatch_off, expected, actual);
                fail_offset = Some(offset + mismatch_off);
                fail_expected = expected;
                fail_actual = actual;
            }
            break;
        }
        offset += chunk_len;
    }

    BenchResult {
        elapsed_us: started.elapsed().as_micros(),
        done_bytes: offset,
        ok,
        fail_reason,
        fail_offset,
        fail_expected,
        fail_actual,
    }
}

#[cfg(feature = "psram_qpi_backend_diag")]
fn bench_read_qpi_rx_dma(
    hal: &mut PicoCalcQpiPsram<'_>,
    dst: &mut [u8; PSRAM_BENCH_CHUNK_BYTES],
    bytes: usize,
    chunk_bytes: usize,
) -> BenchResult {
    let base = PSRAM_BENCH_BASE_BLOCK * PSRAM_BLOCK_SIZE as u32;
    let started = Instant::now();
    let mut offset = 0usize;
    let mut ok = true;
    let mut fail_reason = "none";
    let mut fail_offset = None;
    let mut fail_expected = 0;
    let mut fail_actual = 0;

    while offset < bytes {
        let chunk_len = (bytes - offset).min(dst.len());
        let addr = base + offset as u32;
        if hal
            .read_rx_dma_for_diag(addr, &mut dst[..chunk_len], chunk_bytes)
            .is_err()
        {
            ok = false;
            fail_reason = "rx_dma_read_err";
            fail_offset = Some(offset);
            break;
        }
        if !verify_psram_bench_pattern(&dst[..chunk_len], addr) {
            ok = false;
            if let Some((mismatch_off, expected, actual)) =
                first_psram_bench_mismatch(&dst[..chunk_len], addr)
            {
                fail_reason = classify_verify_failure(mismatch_off, expected, actual);
                fail_offset = Some(offset + mismatch_off);
                fail_expected = expected;
                fail_actual = actual;
            }
            break;
        }
        offset += chunk_len;
    }

    BenchResult {
        elapsed_us: started.elapsed().as_micros(),
        done_bytes: offset,
        ok,
        fail_reason,
        fail_offset,
        fail_expected,
        fail_actual,
    }
}

#[cfg(feature = "psram_qpi_backend_diag")]
fn log_qpi_bench_result(
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
    line: &mut LineBuffer,
    hal: &mut PicoCalcQpiPsram<'_>,
    mode: &str,
    bytes: usize,
    chunk_bytes: usize,
    rx_dma: bool,
    result: &BenchResult,
) {
    log_psram_bench(
        uart,
        line,
        mode,
        bytes,
        result.done_bytes,
        result.elapsed_us,
        chunk_bytes,
        result.done_bytes.div_ceil(chunk_bytes),
        PSRAM_QPI_DUMMY_NIBBLES,
        0,
        hal.diag_state(),
        result.ok,
        result.fail_reason,
        result.fail_offset,
        result.fail_expected,
        result.fail_actual,
    );
    let _ = rx_dma;
}

#[cfg(feature = "psram_qpi_backend_diag")]
fn log_qpi_large_bench_result(
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
    line: &mut LineBuffer,
    hal: &mut PicoCalcQpiPsram<'_>,
    mode: &str,
    bytes: usize,
    chunk_bytes: usize,
    rx_dma: bool,
    result: &BenchResult,
) {
    let elapsed_us = result.elapsed_us;
    let milli_mb_s = if elapsed_us == 0 {
        0
    } else {
        (result.done_bytes as u64)
            .saturating_mul(1_000_000)
            .saturating_mul(1000)
            .saturating_div(elapsed_us)
            .saturating_div(1_000_000)
    };
    let state = hal.diag_state();
    line.clear();
    let _ = write!(
        line,
        "phase=334 psram-bench mode={} bytes={} done_bytes={} elapsed_us={} mb_s={}.{:03} chunk={} txns_done={} txn_bytes={} clkdiv={}.{:03} sm_hz={} cycles_per_bit={} dummy={} inter_tx_delay_us=0 rx_autopush={} rx_fjoin={} rx_thresh={} tx_autopull={} tx_fjoin={} tx_thresh={} input_sync_bypass={} flevel=0x{:08x} fstat=0x{:08x} fdebug=0x{:08x} rx_dma={} ok={} fail={} fail_off={} fail_exp=0x{:02x} fail_got=0x{:02x}\r\n",
        mode,
        bytes,
        result.done_bytes,
        elapsed_us,
        milli_mb_s / 1000,
        milli_mb_s % 1000,
        chunk_bytes,
        result.done_bytes.div_ceil(chunk_bytes),
        chunk_bytes,
        state.clkdiv,
        ((u32::from(state.clkdiv_frac) * 1000) / 256),
        state.sm_hz,
        state.cycles_per_bit,
        PSRAM_QPI_DUMMY_NIBBLES,
        state.rx_autopush,
        state.rx_fjoin,
        state.rx_threshold,
        state.tx_autopull,
        state.tx_fjoin,
        state.tx_threshold,
        if state.qpi_input_sync_bypass { 1 } else { 0 },
        state.flevel,
        state.fstat,
        state.fdebug,
        if rx_dma { 1 } else { 0 },
        if result.ok { 1 } else { 0 },
        result.fail_reason,
        result.fail_offset.map(|v| v as i32).unwrap_or(-1),
        result.fail_expected,
        result.fail_actual,
    );
    let _ = uart.blocking_write(line.as_bytes());
}

#[cfg(feature = "psram_qpi_backend_diag")]
fn run_qpi_bench_matrix(
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
    line: &mut LineBuffer,
    hal: &mut PicoCalcQpiPsram<'_>,
    src: &mut [u8; PSRAM_BENCH_CHUNK_BYTES],
    dst: &mut [u8; PSRAM_BENCH_CHUNK_BYTES],
) {
    let prep_ok = prepare_qpi_bench_region(hal, src, PSRAM_QPI_CHUNK_BYTES);
    line.clear();
    let _ = write!(
        line,
        "phase=334 psram-bench-prepare base={} bytes={} chunk={} ok={}\r\n",
        PSRAM_BENCH_BASE_BLOCK * PSRAM_BLOCK_SIZE as u32,
        PSRAM_BENCH_TOTAL_BYTES,
        PSRAM_QPI_CHUNK_BYTES,
        if prep_ok { 1 } else { 0 }
    );
    let _ = uart.blocking_write(line.as_bytes());
    if !prep_ok {
        return;
    }

    for bytes in PSRAM_BENCH_SIZES {
        let result = bench_read_qpi_cpu(hal, dst, bytes, PSRAM_QPI_CHUNK_BYTES);
        log_qpi_large_bench_result(
            uart,
            line,
            hal,
            "qpi_cpu64",
            bytes,
            PSRAM_QPI_CHUNK_BYTES,
            false,
            &result,
        );
        if !result.ok {
            return;
        }

        let result = bench_read_qpi_rx_dma(hal, dst, bytes, PSRAM_QPI_CHUNK_BYTES);
        log_qpi_large_bench_result(
            uart,
            line,
            hal,
            "qpi_rx_dma64",
            bytes,
            PSRAM_QPI_CHUNK_BYTES,
            true,
            &result,
        );
        if !result.ok {
            return;
        }
    }

    let prep_ok = prepare_qpi_bench_region(hal, src, PSRAM_QPI_LARGE_CHUNK_BYTES);
    line.clear();
    let _ = write!(
        line,
        "phase=334 psram-bench-prepare base={} bytes={} chunk={} ok={}\r\n",
        PSRAM_BENCH_BASE_BLOCK * PSRAM_BLOCK_SIZE as u32,
        PSRAM_BENCH_TOTAL_BYTES,
        PSRAM_QPI_LARGE_CHUNK_BYTES,
        if prep_ok { 1 } else { 0 }
    );
    let _ = uart.blocking_write(line.as_bytes());
    if !prep_ok {
        return;
    }

    for bytes in PSRAM_BENCH_SIZES {
        let result = bench_read_qpi_cpu(hal, dst, bytes, PSRAM_QPI_LARGE_CHUNK_BYTES);
        log_qpi_large_bench_result(
            uart,
            line,
            hal,
            "qpi_cpu120",
            bytes,
            PSRAM_QPI_LARGE_CHUNK_BYTES,
            false,
            &result,
        );
        if !result.ok {
            return;
        }

        let result = bench_read_qpi_rx_dma(hal, dst, bytes, PSRAM_QPI_LARGE_CHUNK_BYTES);
        log_qpi_large_bench_result(
            uart,
            line,
            hal,
            "qpi_rx_dma120",
            bytes,
            PSRAM_QPI_LARGE_CHUNK_BYTES,
            true,
            &result,
        );
        if !result.ok {
            return;
        }
    }
}

#[cfg(feature = "psram_qpi_backend_diag")]
fn qpi_first_mismatch(actual: &[u8], base_address: u32) -> (Option<usize>, u8, u8) {
    if let Some((offset, expected, got)) = first_psram_bench_mismatch(actual, base_address) {
        (Some(offset), expected, got)
    } else {
        (None, 0, 0)
    }
}

#[cfg(feature = "psram_qpi_backend_diag")]
#[derive(Clone, Copy)]
enum QpiWriteDiagPattern {
    Zero,
    Ones,
    Aa,
    FiftyFive,
    LowNibble,
    HighNibble,
    Incrementing64,
    PseudoRandom,
}

#[cfg(feature = "psram_qpi_backend_diag")]
#[derive(Clone, Copy)]
enum QpiWriteDiagMode {
    QspiPsramRw,
    Qspi4WireWrite,
}

#[cfg(feature = "psram_qpi_backend_diag")]
#[derive(Clone, Copy)]
enum QpiStressWriteMode {
    QspiPsramRw,
    Qspi4WireWrite,
}

#[cfg(feature = "psram_qpi_backend_diag")]
const PSRAM_QPI_WRITE_DIAG_PATTERNS: [QpiWriteDiagPattern; 8] = [
    QpiWriteDiagPattern::Zero,
    QpiWriteDiagPattern::Ones,
    QpiWriteDiagPattern::Aa,
    QpiWriteDiagPattern::FiftyFive,
    QpiWriteDiagPattern::LowNibble,
    QpiWriteDiagPattern::HighNibble,
    QpiWriteDiagPattern::Incrementing64,
    QpiWriteDiagPattern::PseudoRandom,
];

#[cfg(feature = "psram_qpi_backend_diag")]
fn qpi_write_diag_pattern_label(pattern: QpiWriteDiagPattern) -> &'static str {
    match pattern {
        QpiWriteDiagPattern::Zero => "00",
        QpiWriteDiagPattern::Ones => "ff",
        QpiWriteDiagPattern::Aa => "aa",
        QpiWriteDiagPattern::FiftyFive => "55",
        QpiWriteDiagPattern::LowNibble => "0f",
        QpiWriteDiagPattern::HighNibble => "f0",
        QpiWriteDiagPattern::Incrementing64 => "inc00_3f",
        QpiWriteDiagPattern::PseudoRandom => "pseudo_random",
    }
}

#[cfg(feature = "psram_qpi_backend_diag")]
fn qpi_write_diag_mode_label(mode: QpiWriteDiagMode) -> &'static str {
    match mode {
        QpiWriteDiagMode::QspiPsramRw => "qspi_psram_rw",
        QpiWriteDiagMode::Qspi4WireWrite => "qspi_4wire_write",
    }
}

#[cfg(feature = "psram_qpi_backend_diag")]
fn qpi_stress_write_mode_label(mode: QpiStressWriteMode) -> &'static str {
    match mode {
        QpiStressWriteMode::QspiPsramRw => "qspi_psram_rw",
        QpiStressWriteMode::Qspi4WireWrite => "qspi_4wire_write",
    }
}

#[cfg(all(feature = "psram_qpi_backend_diag", feature = "psram_qpi_backend_v2"))]
fn qpi_v2_mode_label(mode: PsramMode) -> &'static str {
    match mode {
        PsramMode::Unknown => "unknown",
        PsramMode::QpiRw => "qpi_rw",
        PsramMode::QpiWriteOnly => "qpi_write_only",
        PsramMode::RecoverSerial => "recover_serial",
    }
}

#[cfg(all(feature = "psram_qpi_backend_diag", feature = "psram_qpi_backend_v2"))]
#[derive(Clone, Copy)]
struct QpiV2StepResult {
    ok: bool,
    fail_off: i32,
    fail_exp: u8,
    fail_got: u8,
    first16: [u8; 16],
    around_fail: [u8; 16],
}

#[cfg(all(feature = "psram_qpi_backend_diag", feature = "psram_qpi_backend_v2"))]
impl QpiV2StepResult {
    fn pass() -> Self {
        Self {
            ok: true,
            fail_off: -1,
            fail_exp: 0,
            fail_got: 0,
            first16: [0; 16],
            around_fail: [0; 16],
        }
    }
}

#[cfg(all(feature = "psram_qpi_backend_diag", feature = "psram_qpi_backend_v2"))]
fn fill_around_fail_bytes(dst: &mut [u8; 16], chunk: &[u8], mismatch_off: usize) {
    let start = mismatch_off.saturating_sub(8);
    for (i, slot) in dst.iter_mut().enumerate() {
        let idx = start + i;
        *slot = if idx < chunk.len() { chunk[idx] } else { 0 };
    }
}

#[cfg(all(feature = "psram_qpi_backend_diag", feature = "psram_qpi_backend_v2"))]
fn qpi_v2_write_generated<F>(
    backend: &mut PicoCalcPsramQpiV2<'_>,
    addr: u32,
    len: usize,
    byte_at: F,
) -> bool
where
    F: Fn(usize) -> u8,
{
    let mut chunk = [0u8; PSRAM_QPI_V2_CHUNK_BYTES];
    let mut offset = 0usize;
    while offset < len {
        let n = (len - offset).min(chunk.len());
        for i in 0..n {
            chunk[i] = byte_at(offset + i);
        }
        if backend.write(addr + offset as u32, &chunk[..n]).is_err() {
            return false;
        }
        offset += n;
    }
    true
}

#[cfg(all(feature = "psram_qpi_backend_diag", feature = "psram_qpi_backend_v2"))]
fn qpi_v2_read_verify_generated<F>(
    backend: &mut PicoCalcPsramQpiV2<'_>,
    addr: u32,
    len: usize,
    byte_at: F,
) -> QpiV2StepResult
where
    F: Fn(usize) -> u8,
{
    let mut out = QpiV2StepResult::pass();
    let mut chunk = [0u8; PSRAM_QPI_V2_CHUNK_BYTES];
    let mut offset = 0usize;
    while offset < len {
        let n = (len - offset).min(chunk.len());
        if backend.read(addr + offset as u32, &mut chunk[..n]).is_err() {
            out.ok = false;
            out.fail_off = offset as i32;
            return out;
        }
        if offset < out.first16.len() {
            let copy = (out.first16.len() - offset).min(n);
            out.first16[offset..offset + copy].copy_from_slice(&chunk[..copy]);
        }
        for i in 0..n {
            let expected = byte_at(offset + i);
            let got = chunk[i];
            if expected != got {
                out.ok = false;
                out.fail_off = (offset + i) as i32;
                out.fail_exp = expected;
                out.fail_got = got;
                fill_around_fail_bytes(&mut out.around_fail, &chunk[..n], i);
                return out;
            }
        }
        offset += n;
    }
    out
}

#[cfg(all(feature = "psram_qpi_backend_diag", feature = "psram_qpi_backend_v2"))]
fn log_qpi_v2_step(
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
    line: &mut LineBuffer,
    step: &str,
    addr: u32,
    len: usize,
    mode_before: PsramMode,
    mode_after: PsramMode,
    result: &QpiV2StepResult,
) {
    line.clear();
    let _ = write!(
        line,
        "phase=338 psram-qpi-v2-stress step={} addr=0x{:08x} len={} mode_before={} mode_after={} read_clkdiv={} write_clkdiv={} chunk={} ok={} fail_off={} fail_exp=0x{:02x} fail_got=0x{:02x} first16=",
        step,
        addr,
        len,
        qpi_v2_mode_label(mode_before),
        qpi_v2_mode_label(mode_after),
        PSRAM_QPI_V2_READ_CLOCK_DIVIDER,
        PSRAM_QPI_V2_WRITE_CLOCK_DIVIDER,
        PSRAM_QPI_V2_CHUNK_BYTES,
        if result.ok { 1 } else { 0 },
        result.fail_off,
        result.fail_exp,
        result.fail_got,
    );
    write_hex(line, &result.first16);
    let _ = write!(line, " around_fail=");
    write_hex(line, &result.around_fail);
    let _ = write!(line, "\r\n");
    let _ = uart.blocking_write(line.as_bytes());
}

#[cfg(all(feature = "psram_qpi_backend_diag", feature = "psram_qpi_backend_v2"))]
fn run_qpi_v2_app_stage_stress(
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
    line: &mut LineBuffer,
    backend: &mut PicoCalcPsramQpiV2<'_>,
) {
    let base_addr = 0u32;

    let mode_before = backend.mode();
    let write_ok =
        qpi_v2_write_generated(backend, base_addr, PSRAM_QPI_V2_APP_STAGE_BYTES, |off| {
            psram_bench_byte(base_addr + off as u32)
        });
    let mut step1 = if write_ok {
        qpi_v2_read_verify_generated(backend, base_addr, PSRAM_QPI_V2_APP_STAGE_BYTES, |off| {
            psram_bench_byte(base_addr + off as u32)
        })
    } else {
        let mut r = QpiV2StepResult::pass();
        r.ok = false;
        r.fail_off = 0;
        r
    };
    if !write_ok {
        step1.fail_exp = psram_bench_byte(base_addr);
    }
    log_qpi_v2_step(
        uart,
        line,
        "wr27004_at0",
        base_addr,
        PSRAM_QPI_V2_APP_STAGE_BYTES,
        mode_before,
        backend.mode(),
        &step1,
    );

    let mode_before = backend.mode();
    let head_ok = qpi_v2_write_generated(backend, 0, PSRAM_QPI_V2_STAGE_HEAD_BYTES, |off| {
        psram_bench_byte(off as u32)
    });
    let tail_ok = head_ok
        && qpi_v2_write_generated(backend, 0x4000, PSRAM_QPI_V2_STAGE_TAIL_BYTES, |off| {
            psram_bench_byte(0x4000 + off as u32)
        });
    let step2 = if tail_ok {
        qpi_v2_read_verify_generated(backend, 0, PSRAM_QPI_V2_APP_STAGE_BYTES, |off| {
            if off < PSRAM_QPI_V2_STAGE_HEAD_BYTES {
                psram_bench_byte(off as u32)
            } else {
                psram_bench_byte(0x4000 + (off - PSRAM_QPI_V2_STAGE_HEAD_BYTES) as u32)
            }
        })
    } else {
        let mut r = QpiV2StepResult::pass();
        r.ok = false;
        r.fail_off = 0;
        r
    };
    log_qpi_v2_step(
        uart,
        line,
        "wr16k_at0_wr10620_at4000_rd27004",
        0,
        PSRAM_QPI_V2_APP_STAGE_BYTES,
        mode_before,
        backend.mode(),
        &step2,
    );

    let mode_before = backend.mode();
    let step3 = qpi_v2_read_verify_generated(backend, 0, PSRAM_QPI_V2_TILE_BYTES, |off| {
        psram_bench_byte(off as u32)
    });
    log_qpi_v2_step(
        uart,
        line,
        "read_codewindow_tile_16k",
        0,
        PSRAM_QPI_V2_TILE_BYTES,
        mode_before,
        backend.mode(),
        &step3,
    );

    let mode_before = backend.mode();
    let trans_addr = 0x0002_0000u32;
    let mut step4 = QpiV2StepResult::pass();
    for _ in 0..4 {
        if !qpi_v2_write_generated(backend, trans_addr, PSRAM_QPI_V2_TRANSITION_BYTES, |off| {
            psram_bench_byte(trans_addr + off as u32)
        }) {
            step4.ok = false;
            step4.fail_off = 0;
            break;
        }
        step4 = qpi_v2_read_verify_generated(
            backend,
            trans_addr,
            PSRAM_QPI_V2_TRANSITION_BYTES,
            |off| psram_bench_byte(trans_addr + off as u32),
        );
        if !step4.ok {
            break;
        }
        step4 = qpi_v2_read_verify_generated(
            backend,
            trans_addr,
            PSRAM_QPI_V2_TRANSITION_BYTES,
            |off| psram_bench_byte(trans_addr + off as u32),
        );
        if !step4.ok {
            break;
        }
        if !qpi_v2_write_generated(backend, trans_addr, PSRAM_QPI_V2_TRANSITION_BYTES, |off| {
            !psram_bench_byte(trans_addr + off as u32)
        }) {
            step4.ok = false;
            step4.fail_off = 0;
            break;
        }
        step4 = qpi_v2_read_verify_generated(
            backend,
            trans_addr,
            PSRAM_QPI_V2_TRANSITION_BYTES,
            |off| !psram_bench_byte(trans_addr + off as u32),
        );
        if !step4.ok {
            break;
        }
    }
    log_qpi_v2_step(
        uart,
        line,
        "transition_wr_rd_rd_wr",
        trans_addr,
        PSRAM_QPI_V2_TRANSITION_BYTES,
        mode_before,
        backend.mode(),
        &step4,
    );

    let mode_before = backend.mode();
    let stage_ok = qpi_v2_write_generated(backend, 0, PSRAM_QPI_V2_APP_STAGE_BYTES, |off| {
        // Simulate deterministic bytecode staging payload.
        (off as u8).wrapping_mul(17).wrapping_add(3)
    });
    let step5 = if stage_ok {
        let tile0 = qpi_v2_read_verify_generated(backend, 0, PSRAM_QPI_V2_TILE_BYTES, |off| {
            (off as u8).wrapping_mul(17).wrapping_add(3)
        });
        if !tile0.ok {
            tile0
        } else {
            qpi_v2_read_verify_generated(backend, 0x4000, PSRAM_QPI_V2_STAGE_TAIL_BYTES, |off| {
                ((PSRAM_QPI_V2_STAGE_HEAD_BYTES + off) as u8)
                    .wrapping_mul(17)
                    .wrapping_add(3)
            })
        }
    } else {
        let mut r = QpiV2StepResult::pass();
        r.ok = false;
        r.fail_off = 0;
        r
    };
    log_qpi_v2_step(
        uart,
        line,
        "kotoblocks_stage_sim",
        0,
        PSRAM_QPI_V2_APP_STAGE_BYTES,
        mode_before,
        backend.mode(),
        &step5,
    );
}

#[cfg(feature = "psram_qpi_backend_diag")]
fn fill_qpi_write_diag_pattern(pattern: QpiWriteDiagPattern, base_address: u32, dst: &mut [u8]) {
    for (offset, byte) in dst.iter_mut().enumerate() {
        *byte = match pattern {
            QpiWriteDiagPattern::Zero => 0x00,
            QpiWriteDiagPattern::Ones => 0xff,
            QpiWriteDiagPattern::Aa => 0xaa,
            QpiWriteDiagPattern::FiftyFive => 0x55,
            QpiWriteDiagPattern::LowNibble => 0x0f,
            QpiWriteDiagPattern::HighNibble => 0xf0,
            QpiWriteDiagPattern::Incrementing64 => (offset & 0x3f) as u8,
            QpiWriteDiagPattern::PseudoRandom => {
                psram_bench_byte(base_address.wrapping_add(offset as u32))
            }
        };
    }
}

#[cfg(feature = "psram_qpi_backend_diag")]
fn first_slice_mismatch(expected: &[u8], actual: &[u8]) -> Option<(usize, u8, u8)> {
    expected
        .iter()
        .zip(actual.iter())
        .enumerate()
        .find_map(|(offset, (expected, actual))| {
            if expected == actual {
                None
            } else {
                Some((offset, *expected, *actual))
            }
        })
}

#[cfg(feature = "psram_qpi_backend_diag")]
fn qpi_rw_write_nibbles(bytes: usize, chunk_bytes: usize) -> usize {
    let mut remaining = bytes;
    let mut total = 0usize;
    while remaining > 0 {
        let chunk = remaining.min(chunk_bytes);
        total += (4 + chunk) * 2;
        remaining -= chunk;
    }
    total
}

#[cfg(feature = "psram_qpi_backend_diag")]
fn run_qpi_write_diag_case(
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
    line: &mut LineBuffer,
    hal: &mut PicoCalcQpiPsram<'_>,
    expected: &mut [u8; PSRAM_BENCH_CHUNK_BYTES],
    actual: &mut [u8; PSRAM_BENCH_CHUNK_BYTES],
    mode: QpiWriteDiagMode,
    write_clkdiv: u32,
    pattern: QpiWriteDiagPattern,
    bytes: usize,
) {
    let read_len = bytes.max(64).min(expected.len()).min(actual.len());
    let read_chunk = read_len.min(PSRAM_QPI_LARGE_CHUNK_BYTES);
    let rw_write_chunk = bytes.min(PSRAM_QPI_LARGE_CHUNK_BYTES);
    let write_chunk = match mode {
        QpiWriteDiagMode::QspiPsramRw => rw_write_chunk,
        QpiWriteDiagMode::Qspi4WireWrite => bytes,
    };
    let addr = PSRAM_QPI_WRITE_DIAG_BASE_ADDR
        + (u32::from(write_clkdiv) << 12)
        + match mode {
            QpiWriteDiagMode::QspiPsramRw => 0,
            QpiWriteDiagMode::Qspi4WireWrite => 0x1000,
        };

    expected[..read_len].fill(0);
    actual[..read_len].fill(0);

    let read_clkdiv = PSRAM_QPI_WRITE_DIAG_READ_CLKDIV;
    let pre_read_ok = hal.set_clock_divider_for_diag(read_clkdiv).is_ok()
        && hal
            .read_cpu_for_diag(addr, &mut expected[..read_len], read_chunk)
            .is_ok();
    if !pre_read_ok {
        expected[..read_len].fill(0);
    }
    fill_qpi_write_diag_pattern(pattern, addr, &mut expected[..bytes]);

    let write_ok = hal.set_clock_divider_for_diag(write_clkdiv).is_ok()
        && match mode {
            QpiWriteDiagMode::QspiPsramRw => hal
                .write_for_diag(addr, &expected[..bytes], write_chunk)
                .is_ok(),
            QpiWriteDiagMode::Qspi4WireWrite => hal
                .write_4wire_for_diag(addr, &expected[..bytes], write_chunk)
                .is_ok(),
        };
    let read_ok = hal.set_clock_divider_for_diag(read_clkdiv).is_ok()
        && write_ok
        && hal
            .read_cpu_for_diag(addr, &mut actual[..read_len], read_chunk)
            .is_ok();
    let mismatch = if read_ok {
        first_slice_mismatch(&expected[..read_len], &actual[..read_len])
    } else {
        None
    };
    let ok = pre_read_ok && write_ok && read_ok && mismatch.is_none();
    let (fail_off, fail_expected, fail_actual) = match mismatch {
        Some((offset, expected, actual)) => (offset as i32, expected, actual),
        None if !ok && bytes > 0 => (-1, expected[0], actual[0]),
        None => (-1, 0, 0),
    };

    let mut xor_first64 = [0u8; 64];
    for i in 0..64 {
        xor_first64[i] = expected[i] ^ actual[i];
    }
    let write_nibbles = match mode {
        QpiWriteDiagMode::QspiPsramRw => qpi_rw_write_nibbles(bytes, write_chunk),
        QpiWriteDiagMode::Qspi4WireWrite => (4 + bytes) * 2,
    };
    let read_nibbles = read_len * 2;

    line.clear();
    let _ = write!(
        line,
        "phase=337 psram-qpi-write-diag write_mode={} read_mode=safe_qpi write_clkdiv={} read_clkdiv={} pattern={} bytes={} write_nibbles={} read_nibbles={} tx_autopull=true tx_thresh=8 shift_msb_first=true expected_first64=",
        qpi_write_diag_mode_label(mode),
        write_clkdiv,
        read_clkdiv,
        qpi_write_diag_pattern_label(pattern),
        bytes,
        write_nibbles,
        read_nibbles,
    );
    write_hex(line, &expected[..64]);
    let _ = write!(line, " actual_first64=");
    write_hex(line, &actual[..64]);
    let _ = write!(line, " xor_first64=");
    write_hex(line, &xor_first64);
    let _ = write!(
        line,
        " fail_off={} fail_exp=0x{:02x} fail_got=0x{:02x} ok={}\r\n",
        fail_off,
        fail_expected,
        fail_actual,
        if ok { 1 } else { 0 }
    );
    let _ = uart.blocking_write(line.as_bytes());
}

#[cfg(feature = "psram_qpi_backend_diag")]
fn run_qpi_write_diag(
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
    line: &mut LineBuffer,
    hal: &mut PicoCalcQpiPsram<'_>,
    expected: &mut [u8; PSRAM_BENCH_CHUNK_BYTES],
    actual: &mut [u8; PSRAM_BENCH_CHUNK_BYTES],
) {
    hal.set_qpi_input_sync_bypass_for_diag(true);
    for write_clkdiv in PSRAM_QPI_WRITE_DIAG_RW_CLKDIVS {
        for pattern in PSRAM_QPI_WRITE_DIAG_PATTERNS {
            for bytes in PSRAM_QPI_WRITE_DIAG_SIZES {
                run_qpi_write_diag_case(
                    uart,
                    line,
                    hal,
                    expected,
                    actual,
                    QpiWriteDiagMode::QspiPsramRw,
                    write_clkdiv,
                    pattern,
                    bytes,
                );
            }
        }
    }

    for write_clkdiv in PSRAM_QPI_WRITE_DIAG_4WIRE_CLKDIVS {
        for pattern in PSRAM_QPI_WRITE_DIAG_PATTERNS {
            for bytes in PSRAM_QPI_WRITE_DIAG_SIZES {
                run_qpi_write_diag_case(
                    uart,
                    line,
                    hal,
                    expected,
                    actual,
                    QpiWriteDiagMode::Qspi4WireWrite,
                    write_clkdiv,
                    pattern,
                    bytes,
                );
            }
        }
    }

    let _ = hal.set_clock_divider_for_diag(PSRAM_QPI_CLOCK_DIVIDER);
}

#[cfg(feature = "psram_qpi_backend_diag")]
fn log_qpi_oneway_result(
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
    line: &mut LineBuffer,
    hal: &mut PicoCalcQpiPsram<'_>,
    mode: &str,
    bench_kind: &str,
    verify: bool,
    bytes: usize,
    chunk_bytes: usize,
    txns_done: usize,
    result: &BenchResult,
    first16: &[u8],
) {
    let elapsed_us = result.elapsed_us;
    let milli_mb_s = if elapsed_us == 0 {
        0
    } else {
        (result.done_bytes as u64)
            .saturating_mul(1_000_000)
            .saturating_mul(1000)
            .saturating_div(elapsed_us)
            .saturating_div(1_000_000)
    };
    let state = hal.diag_state();
    line.clear();
    let _ = write!(
        line,
        "phase=335 psram-qpi-oneway mode={} bench_kind={} verify={} bytes={} done_bytes={} elapsed_us={} mb_s={}.{:03} chunk={} txns_done={} txn_bytes={} clkdiv={}.{:03} sm_hz={} input_sync_bypass={} ok={} fail={} fail_off={} fail_exp=0x{:02x} fail_got=0x{:02x} first16=",
        mode,
        bench_kind,
        if verify { "on" } else { "off" },
        bytes,
        result.done_bytes,
        elapsed_us,
        milli_mb_s / 1000,
        milli_mb_s % 1000,
        chunk_bytes,
        txns_done,
        chunk_bytes,
        state.clkdiv,
        ((u32::from(state.clkdiv_frac) * 1000) / 256),
        state.sm_hz,
        if state.qpi_input_sync_bypass { 1 } else { 0 },
        if result.ok { 1 } else { 0 },
        result.fail_reason,
        result.fail_offset.map(|v| v as i32).unwrap_or(-1),
        result.fail_expected,
        result.fail_actual,
    );
    write_hex(line, first16);
    let _ = write!(line, "\r\n");
    let _ = uart.blocking_write(line.as_bytes());
}

#[cfg(feature = "psram_qpi_backend_diag")]
fn bench_qpi_write_only(
    hal: &mut PicoCalcQpiPsram<'_>,
    src: &mut [u8; PSRAM_BENCH_CHUNK_BYTES],
    bytes: usize,
    chunk_bytes: usize,
) -> BenchResult {
    let base = PSRAM_BENCH_BASE_BLOCK * PSRAM_BLOCK_SIZE as u32;
    let started = Instant::now();
    let mut offset = 0usize;
    let mut ok = true;
    let mut fail_reason = "none";
    let mut fail_offset = None;

    while offset < bytes {
        let chunk_len = (bytes - offset).min(src.len());
        let addr = base + offset as u32;
        fill_psram_bench_pattern(&mut src[..chunk_len], addr);
        if hal
            .write_for_diag(addr, &src[..chunk_len], chunk_bytes)
            .is_err()
        {
            ok = false;
            fail_reason = "write_err";
            fail_offset = Some(offset);
            break;
        }
        offset += chunk_len;
    }

    BenchResult {
        elapsed_us: started.elapsed().as_micros(),
        done_bytes: offset,
        ok,
        fail_reason,
        fail_offset,
        fail_expected: 0,
        fail_actual: 0,
    }
}

#[cfg(feature = "psram_qpi_backend_diag")]
fn bench_qpi_read_only(
    hal: &mut PicoCalcQpiPsram<'_>,
    dst: &mut [u8; PSRAM_BENCH_CHUNK_BYTES],
    bytes: usize,
    chunk_bytes: usize,
    verify: bool,
) -> BenchResult {
    let base = PSRAM_BENCH_BASE_BLOCK * PSRAM_BLOCK_SIZE as u32;
    let started = Instant::now();
    let mut offset = 0usize;
    let mut ok = true;
    let mut fail_reason = "none";
    let mut fail_offset = None;
    let mut fail_expected = 0;
    let mut fail_actual = 0;

    while offset < bytes {
        let chunk_len = (bytes - offset).min(dst.len());
        let addr = base + offset as u32;
        if hal
            .read_cpu_for_diag(addr, &mut dst[..chunk_len], chunk_bytes)
            .is_err()
        {
            ok = false;
            fail_reason = "read_err";
            fail_offset = Some(offset);
            break;
        }
        if verify && !verify_psram_bench_pattern(&dst[..chunk_len], addr) {
            ok = false;
            let (mismatch, expected, actual) = qpi_first_mismatch(&dst[..chunk_len], addr);
            fail_offset = mismatch.map(|v| offset + v);
            fail_expected = expected;
            fail_actual = actual;
            fail_reason = classify_verify_failure(mismatch.unwrap_or(0), expected, actual);
            offset += chunk_len;
            break;
        }
        offset += chunk_len;
    }

    BenchResult {
        elapsed_us: started.elapsed().as_micros(),
        done_bytes: offset,
        ok,
        fail_reason,
        fail_offset,
        fail_expected,
        fail_actual,
    }
}

#[cfg(feature = "psram_qpi_backend_diag")]
fn bench_qpi_large_read_only(
    hal: &mut PicoCalcQpiPsram<'_>,
    dst: &mut [u8; PSRAM_BENCH_CHUNK_BYTES],
    bytes: usize,
    chunk_bytes: usize,
    verify: bool,
) -> BenchResult {
    let base = PSRAM_BENCH_BASE_BLOCK * PSRAM_BLOCK_SIZE as u32;
    let started = Instant::now();
    let mut offset = 0usize;
    let mut ok = true;
    let mut fail_reason = "none";
    let mut fail_offset = None;
    let mut fail_expected = 0;
    let mut fail_actual = 0;

    while offset < bytes {
        let chunk_len = (bytes - offset).min(dst.len());
        let addr = base + offset as u32;
        if hal
            .read_large_cpu_for_diag(addr, &mut dst[..chunk_len], chunk_bytes)
            .is_err()
        {
            ok = false;
            fail_reason = "large_read_err";
            fail_offset = Some(offset);
            break;
        }
        if verify && !verify_psram_bench_pattern(&dst[..chunk_len], addr) {
            ok = false;
            let (mismatch, expected, actual) = qpi_first_mismatch(&dst[..chunk_len], addr);
            fail_offset = mismatch.map(|v| offset + v);
            fail_expected = expected;
            fail_actual = actual;
            fail_reason = classify_verify_failure(mismatch.unwrap_or(0), expected, actual);
            break;
        }
        offset += chunk_len;
    }

    BenchResult {
        elapsed_us: started.elapsed().as_micros(),
        done_bytes: offset,
        ok,
        fail_reason,
        fail_offset,
        fail_expected,
        fail_actual,
    }
}

#[cfg(feature = "psram_qpi_backend_diag")]
fn bench_qpi_roundtrip(
    hal: &mut PicoCalcQpiPsram<'_>,
    src: &mut [u8; PSRAM_BENCH_CHUNK_BYTES],
    dst: &mut [u8; PSRAM_BENCH_CHUNK_BYTES],
    bytes: usize,
    chunk_bytes: usize,
    verify: bool,
) -> BenchResult {
    let base = PSRAM_BENCH_BASE_BLOCK * PSRAM_BLOCK_SIZE as u32;
    let started = Instant::now();
    let mut offset = 0usize;
    let mut ok = true;
    let mut fail_reason = "none";
    let mut fail_offset = None;
    let mut fail_expected = 0;
    let mut fail_actual = 0;

    while offset < bytes {
        let chunk_len = (bytes - offset).min(src.len()).min(dst.len());
        let addr = base + offset as u32;
        fill_psram_bench_pattern(&mut src[..chunk_len], addr);
        if hal
            .write_for_diag(addr, &src[..chunk_len], chunk_bytes)
            .is_err()
        {
            ok = false;
            fail_reason = "write_err";
            fail_offset = Some(offset);
            break;
        }
        if hal
            .read_cpu_for_diag(addr, &mut dst[..chunk_len], chunk_bytes)
            .is_err()
        {
            ok = false;
            fail_reason = "read_err";
            fail_offset = Some(offset);
            break;
        }
        if verify && !verify_psram_bench_pattern(&dst[..chunk_len], addr) {
            ok = false;
            let (mismatch, expected, actual) = qpi_first_mismatch(&dst[..chunk_len], addr);
            fail_offset = mismatch.map(|v| offset + v);
            fail_expected = expected;
            fail_actual = actual;
            fail_reason = classify_verify_failure(mismatch.unwrap_or(0), expected, actual);
            break;
        }
        offset += chunk_len;
    }

    BenchResult {
        elapsed_us: started.elapsed().as_micros(),
        done_bytes: offset,
        ok,
        fail_reason,
        fail_offset,
        fail_expected,
        fail_actual,
    }
}

#[cfg(feature = "psram_qpi_backend_diag")]
fn prepare_qpi_pattern_at(
    hal: &mut PicoCalcQpiPsram<'_>,
    src: &mut [u8; PSRAM_BENCH_CHUNK_BYTES],
    base: u32,
    bytes: usize,
    chunk_bytes: usize,
) -> bool {
    let mut offset = 0usize;
    while offset < bytes {
        let chunk_len = (bytes - offset).min(src.len());
        let addr = base + offset as u32;
        fill_psram_bench_pattern(&mut src[..chunk_len], addr);
        if hal
            .write_for_diag(addr, &src[..chunk_len], chunk_bytes)
            .is_err()
        {
            return false;
        }
        offset += chunk_len;
    }
    true
}

#[cfg(feature = "psram_qpi_backend_diag")]
fn bench_qpi_read_at(
    hal: &mut PicoCalcQpiPsram<'_>,
    dst: &mut [u8; PSRAM_BENCH_CHUNK_BYTES],
    base: u32,
    bytes: usize,
    chunk_bytes: usize,
) -> BenchResult {
    let started = Instant::now();
    let mut offset = 0usize;
    let mut ok = true;
    let mut fail_reason = "none";
    let mut fail_offset = None;
    let mut fail_expected = 0;
    let mut fail_actual = 0;

    while offset < bytes {
        let chunk_len = (bytes - offset).min(dst.len());
        let addr = base + offset as u32;
        if hal
            .read_cpu_for_diag(addr, &mut dst[..chunk_len], chunk_bytes)
            .is_err()
        {
            ok = false;
            fail_reason = "read_err";
            fail_offset = Some(offset);
            break;
        }
        if !verify_psram_bench_pattern(&dst[..chunk_len], addr) {
            ok = false;
            let (mismatch, expected, actual) = qpi_first_mismatch(&dst[..chunk_len], addr);
            fail_offset = mismatch.map(|v| offset + v);
            fail_expected = expected;
            fail_actual = actual;
            fail_reason = classify_verify_failure(mismatch.unwrap_or(0), expected, actual);
            offset += chunk_len;
            break;
        }
        offset += chunk_len;
    }

    BenchResult {
        elapsed_us: started.elapsed().as_micros(),
        done_bytes: offset,
        ok,
        fail_reason,
        fail_offset,
        fail_expected,
        fail_actual,
    }
}

#[cfg(feature = "psram_qpi_backend_diag")]
fn bench_qpi_roundtrip_at(
    hal: &mut PicoCalcQpiPsram<'_>,
    src: &mut [u8; PSRAM_BENCH_CHUNK_BYTES],
    dst: &mut [u8; PSRAM_BENCH_CHUNK_BYTES],
    base: u32,
    bytes: usize,
    chunk_bytes: usize,
    write_read_delay_us: u32,
) -> BenchResult {
    let started = Instant::now();
    let mut offset = 0usize;
    let mut ok = true;
    let mut fail_reason = "none";
    let mut fail_offset = None;
    let mut fail_expected = 0;
    let mut fail_actual = 0;

    while offset < bytes {
        let chunk_len = (bytes - offset).min(src.len()).min(dst.len());
        let addr = base + offset as u32;
        fill_psram_bench_pattern(&mut src[..chunk_len], addr);
        if hal
            .write_for_diag(addr, &src[..chunk_len], chunk_bytes)
            .is_err()
        {
            ok = false;
            fail_reason = "write_err";
            fail_offset = Some(offset);
            break;
        }
        if write_read_delay_us > 0 {
            block_for(Duration::from_micros(u64::from(write_read_delay_us)));
        }
        if hal
            .read_cpu_for_diag(addr, &mut dst[..chunk_len], chunk_bytes)
            .is_err()
        {
            ok = false;
            fail_reason = "read_err";
            fail_offset = Some(offset);
            break;
        }
        if !verify_psram_bench_pattern(&dst[..chunk_len], addr) {
            ok = false;
            let (mismatch, expected, actual) = qpi_first_mismatch(&dst[..chunk_len], addr);
            fail_offset = mismatch.map(|v| offset + v);
            fail_expected = expected;
            fail_actual = actual;
            fail_reason = classify_verify_failure(mismatch.unwrap_or(0), expected, actual);
            offset += chunk_len;
            break;
        }
        offset += chunk_len;
    }

    BenchResult {
        elapsed_us: started.elapsed().as_micros(),
        done_bytes: offset,
        ok,
        fail_reason,
        fail_offset,
        fail_expected,
        fail_actual,
    }
}

#[cfg(feature = "psram_qpi_backend_diag")]
fn bench_qpi_roundtrip_profile_at(
    hal: &mut PicoCalcQpiPsram<'_>,
    src: &mut [u8; PSRAM_BENCH_CHUNK_BYTES],
    dst: &mut [u8; PSRAM_BENCH_CHUNK_BYTES],
    base: u32,
    bytes: usize,
    chunk_bytes: usize,
    write_read_delay_us: u32,
    write_mode: QpiStressWriteMode,
    write_clkdiv: u32,
    read_clkdiv: u32,
) -> BenchResult {
    let started = Instant::now();
    let mut offset = 0usize;
    let mut ok = true;
    let mut fail_reason = "none";
    let mut fail_offset = None;
    let mut fail_expected = 0;
    let mut fail_actual = 0;

    while offset < bytes {
        let chunk_len = (bytes - offset).min(src.len()).min(dst.len());
        let addr = base + offset as u32;
        fill_psram_bench_pattern(&mut src[..chunk_len], addr);

        let write_ok = hal.set_clock_divider_for_diag(write_clkdiv).is_ok()
            && match write_mode {
                QpiStressWriteMode::QspiPsramRw => hal
                    .write_for_diag(addr, &src[..chunk_len], chunk_bytes)
                    .is_ok(),
                QpiStressWriteMode::Qspi4WireWrite => hal
                    .write_4wire_for_diag(addr, &src[..chunk_len], chunk_bytes)
                    .is_ok(),
            };
        if !write_ok {
            ok = false;
            fail_reason = "write_err";
            fail_offset = Some(offset);
            break;
        }

        if write_read_delay_us > 0 {
            block_for(Duration::from_micros(u64::from(write_read_delay_us)));
        }

        let read_ok = hal.set_clock_divider_for_diag(read_clkdiv).is_ok()
            && hal
                .read_cpu_for_diag(addr, &mut dst[..chunk_len], chunk_bytes)
                .is_ok();
        if !read_ok {
            ok = false;
            fail_reason = "read_err";
            fail_offset = Some(offset);
            break;
        }

        if !verify_psram_bench_pattern(&dst[..chunk_len], addr) {
            ok = false;
            let (mismatch, expected, actual) = qpi_first_mismatch(&dst[..chunk_len], addr);
            fail_offset = mismatch.map(|v| offset + v);
            fail_expected = expected;
            fail_actual = actual;
            fail_reason = classify_verify_failure(mismatch.unwrap_or(0), expected, actual);
            offset += chunk_len;
            break;
        }
        offset += chunk_len;
    }

    BenchResult {
        elapsed_us: started.elapsed().as_micros(),
        done_bytes: offset,
        ok,
        fail_reason,
        fail_offset,
        fail_expected,
        fail_actual,
    }
}

#[cfg(feature = "psram_qpi_backend_diag")]
fn log_qpi_stress_result(
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
    line: &mut LineBuffer,
    hal: &mut PicoCalcQpiPsram<'_>,
    mode: &str,
    write_mode: &str,
    write_clkdiv: u32,
    read_clkdiv: u32,
    iteration: usize,
    addr: u32,
    bench_kind: &str,
    write_read_delay_us: u32,
    chunk_bytes: usize,
    txns_done: usize,
    result: &BenchResult,
    first16: &[u8],
) {
    let elapsed_us = result.elapsed_us;
    let milli_mb_s = if elapsed_us == 0 {
        0
    } else {
        (result.done_bytes as u64)
            .saturating_mul(1_000_000)
            .saturating_mul(1000)
            .saturating_div(elapsed_us)
            .saturating_div(1_000_000)
    };
    let state = hal.diag_state();
    line.clear();
    let _ = write!(
        line,
        "phase=336 psram-qpi-stress mode={} write_mode={} write_clkdiv={} read_clkdiv={} iteration={} addr=0x{:08x} bench_kind={} verify=on write_read_delay_us={} bytes={} done_bytes={} elapsed_us={} mb_s={}.{:03} chunk={} txns_done={} txn_bytes={} clkdiv={}.{:03} sm_hz={} input_sync_bypass={} ok={} fail={} fail_off={} fail_exp=0x{:02x} fail_got=0x{:02x} first16=",
        mode,
        write_mode,
        write_clkdiv,
        read_clkdiv,
        iteration,
        addr,
        bench_kind,
        write_read_delay_us,
        PSRAM_BENCH_TOTAL_BYTES,
        result.done_bytes,
        elapsed_us,
        milli_mb_s / 1000,
        milli_mb_s % 1000,
        chunk_bytes,
        txns_done,
        chunk_bytes,
        state.clkdiv,
        ((u32::from(state.clkdiv_frac) * 1000) / 256),
        state.sm_hz,
        if state.qpi_input_sync_bypass { 1 } else { 0 },
        if result.ok { 1 } else { 0 },
        result.fail_reason,
        result.fail_offset.map(|v| v as i32).unwrap_or(-1),
        result.fail_expected,
        result.fail_actual,
    );
    write_hex(line, first16);
    let _ = write!(line, "\r\n");
    let _ = uart.blocking_write(line.as_bytes());
}

#[cfg(feature = "psram_qpi_backend_diag")]
fn run_qpi_clkdiv_sweep(
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
    line: &mut LineBuffer,
    hal: &mut PicoCalcQpiPsram<'_>,
    src: &mut [u8; PSRAM_BENCH_CHUNK_BYTES],
    dst: &mut [u8; PSRAM_BENCH_CHUNK_BYTES],
) {
    let bytes = PSRAM_BENCH_TOTAL_BYTES;
    let chunk = PSRAM_QPI_LARGE_CHUNK_BYTES;
    let txns = bytes.div_ceil(chunk);
    let roundtrip_txns = txns * 2;

    for bypass in [true, false] {
        hal.set_qpi_input_sync_bypass_for_diag(bypass);
        for clkdiv in PSRAM_QPI_CLKDIV_SWEEP {
            if hal.set_clock_divider_for_diag(clkdiv).is_err() {
                let result = BenchResult {
                    elapsed_us: 0,
                    done_bytes: 0,
                    ok: false,
                    fail_reason: "clkdiv_err",
                    fail_offset: None,
                    fail_expected: 0,
                    fail_actual: 0,
                };
                log_qpi_oneway_result(
                    uart,
                    line,
                    hal,
                    "qpi_cpu120_sweep",
                    "roundtrip256",
                    true,
                    PSRAM_BLOCK_SIZE,
                    chunk,
                    PSRAM_BLOCK_SIZE.div_ceil(chunk) * 2,
                    &result,
                    &dst[..16],
                );
                continue;
            }

            let result = bench_qpi_roundtrip(hal, src, dst, PSRAM_BLOCK_SIZE, chunk, true);
            log_qpi_oneway_result(
                uart,
                line,
                hal,
                "qpi_cpu120_sweep",
                "roundtrip256",
                true,
                PSRAM_BLOCK_SIZE,
                chunk,
                PSRAM_BLOCK_SIZE.div_ceil(chunk) * 2,
                &result,
                &dst[..16],
            );
            if !result.ok {
                continue;
            }

            let result = bench_qpi_write_only(hal, src, bytes, chunk);
            log_qpi_oneway_result(
                uart,
                line,
                hal,
                "qpi_cpu120_sweep",
                "write",
                false,
                bytes,
                chunk,
                txns,
                &result,
                &src[..16],
            );
            if !result.ok {
                continue;
            }

            let result = bench_qpi_read_only(hal, dst, bytes, chunk, false);
            log_qpi_oneway_result(
                uart,
                line,
                hal,
                "qpi_cpu120_sweep",
                "read",
                false,
                bytes,
                chunk,
                txns,
                &result,
                &dst[..16],
            );
            if !result.ok {
                continue;
            }

            let result = bench_qpi_read_only(hal, dst, bytes, chunk, true);
            log_qpi_oneway_result(
                uart,
                line,
                hal,
                "qpi_cpu120_sweep",
                "read",
                true,
                bytes,
                chunk,
                txns,
                &result,
                &dst[..16],
            );
            if !result.ok {
                continue;
            }

            let result = bench_qpi_roundtrip(hal, src, dst, bytes, chunk, false);
            log_qpi_oneway_result(
                uart,
                line,
                hal,
                "qpi_cpu120_sweep",
                "roundtrip",
                false,
                bytes,
                chunk,
                roundtrip_txns,
                &result,
                &dst[..16],
            );
            if !result.ok {
                continue;
            }

            let result = bench_qpi_roundtrip(hal, src, dst, bytes, chunk, true);
            log_qpi_oneway_result(
                uart,
                line,
                hal,
                "qpi_cpu120_sweep",
                "roundtrip",
                true,
                bytes,
                chunk,
                roundtrip_txns,
                &result,
                &dst[..16],
            );
        }
    }

    hal.set_qpi_input_sync_bypass_for_diag(true);
    let _ = hal.set_clock_divider_for_diag(PSRAM_QPI_CLOCK_DIVIDER);
}

#[cfg(feature = "psram_qpi_backend_diag")]
fn run_qpi_cpu120_stress(
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
    line: &mut LineBuffer,
    hal: &mut PicoCalcQpiPsram<'_>,
    src: &mut [u8; PSRAM_BENCH_CHUNK_BYTES],
    dst: &mut [u8; PSRAM_BENCH_CHUNK_BYTES],
) {
    fn run_profile(
        uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
        line: &mut LineBuffer,
        hal: &mut PicoCalcQpiPsram<'_>,
        src: &mut [u8; PSRAM_BENCH_CHUNK_BYTES],
        dst: &mut [u8; PSRAM_BENCH_CHUNK_BYTES],
        mode: &str,
        write_mode: QpiStressWriteMode,
        write_clkdiv: u32,
        read_clkdiv: u32,
        delays_us: &[u32],
    ) {
        let chunk = PSRAM_QPI_LARGE_CHUNK_BYTES;
        let bytes = PSRAM_BENCH_TOTAL_BYTES;
        let read_txns = bytes.div_ceil(chunk);
        let roundtrip_txns = read_txns * 2;

        hal.set_qpi_input_sync_bypass_for_diag(false);
        let _ = hal.set_clock_divider_for_diag(write_clkdiv);

        for addr in PSRAM_QPI_STRESS_ADDRS {
            if !prepare_qpi_pattern_at(hal, src, addr, bytes, chunk) {
                let result = BenchResult {
                    elapsed_us: 0,
                    done_bytes: 0,
                    ok: false,
                    fail_reason: "prepare_write_err",
                    fail_offset: None,
                    fail_expected: 0,
                    fail_actual: 0,
                };
                log_qpi_stress_result(
                    uart,
                    line,
                    hal,
                    mode,
                    qpi_stress_write_mode_label(write_mode),
                    write_clkdiv,
                    read_clkdiv,
                    0,
                    addr,
                    "prepare",
                    0,
                    chunk,
                    0,
                    &result,
                    &src[..16],
                );
                return;
            }
        }

        for iteration in 0..PSRAM_QPI_STRESS_ITERATIONS {
            for addr in PSRAM_QPI_STRESS_ADDRS {
                let _ = hal.set_clock_divider_for_diag(read_clkdiv);
                let result = bench_qpi_read_at(hal, dst, addr, bytes, chunk);
                log_qpi_stress_result(
                    uart,
                    line,
                    hal,
                    mode,
                    qpi_stress_write_mode_label(write_mode),
                    write_clkdiv,
                    read_clkdiv,
                    iteration,
                    addr,
                    "read",
                    0,
                    chunk,
                    read_txns,
                    &result,
                    &dst[..16],
                );

                for write_read_delay_us in delays_us {
                    let result = bench_qpi_roundtrip_profile_at(
                        hal,
                        src,
                        dst,
                        addr,
                        bytes,
                        chunk,
                        *write_read_delay_us,
                        write_mode,
                        write_clkdiv,
                        read_clkdiv,
                    );
                    log_qpi_stress_result(
                        uart,
                        line,
                        hal,
                        mode,
                        qpi_stress_write_mode_label(write_mode),
                        write_clkdiv,
                        read_clkdiv,
                        iteration,
                        addr,
                        "roundtrip",
                        *write_read_delay_us,
                        chunk,
                        roundtrip_txns,
                        &result,
                        &dst[..16],
                    );
                }
            }
        }

        hal.set_qpi_input_sync_bypass_for_diag(true);
        let _ = hal.set_clock_divider_for_diag(read_clkdiv);
        let compare_addr = PSRAM_QPI_STRESS_ADDRS[0];
        let result = bench_qpi_read_at(hal, dst, compare_addr, bytes, chunk);
        log_qpi_stress_result(
            uart,
            line,
            hal,
            mode,
            qpi_stress_write_mode_label(write_mode),
            write_clkdiv,
            read_clkdiv,
            PSRAM_QPI_STRESS_ITERATIONS,
            compare_addr,
            "read_compare",
            0,
            chunk,
            read_txns,
            &result,
            &dst[..16],
        );
        let result = bench_qpi_roundtrip_profile_at(
            hal,
            src,
            dst,
            compare_addr,
            bytes,
            chunk,
            0,
            write_mode,
            write_clkdiv,
            read_clkdiv,
        );
        log_qpi_stress_result(
            uart,
            line,
            hal,
            mode,
            qpi_stress_write_mode_label(write_mode),
            write_clkdiv,
            read_clkdiv,
            PSRAM_QPI_STRESS_ITERATIONS,
            compare_addr,
            "roundtrip_compare",
            0,
            chunk,
            roundtrip_txns,
            &result,
            &dst[..16],
        );

        hal.set_qpi_input_sync_bypass_for_diag(true);
        let _ = hal.set_clock_divider_for_diag(PSRAM_QPI_CLOCK_DIVIDER);
    }

    run_profile(
        uart,
        line,
        hal,
        src,
        dst,
        "qpi_cpu120_stress_clkdiv4",
        QpiStressWriteMode::QspiPsramRw,
        PSRAM_QPI_STRESS_BASELINE_CLKDIV,
        PSRAM_QPI_STRESS_BASELINE_CLKDIV,
        &PSRAM_QPI_WRITE_READ_DELAYS_US,
    );
    run_profile(
        uart,
        line,
        hal,
        src,
        dst,
        "qpi_cpu120_stress_read6",
        QpiStressWriteMode::QspiPsramRw,
        PSRAM_QPI_STRESS_READ6_CLKDIV,
        PSRAM_QPI_STRESS_READ6_CLKDIV,
        &PSRAM_QPI_WRITE_READ_DELAYS_NONE,
    );
    run_profile(
        uart,
        line,
        hal,
        src,
        dst,
        "qpi_cpu120_stress_w2_r8",
        QpiStressWriteMode::Qspi4WireWrite,
        PSRAM_QPI_STRESS_FAST_WRITE4_CLKDIV,
        PSRAM_QPI_STRESS_SAFE_READ_CLKDIV,
        &PSRAM_QPI_WRITE_READ_DELAYS_NONE,
    );
}

#[cfg(feature = "psram_qpi_backend_diag")]
fn run_qpi_large_read_prototype(
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
    line: &mut LineBuffer,
    hal: &mut PicoCalcQpiPsram<'_>,
    src: &mut [u8; PSRAM_BENCH_CHUNK_BYTES],
    dst: &mut [u8; PSRAM_BENCH_CHUNK_BYTES],
) {
    let bytes = PSRAM_BENCH_TOTAL_BYTES;
    if !prepare_qpi_bench_region(hal, src, PSRAM_QPI_LARGE_CHUNK_BYTES) {
        line.clear();
        let _ = write!(
            line,
            "phase=335 psram-qpi-oneway mode=qpi_cpu_ext_read bench_kind=prepare verify=on bytes={} done_bytes=0 elapsed_us=0 mb_s=0.000 chunk={} txns_done=0 txn_bytes={} clkdiv={}.000 sm_hz={} ok=0 fail=prepare_write_err fail_off=-1 fail_exp=0x00 fail_got=0x00 first16=\r\n",
            bytes,
            PSRAM_QPI_LARGE_CHUNK_BYTES,
            PSRAM_QPI_LARGE_CHUNK_BYTES,
            hal.diag_state().clkdiv,
            hal.diag_state().sm_hz,
        );
        let _ = uart.blocking_write(line.as_bytes());
        return;
    }

    for clkdiv in PSRAM_QPI_EXT_READ_CLKDIVS {
        if hal.set_clock_divider_for_diag(clkdiv).is_err() {
            continue;
        }
        for chunk in PSRAM_QPI_EXT_READ_CHUNKS {
            let mode = match chunk {
                512 => "qpi_cpu512",
                1024 => "qpi_cpu1024",
                _ => "qpi_cpu4096",
            };

            let result = bench_qpi_large_read_only(hal, dst, bytes, chunk, false);
            log_qpi_oneway_result(
                uart,
                line,
                hal,
                mode,
                "read",
                false,
                bytes,
                chunk,
                result.done_bytes.div_ceil(chunk),
                &result,
                &dst[..16],
            );
            if !result.ok {
                continue;
            }

            let result = bench_qpi_large_read_only(hal, dst, bytes, chunk, true);
            log_qpi_oneway_result(
                uart,
                line,
                hal,
                mode,
                "read",
                true,
                bytes,
                chunk,
                result.done_bytes.div_ceil(chunk),
                &result,
                &dst[..16],
            );
        }
    }

    let _ = hal.set_clock_divider_for_diag(PSRAM_QPI_CLOCK_DIVIDER);
}

#[cfg(feature = "psram_qpi_backend_diag")]
fn log_qpi_recover(
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
    line: &mut LineBuffer,
    ok: bool,
) {
    line.clear();
    let _ = write!(
        line,
        "phase=342 psram-qpi-recover step=exit_qpi cmd=0xf5 ok={} serial_sanity=not_run\r\n",
        if ok { 1 } else { 0 }
    );
    let _ = uart.blocking_write(line.as_bytes());
}

fn log_psram_bench_abort(
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
    line: &mut LineBuffer,
    reason: &str,
) {
    line.clear();
    let _ = write!(line, "phase=334 psram-bench-abort reason={}\r\n", reason);
    let _ = uart.blocking_write(line.as_bytes());
}

fn recover_psram_bench_after_failure(
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
    line: &mut LineBuffer,
    hal: &mut PicoCalcPsram<'_>,
    dst: &mut [u8; PSRAM_BENCH_CHUNK_BYTES],
) -> bool {
    if hal.recover_after_diag_failure().is_err() {
        log_psram_bench_abort(uart, line, "recover_failed");
        return false;
    }

    let sanity_address = PSRAM_BENCH_BASE_BLOCK * PSRAM_BLOCK_SIZE as u32;
    let sanity_len = PSRAM_PROD_READ_CHUNK_BYTES;
    if hal.read(sanity_address, &mut dst[..sanity_len]).is_err()
        || !verify_psram_bench_pattern(&dst[..sanity_len], sanity_address)
    {
        log_psram_bench_abort(uart, line, "sanity_failed");
        return false;
    }

    true
}

fn prepare_psram_bench_region(
    hal: &mut PicoCalcPsram<'_>,
    scratch: &mut [u8; PSRAM_BENCH_CHUNK_BYTES],
) -> bool {
    let base_address = PSRAM_BENCH_BASE_BLOCK * PSRAM_BLOCK_SIZE as u32;
    let mut offset = 0usize;
    while offset < PSRAM_BENCH_TOTAL_BYTES {
        let chunk_len = (PSRAM_BENCH_TOTAL_BYTES - offset).min(PSRAM_BENCH_CHUNK_BYTES);
        let chunk_address = base_address + offset as u32;
        fill_psram_bench_pattern(&mut scratch[..chunk_len], chunk_address);
        if hal.write(chunk_address, &scratch[..chunk_len]).is_err() {
            return false;
        }
        offset += chunk_len;
    }
    true
}

fn bench_read_prod16(
    hal: &mut PicoCalcPsram<'_>,
    dst: &mut [u8; PSRAM_BENCH_CHUNK_BYTES],
    bytes: usize,
) -> BenchResult {
    bench_read_prod16_with_tuning(hal, dst, bytes, PSRAM_BENCH_DUMMY_BITS_BASELINE, 0)
}

fn bench_read_prod16_with_tuning(
    hal: &mut PicoCalcPsram<'_>,
    dst: &mut [u8; PSRAM_BENCH_CHUNK_BYTES],
    bytes: usize,
    dummy_bits: u8,
    inter_tx_delay_us: u16,
) -> BenchResult {
    let base_address = PSRAM_BENCH_BASE_BLOCK * PSRAM_BLOCK_SIZE as u32;
    let started = Instant::now();
    let mut ok = true;
    let mut fail_reason = "none";
    let mut fail_offset = None;
    let mut fail_expected = 0;
    let mut fail_actual = 0;
    let mut offset = 0usize;
    while offset < bytes {
        let chunk_len = (bytes - offset).min(PSRAM_BENCH_CHUNK_BYTES);
        let chunk_address = base_address + offset as u32;
        if hal
            .read_with_diag_tuning_for_diag(
                chunk_address,
                &mut dst[..chunk_len],
                dummy_bits,
                inter_tx_delay_us,
            )
            .is_err()
        {
            ok = false;
            fail_reason = "read_err";
            break;
        }
        if !verify_psram_bench_pattern(&dst[..chunk_len], chunk_address) {
            ok = false;
            fail_reason = "verify_fail";
            if let Some((mismatch_off, expected, actual)) =
                first_psram_bench_mismatch(&dst[..chunk_len], chunk_address)
            {
                fail_reason = classify_verify_failure(mismatch_off, expected, actual);
                fail_offset = Some(offset + mismatch_off);
                fail_expected = expected;
                fail_actual = actual;
            }
            break;
        }
        offset += chunk_len;
    }
    BenchResult {
        elapsed_us: started.elapsed().as_micros(),
        done_bytes: offset,
        ok,
        fail_reason,
        fail_offset,
        fail_expected,
        fail_actual,
    }
}

fn run_prod16_diag_row(
    hal: &mut PicoCalcPsram<'_>,
    dst: &mut [u8; PSRAM_BENCH_CHUNK_BYTES],
    _mode: &str,
    bytes: usize,
    clkdiv_int: u16,
    clkdiv_frac: u8,
    dummy_bits: u8,
    inter_tx_delay_us: u16,
) -> BenchResult {
    let clkdiv_ok = hal
        .set_clock_divider_parts_for_diag(clkdiv_int, clkdiv_frac)
        .is_ok();
    if clkdiv_ok {
        bench_read_prod16_with_tuning(hal, dst, bytes, dummy_bits, inter_tx_delay_us)
    } else {
        BenchResult {
            elapsed_us: 0,
            done_bytes: 0,
            ok: false,
            fail_reason: "clkdiv_err",
            fail_offset: None,
            fail_expected: 0,
            fail_actual: 0,
        }
    }
}

fn log_prod16_diag_row(
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
    line: &mut LineBuffer,
    hal: &mut PicoCalcPsram<'_>,
    mode: &str,
    bytes: usize,
    dummy_bits: u8,
    inter_tx_delay_us: u16,
    result: &BenchResult,
) {
    log_psram_bench(
        uart,
        line,
        mode,
        bytes,
        result.done_bytes,
        result.elapsed_us,
        PSRAM_PROD_READ_CHUNK_BYTES,
        psram_bench_txns(result.done_bytes),
        dummy_bits,
        inter_tx_delay_us,
        hal.diag_state(),
        result.ok,
        result.fail_reason,
        result.fail_offset,
        result.fail_expected,
        result.fail_actual,
    );
}

fn run_hazardous_prod16_diag_row(
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
    line: &mut LineBuffer,
    hal: &mut PicoCalcPsram<'_>,
    dst: &mut [u8; PSRAM_BENCH_CHUNK_BYTES],
    mode: &str,
    bytes: usize,
    clkdiv_int: u16,
    clkdiv_frac: u8,
    dummy_bits: u8,
    inter_tx_delay_us: u16,
) -> bool {
    let result = run_prod16_diag_row(
        hal,
        dst,
        mode,
        bytes,
        clkdiv_int,
        clkdiv_frac,
        dummy_bits,
        inter_tx_delay_us,
    );
    log_prod16_diag_row(
        uart,
        line,
        hal,
        mode,
        bytes,
        dummy_bits,
        inter_tx_delay_us,
        &result,
    );

    if result.ok {
        true
    } else {
        recover_psram_bench_after_failure(uart, line, hal, dst)
    }
}

fn run_hazardous_experiment_rows(
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
    line: &mut LineBuffer,
    hal: &mut PicoCalcPsram<'_>,
    dst: &mut [u8; PSRAM_BENCH_CHUNK_BYTES],
    bytes: usize,
) -> bool {
    for dummy_bits in PSRAM_BENCH_CLKDIV2P5_DUMMY_SWEEP {
        if !run_hazardous_prod16_diag_row(
            uart,
            line,
            hal,
            dst,
            "prod16_serial_pio_cpu_clkdiv2p5",
            bytes,
            PSRAM_BENCH_CLKDIV_FRAC_2P5_INT,
            PSRAM_BENCH_CLKDIV_FRAC_2P5_FRAC,
            dummy_bits,
            0,
        ) {
            return false;
        }
    }

    for inter_tx_delay_us in PSRAM_BENCH_CLKDIV2P5_INTER_TX_DELAY_US {
        if !run_hazardous_prod16_diag_row(
            uart,
            line,
            hal,
            dst,
            "prod16_serial_pio_cpu_clkdiv2p5_dly",
            bytes,
            PSRAM_BENCH_CLKDIV_FRAC_2P5_INT,
            PSRAM_BENCH_CLKDIV_FRAC_2P5_FRAC,
            PSRAM_BENCH_DUMMY_BITS_BASELINE,
            inter_tx_delay_us,
        ) {
            return false;
        }
    }

    if !run_hazardous_prod16_diag_row(
        uart,
        line,
        hal,
        dst,
        "prod16_serial_pio_cpu_clkdiv2",
        bytes,
        2,
        0,
        PSRAM_BENCH_DUMMY_BITS_BASELINE,
        0,
    ) {
        return false;
    }

    run_hazardous_prod16_diag_row(
        uart,
        line,
        hal,
        dst,
        "prod16_serial_pio_cpu_clkdiv2_dummy9",
        bytes,
        2,
        0,
        PSRAM_BENCH_DUMMY_BITS_PLUS1,
        0,
    )
}

#[cfg(feature = "psram_pio_word_diag")]
fn bench_read_pio_word256(
    hal: &mut PicoCalcPsram<'_>,
    dst: &mut [u8; PSRAM_BENCH_CHUNK_BYTES],
    bytes: usize,
) -> BenchResult {
    let base_address = PSRAM_BENCH_BASE_BLOCK * PSRAM_BLOCK_SIZE as u32;
    let started = Instant::now();
    let mut ok = true;
    let mut fail_reason = "none";
    let mut fail_offset = None;
    let mut fail_expected = 0;
    let mut fail_actual = 0;
    let mut offset = 0usize;
    while offset < bytes {
        let chunk_address = base_address + offset as u32;
        if koto_pico::psram_dma::diag_read_pio_block256(
            hal,
            chunk_address,
            &mut dst[..PSRAM_BLOCK_SIZE],
        )
        .is_err()
        {
            ok = false;
            fail_reason = "read_err";
            break;
        }
        if !verify_psram_bench_pattern(&dst[..PSRAM_BLOCK_SIZE], chunk_address) {
            ok = false;
            fail_reason = "verify_fail";
            if let Some((mismatch_off, expected, actual)) =
                first_psram_bench_mismatch(&dst[..PSRAM_BLOCK_SIZE], chunk_address)
            {
                fail_reason = classify_verify_failure(mismatch_off, expected, actual);
                fail_offset = Some(offset + mismatch_off);
                fail_expected = expected;
                fail_actual = actual;
            }
            break;
        }
        offset += PSRAM_BLOCK_SIZE;
    }
    BenchResult {
        elapsed_us: started.elapsed().as_micros(),
        done_bytes: offset,
        ok,
        fail_reason,
        fail_offset,
        fail_expected,
        fail_actual,
    }
}

#[cfg(feature = "psram_dma_read_api")]
fn bench_read_sm1_dma(
    hal: &mut PicoCalcPsram<'_>,
    dst: &mut [u8; PSRAM_BENCH_CHUNK_BYTES],
    bytes: usize,
) -> BenchResult {
    let base_address = PSRAM_BENCH_BASE_BLOCK * PSRAM_BLOCK_SIZE as u32;
    let started = Instant::now();
    let mut ok = true;
    let mut fail_reason = "none";
    let mut fail_offset = None;
    let mut fail_expected = 0;
    let mut fail_actual = 0;
    let mut offset = 0usize;
    while offset < bytes {
        let chunk_len = (bytes - offset).min(PSRAM_BENCH_CHUNK_BYTES);
        let chunk_address = base_address + offset as u32;
        if koto_pico::psram_dma::read_dma(hal, chunk_address, &mut dst[..chunk_len]).is_err() {
            ok = false;
            fail_reason = "read_err";
            break;
        }
        if !verify_psram_bench_pattern(&dst[..chunk_len], chunk_address) {
            ok = false;
            fail_reason = "verify_fail";
            if let Some((mismatch_off, expected, actual)) =
                first_psram_bench_mismatch(&dst[..chunk_len], chunk_address)
            {
                fail_reason = classify_verify_failure(mismatch_off, expected, actual);
                fail_offset = Some(offset + mismatch_off);
                fail_expected = expected;
                fail_actual = actual;
            }
            break;
        }
        offset += chunk_len;
    }
    BenchResult {
        elapsed_us: started.elapsed().as_micros(),
        done_bytes: offset,
        ok,
        fail_reason,
        fail_offset,
        fail_expected,
        fail_actual,
    }
}

fn run_psram_bench_matrix(
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
    hal: &mut PicoCalcPsram<'_>,
    src: &mut [u8; PSRAM_BENCH_CHUNK_BYTES],
    dst: &mut [u8; PSRAM_BENCH_CHUNK_BYTES],
) {
    let mut line = LineBuffer::new();
    let prep_ok = prepare_psram_bench_region(hal, src);
    line.clear();
    let _ = write!(
        line,
        "phase=334 psram-bench-prepare base={} bytes={} chunk={} ok={}\r\n",
        PSRAM_BENCH_BASE_BLOCK * PSRAM_BLOCK_SIZE as u32,
        PSRAM_BENCH_TOTAL_BYTES,
        PSRAM_BENCH_CHUNK_BYTES,
        if prep_ok { 1 } else { 0 }
    );
    let _ = uart.blocking_write(line.as_bytes());
    if !prep_ok {
        return;
    }

    for bytes in PSRAM_BENCH_SIZES {
        for clkdiv in PSRAM_BENCH_STABLE_CLKDIVS {
            let clkdiv_ok = hal.set_clock_divider_for_diag(clkdiv).is_ok();
            let result = if clkdiv_ok {
                bench_read_prod16(hal, dst, bytes)
            } else {
                BenchResult {
                    elapsed_us: 0,
                    done_bytes: 0,
                    ok: false,
                    fail_reason: "clkdiv_err",
                    fail_offset: None,
                    fail_expected: 0,
                    fail_actual: 0,
                }
            };
            log_psram_bench(
                uart,
                &mut line,
                "prod16_serial_pio_cpu",
                bytes,
                result.done_bytes,
                result.elapsed_us,
                PSRAM_PROD_READ_CHUNK_BYTES,
                psram_bench_txns(result.done_bytes),
                PSRAM_BENCH_DUMMY_BITS_BASELINE,
                0,
                hal.diag_state(),
                clkdiv_ok && result.ok,
                if clkdiv_ok {
                    result.fail_reason
                } else {
                    "clkdiv_err"
                },
                result.fail_offset,
                result.fail_expected,
                result.fail_actual,
            );
            if !result.ok && !recover_psram_bench_after_failure(uart, &mut line, hal, dst) {
                return;
            }

            #[cfg(feature = "psram_pio_word_diag")]
            {
                let result = if clkdiv_ok {
                    bench_read_pio_word256(hal, dst, bytes)
                } else {
                    BenchResult {
                        elapsed_us: 0,
                        done_bytes: 0,
                        ok: false,
                        fail_reason: "clkdiv_err",
                        fail_offset: None,
                        fail_expected: 0,
                        fail_actual: 0,
                    }
                };
                log_psram_bench(
                    uart,
                    &mut line,
                    "pio_word256_serial_pio_cpu",
                    bytes,
                    result.done_bytes,
                    result.elapsed_us,
                    PSRAM_BLOCK_SIZE,
                    psram_bench_txns(result.done_bytes),
                    PSRAM_BENCH_DUMMY_BITS_BASELINE,
                    0,
                    hal.diag_state(),
                    clkdiv_ok && result.ok,
                    if clkdiv_ok {
                        result.fail_reason
                    } else {
                        "clkdiv_err"
                    },
                    result.fail_offset,
                    result.fail_expected,
                    result.fail_actual,
                );
                if !result.ok && !recover_psram_bench_after_failure(uart, &mut line, hal, dst) {
                    return;
                }
            }
        }

        let _ = hal.set_clock_divider_for_diag(4);

        #[cfg(feature = "psram_dma_read_api")]
        {
            let result = bench_read_sm1_dma(hal, dst, bytes);
            log_psram_bench(
                uart,
                &mut line,
                "sm1_cpu_tx_rx_dma_serial",
                bytes,
                result.done_bytes,
                result.elapsed_us,
                PSRAM_BENCH_CHUNK_BYTES,
                psram_bench_txns(result.done_bytes),
                PSRAM_BENCH_DUMMY_BITS_BASELINE,
                0,
                hal.diag_state(),
                result.ok,
                result.fail_reason,
                result.fail_offset,
                result.fail_expected,
                result.fail_actual,
            );
            if !result.ok && !recover_psram_bench_after_failure(uart, &mut line, hal, dst) {
                return;
            }
        }
    }

    if hazardous_bench_enabled() {
        for bytes in PSRAM_BENCH_SIZES {
            if !run_hazardous_experiment_rows(uart, &mut line, hal, dst, bytes) {
                return;
            }
        }

        for small_bytes in PSRAM_BENCH_SMALL_SIZES {
            if !run_hazardous_experiment_rows(uart, &mut line, hal, dst, small_bytes) {
                return;
            }
        }
    }

    let _ = hal.set_clock_divider_for_diag(4);
}

#[cfg(feature = "psram_pio_word_diag")]
fn embassy_faithful_prodphase_legacy_program() -> pio::Program<32> {
    let assembled = pio::pio_asm!(
        ".origin 13",
        ".side_set 2",
        ".wrap_target",
        "begin:",
        "    out x, 8            side 0b01",
        "    out y, 8            side 0b01",
        "    jmp x--, writeloop  side 0b01",
        "writeloop:",
        "    out pins, 1         side 0b00",
        "    jmp x--, writeloop  side 0b10",
        "    jmp !y, begin       side 0b00",
        "readloop:",
        "    in pins, 1          side 0b10",
        "    jmp y--, readloop   side 0b00",
        ".wrap",
        options(max_program_size = 32)
    );
    assembled.program
}

#[cfg(feature = "psram_pio_word_diag")]
fn embassy_faithful_prodphase_origin16_program() -> pio::Program<32> {
    let assembled = pio::pio_asm!(
        ".origin 16",
        ".side_set 2",
        ".wrap_target",
        "begin:",
        "    out x, 8            side 0b01",
        "    out y, 8            side 0b01",
        "    jmp x--, writeloop  side 0b01",
        "writeloop:",
        "    out pins, 1         side 0b00",
        "    jmp x--, writeloop  side 0b10",
        "    jmp !y, begin       side 0b00",
        "readloop:",
        "    in pins, 1          side 0b10",
        "    jmp y--, readloop   side 0b00",
        ".wrap",
        options(max_program_size = 32)
    );
    assembled.program
}

#[cfg(feature = "psram_diag_hazardous_bench")]
fn embassy_falling_edge_origin16_program() -> pio::Program<32> {
    let assembled = pio::pio_asm!(
        ".origin 24",
        ".side_set 2",
        ".wrap_target",
        "begin:",
        "    out x, 8            side 0b01",
        "    out y, 8            side 0b01",
        "    jmp x--, writeloop  side 0b01",
        "writeloop:",
        "    out pins, 1         side 0b00",
        "    jmp x--, writeloop  side 0b10",
        "    jmp !y, begin       side 0b10",
        "readloop:",
        "    in pins, 1          side 0b00",
        "    jmp y--, readloop   side 0b10",
        ".wrap",
        options(max_program_size = 32)
    );
    assembled.program
}

#[cfg(feature = "psram_diag_hazardous_bench")]
fn embassy_falling_edge_fudge_origin16_program() -> pio::Program<32> {
    let assembled = pio::pio_asm!(
        ".origin 23",
        ".side_set 2",
        ".wrap_target",
        "begin:",
        "    out x, 8            side 0b01",
        "    out y, 8            side 0b01",
        "    jmp x--, writeloop  side 0b01",
        "writeloop:",
        "    out pins, 1         side 0b00",
        "    jmp x--, writeloop  side 0b10",
        "    jmp !y, begin       side 0b10",
        "    nop                 side 0b00",
        "readloop:",
        "    in pins, 1          side 0b10",
        "    jmp y--, readloop   side 0b00",
        ".wrap",
        options(max_program_size = 32)
    );
    assembled.program
}

#[embassy_executor::main(
    executor = "embassy_rp::executor::Executor",
    entry = "cortex_m_rt::entry"
)]
async fn main(_spawner: Spawner) {
    let p = koto_pico::board::split_peripherals(embassy_rp::init(Default::default()));

    let mut uart_config = UartConfig::default();
    uart_config.baudrate = 115_200;
    let mut uart = UartTx::new_blocking(p.uart, p.uart_tx, uart_config);
    Timer::after_secs(1).await;
    let _ = uart.blocking_write(BANNER);

    let mut pio = Pio::new(p.psram_pio, Irqs);

    #[cfg(feature = "psram_qpi_backend_diag")]
    {
        let _ = uart.blocking_write(b"psram_diag mode=qpi_cpu64\r\n");
        let mut line = LineBuffer::new();
        let mut src = [0u8; PSRAM_BLOCK_SIZE];
        let mut dst = [0u8; PSRAM_BLOCK_SIZE];
        let bench_src = PSRAM_BENCH_SRC.init([0; PSRAM_BENCH_CHUNK_BYTES]);
        let bench_dst = PSRAM_BENCH_DST.init([0; PSRAM_BENCH_CHUNK_BYTES]);

        match PicoCalcQpiPsram::new(
            &mut pio.common,
            pio.sm0,
            p.psram_cs,
            p.psram_sck,
            p.psram_sio0,
            p.psram_sio1,
            p.psram_sio2,
            p.psram_sio3,
        ) {
            Ok(mut hal) => {
                log_qpi_init(&mut uart, &mut line, true);
                line.clear();
                let _ = write!(
                    line,
                    "psram capacity={} block_size={} blocks={} test_block={} qpi_chunk={} qpi_sm_hz={}\r\n",
                    PSRAM_CAPACITY,
                    PSRAM_BLOCK_SIZE,
                    PSRAM_CAPACITY / PSRAM_BLOCK_SIZE as u32,
                    TEST_BLOCK,
                    PSRAM_QPI_CHUNK_BYTES,
                    PSRAM_QPI_SM_HZ
                );
                let _ = uart.blocking_write(line.as_bytes());

                let qpi_roundtrip_ok =
                    run_qpi_roundtrip(&mut uart, &mut line, &mut hal, &mut src, &mut dst);
                let qpi_cpu120_ok = qpi_roundtrip_ok
                    && run_qpi_roundtrip_mode(
                        &mut uart,
                        &mut line,
                        &mut hal,
                        &mut src,
                        &mut dst,
                        "qpi_cpu120",
                        PSRAM_QPI_LARGE_CHUNK_BYTES,
                        false,
                    );
                let qpi_rx_dma120_ok = qpi_cpu120_ok
                    && run_qpi_roundtrip_mode(
                        &mut uart,
                        &mut line,
                        &mut hal,
                        &mut src,
                        &mut dst,
                        "qpi_rx_dma120",
                        PSRAM_QPI_LARGE_CHUNK_BYTES,
                        true,
                    );

                run_qpi_write_diag(&mut uart, &mut line, &mut hal, bench_src, bench_dst);

                if qpi_roundtrip_ok && qpi_cpu120_ok && qpi_rx_dma120_ok {
                    run_qpi_bench_matrix(&mut uart, &mut line, &mut hal, bench_src, bench_dst);
                    run_qpi_clkdiv_sweep(&mut uart, &mut line, &mut hal, bench_src, bench_dst);
                    run_qpi_cpu120_stress(&mut uart, &mut line, &mut hal, bench_src, bench_dst);
                    if RUN_QPI_EXT_READ_PROTOTYPE {
                        run_qpi_large_read_prototype(
                            &mut uart, &mut line, &mut hal, bench_src, bench_dst,
                        );
                    }
                }
                #[cfg(feature = "psram_qpi_backend_v2")]
                {
                    match PicoCalcPsramQpiV2::new(hal) {
                        Ok(mut backend_v2) => {
                            run_qpi_v2_app_stage_stress(&mut uart, &mut line, &mut backend_v2);
                            let recover_ok = backend_v2.recover_to_serial().is_ok();
                            log_qpi_recover(&mut uart, &mut line, recover_ok);
                        }
                        Err(_) => {
                            let mut fail = QpiV2StepResult::pass();
                            fail.ok = false;
                            fail.fail_off = 0;
                            log_qpi_v2_step(
                                &mut uart,
                                &mut line,
                                "v2_init",
                                0,
                                0,
                                PsramMode::Unknown,
                                PsramMode::Unknown,
                                &fail,
                            );
                            log_qpi_recover(&mut uart, &mut line, false);
                        }
                    }
                }

                #[cfg(not(feature = "psram_qpi_backend_v2"))]
                let recover_ok = hal.recover_exit_qpi().is_ok();
                #[cfg(not(feature = "psram_qpi_backend_v2"))]
                log_qpi_recover(&mut uart, &mut line, recover_ok);
            }
            Err(_) => {
                log_qpi_init(&mut uart, &mut line, false);
            }
        }

        let _ = uart.blocking_write(b"KOTO-0132 psram-diag qpi_cpu64 complete\r\n");
        loop {
            Timer::after_secs(10).await;
        }
    }

    #[cfg(not(feature = "psram_qpi_backend_diag"))]
    {
        // Print selected diagnostic transfer mode (default: prod16)
        let _ = uart.blocking_write(b"psram_diag mode=prod16\r\n");

        #[cfg(feature = "psram_pio_word_diag")]
        let embassy_fifo_echo_ok =
            run_embassy_fifo_echo32_sm1_self_test(&mut pio.common, &mut pio.sm1, &mut uart);

        let mut hal = PicoCalcPsram::new(
            &mut pio.common,
            pio.sm0,
            p.psram_cs,
            p.psram_sck,
            p.psram_sio0,
            p.psram_sio1,
        );

        let mut lb = LineBuffer::new();
        let _ = write!(
            lb,
            "psram capacity={} block_size={} blocks={} test_block={}\r\n",
            PSRAM_CAPACITY,
            PSRAM_BLOCK_SIZE,
            PSRAM_CAPACITY / PSRAM_BLOCK_SIZE as u32,
            TEST_BLOCK
        );
        let _ = uart.blocking_write(lb.as_bytes());

        let mut src = [0u8; PSRAM_BLOCK_SIZE];
        let mut dst = [0u8; PSRAM_BLOCK_SIZE];
        let bench_src = PSRAM_BENCH_SRC.init([0; PSRAM_BENCH_CHUNK_BYTES]);
        let bench_dst = PSRAM_BENCH_DST.init([0; PSRAM_BENCH_CHUNK_BYTES]);

        // Patterns to test
        enum Pattern {
            Incrementing,
            Alternating,
            AddressDerived,
        }
        let patterns = [
            Pattern::Incrementing,
            Pattern::Alternating,
            Pattern::AddressDerived,
        ];

        #[cfg(feature = "psram_pio_word_diag")]
        let mut faithful_enabled = embassy_fifo_echo_ok;
        #[cfg(feature = "psram_pio_word_diag")]
        let mut fifo_echo_checked = true;

        for pat in patterns {
            // Populate source according to pattern
            match pat {
                Pattern::Incrementing => {
                    for (i, b) in src.iter_mut().enumerate() {
                        *b = i as u8;
                    }
                }
                Pattern::Alternating => {
                    for (i, b) in src.iter_mut().enumerate() {
                        *b = if (i & 1) == 0 { 0x00 } else { 0xFF };
                    }
                }
                Pattern::AddressDerived => {
                    let base_addr = TEST_BLOCK * PSRAM_BLOCK_SIZE as u32;
                    for (i, b) in src.iter_mut().enumerate() {
                        let addr = base_addr + i as u32;
                        *b = (addr.wrapping_mul(37).wrapping_add(13) & 0xFF) as u8;
                    }
                }
            }

            lb.clear();
            let _ = write!(lb, "psram diag pattern=");
            match pat {
                Pattern::Incrementing => {
                    let _ = lb.write_str("incrementing");
                }
                Pattern::Alternating => {
                    let _ = lb.write_str("alternating");
                }
                Pattern::AddressDerived => {
                    let _ = lb.write_str("address_derived");
                }
            }
            let _ = write!(lb, " block={}\r\n", TEST_BLOCK);
            let _ = uart.blocking_write(lb.as_bytes());

            // Write
            let write_start = Instant::now();
            let address = TEST_BLOCK * PSRAM_BLOCK_SIZE as u32;
            let write_res = hal.write(address, &src);
            let write_us = write_start.elapsed().as_micros();

            // Read back using production (CPU-pumped) path
            let read_start = Instant::now();
            let read_res = hal.read(address, &mut dst);
            let read_us = read_start.elapsed().as_micros();

            // Verify
            let mismatch = src.iter().zip(dst.iter()).position(|(e, a)| e != a);

            lb.clear();
            let _ = write!(
                lb,
                "psram write={:?} write_us={} read={:?} read_us={} verify={} mismatch={:?}\r\n",
                write_res,
                write_us,
                read_res,
                read_us,
                if mismatch.is_none() { "pass" } else { "fail" },
                mismatch
            );
            let _ = uart.blocking_write(lb.as_bytes());

            if let Some(offset) = mismatch {
                lb.clear();
                let _ = write!(lb, "mismatch offset={} expected=", offset);
                write_hex(&mut lb, &[src[offset]]);
                let _ = write!(lb, " actual=");
                write_hex(&mut lb, &[dst[offset]]);
                let _ = write!(lb, "\r\n");
                let _ = uart.blocking_write(lb.as_bytes());
            } else {
                lb.clear();
                let _ = lb.write_str("verify: ok (first 16 bytes): ");
                write_hex(&mut lb, &dst[..16]);
                let _ = write!(lb, "\r\n");
                let _ = uart.blocking_write(lb.as_bytes());
            }

            // Now run the diagnostic PIO-word-based path (if compiled-in). This
            // collects PIO RX FIFO 32-bit words and reassembles them into bytes.
            // The name `pio_word16` clarifies this is the CPU-pumped word-view
            // diagnostic path; `dma16_v2` is the separate byte-DMA path below.
            #[cfg(feature = "psram_pio_word_diag")]
            {
                use koto_pico::psram_dma::diag_read_pio_word16;
                let mut word_buf = [0u8; 16];
                let start = Instant::now();
                let res = diag_read_pio_word16(&mut hal, address, &mut word_buf);
                let us = start.elapsed().as_micros();

                let mut lb2 = LineBuffer::new();
                match res {
                    Ok(()) => {
                        // verify vs expected
                        let mut mismatch_expected: Option<usize> = None;
                        for i in 0..16usize {
                            if word_buf[i] != src[i] {
                                mismatch_expected = Some(i);
                                break;
                            }
                        }
                        if let Some(off) = mismatch_expected {
                            let _ = write!(lb2, "pio_word16 read_us={} verify=fail ", us);
                            let _ = write!(lb2, "mismatch={} expected=", off);
                            write_hex(&mut lb2, &[src[off]]);
                            let _ = write!(lb2, " actual=");
                            write_hex(&mut lb2, &[word_buf[off]]);
                            let _ = write!(lb2, "\r\n");
                            // Additional diagnostic: fetch raw RX FIFO words and
                            // log them alongside the reconstructed first 16 bytes.
                            let mut raw_words = [0u32; 16];
                            match koto_pico::psram_dma::diag_read_raw_rx_words(
                                &mut hal,
                                address,
                                &mut raw_words,
                            ) {
                                Ok(()) => {
                                    let _ = write!(lb2, "pio_raw_words=");
                                    for i in 0..16usize {
                                        let _ = write!(lb2, "{:08x}", raw_words[i]);
                                        if i + 1 < 16 {
                                            let _ = write!(lb2, ",");
                                        }
                                    }
                                    let _ = write!(lb2, "\r\n");
                                    let _ = write!(lb2, "pio_raw_recon=");
                                    for i in 0..16usize {
                                        let _ = write!(lb2, "{:02x}", (raw_words[i] as u8));
                                    }
                                    let _ = write!(lb2, "\r\n");
                                }
                                Err(e) => {
                                    let _ = write!(lb2, "pio_raw_read_err={:?}\r\n", e);
                                }
                            }
                        } else {
                            // verify vs prod read
                            let mut mismatch_prod: Option<usize> = None;
                            for i in 0..16usize {
                                if word_buf[i] != dst[i] {
                                    mismatch_prod = Some(i);
                                    break;
                                }
                            }
                            if let Some(off) = mismatch_prod {
                                let _ = write!(
                                    lb2,
                                    "pio_word16 read_us={} verify=pass pio_vs_prod=fail ",
                                    us
                                );
                                let _ = write!(lb2, "mismatch={}", off);
                                let _ = write!(lb2, " expected=");
                                write_hex(&mut lb2, &[dst[off]]);
                                let _ = write!(lb2, " actual=");
                                write_hex(&mut lb2, &[word_buf[off]]);
                                let _ = write!(lb2, "\r\n");
                                // Log raw words for diagnosis
                                let mut raw_words = [0u32; 16];
                                match koto_pico::psram_dma::diag_read_raw_rx_words(
                                    &mut hal,
                                    address,
                                    &mut raw_words,
                                ) {
                                    Ok(()) => {
                                        let _ = write!(lb2, "pio_raw_words=");
                                        for i in 0..16usize {
                                            let _ = write!(lb2, "{:08x}", raw_words[i]);
                                            if i + 1 < 16 {
                                                let _ = write!(lb2, ",");
                                            }
                                        }
                                        let _ = write!(lb2, "\r\n");
                                        let _ = write!(lb2, "pio_raw_recon=");
                                        for i in 0..16usize {
                                            let _ = write!(lb2, "{:02x}", (raw_words[i] as u8));
                                        }
                                        let _ = write!(lb2, "\r\n");
                                    }
                                    Err(e) => {
                                        let _ = write!(lb2, "pio_raw_read_err={:?}\r\n", e);
                                    }
                                }
                            } else {
                                let _ = write!(
                                    lb2,
                                    "pio_word16 read_us={} verify=pass pio_vs_prod=pass\r\n",
                                    us
                                );
                            }
                        }
                    }
                    Err(e) => {
                        let _ = write!(lb2, "pio_word16 error={:?} read_us={}\r\n", e, us);
                    }
                }
                let _ = uart.blocking_write(lb2.as_bytes());
            }

            // Run the diagnostic `dma16_v2` path (RX-only byte DMA CH1).
            // This path is retained for comparison, but currently stalls on
            // hardware and is no longer the only DMA direction under investigation.
            #[cfg(feature = "psram_pio_word_diag")]
            if RUN_LEGACY_DMA_DIAGS {
                use koto_pico::psram_dma::diag_read_dma16_v2;
                let mut dma_buf = [0u8; 16];
                let start_dma = Instant::now();
                let res_dma = diag_read_dma16_v2(&mut hal, address, &mut dma_buf);
                let dma_us = start_dma.elapsed().as_micros();

                let mut lb4 = LineBuffer::new();
                match res_dma {
                    Ok(()) => {
                        // verify vs expected (src)
                        let mut mismatch_expected: Option<usize> = None;
                        for i in 0..16usize {
                            if dma_buf[i] != src[i] {
                                mismatch_expected = Some(i);
                                break;
                            }
                        }
                        if let Some(off) = mismatch_expected {
                            let _ = write!(
                                lb4,
                                "dma16_v2 read_us={} data_size=8 transfer_count=16 verify=fail ",
                                dma_us
                            );
                            let _ = write!(lb4, "mismatch={} expected=", off);
                            write_hex(&mut lb4, &[src[off]]);
                            let _ = write!(lb4, " actual=");
                            write_hex(&mut lb4, &[dma_buf[off]]);
                            let _ = write!(lb4, "\r\n");
                        } else {
                            // verify vs prod read
                            let mut mismatch_prod: Option<usize> = None;
                            for i in 0..16usize {
                                if dma_buf[i] != dst[i] {
                                    mismatch_prod = Some(i);
                                    break;
                                }
                            }
                            // verify vs pio_word16 (if available)
                            let mut mismatch_pio: Option<usize> = None;
                            #[cfg(feature = "psram_pio_word_diag")]
                            {
                                use koto_pico::psram_dma::diag_read_pio_word16;
                                let mut pio_cmp = [0u8; 16];
                                if diag_read_pio_word16(&mut hal, address, &mut pio_cmp).is_ok() {
                                    for i in 0..16usize {
                                        if dma_buf[i] != pio_cmp[i] {
                                            mismatch_pio = Some(i);
                                            break;
                                        }
                                    }
                                }
                            }

                            if let Some(off) = mismatch_prod {
                                let _ = write!(
                                lb4,
                                "dma16_v2 read_us={} data_size=8 transfer_count=16 verify=pass dma_vs_prod=fail ",
                                dma_us
                            );
                                let _ = write!(lb4, "mismatch={} expected=", off);
                                write_hex(&mut lb4, &[dst[off]]);
                                let _ = write!(lb4, " actual=");
                                write_hex(&mut lb4, &[dma_buf[off]]);
                                let _ = write!(lb4, "\r\n");
                            } else if let Some(off) = mismatch_pio {
                                let _ = write!(
                                lb4,
                                "dma16_v2 read_us={} data_size=8 transfer_count=16 verify=pass dma_vs_pio=fail ",
                                dma_us
                            );
                                let _ = write!(lb4, "mismatch={} expected=", off);
                                let mut pio_cmp = [0u8; 16];
                                let _ = koto_pico::psram_dma::diag_read_pio_word16(
                                    &mut hal,
                                    address,
                                    &mut pio_cmp,
                                );
                                write_hex(&mut lb4, &[pio_cmp[off]]);
                                let _ = write!(lb4, " actual=");
                                write_hex(&mut lb4, &[dma_buf[off]]);
                                let _ = write!(lb4, "\r\n");
                            } else {
                                let _ = write!(
                                lb4,
                                "dma16_v2 read_us={} data_size=8 transfer_count=16 verify=pass dma_vs_prod=pass dma_vs_pio=pass\r\n",
                                dma_us
                            );
                            }
                        }
                    }
                    Err(e) => {
                        // Error logging for byte-oriented dma16_v2 diagnostics.
                        use koto_pico::psram_dma::DiagError;

                        // Check if this is a DmaTimeout with diagnostic info
                        if let DiagError::DmaTimeout(Some(diag)) = &e {
                            let _ = write!(
                                lb4,
                                "dma16_v2 error=DmaTimeout reason=dma_timeout read_us={}\r\n",
                                dma_us
                            );
                            let _ = write!(
                            lb4,
                            "  dma_ch={} pio_inst={} sm={} treq=PIO1_RX0 data_size=8 transfer_count=16\r\n",
                            diag.dma_channel, diag.pio_instance, diag.sm_index
                        );
                            let _ = write!(lb4, "  treq_mode=pio1_rx0\r\n");
                            let _ = write!(
                                lb4,
                                "  dreq_numeric_pio1_rx0={} dreq_ref_rp2040_pio1_rx0=12\r\n",
                                diag.dreq_numeric_pio1_rx0
                            );
                            let _ = write!(
                                lb4,
                                "  src_intended=0x{:08x} src_written=0x{:08x} dst=0x{:08x}\r\n",
                                diag.src_addr_intended, diag.src_addr_written, diag.dst_addr
                            );
                            let _ = write!(
                                lb4,
                                "  tc_before={} tc_after_arm={} tc_after_cmd={} tc_timeout={}\r\n",
                                diag.trans_count_before,
                                diag.trans_count_after_arm,
                                diag.trans_count_after_cmd,
                                diag.trans_count_timeout
                            );
                            let _ = write!(
                            lb4,
                            "  pio_flevel_before=0x{:08x} pio_flevel_after_cmd=0x{:08x} pio_flevel_timeout=0x{:08x}\r\n",
                            diag.pio_flevel_before,
                            diag.pio_flevel_after_cmd,
                            diag.pio_flevel_timeout
                        );
                            let _ = write!(
                            lb4,
                            "  sm0_tx_level(before/after/timeout)={}/{}/{} sm0_rx_level(before/after/timeout)={}/{}/{}\r\n",
                            diag.pio_tx_level_before,
                            diag.pio_tx_level_after_cmd,
                            diag.pio_tx_level_timeout,
                            diag.pio_rx_level_before,
                            diag.pio_rx_level_after_cmd,
                            diag.pio_rx_level_timeout
                        );
                            let _ = write!(
                                lb4,
                                "  ctrl_before=0x{:08x} ctrl_after_arm=0x{:08x}\r\n",
                                diag.ctrl_before_arm, diag.ctrl_after_arm
                            );
                            let _ = write!(
                                lb4,
                                "  ctrl_after_cmd=0x{:08x} ctrl_timeout=0x{:08x}\r\n",
                                diag.ctrl_after_cmd, diag.ctrl_on_timeout
                            );
                            let _ = write!(
                            lb4,
                            "  ctrl_en={} ctrl_busy={} ctrl_irq_quiet={} read_err={} write_err={} ahb_err={}\r\n",
                            diag.ctrl_en_timeout,
                            diag.ctrl_busy_timeout,
                            diag.ctrl_irq_quiet_timeout,
                            diag.ctrl_read_error_timeout,
                            diag.ctrl_write_error_timeout,
                            diag.ctrl_ahb_error_timeout
                        );
                            let _ = write!(
                            lb4,
                            "  ctrl_cfg data_size={} incr_read={} incr_write={} ring_size={} ring_sel={} chain_to={} treq_sel={} sniff_en={}\r\n",
                            diag.ctrl_data_size_timeout,
                            diag.ctrl_incr_read_timeout,
                            diag.ctrl_incr_write_timeout,
                            diag.ctrl_ring_size_timeout,
                            diag.ctrl_ring_sel_timeout,
                            diag.ctrl_chain_to_timeout,
                            diag.ctrl_treq_sel_timeout,
                            diag.ctrl_sniff_en_timeout
                        );
                            let _ = write!(
                                lb4,
                                "  read_addr_timeout=0x{:08x} write_addr_timeout=0x{:08x}\r\n",
                                diag.read_addr_timeout, diag.write_addr_timeout
                            );
                            let _ = write!(
                            lb4,
                            "  dma_intr=0x{:08x} dma_inte0=0x{:08x} dma_ints0=0x{:08x} ch1_irq_pending={}\r\n",
                            diag.dma_intr_timeout,
                            diag.dma_inte0_timeout,
                            diag.dma_ints0_timeout,
                            diag.dma_ch1_irq_pending_timeout
                        );
                            let _ = uart.blocking_write(lb4.as_bytes());
                            lb4.clear();

                            let _ = write!(
                                lb4,
                                "  pio_fstat_timeout=0x{:08x} pio_fdebug_timeout=0x{:08x}\r\n",
                                diag.pio_fstat_timeout, diag.pio_fdebug_timeout
                            );
                            let _ = write!(
                            lb4,
                            "  sm0_enabled={} sm0_tx_empty={} sm0_rx_empty={} shiftctrl=0x{:08x} execctrl=0x{:08x}\r\n",
                            diag.sm0_enabled_timeout,
                            diag.sm0_tx_empty_timeout,
                            diag.sm0_rx_empty_timeout,
                            diag.sm0_shiftctrl_timeout,
                            diag.sm0_execctrl_timeout
                        );
                            let _ = write!(
                            lb4,
                            "  ahb_err={} tx_empty_when_stall={} rx_empty_when_stall={} snapshot_before_abort={}\r\n",
                            diag.ahb_error,
                            diag.sm0_tx_empty_timeout,
                            diag.sm0_rx_empty_timeout,
                            diag.timeout_snapshot_before_abort
                        );
                            let _ = write!(
                            lb4,
                            "  paced_dma_aborted_before_return={} ctrl_after_abort=0x{:08x}\r\n",
                            diag.paced_dma_aborted_before_return, diag.ctrl_after_abort
                        );
                            let _ = uart.blocking_write(lb4.as_bytes());
                            lb4.clear();
                        } else {
                            let reason = match e {
                                DiagError::DmaTimeout(None) => "dma_timeout_no_diag",
                                DiagError::DmaTimeout(Some(_)) => "dma_timeout_unreachable", // Should not reach here
                                DiagError::DmaDualFailure(_) => "dual_failure_unexpected",
                                DiagError::FaithfulPriorArtFailure(_) => {
                                    "prior_art_failure_unexpected"
                                }
                                DiagError::FaithfulPriorArtCpuTxFailure(_) => {
                                    "prior_art_cpu_tx_failure_unexpected"
                                }
                                DiagError::PriorArtConfigError(_) => "prior_art_config_error",
                                DiagError::DmaAhbError => "dma_ahb_error",
                                DiagError::Unsupported => "unsupported_sm_pio",
                                DiagError::Unavailable => "feature_disabled",
                                DiagError::InvalidArgument => "invalid_arg",
                                DiagError::Hal(_) => "hal_error",
                            };
                            let _ = write!(
                                lb4,
                                "dma16_v2 error={:?} reason={} read_us={}\r\n",
                                e, reason, dma_us
                            );
                        }
                    }
                }
                let _ = uart.blocking_write(lb4.as_bytes());
            }

            // Run the diagnostic `dma16_dual` path (TX DMA + RX DMA), aligned with
            // prior-art sequencing for command push and response drain.
            #[cfg(feature = "psram_pio_word_diag")]
            if RUN_LEGACY_DMA_DIAGS {
                use koto_pico::psram_dma::diag_read_dma16_dual;
                let mut dual_buf = [0u8; 16];
                let start_dual = Instant::now();
                let res_dual = diag_read_dma16_dual(&mut hal, address, &mut dual_buf);
                let dual_us = start_dual.elapsed().as_micros();

                let mut lb5 = LineBuffer::new();
                match res_dual {
                    Ok(()) => {
                        let mut mismatch_expected: Option<usize> = None;
                        for i in 0..16usize {
                            if dual_buf[i] != src[i] {
                                mismatch_expected = Some(i);
                                break;
                            }
                        }

                        if let Some(off) = mismatch_expected {
                            let _ = write!(
                            lb5,
                            "dma16_dual read_us={} tx_data_size=8 tx_count=7 rx_data_size=8 rx_count=16 verify=fail ",
                            dual_us
                        );
                            let _ = write!(lb5, "mismatch={} expected=", off);
                            write_hex(&mut lb5, &[src[off]]);
                            let _ = write!(lb5, " actual=");
                            write_hex(&mut lb5, &[dual_buf[off]]);
                            let _ = write!(lb5, "\r\n");
                        } else {
                            let mut mismatch_prod: Option<usize> = None;
                            for i in 0..16usize {
                                if dual_buf[i] != dst[i] {
                                    mismatch_prod = Some(i);
                                    break;
                                }
                            }

                            let mut mismatch_pio: Option<usize> = None;
                            let mut pio_cmp = [0u8; 16];
                            if koto_pico::psram_dma::diag_read_pio_word16(
                                &mut hal,
                                address,
                                &mut pio_cmp,
                            )
                            .is_ok()
                            {
                                for i in 0..16usize {
                                    if dual_buf[i] != pio_cmp[i] {
                                        mismatch_pio = Some(i);
                                        break;
                                    }
                                }
                            }

                            if let Some(off) = mismatch_prod {
                                let _ = write!(
                                lb5,
                                "dma16_dual read_us={} tx_data_size=8 tx_count=7 rx_data_size=8 rx_count=16 verify=pass dual_vs_prod=fail ",
                                dual_us
                            );
                                let _ = write!(lb5, "mismatch={} expected=", off);
                                write_hex(&mut lb5, &[dst[off]]);
                                let _ = write!(lb5, " actual=");
                                write_hex(&mut lb5, &[dual_buf[off]]);
                                let _ = write!(lb5, "\r\n");
                            } else if let Some(off) = mismatch_pio {
                                let _ = write!(
                                lb5,
                                "dma16_dual read_us={} tx_data_size=8 tx_count=7 rx_data_size=8 rx_count=16 verify=pass dual_vs_pio=fail ",
                                dual_us
                            );
                                let _ = write!(lb5, "mismatch={} expected=", off);
                                write_hex(&mut lb5, &[pio_cmp[off]]);
                                let _ = write!(lb5, " actual=");
                                write_hex(&mut lb5, &[dual_buf[off]]);
                                let _ = write!(lb5, "\r\n");
                            } else {
                                let _ = write!(
                                lb5,
                                "dma16_dual read_us={} tx_data_size=8 tx_count=7 rx_data_size=8 rx_count=16 verify=pass dual_vs_prod=pass dual_vs_pio=pass\r\n",
                                dual_us
                            );
                            }
                        }
                    }
                    Err(e) => {
                        use koto_pico::psram_dma::DiagError;
                        if let DiagError::DmaDualFailure(Some(diag)) = &e {
                            lb5.clear();
                            let _ = write!(
                            lb5,
                            "dma16_dual error=DmaDualFailure reason=timeout_or_ahb read_us={}\r\n",
                            dual_us
                        );
                            let _ = uart.blocking_write(lb5.as_bytes());
                            lb5.clear();
                            let _ = write!(
                            lb5,
                            "  tx_dma_ch={} rx_dma_ch={} tx_treq=PIO1_TX0 rx_treq=PIO1_RX0 timeout_stage={}\r\n",
                            diag.tx_dma_channel,
                            diag.rx_dma_channel,
                            diag.timeout_stage
                        );
                            let _ = uart.blocking_write(lb5.as_bytes());
                            lb5.clear();
                            let _ = write!(
                            lb5,
                            "  tx_src=0x{:08x} tx_dst=0x{:08x} rx_src=0x{:08x} rx_dst=0x{:08x}\r\n",
                            diag.tx_src_addr, diag.tx_dst_addr, diag.rx_src_addr, diag.rx_dst_addr
                        );
                            let _ = uart.blocking_write(lb5.as_bytes());
                            lb5.clear();
                            let _ = write!(
                            lb5,
                            "  tx_ctrl=0x{:08x} tx_tc={} tx_read=0x{:08x} tx_write=0x{:08x}\r\n",
                            diag.tx_ctrl,
                            diag.tx_trans_count,
                            diag.tx_read_addr,
                            diag.tx_write_addr
                        );
                            let _ = uart.blocking_write(lb5.as_bytes());
                            lb5.clear();
                            let _ = write!(
                            lb5,
                            "  rx_ctrl=0x{:08x} rx_tc={} rx_read=0x{:08x} rx_write=0x{:08x}\r\n",
                            diag.rx_ctrl,
                            diag.rx_trans_count,
                            diag.rx_read_addr,
                            diag.rx_write_addr
                        );
                            let _ = uart.blocking_write(lb5.as_bytes());
                            lb5.clear();
                            let _ = write!(
                            lb5,
                            "  pio_flevel=0x{:08x} pio_fstat=0x{:08x} rx_fifo_drained_words={}\r\n",
                            diag.pio_flevel, diag.pio_fstat, diag.rx_fifo_drained_words
                        );
                            let _ = uart.blocking_write(lb5.as_bytes());
                            lb5.clear();
                            let _ = write!(
                            lb5,
                            "  dma_intr0=0x{:08x} dma_inte0=0x{:08x} dma_ints0=0x{:08x} tx_ahb_err={} rx_ahb_err={}\r\n",
                            diag.dma_intr0,
                            diag.dma_inte0,
                            diag.dma_ints0,
                            diag.tx_ahb_error,
                            diag.rx_ahb_error
                        );
                            let _ = uart.blocking_write(lb5.as_bytes());
                            lb5.clear();
                        } else {
                            let _ = write!(
                                lb5,
                                "dma16_dual error={:?} reason=dual_path_failure read_us={}\r\n",
                                e, dual_us
                            );
                        }
                    }
                }
                let _ = uart.blocking_write(lb5.as_bytes());
            }

            // Full-block diagnostic: pio_word256 (repeats pio_word16 16 times)
            #[cfg(feature = "psram_pio_word_diag")]
            {
                use koto_pico::psram_dma::diag_read_pio_block256;
                let mut pio_block = [0u8; PSRAM_BLOCK_SIZE];
                let start_block = Instant::now();
                let res_block = diag_read_pio_block256(&mut hal, address, &mut pio_block);
                let block_us = start_block.elapsed().as_micros();

                let mut lb3 = LineBuffer::new();
                match res_block {
                    Ok(()) => {
                        // verify vs expected (src)
                        let mismatch_expected =
                            src.iter().zip(pio_block.iter()).position(|(e, a)| e != a);
                        if let Some(off) = mismatch_expected {
                            let _ = write!(lb3, "pio_word256 read_us={} verify=fail ", block_us);
                            let _ = write!(lb3, "mismatch={} expected=", off);
                            write_hex(&mut lb3, &[src[off]]);
                            let _ = write!(lb3, " actual=");
                            write_hex(&mut lb3, &[pio_block[off]]);
                            let _ = write!(lb3, "\r\n");
                        } else {
                            // verify vs prod read
                            let mismatch_prod =
                                dst.iter().zip(pio_block.iter()).position(|(e, a)| e != a);
                            if let Some(off) = mismatch_prod {
                                let _ = write!(
                                    lb3,
                                    "pio_word256 read_us={} verify=pass pio_vs_prod=fail ",
                                    block_us
                                );
                                let _ = write!(lb3, "mismatch={}", off);
                                let _ = write!(lb3, " expected=");
                                write_hex(&mut lb3, &[dst[off]]);
                                let _ = write!(lb3, " actual=");
                                write_hex(&mut lb3, &[pio_block[off]]);
                                let _ = write!(lb3, "\r\n");
                            } else {
                                let _ = write!(
                                    lb3,
                                    "pio_word256 read_us={} verify=pass pio_vs_prod=pass\r\n",
                                    block_us
                                );
                            }
                        }
                    }
                    Err(e) => {
                        let _ = write!(lb3, "pio_word256 error={:?} read_us={}\r\n", e, block_us);
                    }
                }
                let _ = uart.blocking_write(lb3.as_bytes());

                let renamed_program = embassy_faithful_prodphase_origin16_program();
                let renamed_program_len = renamed_program.code.len() as u8;
                let mut renamed_loaded = Some(pio.common.load_program(&renamed_program));

                let prior_cpu = run_embassy_faithful_variant(
                    "psram_sm1_prodphase_cpu_tx_rx_dma16",
                    "embassy_faithful_prodphase_origin16",
                    renamed_program_len,
                    &mut pio.sm1,
                    renamed_loaded.as_ref().unwrap(),
                    address,
                    &src[..16],
                    EmbassyFaithfulTxMode::Cpu,
                    EmbassyFaithfulRxMode::Dma,
                    4,
                    0,
                );

                lb3.clear();
                let _ = write!(
                    lb3,
                    "variant={} rx_len=16 read_us={} rx_remaining={} pass={} first_rx16=",
                    prior_cpu.variant_name,
                    prior_cpu.read_us,
                    prior_cpu.rx_remaining,
                    prior_cpu.pass
                );
                write_hex(&mut lb3, &prior_cpu.first_16);
                let _ = write!(lb3, "\r\n");
                let _ = uart.blocking_write(lb3.as_bytes());

                #[cfg(feature = "psram_diag_hazardous_bench")]
                if phase_edge_experiments_enabled() {
                    let loaded = renamed_loaded.take().unwrap();
                    unsafe {
                        pio.common.free_instr(loaded.used_memory);
                    }
                    log_phase_edge_experiment(
                        &mut uart,
                        &mut pio.common,
                        &mut pio.sm1,
                        "psram_sm1_falling_edge_cpu_tx_rx_dma16",
                        "embassy_falling_edge_origin16",
                        embassy_falling_edge_origin16_program(),
                        3,
                        0,
                        address,
                        &src[..16],
                    );
                    log_phase_edge_experiment(
                        &mut uart,
                        &mut pio.common,
                        &mut pio.sm1,
                        "psram_sm1_falling_edge_cpu_tx_rx_dma16",
                        "embassy_falling_edge_origin16",
                        embassy_falling_edge_origin16_program(),
                        2,
                        128,
                        address,
                        &src[..16],
                    );
                    log_phase_edge_experiment(
                        &mut uart,
                        &mut pio.common,
                        &mut pio.sm1,
                        "psram_sm1_falling_edge_fudge_cpu_tx_rx_dma16",
                        "embassy_falling_edge_fudge_origin16",
                        embassy_falling_edge_fudge_origin16_program(),
                        3,
                        0,
                        address,
                        &src[..16],
                    );
                    log_phase_edge_experiment(
                        &mut uart,
                        &mut pio.common,
                        &mut pio.sm1,
                        "psram_sm1_falling_edge_fudge_cpu_tx_rx_dma16",
                        "embassy_falling_edge_fudge_origin16",
                        embassy_falling_edge_fudge_origin16_program(),
                        2,
                        128,
                        address,
                        &src[..16],
                    );
                    renamed_loaded = Some(pio.common.load_program(&renamed_program));
                }

                let ladder_pattern = match pat {
                    Pattern::Incrementing => Some(ReadDiagPattern::Incrementing),
                    Pattern::AddressDerived => Some(ReadDiagPattern::AddressDerived),
                    Pattern::Alternating => None,
                };

                if let Some(read_pattern) = ladder_pattern {
                    // Prepare 4KiB source region so the size ladder reads from known data.
                    let mut pattern_block = [0u8; PSRAM_BLOCK_SIZE];
                    let ladder_max_len =
                        VERIFIED_READ_LADDER_SIZES[VERIFIED_READ_LADDER_SIZES.len() - 1];
                    let mut prep_ok = true;
                    let mut prep_err_line = LineBuffer::new();
                    for block_off in (0..ladder_max_len).step_by(PSRAM_BLOCK_SIZE) {
                        let remain = ladder_max_len - block_off;
                        let chunk_len = core::cmp::min(PSRAM_BLOCK_SIZE, remain);
                        for i in 0..chunk_len {
                            pattern_block[i] = pattern_byte(read_pattern, address, block_off + i);
                        }
                        if let Err(e) =
                            hal.write(address + block_off as u32, &pattern_block[..chunk_len])
                        {
                            prep_ok = false;
                            prep_err_line.clear();
                            let _ = write!(
                            prep_err_line,
                            "variant=psram_sm1_prodphase_cpu_tx_rx_dma_ladder pattern={} prep_write=fail addr={} len={} error={:?}\r\n",
                            pattern_label(read_pattern),
                            address + block_off as u32,
                            chunk_len,
                            e
                        );
                            let _ = uart.blocking_write(prep_err_line.as_bytes());
                            break;
                        }
                    }

                    if prep_ok {
                        if RUN_READ_LADDER_DIAG {
                            let renamed_loaded = renamed_loaded.as_ref().unwrap();
                            for rx_len in VERIFIED_READ_LADDER_SIZES {
                                let ladder = run_embassy_faithful_read_diag_variant(
                                    "psram_sm1_prodphase_cpu_tx_rx_dma_ladder",
                                    "embassy_faithful_prodphase_origin16",
                                    renamed_program_len,
                                    &mut pio.sm1,
                                    &renamed_loaded,
                                    address,
                                    rx_len,
                                    read_pattern,
                                );

                                lb3.clear();
                                let _ = write!(
                                lb3,
                                "variant={} pattern={} rx_len={} read_us={} bw_mb_s={}.{:03} rx_remaining={} pass={} first_rx16=",
                                ladder.variant_name,
                                pattern_label(read_pattern),
                                ladder.rx_len,
                                ladder.read_us,
                                ladder.bandwidth_milli_mb_s / 1000,
                                ladder.bandwidth_milli_mb_s % 1000,
                                ladder.rx_remaining,
                                ladder.pass
                            );
                                write_hex(&mut lb3, &ladder.first_16);
                                let _ = write!(lb3, " last_rx16=");
                                write_hex(&mut lb3, &ladder.last_16);
                                let _ = write!(lb3, "\r\n");
                                let _ = write!(
                                lb3,
                                "  timeout={} ahb_error={} control_leakage={} leading_dummy={}\r\n",
                                ladder.timed_out,
                                ladder.ahb_error,
                                ladder.control_leakage,
                                ladder.leading_dummy_byte
                            );
                                if let Some(off) = ladder.first_mismatch {
                                    let _ = write!(
                                        lb3,
                                        "  first_mismatch={} expected={:02x} actual={:02x}\r\n",
                                        off, ladder.mismatch_expected, ladder.mismatch_actual
                                    );
                                }
                                let _ = uart.blocking_write(lb3.as_bytes());
                            }
                        }

                        // Boundary-address checks with the verified 256-byte read path.
                        let boundary_addresses = [
                            0u32,
                            1,
                            15,
                            16,
                            255,
                            256,
                            257,
                            4095,
                            4096,
                            PSRAM_CAPACITY.saturating_sub(VERIFIED_BOUNDARY_READ_LEN as u32),
                        ];

                        if RUN_BOUNDARY_DIAG {
                            let renamed_loaded = renamed_loaded.as_ref().unwrap();
                            for boundary_addr in boundary_addresses {
                                for i in 0..VERIFIED_BOUNDARY_READ_LEN {
                                    pattern_block[i] = pattern_byte(read_pattern, boundary_addr, i);
                                }
                                if let Err(e) = hal.write(
                                    boundary_addr,
                                    &pattern_block[..VERIFIED_BOUNDARY_READ_LEN],
                                ) {
                                    lb3.clear();
                                    let _ = write!(
                                    lb3,
                                    "variant=psram_sm1_prodphase_cpu_tx_rx_dma_boundary pattern={} addr={} rx_len={} prep_write=fail error={:?}\r\n",
                                    pattern_label(read_pattern),
                                    boundary_addr,
                                    VERIFIED_BOUNDARY_READ_LEN,
                                    e
                                );
                                    let _ = uart.blocking_write(lb3.as_bytes());
                                    continue;
                                }

                                let boundary = run_embassy_faithful_read_diag_variant(
                                    "psram_sm1_prodphase_cpu_tx_rx_dma_boundary",
                                    "embassy_faithful_prodphase_origin16",
                                    renamed_program_len,
                                    &mut pio.sm1,
                                    &renamed_loaded,
                                    boundary_addr,
                                    VERIFIED_BOUNDARY_READ_LEN,
                                    read_pattern,
                                );

                                lb3.clear();
                                let _ = write!(
                                lb3,
                                "variant={} pattern={} addr={} rx_len={} read_us={} bw_mb_s={}.{:03} rx_remaining={} pass={} first_rx16=",
                                boundary.variant_name,
                                pattern_label(read_pattern),
                                boundary_addr,
                                boundary.rx_len,
                                boundary.read_us,
                                boundary.bandwidth_milli_mb_s / 1000,
                                boundary.bandwidth_milli_mb_s % 1000,
                                boundary.rx_remaining,
                                boundary.pass
                            );
                                write_hex(&mut lb3, &boundary.first_16);
                                let _ = write!(lb3, " last_rx16=");
                                write_hex(&mut lb3, &boundary.last_16);
                                let _ = write!(lb3, "\r\n");
                                let _ = write!(
                                lb3,
                                "  timeout={} ahb_error={} control_leakage={} leading_dummy={}\r\n",
                                boundary.timed_out,
                                boundary.ahb_error,
                                boundary.control_leakage,
                                boundary.leading_dummy_byte
                            );
                                if let Some(off) = boundary.first_mismatch {
                                    let _ = write!(
                                        lb3,
                                        "  first_mismatch={} expected={:02x} actual={:02x}\r\n",
                                        off, boundary.mismatch_expected, boundary.mismatch_actual
                                    );
                                }
                                let _ = uart.blocking_write(lb3.as_bytes());
                            }
                        }

                        if RUN_SMALL_READ_DIAG {
                            let renamed_loaded = renamed_loaded.as_ref().unwrap();
                            // Tiny-read checks to validate verifier-like small reads.
                            for small_len in VERIFIED_SMALL_READ_LENS {
                                for small_addr in VERIFIED_SMALL_READ_ADDRS {
                                    for i in 0..small_len {
                                        pattern_block[i] = pattern_byte(
                                            ReadDiagPattern::AddressDerived,
                                            small_addr,
                                            i,
                                        );
                                    }
                                    if let Err(e) =
                                        hal.write(small_addr, &pattern_block[..small_len])
                                    {
                                        lb3.clear();
                                        let _ = write!(
                                        lb3,
                                        "variant=psram_sm1_prodphase_cpu_tx_rx_dma_small pattern=address_derived len={} addr={} prep_write=fail error={:?}\r\n",
                                        small_len,
                                        small_addr,
                                        e
                                    );
                                        let _ = uart.blocking_write(lb3.as_bytes());
                                        continue;
                                    }

                                    let small = run_embassy_faithful_read_diag_variant(
                                        "psram_sm1_prodphase_cpu_tx_rx_dma_small",
                                        "embassy_faithful_prodphase_origin16",
                                        renamed_program_len,
                                        &mut pio.sm1,
                                        &renamed_loaded,
                                        small_addr,
                                        small_len,
                                        ReadDiagPattern::AddressDerived,
                                    );

                                    lb3.clear();
                                    let _ = write!(
                                    lb3,
                                    "variant={} pattern=address_derived len={} addr={} read_us={} rx_remaining={} pass={} timeout={} ahb_error={} control_leakage={} leading_dummy={} first=",
                                    small.variant_name,
                                    small_len,
                                    small_addr,
                                    small.read_us,
                                    small.rx_remaining,
                                    small.pass,
                                    small.timed_out,
                                    small.ahb_error,
                                    small.control_leakage,
                                    small.leading_dummy_byte
                                );
                                    write_hex(&mut lb3, &small.first_16[..small_len.min(16)]);
                                    let _ = write!(lb3, " expected=");
                                    write_hex(&mut lb3, &pattern_block[..small_len.min(16)]);
                                    if let Some(off) = small.first_mismatch {
                                        let _ = write!(
                                            lb3,
                                            " first_mismatch={} expected={:02x} actual={:02x}",
                                            off, small.mismatch_expected, small.mismatch_actual
                                        );
                                    }
                                    let _ = write!(lb3, "\r\n");
                                    let _ = uart.blocking_write(lb3.as_bytes());
                                }
                            }
                        }

                        if RUN_STRESS_DIAG
                            && matches!(read_pattern, ReadDiagPattern::AddressDerived)
                        {
                            let renamed_loaded = renamed_loaded.as_ref().unwrap();
                            let stress_sizes = [256usize, 1024, 4096];
                            for rx_len in stress_sizes {
                                let mut min_us = u64::MAX;
                                let mut max_us = 0u64;
                                let mut sum_us = 0u64;
                                let mut timeout_count = 0u32;
                                let mut ahb_error_count = 0u32;
                                let mut mismatch_count = 0u32;
                                let mut rx_remaining_error_count = 0u32;

                                for _ in 0..1000u32 {
                                    let run = run_embassy_faithful_read_diag_variant(
                                        "psram_sm1_prodphase_cpu_tx_rx_dma_stress",
                                        "embassy_faithful_prodphase_origin16",
                                        renamed_program_len,
                                        &mut pio.sm1,
                                        &renamed_loaded,
                                        address,
                                        rx_len,
                                        ReadDiagPattern::AddressDerived,
                                    );

                                    min_us = core::cmp::min(min_us, run.read_us);
                                    max_us = core::cmp::max(max_us, run.read_us);
                                    sum_us = sum_us.saturating_add(run.read_us);
                                    if run.timed_out {
                                        timeout_count = timeout_count.saturating_add(1);
                                    }
                                    if run.ahb_error {
                                        ahb_error_count = ahb_error_count.saturating_add(1);
                                    }
                                    if run.first_mismatch.is_some() {
                                        mismatch_count = mismatch_count.saturating_add(1);
                                    }
                                    if run.rx_remaining != 0 {
                                        rx_remaining_error_count =
                                            rx_remaining_error_count.saturating_add(1);
                                    }
                                }

                                let avg_us = sum_us / 1000;
                                let pass = timeout_count == 0
                                    && ahb_error_count == 0
                                    && mismatch_count == 0
                                    && rx_remaining_error_count == 0;

                                lb3.clear();
                                let _ = write!(
                                lb3,
                                "variant=psram_sm1_prodphase_cpu_tx_rx_dma_stress pattern=address_derived rx_len={} iterations=1000 min_us={} max_us={} avg_us={} timeout_count={} ahb_error_count={} mismatch_count={} rx_remaining_error_count={} pass={}\r\n",
                                rx_len,
                                min_us,
                                max_us,
                                avg_us,
                                timeout_count,
                                ahb_error_count,
                                mismatch_count,
                                rx_remaining_error_count,
                                pass
                            );
                                let _ = uart.blocking_write(lb3.as_bytes());
                            }
                        }
                    }
                }

                if let Some(renamed_loaded) = renamed_loaded {
                    unsafe {
                        pio.common.free_instr(renamed_loaded.used_memory);
                    }
                }
            }

            // Minimal SM1 FIFO/shift self-test before faithful prior-art diagnostics.
            #[cfg(feature = "psram_pio_word_diag")]
            {
                if !fifo_echo_checked {
                    use koto_pico::psram_dma::diag_run_fifo_echo8_sm1;
                    let start_echo = Instant::now();
                    let res_echo = diag_run_fifo_echo8_sm1();
                    let echo_us = start_echo.elapsed().as_micros();

                    let mut lb_echo = LineBuffer::new();
                    match res_echo {
                        Ok(report) => {
                            let _ = write!(
                            lb_echo,
                            "pio_variant={} pio={} sm={} prog_off={} read_us={} pass={} sm_enable=0x{:02x} sm_pc={}\r\n",
                            report.pio_variant,
                            report.pio_instance,
                            report.sm_index,
                            report.program_offset,
                            echo_us,
                            report.pass,
                            report.sm_enable_bits,
                            report.sm_pc
                        );
                            let _ = write!(
                                lb_echo,
                                "  shiftctrl=0x{:08x} execctrl=0x{:08x} pinctrl=0x{:08x}\r\n",
                                report.shiftctrl, report.execctrl, report.pinctrl
                            );
                            let _ = uart.blocking_write(lb_echo.as_bytes());
                            lb_echo.clear();

                            let case_a = report.case_a_cpu_u8_tx_cpu_rx;
                            let _ = write!(
                            lb_echo,
                            "  case=A input=0x{:02x} received={} pass={} fifo_levels=0x{:08x} fifo_stat=0x{:08x} fdebug=0x{:08x}\r\n",
                            case_a.input_byte,
                            case_a.received_byte.unwrap_or(0),
                            case_a.pass,
                            case_a.fifo_levels,
                            case_a.fifo_stat,
                            case_a.fdebug_raw
                        );
                            let _ = write!(
                                lb_echo,
                                "    fdebug_decoded txstall={} rxstall={} txover={} rxunder={}\r\n",
                                case_a.fdebug_sm_txstall,
                                case_a.fdebug_sm_rxstall,
                                case_a.fdebug_sm_txover,
                                case_a.fdebug_sm_rxunder
                            );
                            let _ = uart.blocking_write(lb_echo.as_bytes());
                            lb_echo.clear();

                            let case_b = report.case_b_packed_tx_cpu_rx;
                            let _ = write!(
                            lb_echo,
                            "  case=B input=0x{:02x} received={} pass={} fifo_levels=0x{:08x} fifo_stat=0x{:08x} fdebug=0x{:08x}\r\n",
                            case_b.input_byte,
                            case_b.received_byte.unwrap_or(0),
                            case_b.pass,
                            case_b.fifo_levels,
                            case_b.fifo_stat,
                            case_b.fdebug_raw
                        );
                            let _ = write!(
                                lb_echo,
                                "    fdebug_decoded txstall={} rxstall={} txover={} rxunder={}\r\n",
                                case_b.fdebug_sm_txstall,
                                case_b.fdebug_sm_rxstall,
                                case_b.fdebug_sm_txover,
                                case_b.fdebug_sm_rxunder
                            );
                            let _ = uart.blocking_write(lb_echo.as_bytes());
                            lb_echo.clear();

                            let case_c = report.case_c_cpu_u8_tx_dma_rx;
                            let _ = write!(
                            lb_echo,
                            "  case=C input=0x{:02x} received={} pass={} rx_ch={} rx_remaining={} rx_busy={} fifo_levels=0x{:08x} fifo_stat=0x{:08x} fdebug=0x{:08x}\r\n",
                            case_c.input_byte,
                            case_c.received_byte.unwrap_or(0),
                            case_c.pass,
                            case_c.rx_dma_channel.unwrap_or(255),
                            case_c.rx_remaining.unwrap_or(0),
                            case_c.rx_busy.unwrap_or(false),
                            case_c.fifo_levels,
                            case_c.fifo_stat,
                            case_c.fdebug_raw
                        );
                            let _ = write!(
                                lb_echo,
                                "    fdebug_decoded txstall={} rxstall={} txover={} rxunder={}\r\n",
                                case_c.fdebug_sm_txstall,
                                case_c.fdebug_sm_rxstall,
                                case_c.fdebug_sm_txover,
                                case_c.fdebug_sm_rxunder
                            );
                            let _ = uart.blocking_write(lb_echo.as_bytes());
                            lb_echo.clear();

                            if let Some(case_d) = report.case_d_tx_dma_rx_dma {
                                let _ = write!(
                                lb_echo,
                                "  case=D input=0x{:02x} received={} pass={} tx_ch={} tx_remaining={} tx_busy={} rx_ch={} rx_remaining={} rx_busy={} fifo_levels=0x{:08x} fifo_stat=0x{:08x} fdebug=0x{:08x}\r\n",
                                case_d.input_byte,
                                case_d.received_byte.unwrap_or(0),
                                case_d.pass,
                                case_d.tx_dma_channel.unwrap_or(255),
                                case_d.tx_remaining.unwrap_or(0),
                                case_d.tx_busy.unwrap_or(false),
                                case_d.rx_dma_channel.unwrap_or(255),
                                case_d.rx_remaining.unwrap_or(0),
                                case_d.rx_busy.unwrap_or(false),
                                case_d.fifo_levels,
                                case_d.fifo_stat,
                                case_d.fdebug_raw
                            );
                                let _ = write!(
                                lb_echo,
                                "    fdebug_decoded txstall={} rxstall={} txover={} rxunder={}\r\n",
                                case_d.fdebug_sm_txstall,
                                case_d.fdebug_sm_rxstall,
                                case_d.fdebug_sm_txover,
                                case_d.fdebug_sm_rxunder
                            );
                                let _ = uart.blocking_write(lb_echo.as_bytes());
                                lb_echo.clear();
                            }

                            let case_e = report.case_e_cpu_u32_tx_cpu_u32_rx;
                            let _ = write!(
                            lb_echo,
                            "  case=E input_word=0x{:08x} received_word=0x{:08x} pass={} fifo_levels=0x{:08x} fifo_stat=0x{:08x} fdebug=0x{:08x}\r\n",
                            case_e.input_word.unwrap_or(0),
                            case_e.received_word.unwrap_or(0),
                            case_e.pass,
                            case_e.fifo_levels,
                            case_e.fifo_stat,
                            case_e.fdebug_raw
                        );
                            let _ = write!(
                                lb_echo,
                                "    fdebug_decoded txstall={} rxstall={} txover={} rxunder={}\r\n",
                                case_e.fdebug_sm_txstall,
                                case_e.fdebug_sm_rxstall,
                                case_e.fdebug_sm_txover,
                                case_e.fdebug_sm_rxunder
                            );
                            let _ = uart.blocking_write(lb_echo.as_bytes());

                            if !report.pass {
                                faithful_enabled = false;
                                lb_echo.clear();
                                let _ = write!(
                                lb_echo,
                                "  fifo_echo8_sm1 failed: skipping faithful prior-art diagnostics in this run\r\n"
                            );
                                let _ = uart.blocking_write(lb_echo.as_bytes());
                            }
                        }
                        Err(e) => {
                            faithful_enabled = false;
                            let _ = write!(
                                lb_echo,
                                "pio_variant=fifo_echo8_sm1 error={:?} read_us={} pass=false\r\n",
                                e, echo_us
                            );
                            let _ = write!(
                            lb_echo,
                            "  fifo_echo8_sm1 failed: skipping faithful prior-art diagnostics in this run\r\n"
                        );
                            let _ = uart.blocking_write(lb_echo.as_bytes());
                        }
                    }

                    fifo_echo_checked = true;
                }
            }

            #[cfg(feature = "psram_pio_word_diag")]
            if !faithful_enabled {
                let mut lb_skip = LineBuffer::new();
                let _ = write!(
                    lb_skip,
                    "faithful_prior_art skipped reason=fifo_echo32_embassy_sm1_failed\r\n"
                );
                let _ = uart.blocking_write(lb_skip.as_bytes());
                Timer::after_secs(1).await;
                continue;
            }

            #[cfg(feature = "psram_pio_word_diag")]
            if RUN_RETIRED_SM1_COMPARE {
                let legacy_program = embassy_faithful_prodphase_legacy_program();
                let legacy_program_len = legacy_program.code.len() as u8;
                let prior = {
                    let legacy_loaded = pio.common.load_program(&legacy_program);
                    run_embassy_faithful_variant(
                        "faithful_prior_art_dma16",
                        "embassy_faithful_prodphase_legacy",
                        legacy_program_len,
                        &mut pio.sm1,
                        &legacy_loaded,
                        address,
                        &src[..16],
                        EmbassyFaithfulTxMode::Dma,
                        EmbassyFaithfulRxMode::Dma,
                        4,
                        0,
                    )
                };
                let mut lb6 = LineBuffer::new();
                let _ = write!(
                lb6,
                "{} prog_off={} read_us={} pass={} tx_remaining={} rx_remaining={} tx_busy={} rx_busy={} tx_ahb_err={} rx_ahb_err={} sm_pc={}\r\n",
                prior.variant_name,
                prior.program_offset,
                prior.read_us,
                prior.pass,
                prior.tx_remaining.unwrap_or(0),
                prior.rx_remaining,
                prior.tx_busy.unwrap_or(false),
                prior.rx_busy,
                prior.tx_ahb_error.unwrap_or(false),
                prior.rx_ahb_error,
                prior.sm_pc
            );
                let _ = write!(
                lb6,
                "  fifo_levels=0x{:08x} fifo_stat=0x{:08x} fdebug=0x{:08x} shiftctrl=0x{:08x} execctrl=0x{:08x} pinctrl=0x{:08x}\r\n",
                prior.fifo_levels,
                prior.fifo_stat,
                prior.fdebug,
                prior.shiftctrl,
                prior.execctrl,
                prior.pinctrl
            );
                let _ = write!(lb6, "  first_rx16=");
                write_hex(&mut lb6, &prior.first_16);
                let _ = write!(lb6, "\r\n");
                let _ = uart.blocking_write(lb6.as_bytes());
            }

            // Small pause between patterns
            Timer::after_secs(1).await;
        }

        run_psram_bench_matrix(&mut uart, &mut hal, bench_src, bench_dst);

        let _ = uart.blocking_write(b"KOTO-0132 psram-diag complete\r\n");
        loop {
            Timer::after_secs(10).await;
        }
    }
}

#[cfg(feature = "psram_pio_word_diag")]
fn run_embassy_fifo_echo32_sm1_self_test<'a>(
    common: &mut embassy_rp::pio::Common<'a, peripherals::PIO1>,
    sm: &mut StateMachine<'a, peripherals::PIO1, 1>,
    uart: &mut UartTx<'_, Blocking>,
) -> bool {
    let program = pio::pio_asm!(
        ".wrap_target",
        "    pull block",
        "    mov isr, osr",
        "    push block",
        ".wrap",
        options(max_program_size = 32)
    );
    let loaded = common.load_program(&program.program);

    let mut cfg = PioConfig::default();
    cfg.use_program(&loaded, &[]);
    cfg.fifo_join = FifoJoin::Duplex;
    cfg.shift_out.auto_fill = false;
    cfg.shift_out.direction = ShiftDirection::Left;
    cfg.shift_out.threshold = 32;
    cfg.shift_in.auto_fill = false;
    cfg.shift_in.direction = ShiftDirection::Left;
    cfg.shift_in.threshold = 32;
    cfg.clock_divider = 1u8.into();

    sm.set_enable(false);
    sm.clear_fifos();
    sm.restart();
    sm.clkdiv_restart();
    sm.set_config(&cfg);
    sm.set_enable(true);

    let input_word = 0xDEAD_BEEF_u32;
    sm.tx().push(input_word);

    let mut waited = 0u32;
    while sm.rx().empty() && waited < 2_000 {
        for _ in 0..10 {
            core::hint::spin_loop();
        }
        waited = waited.saturating_add(1);
    }
    let received_word = if sm.rx().empty() {
        None
    } else {
        Some(sm.rx().pull())
    };

    let pio1 = &embassy_rp::pac::PIO1;
    let mut lb = LineBuffer::new();
    let pass = received_word == Some(input_word);
    let _ = write!(
        lb,
        "pio_variant=fifo_echo32_embassy_sm1 pio=1 sm=1 read_us=0 pass={} sm_enable={} sm_pc={}\r\n",
        pass,
        sm.is_enabled(),
        sm.get_addr()
    );
    let _ = uart.blocking_write(lb.as_bytes());
    lb.clear();
    let _ = write!(
        lb,
        "  input_word=0x{:08x} received_word=0x{:08x} shiftctrl=0x{:08x} execctrl=0x{:08x} pinctrl=0x{:08x}\r\n",
        input_word,
        received_word.unwrap_or(0),
        pio1.sm(1).shiftctrl().read().0,
        pio1.sm(1).execctrl().read().0,
        pio1.sm(1).pinctrl().read().0
    );
    let _ = uart.blocking_write(lb.as_bytes());
    lb.clear();
    let _ = write!(
        lb,
        "  fifo_levels=0x{:08x} fifo_stat=0x{:08x} fdebug=0x{:08x}\r\n",
        pio1.flevel().read().0,
        pio1.fstat().read().0,
        pio1.fdebug().read().0
    );
    let _ = uart.blocking_write(lb.as_bytes());

    sm.set_enable(false);
    sm.clear_fifos();
    pass
}

#[cfg(feature = "psram_pio_word_diag")]
#[derive(Clone, Copy)]
#[allow(dead_code)]
struct EmbassyFaithfulReport {
    variant_name: &'static str,
    program_name: &'static str,
    rx_mode_name: &'static str,
    program_length: u8,
    program_offset: u8,
    wrap_bottom: u8,
    wrap_top: u8,
    read_us: u64,
    sm1_tx_level_before: u32,
    sm1_rx_level_before: u32,
    fdebug_before: u32,
    tx_remaining: Option<u32>,
    rx_remaining: u32,
    tx_busy: Option<bool>,
    rx_busy: bool,
    tx_ahb_error: Option<bool>,
    rx_ahb_error: bool,
    sm_pc: u8,
    shiftctrl: u32,
    execctrl: u32,
    pinctrl: u32,
    fifo_levels: u32,
    fifo_stat: u32,
    fdebug: u32,
    command_words: [u8; 7],
    out_bits: u8,
    in_bits: u8,
    first_16: [u8; 16],
    pass: bool,
    classification: Option<&'static str>,
    likely_cause: Option<&'static str>,
}

#[cfg(feature = "psram_pio_word_diag")]
#[derive(Clone, Copy)]
struct EmbassyFaithfulReadDiagReport {
    variant_name: &'static str,
    rx_len: usize,
    read_us: u64,
    bandwidth_milli_mb_s: u64,
    rx_remaining: u32,
    timed_out: bool,
    ahb_error: bool,
    control_leakage: bool,
    leading_dummy_byte: bool,
    pass: bool,
    first_16: [u8; 16],
    last_16: [u8; 16],
    first_mismatch: Option<usize>,
    mismatch_expected: u8,
    mismatch_actual: u8,
}

#[cfg(feature = "psram_pio_word_diag")]
enum EmbassyFaithfulTxMode {
    Cpu,
    Dma,
}

#[cfg(feature = "psram_pio_word_diag")]
#[allow(dead_code)]
enum EmbassyFaithfulRxMode {
    Cpu,
    Dma,
}

#[cfg(feature = "psram_pio_word_diag")]
fn run_embassy_faithful_variant<'a>(
    variant_name: &'static str,
    program_name: &'static str,
    program_length: u8,
    sm: &mut StateMachine<'a, peripherals::PIO1, 1>,
    loaded: &LoadedProgram<'a, peripherals::PIO1>,
    address: u32,
    expected: &[u8],
    tx_mode: EmbassyFaithfulTxMode,
    rx_mode: EmbassyFaithfulRxMode,
    clkdiv_int: u16,
    clkdiv_frac: u8,
) -> EmbassyFaithfulReport {
    use embassy_rp::pac;
    use embassy_rp::pac::dma::regs::CtrlTrig;
    use embassy_rp::pac::pio::regs::Fdebug;

    let pio1 = &pac::PIO1;
    let dma = &pac::DMA;
    let tx_ch = dma.ch(0);
    let rx_ch = dma.ch(1);
    let sm0_was_enabled = (pio1.ctrl().read().sm_enable() & 0x01) != 0;
    let input_sync_bypass_before = pio1.input_sync_bypass().read();

    let mut cfg = PioConfig::default();
    let mut exec = cfg.get_exec();
    exec.side_en = loaded.side_set.optional();
    exec.side_pindir = loaded.side_set.pindirs();
    exec.jmp_pin = 0;
    exec.wrap_bottom = loaded.wrap.target;
    exec.wrap_top = loaded.wrap.source;
    unsafe { cfg.set_exec(exec) };

    let mut pins = cfg.get_pins();
    pins.sideset_count = loaded.side_set.bits();
    pins.set_count = 0;
    pins.out_count = 1;
    pins.in_base = 3;
    pins.sideset_base = 20;
    pins.set_base = 0;
    pins.out_base = 2;
    unsafe { cfg.set_pins(pins) };

    cfg.fifo_join = FifoJoin::Duplex;
    cfg.shift_out.auto_fill = true;
    cfg.shift_out.direction = ShiftDirection::Left;
    cfg.shift_out.threshold = 8;
    cfg.shift_in.auto_fill = true;
    cfg.shift_in.direction = ShiftDirection::Left;
    cfg.shift_in.threshold = 8;
    cfg.clock_divider = 4u8.into();

    pio1.ctrl().modify(|w| {
        w.set_sm_enable(w.sm_enable() & !(1u8 << 0));
    });
    pio1.input_sync_bypass()
        .write(|w| *w = input_sync_bypass_before | (1u32 << 3));
    pio1.fdebug().write_value(Fdebug(0xffff_ffff));

    sm.set_enable(false);
    tx_ch.ctrl_trig().write_value(CtrlTrig(0));
    rx_ch.ctrl_trig().write_value(CtrlTrig(0));
    sm.clear_fifos();
    sm.set_config(&cfg);
    pio1.sm(1).clkdiv().write(|w| {
        w.0 = (u32::from(clkdiv_int) << 16) | (u32::from(clkdiv_frac) << 8);
    });
    sm.restart();
    sm.clkdiv_restart();
    pio1.sm(1).instr().write(|w| {
        w.set_instr((loaded.origin & 0x1f) as u16);
    });
    sm.set_enable(true);
    pio1.sm(1).instr().write(|w| {
        w.set_instr((loaded.origin & 0x1f) as u16);
    });

    while !sm.rx().empty() {
        let _ = sm.rx().pull();
    }

    let flevel_before = pio1.flevel().read().0;
    let sm1_tx_level_before = (flevel_before >> 8) & 0x0f;
    let sm1_rx_level_before = (flevel_before >> 12) & 0x0f;
    let fdebug_before = pio1.fdebug().read().0;

    let read_len = expected.len() as u8;
    let cmd = [
        40u8,
        read_len.saturating_mul(8).saturating_sub(1),
        0x0b,
        (address >> 16) as u8,
        (address >> 8) as u8,
        address as u8,
        0,
    ];
    let mut dst = [0u8; 16];
    let rx_fifo_ptr = sm.rx_fifo_ptr() as u32;
    let rx_treq = sm.rx_treq();

    let arm_rx_dma = |rx_ch: &embassy_rp::pac::dma::Channel, dst: &mut [u8; 16]| {
        rx_ch.read_addr().write_value(rx_fifo_ptr);
        rx_ch.write_addr().write_value(dst.as_mut_ptr() as u32);
        rx_ch.trans_count().write_value(expected.len() as u32);
        rx_ch.ctrl_trig().write(|w| {
            use embassy_rp::pac::dma::vals::DataSize;
            w.set_data_size(DataSize::SIZE_BYTE);
            w.set_incr_read(false);
            w.set_incr_write(true);
            w.set_treq_sel(rx_treq);
            w.set_irq_quiet(true);
            w.set_en(true);
        });
    };

    let start = Instant::now();
    match tx_mode {
        EmbassyFaithfulTxMode::Cpu => {
            for byte in cmd {
                let word = u32::from_be_bytes([byte, 0, 0, 0]);
                while !sm.tx().try_push(word) {}
            }
            if matches!(rx_mode, EmbassyFaithfulRxMode::Dma) {
                arm_rx_dma(&rx_ch, &mut dst);
            }
        }
        EmbassyFaithfulTxMode::Dma => {
            if matches!(rx_mode, EmbassyFaithfulRxMode::Dma) {
                arm_rx_dma(&rx_ch, &mut dst);
            }
            tx_ch.read_addr().write_value(cmd.as_ptr() as u32);
            tx_ch.write_addr().write_value(sm.tx_fifo_ptr() as u32);
            tx_ch.trans_count().write_value(cmd.len() as u32);
            tx_ch.ctrl_trig().write(|w| {
                use embassy_rp::pac::dma::vals::DataSize;
                w.set_data_size(DataSize::SIZE_BYTE);
                w.set_incr_read(true);
                w.set_incr_write(false);
                w.set_treq_sel(sm.tx_treq());
                w.set_irq_quiet(true);
                w.set_en(true);
            });
        }
    }

    let mut elapsed = 0u32;
    match rx_mode {
        EmbassyFaithfulRxMode::Dma => {
            while rx_ch.ctrl_trig().read().busy() && elapsed < 10_000 {
                for _ in 0..10 {
                    core::hint::spin_loop();
                }
                elapsed = elapsed.saturating_add(1);
            }
        }
        EmbassyFaithfulRxMode::Cpu => {
            for slot in dst.iter_mut().take(expected.len()) {
                while sm.rx().empty() && elapsed < 10_000 {
                    for _ in 0..10 {
                        core::hint::spin_loop();
                    }
                    elapsed = elapsed.saturating_add(1);
                }
                if sm.rx().empty() {
                    break;
                }
                *slot = sm.rx().pull() as u8;
            }
        }
    }
    let read_us = start.elapsed().as_micros();

    let pass = expected.iter().zip(dst.iter()).all(|(a, b)| a == b);
    let leading_out_bits_byte =
        expected.len() >= 16 && dst[0] == 0x28 && dst[1..16] == expected[..15];
    let report = EmbassyFaithfulReport {
        variant_name,
        program_name,
        rx_mode_name: match rx_mode {
            EmbassyFaithfulRxMode::Cpu => "cpu",
            EmbassyFaithfulRxMode::Dma => "dma",
        },
        program_length,
        program_offset: loaded.origin,
        wrap_bottom: loaded.wrap.target,
        wrap_top: loaded.wrap.source,
        read_us,
        sm1_tx_level_before,
        sm1_rx_level_before,
        fdebug_before,
        tx_remaining: match tx_mode {
            EmbassyFaithfulTxMode::Cpu => None,
            EmbassyFaithfulTxMode::Dma => Some(tx_ch.trans_count().read()),
        },
        rx_remaining: match rx_mode {
            EmbassyFaithfulRxMode::Cpu => 0,
            EmbassyFaithfulRxMode::Dma => rx_ch.trans_count().read(),
        },
        tx_busy: match tx_mode {
            EmbassyFaithfulTxMode::Cpu => None,
            EmbassyFaithfulTxMode::Dma => Some(tx_ch.ctrl_trig().read().busy()),
        },
        rx_busy: match rx_mode {
            EmbassyFaithfulRxMode::Cpu => false,
            EmbassyFaithfulRxMode::Dma => rx_ch.ctrl_trig().read().busy(),
        },
        tx_ahb_error: match tx_mode {
            EmbassyFaithfulTxMode::Cpu => None,
            EmbassyFaithfulTxMode::Dma => Some(tx_ch.ctrl_trig().read().ahb_error()),
        },
        rx_ahb_error: match rx_mode {
            EmbassyFaithfulRxMode::Cpu => false,
            EmbassyFaithfulRxMode::Dma => rx_ch.ctrl_trig().read().ahb_error(),
        },
        sm_pc: sm.get_addr(),
        shiftctrl: pio1.sm(1).shiftctrl().read().0,
        execctrl: pio1.sm(1).execctrl().read().0,
        pinctrl: pio1.sm(1).pinctrl().read().0,
        fifo_levels: pio1.flevel().read().0,
        fifo_stat: pio1.fstat().read().0,
        fdebug: pio1.fdebug().read().0,
        command_words: cmd,
        out_bits: cmd[0],
        in_bits: cmd[1],
        first_16: dst,
        pass,
        classification: if leading_out_bits_byte {
            Some("leading_out_bits_byte")
        } else {
            None
        },
        likely_cause: if leading_out_bits_byte {
            Some("command/control byte leaked into RX stream or RX FIFO/ISR was not empty before DMA")
        } else {
            None
        },
    };

    tx_ch.ctrl_trig().write_value(CtrlTrig(0));
    rx_ch.ctrl_trig().write_value(CtrlTrig(0));
    sm.set_enable(false);
    sm.clear_fifos();
    pio1.input_sync_bypass()
        .write(|w| *w = input_sync_bypass_before);
    if sm0_was_enabled {
        pio1.ctrl().modify(|w| {
            w.set_sm_enable(w.sm_enable() | (1u8 << 0));
        });
    }

    report
}

#[cfg(feature = "psram_diag_hazardous_bench")]
fn log_phase_edge_experiment<'a>(
    uart: &mut UartTx<'_, Blocking>,
    common: &mut embassy_rp::pio::Common<'a, peripherals::PIO1>,
    sm: &mut StateMachine<'a, peripherals::PIO1, 1>,
    variant_name: &'static str,
    program_name: &'static str,
    program: pio::Program<32>,
    clkdiv_int: u16,
    clkdiv_frac: u8,
    address: u32,
    expected: &[u8],
) {
    let program_len = program.code.len() as u8;
    let loaded = common.load_program(&program);
    let report = run_embassy_faithful_variant(
        variant_name,
        program_name,
        program_len,
        sm,
        &loaded,
        address,
        expected,
        EmbassyFaithfulTxMode::Cpu,
        EmbassyFaithfulRxMode::Dma,
        clkdiv_int,
        clkdiv_frac,
    );

    let mut lb = LineBuffer::new();
    let _ = write!(
        lb,
        "phase=335 psram-phase variant={} program={} clkdiv={}.{:03} read_us={} rx_remaining={} pass={} classification={} first_rx16=",
        report.variant_name,
        report.program_name,
        clkdiv_int,
        ((u32::from(clkdiv_frac) * 1000) / 256),
        report.read_us,
        report.rx_remaining,
        report.pass,
        report.classification.unwrap_or("none")
    );
    write_hex(&mut lb, &report.first_16);
    let _ = write!(lb, "\r\n");
    let _ = uart.blocking_write(lb.as_bytes());

    unsafe {
        common.free_instr(loaded.used_memory);
    }
}

#[cfg(feature = "psram_pio_word_diag")]
fn run_embassy_faithful_read_diag_variant<'a>(
    variant_name: &'static str,
    program_name: &'static str,
    program_length: u8,
    sm: &mut StateMachine<'a, peripherals::PIO1, 1>,
    loaded: &LoadedProgram<'a, peripherals::PIO1>,
    base_address: u32,
    rx_len: usize,
    pattern: ReadDiagPattern,
) -> EmbassyFaithfulReadDiagReport {
    if rx_len == 0 {
        return EmbassyFaithfulReadDiagReport {
            variant_name,
            rx_len,
            read_us: 0,
            bandwidth_milli_mb_s: 0,
            rx_remaining: 0,
            timed_out: false,
            ahb_error: false,
            control_leakage: false,
            leading_dummy_byte: false,
            pass: false,
            first_16: [0; 16],
            last_16: [0; 16],
            first_mismatch: Some(0),
            mismatch_expected: 0,
            mismatch_actual: 0,
        };
    }

    let mut total_us = 0u64;
    let mut rx_remaining = 0u32;
    let mut timed_out = false;
    let mut ahb_error = false;
    let mut control_leakage = false;
    let mut leading_dummy_byte = false;
    let mut first_mismatch: Option<usize> = None;
    let mut mismatch_expected = 0u8;
    let mut mismatch_actual = 0u8;
    let mut first_16 = [0u8; 16];
    let mut last_16 = [0u8; 16];
    let mut chunk_base_offset = 0usize;

    while chunk_base_offset < rx_len {
        let chunk_len = (rx_len - chunk_base_offset).min(16);
        let chunk_address = base_address + chunk_base_offset as u32;
        let mut expected_chunk = [0u8; 16];
        for (i, slot) in expected_chunk.iter_mut().take(chunk_len).enumerate() {
            *slot = pattern_byte(pattern, base_address, chunk_base_offset + i);
        }

        let report = run_embassy_faithful_variant(
            variant_name,
            program_name,
            program_length,
            sm,
            loaded,
            chunk_address,
            &expected_chunk[..chunk_len],
            EmbassyFaithfulTxMode::Cpu,
            EmbassyFaithfulRxMode::Dma,
            4,
            0,
        );

        if chunk_base_offset == 0 {
            first_16.copy_from_slice(&report.first_16);
            control_leakage = report.classification == Some("leading_out_bits_byte");
            if chunk_len >= 2 {
                leading_dummy_byte = report.first_16[0] == 0
                    && report.first_16[1..chunk_len] == expected_chunk[..chunk_len - 1];
            }
        }
        if chunk_len == 16 {
            last_16.copy_from_slice(&report.first_16);
        } else {
            last_16.fill(0);
            last_16[..chunk_len].copy_from_slice(&report.first_16[..chunk_len]);
        }

        if first_mismatch.is_none() {
            if let Some(local_off) = expected_chunk[..chunk_len]
                .iter()
                .zip(report.first_16[..chunk_len].iter())
                .position(|(e, a)| e != a)
            {
                first_mismatch = Some(chunk_base_offset + local_off);
                mismatch_expected = expected_chunk[local_off];
                mismatch_actual = report.first_16[local_off];
            }
        }

        total_us = total_us.saturating_add(report.read_us);
        rx_remaining = report.rx_remaining;
        timed_out |= report.rx_busy || report.rx_remaining != 0;
        ahb_error |= report.rx_ahb_error;
        chunk_base_offset += chunk_len;
    }

    let bytes = rx_len as u64;
    let bandwidth_milli_mb_s = if total_us == 0 {
        0
    } else {
        bytes
            .saturating_mul(1_000_000)
            .saturating_mul(1000)
            .saturating_div(total_us)
            .saturating_div(1_000_000)
    };

    let pass = first_mismatch.is_none()
        && rx_remaining == 0
        && !timed_out
        && !ahb_error
        && !control_leakage
        && !leading_dummy_byte;

    EmbassyFaithfulReadDiagReport {
        variant_name,
        rx_len,
        read_us: total_us,
        bandwidth_milli_mb_s,
        rx_remaining,
        timed_out,
        ahb_error,
        control_leakage,
        leading_dummy_byte,
        pass,
        first_16,
        last_16,
        first_mismatch,
        mismatch_expected,
        mismatch_actual,
    }
}

fn write_hex(lb: &mut LineBuffer, bytes: &[u8]) {
    for b in bytes {
        let _ = write!(lb, "{:02x}", b);
    }
}

struct LineBuffer {
    // Keep stack usage modest on embedded target; detailed logs are flushed
    // in multiple chunks for dma16_v2 timeout path.
    bytes: [u8; 1024],
    len: usize,
}

impl LineBuffer {
    const fn new() -> Self {
        Self {
            bytes: [0; 1024],
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
