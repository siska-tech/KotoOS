#![cfg_attr(all(feature = "rp2040-embassy", target_os = "none"), no_std)]
#![cfg_attr(all(feature = "rp2040-embassy", target_os = "none"), no_main)]

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
use embassy_rp::{
    uart::{Blocking, Config as UartConfig, UartTx},
    Peri,
};

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
use embassy_time::Instant;

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
use koto_psram::{
    addr::PsramAddr,
    bus::PsramBus,
    config::{Pins, TimingConfig},
    pio::blocking::{BlockingDriver, BlockingPio},
    rp2040_embassy::{
        EmbassyRpQpiBackend, PayloadTransferPath, TransactionPioDiagnostics,
        TransactionPioFastReadLoopVariant, TransactionPioTxDmaStep, WordStreamReadDiagnostics,
    },
};

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
const PICOCALC_UART_USB_BAUD: u32 = 115_200;

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
const BENCH_ADDR: u32 = 0x0000_2000;

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
const CHUNK_LEN: usize = 512;

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
const MAX_BUFFER_BYTES: usize = 64 * 1024;

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
const TRANSACTION_PIO_INITIAL_TOTAL_BYTES: [usize; 1] = [16];

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
const TRANSACTION_PIO_EXTENDED_TOTAL_BYTES: [usize; 3] = [32, 64, 512];

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
const TRANSACTION_PIO_MULTI_TOTAL_BYTES: [usize; 2] = [1024, 4096];

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
const TRANSACTION_PIO_OPTIONAL_TOTAL_BYTES: [usize; 1] = [16 * 1024];

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
const TRANSACTION_PIO_TX_DMA_TOTAL_BYTES: [usize; 4] = [16, 512, 4096, 16 * 1024];

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
const TRANSACTION_PIO_RX_DMA_TOTAL_BYTES: [usize; 4] = [16, 512, 4096, 16 * 1024];

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
const TRANSACTION_PIO_TX_RX_DMA_TOTAL_BYTES: [usize; 4] = [16, 512, 4096, 16 * 1024];

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
const TRANSACTION_PIO_RX_DMA_STAGING_BYTES: usize = 4096;

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
const RX_BYTE_FIFO_PROBE_TIMEOUT_POLLS: u32 = 10_000;

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
const FAST_CURRENT_CLKDIV_SWEEP: [ReadClkdivCandidate; 5] = [
    ReadClkdivCandidate::new(40),
    ReadClkdivCandidate::new(30),
    ReadClkdivCandidate::new(20),
    ReadClkdivCandidate::new(15),
    ReadClkdivCandidate::new(10),
];

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
const FAST_CURRENT_CLKDIV_SWEEP_CASES: [FastCurrentClkdivSweepCase; 6] = [
    FastCurrentClkdivSweepCase::new(16, BenchPattern::RepeatedAd),
    FastCurrentClkdivSweepCase::new(16, BenchPattern::WalkingByte),
    FastCurrentClkdivSweepCase::new(16, BenchPattern::AddressDerived),
    FastCurrentClkdivSweepCase::new(512, BenchPattern::AddressDerived),
    FastCurrentClkdivSweepCase::new(4096, BenchPattern::AddressDerived),
    FastCurrentClkdivSweepCase::new(16 * 1024, BenchPattern::AddressDerived),
];

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
const FAST_READ_LOOP_VARIANTS: [TransactionPioFastReadLoopVariant; 4] = [
    TransactionPioFastReadLoopVariant::CurrentNoDelay,
    TransactionPioFastReadLoopVariant::OppositePolarityNoDelay,
    TransactionPioFastReadLoopVariant::DelayOnIn,
    TransactionPioFastReadLoopVariant::DelayOnJmp,
];

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
const FAST_FALLING_READ_LOOP_VARIANTS: [TransactionPioFastReadLoopVariant; 4] = [
    TransactionPioFastReadLoopVariant::FallingFudgeA,
    TransactionPioFastReadLoopVariant::FallingFudgeB,
    TransactionPioFastReadLoopVariant::FallingNoFudge,
    TransactionPioFastReadLoopVariant::FallingFudgeExtraLow,
];

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
const FAST_FALLING_ALIGNMENT_VARIANTS: [FastFallingAlignmentVariant; 4] = [
    FastFallingAlignmentVariant::DiscardFirstByte,
    FastFallingAlignmentVariant::DiscardFirstNibble,
    FastFallingAlignmentVariant::ExtraDummyHalfCycle,
    FastFallingAlignmentVariant::ExtraDummyByte,
];

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
const FAST_FALLING_ALIGNMENT_SHORT_CASES: [FastCurrentClkdivSweepCase; 3] = [
    FastCurrentClkdivSweepCase::new(16, BenchPattern::RepeatedAd),
    FastCurrentClkdivSweepCase::new(16, BenchPattern::WalkingByte),
    FastCurrentClkdivSweepCase::new(16, BenchPattern::AddressDerived),
];

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
const FAST_FALLING_ALIGNMENT_LONG_CASES: [FastCurrentClkdivSweepCase; 3] = [
    FastCurrentClkdivSweepCase::new(512, BenchPattern::AddressDerived),
    FastCurrentClkdivSweepCase::new(4096, BenchPattern::AddressDerived),
    FastCurrentClkdivSweepCase::new(16 * 1024, BenchPattern::AddressDerived),
];

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
const BENCH_PATTERNS: [BenchPattern; 3] = [
    BenchPattern::RepeatedAd,
    BenchPattern::WalkingByte,
    BenchPattern::AddressDerived,
];

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
static mut WRITE_BUF: [u8; MAX_BUFFER_BYTES] = [0; MAX_BUFFER_BYTES];

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
static mut READ_BUF: [u8; MAX_BUFFER_BYTES] = [0; MAX_BUFFER_BYTES];

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
embassy_rp::bind_interrupts!(struct Irqs {
    PIO0_IRQ_0 => embassy_rp::pio::InterruptHandler<embassy_rp::peripherals::PIO0>;
    DMA_IRQ_0 => embassy_rp::dma::InterruptHandler<embassy_rp::peripherals::DMA_CH0>, embassy_rp::dma::InterruptHandler<embassy_rp::peripherals::DMA_CH1>;
});

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
type PicocalcPayloadBackend<'d> = EmbassyRpQpiBackend<
    'd,
    embassy_rp::peripherals::PIO0,
    0,
    embassy_rp::peripherals::PIN_2,
    embassy_rp::peripherals::PIN_3,
    embassy_rp::peripherals::PIN_4,
    embassy_rp::peripherals::PIN_5,
    embassy_rp::peripherals::PIN_20,
    embassy_rp::peripherals::PIN_21,
>;

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
type PayloadBackendError = <PicocalcPayloadBackend<'static> as BlockingPio>::Error;

#[cfg(not(all(feature = "rp2040-embassy", target_os = "none")))]
fn main() {}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
#[cortex_m_rt::entry]
fn embedded_main() -> ! {
    let peripherals = embassy_rp::init(Default::default());
    let mut uart = picocalc_uart_usb_tx(peripherals.UART0, peripherals.PIN_0);
    register_panic_uart(&mut uart);

    log_line(&mut uart, "transaction pio diag start");
    log_line(&mut uart, "backend configure start");
    let pio = embassy_rp::pio::Pio::new(peripherals.PIO0, Irqs);
    let backend = EmbassyRpQpiBackend::new(
        pio.common,
        pio.sm0,
        peripherals.PIN_2,
        peripherals.PIN_3,
        peripherals.PIN_4,
        peripherals.PIN_5,
        peripherals.PIN_20,
        peripherals.PIN_21,
    );
    let tx_dma_channel = embassy_rp::dma::Channel::new(peripherals.DMA_CH0, Irqs);
    let rx_dma_channel = embassy_rp::dma::Channel::new(peripherals.DMA_CH1, Irqs);
    let backend = backend.with_tx_dma_channel_id(tx_dma_channel, 0);
    let backend = backend.with_rx_dma_channel_id(rx_dma_channel, 1);
    log_line(&mut uart, "backend configure ok");

    let write = unsafe { &mut *core::ptr::addr_of_mut!(WRITE_BUF) };
    let read = unsafe { &mut *core::ptr::addr_of_mut!(READ_BUF) };
    run_transaction_pio_diag(&mut uart, backend, write, read);
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn run_transaction_pio_diag(
    uart: &mut UartTx<'static, Blocking>,
    backend: PicocalcPayloadBackend<'static>,
    write: &mut [u8; MAX_BUFFER_BYTES],
    read: &mut [u8; MAX_BUFFER_BYTES],
) -> ! {
    let timing = TimingConfig {
        max_chunk_len: CHUNK_LEN,
        ..TimingConfig::PICOCALC_FAST_CANDIDATE
    };
    let mut driver = BlockingDriver::with_config(backend, Pins::PICOCALC, timing);

    log_line(uart, "driver.init start");
    match driver.init() {
        Ok(id) => {
            log_str(uart, "driver.init ok id=");
            log_hex_u8(uart, id.raw[0]);
            log_byte(uart, b' ');
            log_hex_u8(uart, id.raw[1]);
            log_byte(uart, b' ');
            log_hex_u8(uart, id.raw[2]);
            log_newline(uart);
        }
        Err(_error) => {
            log_line(uart, "driver.init error");
            loop {}
        }
    }

    run_transaction_pio_diagnostic_bench(uart, &mut driver, write, read);
    run_transaction_pio_tx_dma_diagnostic_bench(uart, &mut driver, write, read);
    run_transaction_pio_rx_dma_diagnostic_bench(uart, &mut driver, write, read);
    run_transaction_pio_tx_rx_dma_diagnostic_bench(uart, &mut driver, write, read);
    run_transaction_pio_rx_byte_fifo_probe(uart, &mut driver, write, read);

    log_line(uart, "transaction pio diag done");
    loop {}
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn run_transaction_pio_tx_dma_diagnostic_bench(
    uart: &mut UartTx<'static, Blocking>,
    driver: &mut BlockingDriver<PicocalcPayloadBackend<'static>>,
    write: &mut [u8; MAX_BUFFER_BYTES],
    read: &mut [u8; MAX_BUFFER_BYTES],
) {
    set_word_stream_read_diagnostics(uart, driver, WordStreamReadDiagnostics::default());

    for total_bytes in TRANSACTION_PIO_TX_DMA_TOTAL_BYTES {
        for pattern in BENCH_PATTERNS {
            if !run_one_transaction_pio_diagnostic_bench_with_path(
                uart,
                driver,
                write,
                read,
                PayloadTransferPath::TransactionPioTxDmaDiagnostic,
                total_bytes,
                pattern,
            ) {
                return;
            }
        }
    }
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn run_transaction_pio_rx_dma_diagnostic_bench(
    uart: &mut UartTx<'static, Blocking>,
    driver: &mut BlockingDriver<PicocalcPayloadBackend<'static>>,
    write: &mut [u8; MAX_BUFFER_BYTES],
    read: &mut [u8; MAX_BUFFER_BYTES],
) {
    set_word_stream_read_diagnostics(uart, driver, WordStreamReadDiagnostics::default());

    for total_bytes in TRANSACTION_PIO_RX_DMA_TOTAL_BYTES {
        for pattern in BENCH_PATTERNS {
            if !run_one_transaction_pio_diagnostic_bench_with_path(
                uart,
                driver,
                write,
                read,
                PayloadTransferPath::TransactionPioRxDmaDiagnostic,
                total_bytes,
                pattern,
            ) {
                return;
            }
        }
    }
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn run_transaction_pio_rx_byte_fifo_probe(
    uart: &mut UartTx<'static, Blocking>,
    driver: &mut BlockingDriver<PicocalcPayloadBackend<'static>>,
    write: &mut [u8; MAX_BUFFER_BYTES],
    read: &mut [u8; MAX_BUFFER_BYTES],
) {
    set_word_stream_read_diagnostics(uart, driver, WordStreamReadDiagnostics::default());
    log_line(uart, "rx_byte_fifo_probe start");

    let timing = TimingConfig {
        max_chunk_len: TRANSACTION_PIO_RX_DMA_STAGING_BYTES,
        timeout_polls: RX_BYTE_FIFO_PROBE_TIMEOUT_POLLS,
        ..TimingConfig::PICOCALC_FAST_CANDIDATE
    };
    if driver.configure_timing(timing).is_err() {
        log_line(
            uart,
            "transaction_pio_rx_byte_fifo_probe status=unsupported reason=configure_timing",
        );
        log_line(uart, "rx_byte_fifo_probe done");
        return;
    }

    for pattern in BENCH_PATTERNS {
        if !run_one_transaction_pio_rx_byte_fifo_probe(
            uart,
            driver,
            write,
            read,
            PayloadTransferPath::TransactionPioRxByteFifoDiagnostic,
            16,
            pattern,
            None,
        ) {
            log_line(uart, "rx_byte_fifo_probe done");
            return;
        }
    }

    log_line(uart, "rx_byte_fifo_dma_probe start");
    for pattern in BENCH_PATTERNS {
        if !run_one_transaction_pio_rx_byte_fifo_probe(
            uart,
            driver,
            write,
            read,
            PayloadTransferPath::TransactionPioRxByteFifoRxDmaDiagnostic,
            16,
            pattern,
            None,
        ) {
            log_line(uart, "rx_byte_fifo_dma_probe done");
            log_line(uart, "rx_byte_fifo_probe done");
            return;
        }
    }

    for total_bytes in [512, 4096, 16 * 1024] {
        if !run_one_transaction_pio_rx_byte_fifo_probe(
            uart,
            driver,
            write,
            read,
            PayloadTransferPath::TransactionPioRxByteFifoRxDmaDiagnostic,
            total_bytes,
            BenchPattern::AddressDerived,
            None,
        ) {
            log_line(uart, "rx_byte_fifo_dma_probe done");
            log_line(uart, "rx_byte_fifo_probe done");
            return;
        }
    }

    log_line(uart, "rx_byte_fifo_dma_probe done");

    run_transaction_pio_fast_rx_byte_fifo_rx_dma_probe(uart, driver, write, read);
    run_transaction_pio_fast_current_clkdiv_sweep(uart, driver, write, read);
    run_transaction_pio_fast_falling_clkdiv_diagnostic(uart, driver, write, read);
    run_transaction_pio_fast_falling_alignment_diagnostic(uart, driver, write, read);

    log_line(uart, "rx_byte_fifo_probe done");
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn run_transaction_pio_fast_rx_byte_fifo_rx_dma_probe(
    uart: &mut UartTx<'static, Blocking>,
    driver: &mut BlockingDriver<PicocalcPayloadBackend<'static>>,
    write: &mut [u8; MAX_BUFFER_BYTES],
    read: &mut [u8; MAX_BUFFER_BYTES],
) {
    log_line(uart, "fast_rx_byte_fifo_dma_probe start");

    for variant in FAST_READ_LOOP_VARIANTS {
        log_str(uart, "fast_rx_byte_fifo_dma_probe variant=");
        log_fast_read_loop_variant(uart, variant);
        log_line(uart, " start");

        if !run_one_transaction_pio_rx_byte_fifo_probe(
            uart,
            driver,
            write,
            read,
            PayloadTransferPath::TransactionPioFastRxByteFifoRxDmaDiagnostic,
            16,
            BenchPattern::RepeatedAd,
            Some(variant),
        ) {
            log_str(uart, "fast_rx_byte_fifo_dma_probe variant=");
            log_fast_read_loop_variant(uart, variant);
            log_line(uart, " status=failed_initial");
            continue;
        }

        let mut short_patterns_passed = true;
        for pattern in [BenchPattern::WalkingByte, BenchPattern::AddressDerived] {
            if !run_one_transaction_pio_rx_byte_fifo_probe(
                uart,
                driver,
                write,
                read,
                PayloadTransferPath::TransactionPioFastRxByteFifoRxDmaDiagnostic,
                16,
                pattern,
                Some(variant),
            ) {
                short_patterns_passed = false;
                break;
            }
        }
        if !short_patterns_passed {
            log_str(uart, "fast_rx_byte_fifo_dma_probe variant=");
            log_fast_read_loop_variant(uart, variant);
            log_line(uart, " status=failed_short_patterns");
            continue;
        }

        let mut long_patterns_passed = true;
        for total_bytes in [512, 4096, 16 * 1024] {
            if !run_one_transaction_pio_rx_byte_fifo_probe(
                uart,
                driver,
                write,
                read,
                PayloadTransferPath::TransactionPioFastRxByteFifoRxDmaDiagnostic,
                total_bytes,
                BenchPattern::AddressDerived,
                Some(variant),
            ) {
                long_patterns_passed = false;
                break;
            }
        }

        log_str(uart, "fast_rx_byte_fifo_dma_probe variant=");
        log_fast_read_loop_variant(uart, variant);
        log_str(uart, " status=");
        if long_patterns_passed {
            log_str(uart, "passed expected_ceiling_mb_s_x100=");
            log_dec_u64(
                uart,
                expected_fast_read_loop_ceiling_mb_s_x100(variant, driver.timing().read_clkdiv),
            );
        } else {
            log_str(uart, "failed_long_patterns expected_ceiling_mb_s_x100=");
            log_dec_u64(
                uart,
                expected_fast_read_loop_ceiling_mb_s_x100(variant, driver.timing().read_clkdiv),
            );
        }
        log_newline(uart);
    }

    log_line(uart, "fast_rx_byte_fifo_dma_probe done");
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn run_transaction_pio_fast_current_clkdiv_sweep(
    uart: &mut UartTx<'static, Blocking>,
    driver: &mut BlockingDriver<PicocalcPayloadBackend<'static>>,
    write: &mut [u8; MAX_BUFFER_BYTES],
    read: &mut [u8; MAX_BUFFER_BYTES],
) {
    log_line(uart, "fast_current_clkdiv_sweep start");

    let mut fastest_stable = None;
    for candidate in FAST_CURRENT_CLKDIV_SWEEP {
        log_str(
            uart,
            "fast_current_clkdiv_sweep variant=current_no_delay read_clkdiv=",
        );
        log_read_clkdiv(uart, candidate);
        log_line(uart, " start");

        if configure_fast_current_clkdiv_sweep_timing(uart, driver, candidate).is_err() {
            log_str(
                uart,
                "fast_current_clkdiv_sweep variant=current_no_delay read_clkdiv=",
            );
            log_read_clkdiv(uart, candidate);
            log_line(uart, " status=error error=configure_timing");
            continue;
        }

        let mut passed = true;
        for case in FAST_CURRENT_CLKDIV_SWEEP_CASES {
            if !run_one_transaction_pio_rx_byte_fifo_probe(
                uart,
                driver,
                write,
                read,
                PayloadTransferPath::TransactionPioFastRxByteFifoRxDmaDiagnostic,
                case.total_bytes,
                case.pattern,
                Some(TransactionPioFastReadLoopVariant::CurrentNoDelay),
            ) {
                passed = false;
                break;
            }
        }

        log_str(
            uart,
            "fast_current_clkdiv_sweep variant=current_no_delay read_clkdiv=",
        );
        log_read_clkdiv(uart, candidate);
        log_str(uart, " cycles_per_nibble=2 expected_ceiling_mb_s_x100=");
        log_dec_u64(uart, expected_fast_current_ceiling_mb_s_x100(candidate));
        log_str(uart, " status=");
        if passed {
            fastest_stable = Some(candidate);
            log_str(uart, "passed");
        } else {
            log_str(uart, "failed");
        }
        log_newline(uart);
    }

    log_str(
        uart,
        "fast_current_clkdiv_sweep fastest_stable_read_clkdiv=",
    );
    if let Some(candidate) = fastest_stable {
        log_read_clkdiv(uart, candidate);
        log_str(uart, " expected_ceiling_mb_s_x100=");
        log_dec_u64(uart, expected_fast_current_ceiling_mb_s_x100(candidate));
        log_line(uart, " status=ok");
    } else {
        log_line(uart, "none status=error");
    }

    let safe = ReadClkdivCandidate::new(40);
    let _ = configure_fast_current_clkdiv_sweep_timing(uart, driver, safe);
    log_line(uart, "fast_current_clkdiv_sweep done");
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn run_transaction_pio_fast_falling_clkdiv_diagnostic(
    uart: &mut UartTx<'static, Blocking>,
    driver: &mut BlockingDriver<PicocalcPayloadBackend<'static>>,
    write: &mut [u8; MAX_BUFFER_BYTES],
    read: &mut [u8; MAX_BUFFER_BYTES],
) {
    log_line(uart, "fast_falling_clkdiv_diagnostic start");

    let clkdiv_2 = ReadClkdivCandidate::new(20);
    let clkdiv_2_passed =
        run_transaction_pio_fast_falling_clkdiv_diagnostic_at(uart, driver, write, read, clkdiv_2);

    if !clkdiv_2_passed {
        let clkdiv_2_5 = ReadClkdivCandidate::new(25);
        let clkdiv_2_5_passed = run_transaction_pio_fast_falling_clkdiv_diagnostic_at(
            uart, driver, write, read, clkdiv_2_5,
        );
        log_str(uart, "fast_falling_clkdiv_diagnostic fallback_read_clkdiv=");
        log_read_clkdiv(uart, clkdiv_2_5);
        log_str(uart, " status=");
        if clkdiv_2_5_passed {
            log_str(uart, "passed");
        } else {
            log_str(uart, "failed");
        }
        log_newline(uart);
    }

    let safe = ReadClkdivCandidate::new(40);
    let _ = configure_fast_current_clkdiv_sweep_timing(uart, driver, safe);
    log_line(uart, "fast_falling_clkdiv_diagnostic done");
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn run_transaction_pio_fast_falling_clkdiv_diagnostic_at(
    uart: &mut UartTx<'static, Blocking>,
    driver: &mut BlockingDriver<PicocalcPayloadBackend<'static>>,
    write: &mut [u8; MAX_BUFFER_BYTES],
    read: &mut [u8; MAX_BUFFER_BYTES],
    candidate: ReadClkdivCandidate,
) -> bool {
    let mut any_variant_passed = false;

    for variant in FAST_FALLING_READ_LOOP_VARIANTS {
        log_str(uart, "fast_falling_clkdiv_diagnostic variant=");
        log_fast_read_loop_variant(uart, variant);
        log_str(uart, " read_clkdiv=");
        log_read_clkdiv(uart, candidate);
        log_line(uart, " start");

        if configure_fast_read_loop_clkdiv_timing(uart, driver, candidate, variant).is_err() {
            log_str(uart, "fast_falling_clkdiv_diagnostic variant=");
            log_fast_read_loop_variant(uart, variant);
            log_str(uart, " read_clkdiv=");
            log_read_clkdiv(uart, candidate);
            log_line(uart, " status=error error=configure_timing");
            continue;
        }

        let mut passed = true;
        for case in FAST_CURRENT_CLKDIV_SWEEP_CASES {
            if !run_one_transaction_pio_rx_byte_fifo_probe(
                uart,
                driver,
                write,
                read,
                PayloadTransferPath::TransactionPioFastRxByteFifoRxDmaDiagnostic,
                case.total_bytes,
                case.pattern,
                Some(variant),
            ) {
                passed = false;
                break;
            }
        }

        log_str(uart, "fast_falling_clkdiv_diagnostic variant=");
        log_fast_read_loop_variant(uart, variant);
        log_str(uart, " read_clkdiv=");
        log_read_clkdiv(uart, candidate);
        log_str(uart, " cycles_per_nibble=");
        log_dec_usize(uart, fast_read_loop_cycles_per_nibble(variant));
        log_str(uart, " pre_read_fudge_cycles=");
        log_dec_usize(uart, fast_read_loop_pre_read_fudge_cycles(variant));
        log_str(uart, " sample_edge=");
        log_fast_read_loop_sample_edge(uart, variant);
        log_str(uart, " expected_ceiling_mb_s_x100=");
        log_dec_u64(
            uart,
            expected_fast_read_loop_ceiling_mb_s_x100(variant, candidate.as_f32()),
        );
        log_str(uart, " baseline_clkdiv3_mb_s_x100=753 status=");
        if passed {
            any_variant_passed = true;
            log_str(uart, "passed");
        } else {
            log_str(uart, "failed");
        }
        log_newline(uart);
    }

    log_str(uart, "fast_falling_clkdiv_diagnostic read_clkdiv=");
    log_read_clkdiv(uart, candidate);
    log_str(uart, " status=");
    if any_variant_passed {
        log_str(uart, "passed");
    } else {
        log_str(uart, "failed");
    }
    log_newline(uart);

    any_variant_passed
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn run_transaction_pio_fast_falling_alignment_diagnostic(
    uart: &mut UartTx<'static, Blocking>,
    driver: &mut BlockingDriver<PicocalcPayloadBackend<'static>>,
    write: &mut [u8; MAX_BUFFER_BYTES],
    read: &mut [u8; MAX_BUFFER_BYTES],
) {
    log_line(uart, "fast_falling_alignment_diagnostic start");

    let candidate = ReadClkdivCandidate::new(20);
    let mut any_variant_passed = false;
    for alignment_variant in FAST_FALLING_ALIGNMENT_VARIANTS {
        log_str(uart, "fast_falling_alignment_diagnostic variant=");
        log_fast_falling_alignment_variant(uart, alignment_variant);
        log_str(uart, " read_clkdiv=");
        log_read_clkdiv(uart, candidate);
        log_line(uart, " start");

        let pio_variant = alignment_variant.fast_read_loop_variant();
        if configure_fast_read_loop_clkdiv_timing(uart, driver, candidate, pio_variant).is_err() {
            log_str(uart, "fast_falling_alignment_diagnostic variant=");
            log_fast_falling_alignment_variant(uart, alignment_variant);
            log_line(uart, " status=error error=configure_timing");
            continue;
        }

        let mut short_passed = true;
        for case in FAST_FALLING_ALIGNMENT_SHORT_CASES {
            if !run_one_fast_falling_alignment_probe(
                uart,
                driver,
                write,
                read,
                alignment_variant,
                case.total_bytes,
                case.pattern,
            ) {
                short_passed = false;
                break;
            }
        }

        if short_passed {
            for case in FAST_FALLING_ALIGNMENT_LONG_CASES {
                if !run_one_fast_falling_alignment_probe(
                    uart,
                    driver,
                    write,
                    read,
                    alignment_variant,
                    case.total_bytes,
                    case.pattern,
                ) {
                    short_passed = false;
                    break;
                }
            }
        }

        log_str(uart, "fast_falling_alignment_diagnostic variant=");
        log_fast_falling_alignment_variant(uart, alignment_variant);
        log_str(uart, " read_clkdiv=");
        log_read_clkdiv(uart, candidate);
        log_str(uart, " sample_edge=falling pre_read_fudge_cycles=0 status=");
        if short_passed {
            any_variant_passed = true;
            log_str(uart, "passed");
        } else {
            log_str(uart, "failed");
        }
        log_newline(uart);
    }

    log_str(uart, "fast_falling_alignment_diagnostic read_clkdiv=");
    log_read_clkdiv(uart, candidate);
    log_str(uart, " status=");
    if any_variant_passed {
        log_str(uart, "passed");
    } else {
        log_str(uart, "failed reason=first_byte_alignment_not_sufficient");
    }
    log_newline(uart);

    let safe = ReadClkdivCandidate::new(40);
    let _ = configure_fast_current_clkdiv_sweep_timing(uart, driver, safe);
    log_line(uart, "fast_falling_alignment_diagnostic done");
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn configure_fast_current_clkdiv_sweep_timing(
    uart: &mut UartTx<'static, Blocking>,
    driver: &mut BlockingDriver<PicocalcPayloadBackend<'static>>,
    candidate: ReadClkdivCandidate,
) -> Result<(), PayloadBackendError> {
    configure_fast_read_loop_clkdiv_timing(
        uart,
        driver,
        candidate,
        TransactionPioFastReadLoopVariant::CurrentNoDelay,
    )
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn configure_fast_read_loop_clkdiv_timing(
    uart: &mut UartTx<'static, Blocking>,
    driver: &mut BlockingDriver<PicocalcPayloadBackend<'static>>,
    candidate: ReadClkdivCandidate,
    variant: TransactionPioFastReadLoopVariant,
) -> Result<(), PayloadBackendError> {
    let timing = TimingConfig {
        read_clkdiv: candidate.as_f32(),
        fallback_read_clkdiv: TimingConfig::PICOCALC_SAFE
            .fallback_read_clkdiv
            .max(candidate.as_f32()),
        max_chunk_len: TRANSACTION_PIO_RX_DMA_STAGING_BYTES,
        timeout_polls: RX_BYTE_FIFO_PROBE_TIMEOUT_POLLS,
        ..TimingConfig::PICOCALC_FAST_CANDIDATE
    };
    driver.configure_timing(timing)?;
    driver
        .backend_mut_for_diagnostics()
        .set_transaction_pio_fast_read_loop_variant_for_diagnostics(variant);
    set_payload_path(
        uart,
        driver,
        PayloadTransferPath::TransactionPioFastRxByteFifoRxDmaDiagnostic,
    );
    Ok(())
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn run_one_fast_falling_alignment_probe(
    uart: &mut UartTx<'static, Blocking>,
    driver: &mut BlockingDriver<PicocalcPayloadBackend<'static>>,
    write: &mut [u8; MAX_BUFFER_BYTES],
    read: &mut [u8; MAX_BUFFER_BYTES],
    alignment_variant: FastFallingAlignmentVariant,
    total_bytes: usize,
    pattern: BenchPattern,
) -> bool {
    set_payload_path(uart, driver, PayloadTransferPath::ByteFallback);
    if write_transfer(driver, write, total_bytes, pattern).is_err() {
        log_fast_falling_alignment_prefix(
            uart,
            "prefill",
            alignment_variant,
            total_bytes,
            pattern,
            0,
            TransactionPioDiagnostics::default(),
            driver.timing().read_clkdiv,
        );
        log_line(uart, " status=error error=prefill");
        return false;
    }

    driver
        .backend_mut_for_diagnostics()
        .set_transaction_pio_fast_read_loop_variant_for_diagnostics(
            alignment_variant.fast_read_loop_variant(),
        );
    set_payload_path(
        uart,
        driver,
        PayloadTransferPath::TransactionPioFastRxByteFifoRxDmaDiagnostic,
    );
    driver
        .backend_mut_for_diagnostics()
        .reset_qpi_timing_for_diagnostics();
    read[..total_bytes + alignment_variant.scratch_extra_bytes()].fill(0);

    let start = Instant::now();
    let read_result = read_fast_falling_alignment_transfer(
        driver,
        &mut read[..],
        total_bytes,
        TRANSACTION_PIO_RX_DMA_STAGING_BYTES,
        alignment_variant,
    );
    let elapsed_us = Instant::now().duration_since(start).as_micros();
    let read_result_ok = read_result.is_ok();
    let diagnostics = read_result.unwrap_or_else(|diagnostics| diagnostics);

    log_fast_falling_alignment_prefix(
        uart,
        "read",
        alignment_variant,
        total_bytes,
        pattern,
        elapsed_us,
        diagnostics,
        driver.timing().read_clkdiv,
    );
    log_str(uart, " raw_first32=");
    let raw_start = alignment_variant.raw_log_start(total_bytes);
    let raw_len = alignment_variant.raw_log_len(total_bytes);
    log_bytes_hex(uart, &read[raw_start..raw_start + raw_len]);
    log_str(uart, " visible_first16=");
    log_bytes_hex(uart, &read[..total_bytes.min(16)]);
    if read_result_ok {
        log_line(uart, " status=ok");
    } else {
        log_str(uart, " status=unsupported reason=fast_pio_timeout");
        log_rx_byte_fifo_dma_status_suffix(uart, diagnostics, true);
        return false;
    }

    fill_pattern(&mut write[..total_bytes], 0, pattern);
    let mismatch = first_mismatch(&write[..total_bytes], &read[..total_bytes]);
    log_fast_falling_alignment_prefix(
        uart,
        "compare",
        alignment_variant,
        total_bytes,
        pattern,
        0,
        diagnostics,
        driver.timing().read_clkdiv,
    );
    log_str(uart, " visible_first16=");
    log_bytes_hex(uart, &read[..total_bytes.min(16)]);
    match mismatch {
        Some(mismatch) => {
            log_str(uart, " status=mismatch fail_off=");
            log_dec_usize(uart, mismatch.offset);
            log_str(uart, " expected=0x");
            log_hex_u8(uart, mismatch.expected);
            log_str(uart, " actual=0x");
            log_hex_u8(uart, mismatch.actual);
            log_rx_byte_fifo_dma_status_suffix(uart, diagnostics, true);
            false
        }
        None => {
            log_line(uart, " status=ok");
            true
        }
    }
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn read_fast_falling_alignment_transfer(
    driver: &mut BlockingDriver<PicocalcPayloadBackend<'static>>,
    read: &mut [u8],
    total_bytes: usize,
    chunk_len: usize,
    alignment_variant: FastFallingAlignmentVariant,
) -> Result<TransactionPioDiagnostics, TransactionPioDiagnostics> {
    let mut offset = 0;
    let mut aggregate = TransactionPioDiagnostics::default();
    aggregate.rx_dma = true;

    while offset < total_bytes {
        let len = (total_bytes - offset).min(chunk_len);
        let scratch_extra = alignment_variant.scratch_extra_bytes();
        let scratch_len = len + scratch_extra;
        let scratch_start = if scratch_extra == 0 {
            offset
        } else {
            total_bytes + 32
        };
        read[scratch_start..scratch_start + scratch_len].fill(0);

        let result = driver
            .backend_mut_for_diagnostics()
            .read_qpi_chunk_transaction_pio_fast_rx_byte_fifo_rx_dma_diagnostic_for_diagnostics(
                bench_addr_at(offset),
                &mut read[scratch_start..scratch_start + scratch_len],
                |_| {},
            );
        let chunk_diagnostics = driver
            .backend_mut_for_diagnostics()
            .transaction_pio_diagnostics();
        merge_transaction_pio_diagnostics(&mut aggregate, chunk_diagnostics);
        if result.is_err() {
            return Err(aggregate);
        }

        if scratch_extra == 0 {
            offset += len;
            continue;
        }

        read.copy_within(
            scratch_start + scratch_extra..scratch_start + scratch_len,
            offset,
        );
        if offset == 0 {
            let raw_len = scratch_len.min(32);
            read.copy_within(scratch_start..scratch_start + raw_len, total_bytes);
        }
        offset += len;
    }

    Ok(aggregate)
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn run_one_transaction_pio_rx_byte_fifo_probe(
    uart: &mut UartTx<'static, Blocking>,
    driver: &mut BlockingDriver<PicocalcPayloadBackend<'static>>,
    write: &mut [u8; MAX_BUFFER_BYTES],
    read: &mut [u8; MAX_BUFFER_BYTES],
    path: PayloadTransferPath,
    total_bytes: usize,
    pattern: BenchPattern,
    fast_variant: Option<TransactionPioFastReadLoopVariant>,
) -> bool {
    let is_dma = path == PayloadTransferPath::TransactionPioRxByteFifoRxDmaDiagnostic
        || path == PayloadTransferPath::TransactionPioFastRxByteFifoRxDmaDiagnostic;

    set_payload_path(uart, driver, PayloadTransferPath::ByteFallback);
    if write_transfer(driver, write, total_bytes, pattern).is_err() {
        log_rx_byte_fifo_probe_prefix(
            uart,
            "prefill",
            path,
            total_bytes,
            pattern,
            0,
            TransactionPioDiagnostics::default(),
            fast_variant,
            driver.timing().read_clkdiv,
        );
        log_line(uart, " status=error error=prefill");
        return false;
    }

    if let Some(variant) = fast_variant {
        driver
            .backend_mut_for_diagnostics()
            .set_transaction_pio_fast_read_loop_variant_for_diagnostics(variant);
    }
    set_payload_path(uart, driver, path);
    driver
        .backend_mut_for_diagnostics()
        .reset_qpi_timing_for_diagnostics();
    read[..total_bytes].fill(0);

    let start = Instant::now();
    let read_result = if path == PayloadTransferPath::TransactionPioFastRxByteFifoRxDmaDiagnostic {
        read_transaction_pio_fast_rx_byte_fifo_rx_dma_transfer(
            driver,
            &mut read[..total_bytes],
            total_bytes,
            TRANSACTION_PIO_RX_DMA_STAGING_BYTES,
        )
    } else if is_dma {
        read_transaction_pio_rx_byte_fifo_rx_dma_transfer(
            driver,
            &mut read[..total_bytes],
            total_bytes,
            TRANSACTION_PIO_RX_DMA_STAGING_BYTES,
        )
    } else {
        driver
            .backend_mut_for_diagnostics()
            .read_qpi_chunk_transaction_pio_rx_byte_fifo_diagnostic_for_diagnostics(
                bench_addr_at(0),
                &mut read[..total_bytes],
            )
            .map(|_| {
                driver
                    .backend_mut_for_diagnostics()
                    .transaction_pio_diagnostics()
            })
            .map_err(|_| {
                driver
                    .backend_mut_for_diagnostics()
                    .transaction_pio_diagnostics()
            })
    };
    let elapsed_us = Instant::now().duration_since(start).as_micros();
    let read_result_ok = read_result.is_ok();
    let diagnostics = read_result.unwrap_or_else(|diagnostics| diagnostics);

    log_rx_byte_fifo_probe_prefix(
        uart,
        "read",
        path,
        total_bytes,
        pattern,
        elapsed_us,
        diagnostics,
        fast_variant,
        driver.timing().read_clkdiv,
    );
    log_str(uart, " received_bytes=");
    log_bytes_hex(uart, &read[..total_bytes.min(16)]);
    if read_result_ok {
        log_line(uart, " status=ok");
    } else {
        if path == PayloadTransferPath::TransactionPioFastRxByteFifoRxDmaDiagnostic {
            log_str(uart, " status=unsupported reason=fast_pio_timeout");
        } else {
            log_str(uart, " status=unsupported reason=rx_byte_fifo_timeout");
        }
        log_rx_byte_fifo_dma_status_suffix(uart, diagnostics, is_dma);
        return false;
    }

    fill_pattern(&mut write[..total_bytes], 0, pattern);
    let mismatch = first_mismatch(&write[..total_bytes], &read[..total_bytes]);
    log_rx_byte_fifo_probe_prefix(
        uart,
        "compare",
        path,
        total_bytes,
        pattern,
        0,
        diagnostics,
        fast_variant,
        driver.timing().read_clkdiv,
    );
    log_str(uart, " received_bytes=");
    log_bytes_hex(uart, &read[..total_bytes.min(16)]);
    match mismatch {
        Some(mismatch) => {
            log_str(uart, " status=mismatch fail_off=");
            log_dec_usize(uart, mismatch.offset);
            log_str(uart, " expected=0x");
            log_hex_u8(uart, mismatch.expected);
            log_str(uart, " actual=0x");
            log_hex_u8(uart, mismatch.actual);
            log_rx_byte_fifo_dma_status_suffix(uart, diagnostics, is_dma);
            false
        }
        None => {
            log_line(uart, " status=ok");
            true
        }
    }
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn read_transaction_pio_rx_byte_fifo_rx_dma_transfer(
    driver: &mut BlockingDriver<PicocalcPayloadBackend<'static>>,
    read: &mut [u8],
    total_bytes: usize,
    chunk_len: usize,
) -> Result<TransactionPioDiagnostics, TransactionPioDiagnostics> {
    let mut offset = 0;
    let mut aggregate = TransactionPioDiagnostics::default();
    aggregate.rx_dma = true;

    while offset < total_bytes {
        let len = (total_bytes - offset).min(chunk_len);
        let result = driver
            .backend_mut_for_diagnostics()
            .read_qpi_chunk_transaction_pio_rx_byte_fifo_rx_dma_diagnostic_for_diagnostics(
                bench_addr_at(offset),
                &mut read[offset..offset + len],
                |_| {},
            );
        let chunk_diagnostics = driver
            .backend_mut_for_diagnostics()
            .transaction_pio_diagnostics();
        merge_transaction_pio_diagnostics(&mut aggregate, chunk_diagnostics);
        if result.is_err() {
            return Err(aggregate);
        }
        offset += len;
    }

    Ok(aggregate)
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn read_transaction_pio_fast_rx_byte_fifo_rx_dma_transfer(
    driver: &mut BlockingDriver<PicocalcPayloadBackend<'static>>,
    read: &mut [u8],
    total_bytes: usize,
    chunk_len: usize,
) -> Result<TransactionPioDiagnostics, TransactionPioDiagnostics> {
    let mut offset = 0;
    let mut aggregate = TransactionPioDiagnostics::default();
    aggregate.rx_dma = true;

    while offset < total_bytes {
        let len = (total_bytes - offset).min(chunk_len);
        let result = driver
            .backend_mut_for_diagnostics()
            .read_qpi_chunk_transaction_pio_fast_rx_byte_fifo_rx_dma_diagnostic_for_diagnostics(
                bench_addr_at(offset),
                &mut read[offset..offset + len],
                |_| {},
            );
        let chunk_diagnostics = driver
            .backend_mut_for_diagnostics()
            .transaction_pio_diagnostics();
        merge_transaction_pio_diagnostics(&mut aggregate, chunk_diagnostics);
        if result.is_err() {
            return Err(aggregate);
        }
        offset += len;
    }

    Ok(aggregate)
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn run_transaction_pio_tx_rx_dma_diagnostic_bench(
    uart: &mut UartTx<'static, Blocking>,
    driver: &mut BlockingDriver<PicocalcPayloadBackend<'static>>,
    write: &mut [u8; MAX_BUFFER_BYTES],
    read: &mut [u8; MAX_BUFFER_BYTES],
) {
    set_word_stream_read_diagnostics(uart, driver, WordStreamReadDiagnostics::default());

    if !run_one_transaction_pio_diagnostic_bench_with_path(
        uart,
        driver,
        write,
        read,
        PayloadTransferPath::TransactionPioTxRxDmaDiagnostic,
        16,
        BenchPattern::RepeatedAd,
    ) {
        return;
    }

    for pattern in [BenchPattern::WalkingByte, BenchPattern::AddressDerived] {
        if !run_one_transaction_pio_diagnostic_bench_with_path(
            uart,
            driver,
            write,
            read,
            PayloadTransferPath::TransactionPioTxRxDmaDiagnostic,
            16,
            pattern,
        ) {
            return;
        }
    }

    for total_bytes in TRANSACTION_PIO_TX_RX_DMA_TOTAL_BYTES
        .iter()
        .copied()
        .skip(1)
    {
        for pattern in BENCH_PATTERNS {
            if !run_one_transaction_pio_diagnostic_bench_with_path(
                uart,
                driver,
                write,
                read,
                PayloadTransferPath::TransactionPioTxRxDmaDiagnostic,
                total_bytes,
                pattern,
            ) {
                return;
            }
        }
    }
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn set_payload_path(
    uart: &mut UartTx<'static, Blocking>,
    driver: &mut BlockingDriver<PicocalcPayloadBackend<'static>>,
    path: PayloadTransferPath,
) {
    driver
        .backend_mut_for_diagnostics()
        .set_payload_transfer_path_for_diagnostics(path);
    log_str(uart, "payload_path=");
    log_payload_path(uart, path);
    log_newline(uart);
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn set_word_stream_read_diagnostics(
    uart: &mut UartTx<'static, Blocking>,
    driver: &mut BlockingDriver<PicocalcPayloadBackend<'static>>,
    diagnostics: WordStreamReadDiagnostics,
) {
    driver
        .backend_mut_for_diagnostics()
        .set_word_stream_read_diagnostics(diagnostics);
    log_str(uart, "word_stream_read batch_words=");
    log_dec_usize(uart, diagnostics.batch_words);
    log_str(uart, " rx_fifo_join=");
    log_bool(uart, diagnostics.rx_fifo_join);
    log_newline(uart);
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn run_transaction_pio_diagnostic_bench(
    uart: &mut UartTx<'static, Blocking>,
    driver: &mut BlockingDriver<PicocalcPayloadBackend<'static>>,
    write: &mut [u8; MAX_BUFFER_BYTES],
    read: &mut [u8; MAX_BUFFER_BYTES],
) {
    set_word_stream_read_diagnostics(uart, driver, WordStreamReadDiagnostics::default());

    for total_bytes in TRANSACTION_PIO_INITIAL_TOTAL_BYTES {
        for pattern in BENCH_PATTERNS {
            if !run_one_transaction_pio_diagnostic_bench(
                uart,
                driver,
                write,
                read,
                total_bytes,
                pattern,
            ) {
                return;
            }
        }
    }

    for total_bytes in TRANSACTION_PIO_EXTENDED_TOTAL_BYTES {
        for pattern in BENCH_PATTERNS {
            if !run_one_transaction_pio_diagnostic_bench(
                uart,
                driver,
                write,
                read,
                total_bytes,
                pattern,
            ) {
                return;
            }
        }
    }

    for total_bytes in TRANSACTION_PIO_MULTI_TOTAL_BYTES {
        for pattern in BENCH_PATTERNS {
            if !run_one_transaction_pio_diagnostic_bench(
                uart,
                driver,
                write,
                read,
                total_bytes,
                pattern,
            ) {
                return;
            }
        }
    }

    for total_bytes in TRANSACTION_PIO_OPTIONAL_TOTAL_BYTES {
        for pattern in BENCH_PATTERNS {
            if !run_one_transaction_pio_diagnostic_bench(
                uart,
                driver,
                write,
                read,
                total_bytes,
                pattern,
            ) {
                return;
            }
        }
    }
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn run_one_transaction_pio_diagnostic_bench(
    uart: &mut UartTx<'static, Blocking>,
    driver: &mut BlockingDriver<PicocalcPayloadBackend<'static>>,
    write: &mut [u8; MAX_BUFFER_BYTES],
    read: &mut [u8; MAX_BUFFER_BYTES],
    total_bytes: usize,
    pattern: BenchPattern,
) -> bool {
    run_one_transaction_pio_diagnostic_bench_with_path(
        uart,
        driver,
        write,
        read,
        PayloadTransferPath::TransactionPioDiagnostic,
        total_bytes,
        pattern,
    )
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn run_one_transaction_pio_diagnostic_bench_with_path(
    uart: &mut UartTx<'static, Blocking>,
    driver: &mut BlockingDriver<PicocalcPayloadBackend<'static>>,
    write: &mut [u8; MAX_BUFFER_BYTES],
    read: &mut [u8; MAX_BUFFER_BYTES],
    path: PayloadTransferPath,
    total_bytes: usize,
    pattern: BenchPattern,
) -> bool {
    let transaction_count = transaction_count_for_chunk(total_bytes, CHUNK_LEN);
    set_payload_path(uart, driver, PayloadTransferPath::ByteFallback);
    let write_result = write_transfer(driver, write, total_bytes, pattern);
    if write_result.is_err() {
        log_transaction_pio_result_prefix(
            uart,
            "prefill",
            total_bytes,
            transaction_count,
            pattern,
            0,
            TransactionPioDiagnostics::default(),
        );
        log_line(uart, " status=error");
        return false;
    }

    set_payload_path(uart, driver, path);
    driver
        .backend_mut_for_diagnostics()
        .reset_qpi_timing_for_diagnostics();

    let detailed_tx_dma_logs = tx_dma_detailed_logs_enabled(total_bytes, pattern);
    let detailed_rx_dma_logs = rx_dma_detailed_logs_enabled(total_bytes, pattern);
    let detailed_tx_rx_dma_logs = tx_rx_dma_detailed_logs_enabled(total_bytes, pattern);
    let detailed_txpio_logs = txpio_detailed_logs_enabled(total_bytes, pattern);
    if path == PayloadTransferPath::TransactionPioTxDmaDiagnostic && detailed_tx_dma_logs {
        log_line(uart, "txdma step=start");
        log_line(uart, "txdma step=transaction_loop_start");
        let preflight = driver
            .backend_mut_for_diagnostics()
            .transaction_pio_tx_dma_buffer_preflight_for_diagnostics(
                bench_addr_at(0),
                total_bytes.min(CHUNK_LEN),
            );
        log_transaction_pio_tx_dma_buffer_preflight(uart, 0, preflight);
        if preflight.overflow {
            log_transaction_pio_result_prefix(
                uart,
                "read",
                total_bytes,
                transaction_count,
                pattern,
                0,
                preflight.diagnostics,
            );
            log_line(uart, " status=error error=tx_buffer_overflow");
            return false;
        }
    } else if path == PayloadTransferPath::TransactionPioRxDmaDiagnostic && detailed_rx_dma_logs {
        log_line(uart, "rxdma step=start");
        log_line(uart, "rxdma step=transaction_loop_start");
    } else if path == PayloadTransferPath::TransactionPioTxRxDmaDiagnostic
        && detailed_tx_rx_dma_logs
    {
        log_line(uart, "txrxdma step=start");
        log_line(uart, "txrxdma step=transaction_loop_start");
        log_line(uart, "txrxdma step=buffer_build_start");
    } else if path == PayloadTransferPath::TransactionPioDiagnostic && detailed_txpio_logs {
        log_line(uart, "txpio step=start");
        log_line(uart, "txpio step=transaction_loop_start");
    }
    let start = Instant::now();
    read[..total_bytes].fill(0);
    let read_result = if path == PayloadTransferPath::TransactionPioTxDmaDiagnostic {
        read_transaction_pio_tx_dma_transfer(
            uart,
            driver,
            &mut read[..total_bytes],
            total_bytes,
            CHUNK_LEN,
            detailed_tx_dma_logs,
        )
    } else if path == PayloadTransferPath::TransactionPioRxDmaDiagnostic {
        read_transaction_pio_rx_dma_transfer(
            uart,
            driver,
            &mut read[..total_bytes],
            total_bytes,
            CHUNK_LEN,
            detailed_rx_dma_logs,
        )
    } else if path == PayloadTransferPath::TransactionPioTxRxDmaDiagnostic {
        read_transaction_pio_tx_rx_dma_transfer(
            uart,
            driver,
            &mut read[..total_bytes],
            total_bytes,
            CHUNK_LEN,
            detailed_tx_rx_dma_logs,
        )
    } else {
        read_transaction_pio_transfer(driver, &mut read[..total_bytes], total_bytes, CHUNK_LEN)
    };
    let elapsed_us = Instant::now().duration_since(start).as_micros();
    let read_result_ok = read_result.is_ok();
    let txpio = read_result.unwrap_or_else(|diagnostics| diagnostics);
    if read_result_ok {
        if path == PayloadTransferPath::TransactionPioTxDmaDiagnostic {
            if detailed_tx_dma_logs {
                log_line(uart, "txdma step=transaction_loop_done");
            }
        } else if path == PayloadTransferPath::TransactionPioRxDmaDiagnostic {
            if detailed_rx_dma_logs {
                log_line(uart, "rxdma step=transaction_loop_done");
            }
        } else if path == PayloadTransferPath::TransactionPioTxRxDmaDiagnostic {
            if detailed_tx_rx_dma_logs {
                log_line(uart, "txrxdma step=transaction_loop_done");
            }
        } else if detailed_txpio_logs {
            log_line(uart, "txpio step=transaction_loop_done");
        }
    }
    if path != PayloadTransferPath::TransactionPioRxDmaDiagnostic
        && path != PayloadTransferPath::TransactionPioTxRxDmaDiagnostic
        && (path != PayloadTransferPath::TransactionPioTxDmaDiagnostic || detailed_tx_dma_logs)
        && (path != PayloadTransferPath::TransactionPioDiagnostic || detailed_txpio_logs)
    {
        log_transaction_pio_progress(uart, total_bytes, txpio.progress_flags);
    }
    if total_bytes == 16 * 1024 {
        log_transaction_pio_timing_summary_prefix(
            uart,
            "read",
            CHUNK_LEN,
            total_bytes,
            transaction_count,
            elapsed_us,
            txpio,
        );
    } else if path == PayloadTransferPath::TransactionPioTxDmaDiagnostic && !detailed_tx_dma_logs {
        log_transaction_pio_tx_dma_summary_prefix(
            uart,
            "read",
            total_bytes,
            transaction_count,
            pattern,
            elapsed_us,
            txpio,
        );
    } else if path == PayloadTransferPath::TransactionPioRxDmaDiagnostic {
        log_transaction_pio_rx_dma_summary_prefix(
            uart,
            "read",
            total_bytes,
            transaction_count,
            pattern,
            elapsed_us,
            txpio,
        );
    } else if path == PayloadTransferPath::TransactionPioTxRxDmaDiagnostic {
        log_transaction_pio_tx_rx_dma_summary_prefix(
            uart,
            "read",
            total_bytes,
            transaction_count,
            pattern,
            elapsed_us,
            txpio,
        );
    } else {
        log_transaction_pio_result_prefix(
            uart,
            "read",
            total_bytes,
            transaction_count,
            pattern,
            elapsed_us,
            txpio,
        );
    }
    if read_result_ok {
        log_line(uart, " status=ok");
    } else {
        if path == PayloadTransferPath::TransactionPioTxRxDmaDiagnostic {
            log_tx_rx_dma_error_suffix(uart, txpio);
        } else if path == PayloadTransferPath::TransactionPioRxDmaDiagnostic {
            log_rx_dma_error_suffix(uart, txpio);
        } else {
            log_line(uart, " status=error");
        }
        return false;
    }

    fill_pattern(&mut write[..total_bytes], 0, pattern);
    let mismatch = first_mismatch(&write[..total_bytes], &read[..total_bytes]);
    if total_bytes == 16 * 1024 {
        log_transaction_pio_timing_summary_prefix(
            uart,
            "compare",
            CHUNK_LEN,
            total_bytes,
            transaction_count,
            0,
            txpio,
        );
    } else if path == PayloadTransferPath::TransactionPioTxDmaDiagnostic && !detailed_tx_dma_logs {
        log_transaction_pio_tx_dma_summary_prefix(
            uart,
            "compare",
            total_bytes,
            transaction_count,
            pattern,
            0,
            txpio,
        );
    } else if path == PayloadTransferPath::TransactionPioRxDmaDiagnostic {
        log_transaction_pio_rx_dma_summary_prefix(
            uart,
            "compare",
            total_bytes,
            transaction_count,
            pattern,
            0,
            txpio,
        );
    } else if path == PayloadTransferPath::TransactionPioTxRxDmaDiagnostic {
        log_transaction_pio_tx_rx_dma_summary_prefix(
            uart,
            "compare",
            total_bytes,
            transaction_count,
            pattern,
            0,
            txpio,
        );
    } else {
        log_transaction_pio_result_prefix(
            uart,
            "compare",
            total_bytes,
            transaction_count,
            pattern,
            0,
            txpio,
        );
    }
    match mismatch {
        Some(mismatch) => {
            if path == PayloadTransferPath::TransactionPioTxDmaDiagnostic && detailed_tx_dma_logs {
                log_line(uart, "txdma compare status=error");
            }
            if path == PayloadTransferPath::TransactionPioRxDmaDiagnostic && detailed_rx_dma_logs {
                log_line(uart, "rxdma compare status=error");
            }
            if path == PayloadTransferPath::TransactionPioTxRxDmaDiagnostic
                && detailed_tx_rx_dma_logs
            {
                log_line(uart, "txrxdma compare status=error");
            }
            if path == PayloadTransferPath::TransactionPioTxRxDmaDiagnostic {
                log_transaction_pio_tx_rx_dma_mismatch(uart, mismatch, txpio);
            } else if path == PayloadTransferPath::TransactionPioRxDmaDiagnostic {
                log_transaction_pio_rx_dma_mismatch(uart, mismatch, txpio);
            } else {
                log_transaction_pio_mismatch(uart, mismatch);
            }
            false
        }
        None => {
            if path == PayloadTransferPath::TransactionPioTxDmaDiagnostic && detailed_tx_dma_logs {
                log_line(uart, "txdma compare status=ok");
            }
            if path == PayloadTransferPath::TransactionPioRxDmaDiagnostic && detailed_rx_dma_logs {
                log_line(uart, "rxdma compare status=ok");
            }
            if path == PayloadTransferPath::TransactionPioTxRxDmaDiagnostic
                && detailed_tx_rx_dma_logs
            {
                log_line(uart, "txrxdma compare status=ok");
            }
            log_line(uart, " status=ok");
            true
        }
    }
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn read_transaction_pio_transfer(
    driver: &mut BlockingDriver<PicocalcPayloadBackend<'static>>,
    read: &mut [u8],
    total_bytes: usize,
    chunk_len: usize,
) -> Result<TransactionPioDiagnostics, TransactionPioDiagnostics> {
    let mut offset = 0;
    let mut aggregate = TransactionPioDiagnostics::default();

    while offset < total_bytes {
        let len = (total_bytes - offset).min(chunk_len);
        let result = driver.read_exact(bench_addr_at(offset), &mut read[offset..offset + len]);
        let chunk_diagnostics = driver
            .backend_mut_for_diagnostics()
            .transaction_pio_diagnostics();
        merge_transaction_pio_diagnostics(&mut aggregate, chunk_diagnostics);
        if result.is_err() {
            return Err(aggregate);
        }
        offset += len;
    }

    Ok(aggregate)
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn read_transaction_pio_tx_dma_transfer(
    uart: &mut UartTx<'static, Blocking>,
    driver: &mut BlockingDriver<PicocalcPayloadBackend<'static>>,
    read: &mut [u8],
    total_bytes: usize,
    chunk_len: usize,
    detailed_logs: bool,
) -> Result<TransactionPioDiagnostics, TransactionPioDiagnostics> {
    let mut offset = 0;
    let mut aggregate = TransactionPioDiagnostics::default();
    aggregate.tx_dma = true;
    aggregate.rx_dma = false;

    while offset < total_bytes {
        let len = (total_bytes - offset).min(chunk_len);
        let result = if detailed_logs {
            driver
                .backend_mut_for_diagnostics()
                .read_qpi_chunk_transaction_pio_tx_dma_diagnostic_for_diagnostics(
                    bench_addr_at(offset),
                    &mut read[offset..offset + len],
                    |step| log_transaction_pio_tx_dma_step(uart, step),
                )
        } else {
            driver
                .backend_mut_for_diagnostics()
                .read_qpi_chunk_transaction_pio_tx_dma_diagnostic_for_diagnostics(
                    bench_addr_at(offset),
                    &mut read[offset..offset + len],
                    |_| {},
                )
        };
        let chunk_diagnostics = driver
            .backend_mut_for_diagnostics()
            .transaction_pio_diagnostics();
        merge_transaction_pio_diagnostics(&mut aggregate, chunk_diagnostics);
        if result.is_err() {
            return Err(aggregate);
        }
        offset += len;
    }

    Ok(aggregate)
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn read_transaction_pio_rx_dma_transfer(
    uart: &mut UartTx<'static, Blocking>,
    driver: &mut BlockingDriver<PicocalcPayloadBackend<'static>>,
    read: &mut [u8],
    total_bytes: usize,
    chunk_len: usize,
    detailed_logs: bool,
) -> Result<TransactionPioDiagnostics, TransactionPioDiagnostics> {
    let mut offset = 0;
    let mut aggregate = TransactionPioDiagnostics::default();
    aggregate.tx_dma = false;
    aggregate.rx_dma = true;

    while offset < total_bytes {
        let len = (total_bytes - offset).min(chunk_len);
        let result = if detailed_logs {
            driver
                .backend_mut_for_diagnostics()
                .read_qpi_chunk_transaction_pio_rx_dma_diagnostic_for_diagnostics(
                    bench_addr_at(offset),
                    &mut read[offset..offset + len],
                    |step| log_transaction_pio_tx_dma_step(uart, step),
                )
        } else {
            driver
                .backend_mut_for_diagnostics()
                .read_qpi_chunk_transaction_pio_rx_dma_diagnostic_for_diagnostics(
                    bench_addr_at(offset),
                    &mut read[offset..offset + len],
                    |_| {},
                )
        };
        let chunk_diagnostics = driver
            .backend_mut_for_diagnostics()
            .transaction_pio_diagnostics();
        merge_transaction_pio_diagnostics(&mut aggregate, chunk_diagnostics);
        if result.is_err() {
            return Err(aggregate);
        }
        offset += len;
    }

    Ok(aggregate)
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn read_transaction_pio_tx_rx_dma_transfer(
    uart: &mut UartTx<'static, Blocking>,
    driver: &mut BlockingDriver<PicocalcPayloadBackend<'static>>,
    read: &mut [u8],
    total_bytes: usize,
    chunk_len: usize,
    detailed_logs: bool,
) -> Result<TransactionPioDiagnostics, TransactionPioDiagnostics> {
    let mut offset = 0;
    let mut aggregate = TransactionPioDiagnostics::default();
    aggregate.tx_dma = true;
    aggregate.rx_dma = true;

    while offset < total_bytes {
        let len = (total_bytes - offset).min(chunk_len);
        let result = if detailed_logs {
            driver
                .backend_mut_for_diagnostics()
                .read_qpi_chunk_transaction_pio_tx_rx_dma_diagnostic_for_diagnostics(
                    bench_addr_at(offset),
                    &mut read[offset..offset + len],
                    |step| log_transaction_pio_tx_rx_dma_step(uart, step),
                )
        } else {
            driver
                .backend_mut_for_diagnostics()
                .read_qpi_chunk_transaction_pio_tx_rx_dma_diagnostic_for_diagnostics(
                    bench_addr_at(offset),
                    &mut read[offset..offset + len],
                    |_| {},
                )
        };
        let chunk_diagnostics = driver
            .backend_mut_for_diagnostics()
            .transaction_pio_diagnostics();
        merge_transaction_pio_diagnostics(&mut aggregate, chunk_diagnostics);
        if result.is_err() {
            return Err(aggregate);
        }
        offset += len;
    }

    Ok(aggregate)
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn merge_transaction_pio_diagnostics(
    aggregate: &mut TransactionPioDiagnostics,
    chunk: TransactionPioDiagnostics,
) {
    aggregate.tx_dma |= chunk.tx_dma;
    aggregate.rx_dma |= chunk.rx_dma;
    aggregate.tx_dma_setup_us = aggregate
        .tx_dma_setup_us
        .saturating_add(chunk.tx_dma_setup_us);
    aggregate.tx_dma_wait_us = aggregate
        .tx_dma_wait_us
        .saturating_add(chunk.tx_dma_wait_us);
    aggregate.rx_dma_wait_us = aggregate
        .rx_dma_wait_us
        .saturating_add(chunk.rx_dma_wait_us);
    aggregate.program_config_us = aggregate
        .program_config_us
        .saturating_add(chunk.program_config_us);
    aggregate.tx_buffer_build_us = aggregate
        .tx_buffer_build_us
        .saturating_add(chunk.tx_buffer_build_us);
    aggregate.tx_dma_arm_us = aggregate.tx_dma_arm_us.saturating_add(chunk.tx_dma_arm_us);
    aggregate.rx_dma_arm_us = aggregate.rx_dma_arm_us.saturating_add(chunk.rx_dma_arm_us);
    aggregate.sm_enable_to_tx_done_us = aggregate
        .sm_enable_to_tx_done_us
        .saturating_add(chunk.sm_enable_to_tx_done_us);
    aggregate.sm_enable_to_rx_done_us = aggregate
        .sm_enable_to_rx_done_us
        .saturating_add(chunk.sm_enable_to_rx_done_us);
    aggregate.rx_unpack_us = aggregate.rx_unpack_us.saturating_add(chunk.rx_unpack_us);
    aggregate.cleanup_us = aggregate.cleanup_us.saturating_add(chunk.cleanup_us);
    aggregate.total_chunk_us = aggregate
        .total_chunk_us
        .saturating_add(chunk.total_chunk_us);
    aggregate.tx_buf_capacity = chunk.tx_buf_capacity;
    aggregate.tx_len = aggregate.tx_len.saturating_add(chunk.tx_len);
    aggregate.tx_dma_transfer_size_bytes = chunk.tx_dma_transfer_size_bytes;
    aggregate.tx_dma_count = aggregate.tx_dma_count.saturating_add(chunk.tx_dma_count);
    aggregate.tx_dma_src_addr = chunk.tx_dma_src_addr;
    aggregate.tx_dma_dst_addr = chunk.tx_dma_dst_addr;
    aggregate.tx_dma_dreq_id = chunk.tx_dma_dreq_id;
    aggregate.tx_dma_channel_id = chunk.tx_dma_channel_id;
    aggregate.tx_dma_busy = chunk.tx_dma_busy;
    aggregate.tx_dma_read_error = chunk.tx_dma_read_error;
    aggregate.tx_dma_write_error = chunk.tx_dma_write_error;
    aggregate.tx_dma_ahb_error = chunk.tx_dma_ahb_error;
    aggregate.rx_dma_transfer_size_bytes = chunk.rx_dma_transfer_size_bytes;
    aggregate.rx_dma_count = aggregate.rx_dma_count.saturating_add(chunk.rx_dma_count);
    aggregate.rx_dma_src_addr = chunk.rx_dma_src_addr;
    aggregate.rx_dma_dst_addr = chunk.rx_dma_dst_addr;
    aggregate.rx_dma_dreq_id = chunk.rx_dma_dreq_id;
    aggregate.rx_dma_channel_id = chunk.rx_dma_channel_id;
    aggregate.rx_dma_busy = chunk.rx_dma_busy;
    aggregate.rx_dma_read_error = chunk.rx_dma_read_error;
    aggregate.rx_dma_write_error = chunk.rx_dma_write_error;
    aggregate.rx_dma_ahb_error = chunk.rx_dma_ahb_error;
    aggregate.output_bytes = chunk.output_bytes;
    aggregate.tx_buffer_overflow |= chunk.tx_buffer_overflow;
    aggregate.output_nibbles = chunk.output_nibbles;
    aggregate.input_nibbles = aggregate.input_nibbles.saturating_add(chunk.input_nibbles);
    aggregate.byte_len = aggregate.byte_len.saturating_add(chunk.byte_len);
    aggregate.word_count = aggregate.word_count.saturating_add(chunk.word_count);
    aggregate.progress_flags |= chunk.progress_flags;
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn txpio_detailed_logs_enabled(total_bytes: usize, pattern: BenchPattern) -> bool {
    total_bytes == 16 && pattern == BenchPattern::RepeatedAd
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn tx_dma_detailed_logs_enabled(total_bytes: usize, pattern: BenchPattern) -> bool {
    total_bytes == 16 && pattern == BenchPattern::RepeatedAd
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn rx_dma_detailed_logs_enabled(total_bytes: usize, pattern: BenchPattern) -> bool {
    total_bytes == 16 && pattern == BenchPattern::RepeatedAd
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn tx_rx_dma_detailed_logs_enabled(total_bytes: usize, pattern: BenchPattern) -> bool {
    total_bytes == 16 && pattern == BenchPattern::RepeatedAd
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn write_transfer(
    driver: &mut BlockingDriver<PicocalcPayloadBackend<'static>>,
    write: &mut [u8; MAX_BUFFER_BYTES],
    total_bytes: usize,
    pattern: BenchPattern,
) -> Result<(), PayloadBackendError> {
    let mut offset = 0;
    while offset < total_bytes {
        let len = (total_bytes - offset).min(write.len());
        fill_pattern(&mut write[..len], offset, pattern);
        driver.write_all(bench_addr_at(offset), &write[..len])?;
        offset += len;
    }

    Ok(())
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn bench_addr_at(offset: usize) -> PsramAddr {
    PsramAddr::new(BENCH_ADDR + offset as u32).unwrap()
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn transaction_count_for_chunk(total_bytes: usize, chunk_len: usize) -> usize {
    (total_bytes + chunk_len - 1) / chunk_len
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BenchPattern {
    RepeatedAd,
    WalkingByte,
    AddressDerived,
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ReadClkdivCandidate {
    x10: u32,
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
impl ReadClkdivCandidate {
    const fn new(x10: u32) -> Self {
        Self { x10 }
    }

    fn as_f32(self) -> f32 {
        self.x10 as f32 / 10.0
    }
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FastCurrentClkdivSweepCase {
    total_bytes: usize,
    pattern: BenchPattern,
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
impl FastCurrentClkdivSweepCase {
    const fn new(total_bytes: usize, pattern: BenchPattern) -> Self {
        Self {
            total_bytes,
            pattern,
        }
    }
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FastFallingAlignmentVariant {
    DiscardFirstByte,
    DiscardFirstNibble,
    ExtraDummyHalfCycle,
    ExtraDummyByte,
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
impl FastFallingAlignmentVariant {
    fn fast_read_loop_variant(self) -> TransactionPioFastReadLoopVariant {
        match self {
            Self::DiscardFirstByte => TransactionPioFastReadLoopVariant::FallingNoFudge,
            Self::DiscardFirstNibble => {
                TransactionPioFastReadLoopVariant::FallingDiscardFirstNibble
            }
            Self::ExtraDummyHalfCycle => {
                TransactionPioFastReadLoopVariant::FallingExtraDummyHalfCycle
            }
            Self::ExtraDummyByte => TransactionPioFastReadLoopVariant::FallingExtraDummyByte,
        }
    }

    fn discard_bytes(self) -> usize {
        match self {
            Self::DiscardFirstByte | Self::ExtraDummyByte => 1,
            _ => 0,
        }
    }

    fn discard_nibbles(self) -> usize {
        match self {
            Self::DiscardFirstNibble => 1,
            Self::DiscardFirstByte | Self::ExtraDummyByte => 2,
            Self::ExtraDummyHalfCycle => 0,
        }
    }

    fn scratch_extra_bytes(self) -> usize {
        match self {
            Self::DiscardFirstByte => 1,
            _ => 0,
        }
    }

    fn raw_log_start(self, total_bytes: usize) -> usize {
        match self {
            Self::DiscardFirstByte => total_bytes,
            _ => 0,
        }
    }

    fn raw_log_len(self, total_bytes: usize) -> usize {
        match self {
            Self::DiscardFirstByte => (total_bytes + 1).min(32),
            _ => total_bytes.min(32),
        }
    }
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn fill_pattern(buf: &mut [u8], base_offset: usize, pattern: BenchPattern) {
    for (offset, byte) in buf.iter_mut().enumerate() {
        let absolute_offset = base_offset + offset;
        *byte = match pattern {
            BenchPattern::RepeatedAd => 0xad,
            BenchPattern::WalkingByte => absolute_offset as u8,
            BenchPattern::AddressDerived => {
                let addr_byte = BENCH_ADDR.wrapping_add(absolute_offset as u32) as u8;
                addr_byte ^ (CHUNK_LEN as u8).wrapping_mul(3) ^ ((absolute_offset >> 3) as u8)
            }
        };
    }
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BenchMismatch {
    offset: usize,
    expected: u8,
    actual: u8,
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn first_mismatch(expected: &[u8], actual: &[u8]) -> Option<BenchMismatch> {
    for (offset, (&expected, &actual)) in expected.iter().zip(actual.iter()).enumerate() {
        if expected != actual {
            return Some(BenchMismatch {
                offset,
                expected,
                actual,
            });
        }
    }

    None
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn picocalc_uart_usb_tx(
    uart0: Peri<'static, embassy_rp::peripherals::UART0>,
    tx: Peri<'static, embassy_rp::peripherals::PIN_0>,
) -> UartTx<'static, Blocking> {
    let mut config = UartConfig::default();
    config.baudrate = PICOCALC_UART_USB_BAUD;

    // PicoCalc UART-USB bridge: RP2040 UART0 TX on GP0. RX/GP1 is unused.
    UartTx::new_blocking(uart0, tx, config)
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
static mut PANIC_UART: *mut UartTx<'static, Blocking> = core::ptr::null_mut();

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn register_panic_uart(uart: &mut UartTx<'static, Blocking>) {
    unsafe {
        PANIC_UART = uart as *mut _;
    }
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
#[panic_handler]
fn panic(info: &core::panic::PanicInfo<'_>) -> ! {
    unsafe {
        if let Some(uart) = PANIC_UART.as_mut() {
            log_str(uart, "panic");
            if let Some(location) = info.location() {
                log_str(uart, " at ");
                log_str(uart, location.file());
                log_byte(uart, b':');
                log_dec_u32(uart, location.line());
            }
            log_newline(uart);
        }
    }
    loop {}
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_transaction_pio_tx_dma_buffer_preflight(
    uart: &mut UartTx<'static, Blocking>,
    index: usize,
    preflight: koto_psram::rp2040_embassy::TransactionPioTxDmaBufferDiagnostics,
) {
    let diagnostics = preflight.diagnostics;
    log_str(uart, "txdma step=transaction_start index=");
    log_dec_usize(uart, index);
    log_str(uart, " addr=0x");
    log_hex_u8(uart, preflight.addr[0]);
    log_hex_u8(uart, preflight.addr[1]);
    log_hex_u8(uart, preflight.addr[2]);
    log_str(uart, " byte_len=");
    log_dec_usize(uart, diagnostics.byte_len);
    log_newline(uart);

    if diagnostics.progress_flags & TransactionPioDiagnostics::STEP_BUFFER_BUILD_START != 0 {
        log_line(uart, "txdma step=buffer_build_start");
    }
    if diagnostics.progress_flags & TransactionPioDiagnostics::STEP_COUNT_WRITE_START != 0 {
        log_line(uart, "txdma step=count_write_start");
    }
    if diagnostics.progress_flags & TransactionPioDiagnostics::STEP_COUNT_WRITE_DONE != 0 {
        log_line(uart, "txdma step=count_write_done");
    }
    if diagnostics.progress_flags & TransactionPioDiagnostics::STEP_CMD_WRITE_START != 0 {
        log_line(uart, "txdma step=cmd_write_start");
    }
    if diagnostics.progress_flags & TransactionPioDiagnostics::STEP_CMD_WRITE_DONE != 0 {
        log_line(uart, "txdma step=cmd_write_done");
    }
    if diagnostics.progress_flags & TransactionPioDiagnostics::STEP_ADDR_WRITE_DONE != 0 {
        log_line(uart, "txdma step=addr_write_done");
    }
    if diagnostics.progress_flags & TransactionPioDiagnostics::STEP_DUMMY_WRITE_DONE != 0 {
        log_line(uart, "txdma step=dummy_write_done");
    }
    if diagnostics.progress_flags & TransactionPioDiagnostics::STEP_BUFFER_READY != 0 {
        log_str(uart, "txdma step=buffer_ready tx_len=");
        log_dec_usize(uart, diagnostics.tx_len);
        log_str(uart, " output_nibbles=");
        log_dec_usize(uart, diagnostics.output_nibbles);
        log_str(uart, " input_nibbles=");
        log_dec_usize(uart, diagnostics.input_nibbles);
        log_newline(uart);
    }

    log_str(uart, "txdma buffer tx_buf_capacity=");
    log_dec_usize(uart, diagnostics.tx_buf_capacity);
    log_str(uart, " tx_len=");
    log_dec_usize(uart, diagnostics.tx_len);
    log_str(uart, " output_nibbles=");
    log_dec_usize(uart, diagnostics.output_nibbles);
    log_str(uart, " input_nibbles=");
    log_dec_usize(uart, diagnostics.input_nibbles);
    log_str(uart, " output_bytes=");
    log_dec_usize(uart, diagnostics.output_bytes);
    log_str(uart, " byte_len=");
    log_dec_usize(uart, diagnostics.byte_len);
    log_str(uart, " word_count=");
    log_dec_usize(uart, diagnostics.word_count);
    log_newline(uart);
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_transaction_pio_tx_dma_step(
    uart: &mut UartTx<'static, Blocking>,
    step: TransactionPioTxDmaStep,
) {
    match step {
        TransactionPioTxDmaStep::MemStart => {
            log_line(uart, "txdma_mem step=start");
        }
        TransactionPioTxDmaStep::MemConfig {
            tx_len,
            src_addr,
            dst_addr,
            transfer_size,
            count,
        } => {
            log_str(uart, "txdma_mem config tx_len=");
            log_dec_usize(uart, tx_len);
            log_str(uart, " src_addr=0x");
            log_hex_u32(uart, src_addr);
            log_str(uart, " dst_addr=0x");
            log_hex_u32(uart, dst_addr);
            log_str(uart, " transfer_size=");
            log_dec_usize(uart, transfer_size);
            log_str(uart, " count=");
            log_dec_usize(uart, count);
            log_newline(uart);
        }
        TransactionPioTxDmaStep::MemTransferCreateStart => {
            log_line(uart, "txdma_mem step=transfer_create_start");
        }
        TransactionPioTxDmaStep::MemTransferCreateDone => {
            log_line(uart, "txdma_mem step=transfer_create_done");
        }
        TransactionPioTxDmaStep::MemWaitStart => {
            log_line(uart, "txdma_mem step=wait_start");
        }
        TransactionPioTxDmaStep::MemWaitDone => {
            log_line(uart, "txdma_mem step=wait_done");
        }
        TransactionPioTxDmaStep::MemStatus { ok } => {
            if ok {
                log_line(uart, "txdma_mem status=ok");
            } else {
                log_line(uart, "txdma_mem status=error");
            }
        }
        TransactionPioTxDmaStep::DmaConfigStart => {
            log_line(uart, "txdma step=dma_config_start");
        }
        TransactionPioTxDmaStep::PacDmaStart => {
            log_line(uart, "pac_txdma step=start");
        }
        TransactionPioTxDmaStep::DmaConfig {
            tx_len,
            transfer_size,
            count,
            src_addr,
            dst_addr,
            dreq_id,
            channel_id,
        } => {
            log_str(uart, "txdma dma_config tx_len=");
            log_dec_usize(uart, tx_len);
            log_str(uart, " transfer_size=");
            log_dec_usize(uart, transfer_size);
            log_str(uart, " count=");
            log_dec_usize(uart, count);
            log_str(uart, " src_addr=0x");
            log_hex_u32(uart, src_addr);
            log_str(uart, " dst_addr=0x");
            log_hex_u32(uart, dst_addr);
            log_str(uart, " dreq_id=");
            log_dec_u32(uart, dreq_id);
            log_str(uart, " dma_channel=");
            match channel_id {
                Some(channel_id) => log_dec_u32(uart, channel_id as u32),
                None => log_str(uart, "unknown"),
            }
            log_newline(uart);
        }
        TransactionPioTxDmaStep::PacDmaConfig {
            read_addr,
            write_addr,
            trans_count,
            ctrl_base,
            ctrl_before_arm,
            ctrl_trig,
            dreq,
            chain_to,
            en,
            busy,
            read_error,
            write_error,
            ahb_error,
        } => {
            log_str(uart, "pac_txdma config read_addr=0x");
            log_hex_u32(uart, read_addr);
            log_str(uart, " write_addr=0x");
            log_hex_u32(uart, write_addr);
            log_str(uart, " trans_count=");
            log_dec_usize(uart, trans_count);
            log_str(uart, " ctrl_base=0x");
            log_hex_u32(uart, ctrl_base);
            log_str(uart, " ctrl_before_arm=0x");
            log_hex_u32(uart, ctrl_before_arm);
            log_str(uart, " ctrl_after_arm=0x");
            log_hex_u32(uart, ctrl_trig);
            log_str(uart, " dreq=");
            log_dec_u32(uart, dreq);
            log_str(uart, " chain_to=");
            log_dec_u32(uart, chain_to as u32);
            log_str(uart, " en=");
            log_bool(uart, en);
            log_str(uart, " busy=");
            log_bool(uart, busy);
            log_str(uart, " read_error=");
            log_bool(uart, read_error);
            log_str(uart, " write_error=");
            log_bool(uart, write_error);
            log_str(uart, " ahb_error=");
            log_bool(uart, ahb_error);
            log_newline(uart);
        }
        TransactionPioTxDmaStep::DmaTransferCreateStart => {
            log_line(uart, "txdma step=dma_transfer_create_start");
        }
        TransactionPioTxDmaStep::DmaTransferCreateDone => {
            log_line(uart, "txdma step=dma_transfer_create_done");
        }
        TransactionPioTxDmaStep::PacDmaReset { before, after } => {
            log_str(uart, "pac_txdma reset ctrl_trig_before_reset=0x");
            log_hex_u32(uart, before.ctrl_trig);
            log_str(uart, " busy_before_reset=");
            log_bool(uart, before.busy);
            log_str(uart, " read_error_before_reset=");
            log_bool(uart, before.read_error);
            log_str(uart, " write_error_before_reset=");
            log_bool(uart, before.write_error);
            log_str(uart, " ahb_error_before_reset=");
            log_bool(uart, before.ahb_error);
            log_str(uart, " ctrl_trig_after_reset=0x");
            log_hex_u32(uart, after.ctrl_trig);
            log_str(uart, " busy_after_reset=");
            log_bool(uart, after.busy);
            log_newline(uart);
        }
        TransactionPioTxDmaStep::PacDmaStarted => {
            log_line(uart, "pac_txdma step=started");
        }
        TransactionPioTxDmaStep::SmEnableStart => {
            log_line(uart, "txdma step=sm_enable_start");
        }
        TransactionPioTxDmaStep::SmEnableDone => {
            log_line(uart, "txdma step=sm_enable_done");
        }
        TransactionPioTxDmaStep::TxWaitStart => {
            log_line(uart, "txdma step=tx_wait_start");
        }
        TransactionPioTxDmaStep::TxWaitDone => {
            log_line(uart, "txdma step=tx_wait_done");
        }
        TransactionPioTxDmaStep::PacDmaWaitStart => {
            log_line(uart, "pac_txdma step=wait_start");
        }
        TransactionPioTxDmaStep::PacDmaWaitDone => {
            log_line(uart, "pac_txdma step=wait_done");
        }
        TransactionPioTxDmaStep::PacDmaStatus {
            busy,
            read_error,
            write_error,
            ahb_error,
        } => {
            log_str(uart, "pac_txdma status busy=");
            log_bool(uart, busy);
            log_str(uart, " read_error=");
            log_bool(uart, read_error);
            log_str(uart, " write_error=");
            log_bool(uart, write_error);
            log_str(uart, " ahb_error=");
            log_bool(uart, ahb_error);
            log_newline(uart);
        }
        TransactionPioTxDmaStep::PacDmaCleanup { final_status } => {
            log_str(uart, "pac_txdma cleanup ctrl_trig=0x");
            log_hex_u32(uart, final_status.ctrl_trig);
            log_str(uart, " busy=");
            log_bool(uart, final_status.busy);
            log_str(uart, " read_error=");
            log_bool(uart, final_status.read_error);
            log_str(uart, " write_error=");
            log_bool(uart, final_status.write_error);
            log_str(uart, " ahb_error=");
            log_bool(uart, final_status.ahb_error);
            log_newline(uart);
        }
        TransactionPioTxDmaStep::RxDmaConfigStart => {
            log_line(uart, "rxdma step=dma_config_start");
        }
        TransactionPioTxDmaStep::PacRxDmaStart => {
            log_line(uart, "pac_rxdma step=start");
        }
        TransactionPioTxDmaStep::PacRxDmaConfig {
            read_addr,
            write_addr,
            trans_count,
            ctrl_base,
            ctrl_before_arm,
            ctrl_trig,
            dreq,
            chain_to,
            en,
            busy,
            read_error,
            write_error,
            ahb_error,
        } => {
            log_str(uart, "pac_rxdma config read_addr=0x");
            log_hex_u32(uart, read_addr);
            log_str(uart, " write_addr=0x");
            log_hex_u32(uart, write_addr);
            log_str(uart, " trans_count=");
            log_dec_usize(uart, trans_count);
            log_str(uart, " ctrl_base=0x");
            log_hex_u32(uart, ctrl_base);
            log_str(uart, " ctrl_before_arm=0x");
            log_hex_u32(uart, ctrl_before_arm);
            log_str(uart, " ctrl_after_arm=0x");
            log_hex_u32(uart, ctrl_trig);
            log_str(uart, " dreq=");
            log_dec_u32(uart, dreq);
            log_str(uart, " chain_to=");
            log_dec_u32(uart, chain_to as u32);
            log_str(uart, " en=");
            log_bool(uart, en);
            log_str(uart, " busy=");
            log_bool(uart, busy);
            log_str(uart, " read_error=");
            log_bool(uart, read_error);
            log_str(uart, " write_error=");
            log_bool(uart, write_error);
            log_str(uart, " ahb_error=");
            log_bool(uart, ahb_error);
            log_newline(uart);
        }
        TransactionPioTxDmaStep::RxWaitStart => {
            log_line(uart, "rxdma step=rx_wait_start");
        }
        TransactionPioTxDmaStep::RxWaitDone => {
            log_line(uart, "rxdma step=rx_wait_done");
        }
        TransactionPioTxDmaStep::PacRxDmaWaitStart => {
            log_line(uart, "pac_rxdma step=rx_wait_start");
        }
        TransactionPioTxDmaStep::PacRxDmaWaitDone => {
            log_line(uart, "pac_rxdma step=rx_wait_done");
        }
        TransactionPioTxDmaStep::PacRxDmaStatus {
            busy,
            read_error,
            write_error,
            ahb_error,
        } => {
            log_str(uart, "pac_rxdma status busy=");
            log_bool(uart, busy);
            log_str(uart, " read_error=");
            log_bool(uart, read_error);
            log_str(uart, " write_error=");
            log_bool(uart, write_error);
            log_str(uart, " ahb_error=");
            log_bool(uart, ahb_error);
            log_newline(uart);
        }
        TransactionPioTxDmaStep::PacRxDmaCleanup { final_status } => {
            log_str(uart, "pac_rxdma cleanup ctrl_trig=0x");
            log_hex_u32(uart, final_status.ctrl_trig);
            log_str(uart, " busy=");
            log_bool(uart, final_status.busy);
            log_str(uart, " read_error=");
            log_bool(uart, final_status.read_error);
            log_str(uart, " write_error=");
            log_bool(uart, final_status.write_error);
            log_str(uart, " ahb_error=");
            log_bool(uart, final_status.ahb_error);
            log_newline(uart);
        }
        TransactionPioTxDmaStep::RxWords { words, count } => {
            log_str(uart, "pac_rxdma first_words count=");
            log_dec_usize(uart, count);
            for word in words.iter().take(count) {
                log_str(uart, " word=0x");
                log_hex_u32(uart, *word);
            }
            log_newline(uart);
        }
        TransactionPioTxDmaStep::RxPullStart => {
            log_line(uart, "txdma step=rx_pull_start");
        }
        TransactionPioTxDmaStep::RxPullDone => {
            log_line(uart, "txdma step=rx_pull_done");
        }
        TransactionPioTxDmaStep::CleanupDone => {
            log_line(uart, "txdma step=cleanup_done");
        }
    }
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_transaction_pio_tx_rx_dma_step(
    uart: &mut UartTx<'static, Blocking>,
    step: TransactionPioTxDmaStep,
) {
    match step {
        TransactionPioTxDmaStep::RxDmaConfigStart => {
            log_line(uart, "txrxdma step=rx_dma_config_start");
        }
        TransactionPioTxDmaStep::DmaConfigStart => {
            log_line(uart, "txrxdma step=tx_dma_config_start");
        }
        TransactionPioTxDmaStep::SmEnableStart => {
            log_line(uart, "txrxdma step=sm_enable_start");
        }
        TransactionPioTxDmaStep::SmEnableDone => {
            log_line(uart, "txrxdma step=sm_enable_done");
        }
        TransactionPioTxDmaStep::TxWaitStart => {
            log_line(uart, "txrxdma step=tx_wait_start");
        }
        TransactionPioTxDmaStep::TxWaitDone => {
            log_line(uart, "txrxdma step=tx_wait_done");
        }
        TransactionPioTxDmaStep::RxWaitStart => {
            log_line(uart, "txrxdma step=rx_wait_start");
        }
        TransactionPioTxDmaStep::RxWaitDone => {
            log_line(uart, "txrxdma step=rx_wait_done");
        }
        TransactionPioTxDmaStep::RxPullDone => {
            log_line(uart, "txrxdma step=unpack_done");
        }
        TransactionPioTxDmaStep::CleanupDone => {
            log_line(uart, "txrxdma step=cleanup_done");
        }
        other => log_transaction_pio_tx_dma_step(uart, other),
    }
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_transaction_pio_result_prefix(
    uart: &mut UartTx<'static, Blocking>,
    operation: &str,
    total_bytes: usize,
    transaction_count: usize,
    pattern: BenchPattern,
    elapsed_us: u64,
    diagnostics: TransactionPioDiagnostics,
) {
    log_str(uart, "bench profile=payload_path");
    log_str(uart, " payload_path=");
    if diagnostics.tx_dma && diagnostics.rx_dma {
        log_str(uart, "transaction_pio_tx_rx_dma_diagnostic");
    } else if diagnostics.tx_dma {
        log_str(uart, "transaction_pio_tx_dma_diagnostic");
    } else if diagnostics.rx_dma {
        log_str(uart, "transaction_pio_rx_dma_diagnostic");
    } else {
        log_str(uart, "transaction_pio_diagnostic");
    }
    log_str(uart, " tx_dma=");
    log_bool(uart, diagnostics.tx_dma);
    log_str(uart, " rx_dma=");
    log_bool(uart, diagnostics.rx_dma);
    log_str(uart, " tx_buf_capacity=");
    log_dec_usize(uart, diagnostics.tx_buf_capacity);
    log_str(uart, " tx_len=");
    log_dec_usize(uart, diagnostics.tx_len);
    log_str(uart, " chunk_len=");
    log_dec_usize(uart, CHUNK_LEN);
    log_str(uart, " total_bytes=");
    log_dec_usize(uart, total_bytes);
    log_str(uart, " transaction_count=");
    log_dec_usize(uart, transaction_count);
    log_str(uart, " pattern=");
    log_pattern(uart, pattern);
    log_str(uart, " operation=");
    log_str(uart, operation);
    log_str(uart, " elapsed_us=");
    log_dec_u64(uart, elapsed_us);
    log_str(uart, " bytes_per_sec=");
    log_dec_u64(uart, bytes_per_sec(total_bytes, elapsed_us));
    log_str(uart, " output_nibbles=");
    log_dec_usize(
        uart,
        diagnostics.output_nibbles.saturating_mul(transaction_count),
    );
    log_str(uart, " input_nibbles=");
    log_dec_usize(uart, total_bytes.saturating_mul(2));
    log_str(uart, " output_bytes=");
    log_dec_usize(uart, diagnostics.output_bytes);
    log_str(uart, " byte_len=");
    log_dec_usize(uart, total_bytes);
    log_str(uart, " word_count=");
    log_dec_usize(uart, total_bytes / 4);
    log_str(uart, " tx_dma_setup_us=");
    log_dec_u64(uart, diagnostics.tx_dma_setup_us);
    log_str(uart, " program_config_us=");
    log_dec_u64(uart, diagnostics.program_config_us);
    log_str(uart, " tx_buffer_build_us=");
    log_dec_u64(uart, diagnostics.tx_buffer_build_us);
    log_str(uart, " tx_dma_arm_us=");
    log_dec_u64(uart, diagnostics.tx_dma_arm_us);
    log_str(uart, " rx_dma_arm_us=");
    log_dec_u64(uart, diagnostics.rx_dma_arm_us);
    log_str(uart, " sm_enable_to_tx_done_us=");
    log_dec_u64(uart, diagnostics.sm_enable_to_tx_done_us);
    log_str(uart, " sm_enable_to_rx_done_us=");
    log_dec_u64(uart, diagnostics.sm_enable_to_rx_done_us);
    log_str(uart, " tx_dma_wait_us=");
    log_dec_u64(uart, diagnostics.tx_dma_wait_us);
    log_str(uart, " rx_dma_wait_us=");
    log_dec_u64(uart, diagnostics.rx_dma_wait_us);
    log_str(uart, " rx_unpack_us=");
    log_dec_u64(uart, diagnostics.rx_unpack_us);
    log_str(uart, " cleanup_us=");
    log_dec_u64(uart, diagnostics.cleanup_us);
    log_str(uart, " total_chunk_us=");
    log_dec_u64(uart, diagnostics.total_chunk_us);
    if diagnostics.tx_dma {
        log_str(uart, " tx_dma_transfer_size=");
        log_dec_usize(uart, diagnostics.tx_dma_transfer_size_bytes);
        log_str(uart, " tx_dma_count=");
        log_dec_usize(uart, diagnostics.tx_dma_count);
        log_str(uart, " tx_dma_src_addr=0x");
        log_hex_u32(uart, diagnostics.tx_dma_src_addr);
        log_str(uart, " tx_dma_dst_addr=0x");
        log_hex_u32(uart, diagnostics.tx_dma_dst_addr);
        log_str(uart, " tx_dma_dreq_id=");
        log_dec_u32(uart, diagnostics.tx_dma_dreq_id);
        log_str(uart, " tx_dma_channel=");
        match diagnostics.tx_dma_channel_id {
            Some(channel_id) => log_dec_u32(uart, channel_id as u32),
            None => log_str(uart, "unknown"),
        }
    }
    if diagnostics.rx_dma {
        log_str(uart, " rx_dma_transfer_size=");
        log_dec_usize(uart, diagnostics.rx_dma_transfer_size_bytes);
        log_str(uart, " rx_dma_count=");
        log_dec_usize(uart, diagnostics.rx_dma_count);
        log_str(uart, " rx_dma_src_addr=0x");
        log_hex_u32(uart, diagnostics.rx_dma_src_addr);
        log_str(uart, " rx_dma_dst_addr=0x");
        log_hex_u32(uart, diagnostics.rx_dma_dst_addr);
        log_str(uart, " rx_dma_dreq_id=");
        log_dec_u32(uart, diagnostics.rx_dma_dreq_id);
        log_str(uart, " rx_dma_channel=");
        match diagnostics.rx_dma_channel_id {
            Some(channel_id) => log_dec_u32(uart, channel_id as u32),
            None => log_str(uart, "unknown"),
        }
    }
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_transaction_pio_timing_summary_prefix(
    uart: &mut UartTx<'static, Blocking>,
    operation: &str,
    chunk_len: usize,
    total_bytes: usize,
    transaction_count: usize,
    elapsed_us: u64,
    diagnostics: TransactionPioDiagnostics,
) {
    log_str(uart, "bench profile=payload_path_timing");
    log_str(uart, " payload_path=");
    if diagnostics.tx_dma && diagnostics.rx_dma {
        log_str(uart, "transaction_pio_tx_rx_dma_diagnostic");
    } else if diagnostics.tx_dma {
        log_str(uart, "transaction_pio_tx_dma_diagnostic");
    } else if diagnostics.rx_dma {
        log_str(uart, "transaction_pio_rx_dma_diagnostic");
    } else {
        log_str(uart, "transaction_pio_diagnostic");
    }
    log_str(uart, " tx_dma=");
    log_bool(uart, diagnostics.tx_dma);
    log_str(uart, " rx_dma=");
    log_bool(uart, diagnostics.rx_dma);
    log_str(uart, " chunk_len=");
    log_dec_usize(uart, chunk_len);
    log_str(uart, " total_bytes=");
    log_dec_usize(uart, total_bytes);
    log_str(uart, " transaction_count=");
    log_dec_usize(uart, transaction_count);
    log_str(uart, " operation=");
    log_str(uart, operation);
    log_str(uart, " elapsed_us=");
    log_dec_u64(uart, elapsed_us);
    log_str(uart, " program_config_us=");
    log_dec_u64(uart, diagnostics.program_config_us);
    log_str(uart, " tx_buffer_build_us=");
    log_dec_u64(uart, diagnostics.tx_buffer_build_us);
    log_str(uart, " tx_dma_arm_us=");
    log_dec_u64(uart, diagnostics.tx_dma_arm_us);
    log_str(uart, " rx_dma_arm_us=");
    log_dec_u64(uart, diagnostics.rx_dma_arm_us);
    log_str(uart, " tx_dma_wait_us=");
    log_dec_u64(uart, diagnostics.tx_dma_wait_us);
    log_str(uart, " rx_dma_wait_us=");
    log_dec_u64(uart, diagnostics.rx_dma_wait_us);
    log_str(uart, " sm_enable_to_tx_done_us=");
    log_dec_u64(uart, diagnostics.sm_enable_to_tx_done_us);
    log_str(uart, " sm_enable_to_rx_done_us=");
    log_dec_u64(uart, diagnostics.sm_enable_to_rx_done_us);
    log_str(uart, " rx_unpack_us=");
    log_dec_u64(uart, diagnostics.rx_unpack_us);
    log_str(uart, " cleanup_us=");
    log_dec_u64(uart, diagnostics.cleanup_us);
    log_str(uart, " total_chunk_us=");
    log_dec_u64(uart, diagnostics.total_chunk_us);
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_transaction_pio_tx_dma_summary_prefix(
    uart: &mut UartTx<'static, Blocking>,
    operation: &str,
    total_bytes: usize,
    transaction_count: usize,
    pattern: BenchPattern,
    elapsed_us: u64,
    diagnostics: TransactionPioDiagnostics,
) {
    log_str(uart, "bench profile=payload_path");
    log_str(uart, " payload_path=transaction_pio_tx_dma_diagnostic");
    log_str(uart, " tx_dma=");
    log_bool(uart, diagnostics.tx_dma);
    log_str(uart, " rx_dma=");
    log_bool(uart, diagnostics.rx_dma);
    log_str(uart, " chunk_len=");
    log_dec_usize(uart, CHUNK_LEN);
    log_str(uart, " total_bytes=");
    log_dec_usize(uart, total_bytes);
    log_str(uart, " transaction_count=");
    log_dec_usize(uart, transaction_count);
    log_str(uart, " pattern=");
    log_pattern(uart, pattern);
    log_str(uart, " operation=");
    log_str(uart, operation);
    log_str(uart, " elapsed_us=");
    log_dec_u64(uart, elapsed_us);
    log_str(uart, " tx_len=");
    log_dec_usize(uart, diagnostics.tx_len);
    log_str(uart, " tx_dma_wait_us=");
    log_dec_u64(uart, diagnostics.tx_dma_wait_us);
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_transaction_pio_rx_dma_summary_prefix(
    uart: &mut UartTx<'static, Blocking>,
    operation: &str,
    total_bytes: usize,
    transaction_count: usize,
    pattern: BenchPattern,
    elapsed_us: u64,
    diagnostics: TransactionPioDiagnostics,
) {
    log_str(uart, "bench profile=payload_path");
    log_str(uart, " payload_path=transaction_pio_rx_dma_diagnostic");
    log_str(uart, " tx_dma=");
    log_bool(uart, diagnostics.tx_dma);
    log_str(uart, " rx_dma=");
    log_bool(uart, diagnostics.rx_dma);
    log_str(uart, " chunk_len=");
    log_dec_usize(uart, CHUNK_LEN);
    log_str(uart, " total_bytes=");
    log_dec_usize(uart, total_bytes);
    log_str(uart, " transaction_count=");
    log_dec_usize(uart, transaction_count);
    log_str(uart, " pattern=");
    log_pattern(uart, pattern);
    log_str(uart, " operation=");
    log_str(uart, operation);
    log_str(uart, " elapsed_us=");
    log_dec_u64(uart, elapsed_us);
    log_str(uart, " rx_dma_count=");
    log_dec_usize(uart, diagnostics.rx_dma_count);
    log_str(uart, " rx_dma_wait_us=");
    log_dec_u64(uart, diagnostics.rx_dma_wait_us);
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_transaction_pio_tx_rx_dma_summary_prefix(
    uart: &mut UartTx<'static, Blocking>,
    operation: &str,
    total_bytes: usize,
    transaction_count: usize,
    pattern: BenchPattern,
    elapsed_us: u64,
    diagnostics: TransactionPioDiagnostics,
) {
    log_str(uart, "bench profile=payload_path");
    log_str(uart, " payload_path=transaction_pio_tx_rx_dma_diagnostic");
    log_str(uart, " tx_dma=");
    log_bool(uart, diagnostics.tx_dma);
    log_str(uart, " rx_dma=");
    log_bool(uart, diagnostics.rx_dma);
    log_str(uart, " chunk_len=");
    log_dec_usize(uart, CHUNK_LEN);
    log_str(uart, " total_bytes=");
    log_dec_usize(uart, total_bytes);
    log_str(uart, " transaction_count=");
    log_dec_usize(uart, transaction_count);
    log_str(uart, " pattern=");
    log_pattern(uart, pattern);
    log_str(uart, " operation=");
    log_str(uart, operation);
    log_str(uart, " elapsed_us=");
    log_dec_u64(uart, elapsed_us);
    log_str(uart, " tx_len=");
    log_dec_usize(uart, diagnostics.tx_len);
    log_str(uart, " tx_dma_wait_us=");
    log_dec_u64(uart, diagnostics.tx_dma_wait_us);
    log_str(uart, " rx_dma_count=");
    log_dec_usize(uart, diagnostics.rx_dma_count);
    log_str(uart, " rx_dma_wait_us=");
    log_dec_u64(uart, diagnostics.rx_dma_wait_us);
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_rx_byte_fifo_probe_prefix(
    uart: &mut UartTx<'static, Blocking>,
    operation: &str,
    path: PayloadTransferPath,
    total_bytes: usize,
    pattern: BenchPattern,
    elapsed_us: u64,
    diagnostics: TransactionPioDiagnostics,
    fast_variant: Option<TransactionPioFastReadLoopVariant>,
    read_clkdiv: f32,
) {
    let fast_pio = path == PayloadTransferPath::TransactionPioFastRxByteFifoRxDmaDiagnostic;
    let byte_fifo = path == PayloadTransferPath::TransactionPioRxByteFifoDiagnostic
        || path == PayloadTransferPath::TransactionPioRxByteFifoRxDmaDiagnostic
        || fast_pio;
    let chunk_len = TRANSACTION_PIO_RX_DMA_STAGING_BYTES.min(total_bytes);

    log_str(uart, "bench profile=rx_byte_fifo_probe payload_path=");
    log_payload_path(uart, path);
    log_str(uart, " fast_pio=");
    log_bool(uart, fast_pio);
    if let Some(variant) = fast_variant {
        log_str(uart, " variant=");
        log_fast_read_loop_variant(uart, variant);
        log_str(uart, " cycles_per_nibble=");
        log_dec_usize(uart, fast_read_loop_cycles_per_nibble(variant));
        log_str(uart, " pre_read_fudge_cycles=");
        log_dec_usize(uart, fast_read_loop_pre_read_fudge_cycles(variant));
        log_str(uart, " sample_edge=");
        log_fast_read_loop_sample_edge(uart, variant);
        log_str(uart, " expected_ceiling_mb_s_x100=");
        log_dec_u64(
            uart,
            expected_fast_read_loop_ceiling_mb_s_x100(variant, read_clkdiv),
        );
    }
    log_str(uart, " byte_fifo=");
    log_bool(uart, byte_fifo);
    log_str(uart, " tx_dma=");
    log_bool(uart, diagnostics.tx_dma);
    log_str(uart, " rx_dma=");
    log_bool(uart, diagnostics.rx_dma);
    log_str(uart, " read_clkdiv=");
    log_read_clkdiv_from_f32(uart, read_clkdiv);
    log_str(uart, " chunk_len=");
    log_dec_usize(uart, chunk_len);
    log_str(uart, " total_bytes=");
    log_dec_usize(uart, total_bytes);
    log_str(uart, " transaction_count=");
    log_dec_usize(uart, transaction_count_for_chunk(total_bytes, chunk_len));
    log_str(uart, " pattern=");
    log_pattern(uart, pattern);
    log_str(uart, " operation=");
    log_str(uart, operation);
    log_str(uart, " elapsed_us=");
    log_dec_u64(uart, elapsed_us);
    log_str(uart, " bytes_per_sec=");
    log_dec_u64(uart, bytes_per_sec(total_bytes, elapsed_us));
    log_str(uart, " mb_s_x100=");
    log_dec_u64(uart, bytes_per_sec(total_bytes, elapsed_us) / 10_000);
    log_str(uart, " rx_dma_transfer_size=");
    log_dec_usize(uart, diagnostics.rx_dma_transfer_size_bytes);
    log_str(uart, " rx_dma_count=");
    log_dec_usize(uart, diagnostics.rx_dma_count);
    log_str(uart, " rx_dma_wait_us=");
    log_dec_u64(uart, diagnostics.rx_dma_wait_us);
    log_str(uart, " sm_enable_to_rx_done_us=");
    log_dec_u64(uart, diagnostics.sm_enable_to_rx_done_us);
    log_str(uart, " progress_flags=0x");
    log_hex_u32(uart, diagnostics.progress_flags);
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_fast_falling_alignment_prefix(
    uart: &mut UartTx<'static, Blocking>,
    operation: &str,
    alignment_variant: FastFallingAlignmentVariant,
    total_bytes: usize,
    pattern: BenchPattern,
    elapsed_us: u64,
    diagnostics: TransactionPioDiagnostics,
    read_clkdiv: f32,
) {
    let pio_variant = alignment_variant.fast_read_loop_variant();
    log_str(
        uart,
        "bench profile=fast_falling_alignment_probe payload_path=",
    );
    log_payload_path(
        uart,
        PayloadTransferPath::TransactionPioFastRxByteFifoRxDmaDiagnostic,
    );
    log_str(uart, " variant=");
    log_fast_falling_alignment_variant(uart, alignment_variant);
    log_str(uart, " pio_variant=");
    log_fast_read_loop_variant(uart, pio_variant);
    log_str(uart, " read_clkdiv=");
    log_read_clkdiv_from_f32(uart, read_clkdiv);
    log_str(uart, " sample_edge=");
    log_fast_read_loop_sample_edge(uart, pio_variant);
    log_str(uart, " pre_read_fudge_cycles=");
    log_dec_usize(uart, fast_read_loop_pre_read_fudge_cycles(pio_variant));
    log_str(uart, " discard_bytes=");
    log_dec_usize(uart, alignment_variant.discard_bytes());
    log_str(uart, " discard_nibbles=");
    log_dec_usize(uart, alignment_variant.discard_nibbles());
    log_str(uart, " rx_dma_count=");
    log_dec_usize(uart, diagnostics.rx_dma_count);
    log_str(uart, " rx_dma_transfer_size=");
    log_dec_usize(uart, diagnostics.rx_dma_transfer_size_bytes);
    log_str(uart, " chunk_len=");
    log_dec_usize(uart, TRANSACTION_PIO_RX_DMA_STAGING_BYTES.min(total_bytes));
    log_str(uart, " total_bytes=");
    log_dec_usize(uart, total_bytes);
    log_str(uart, " pattern=");
    log_pattern(uart, pattern);
    log_str(uart, " operation=");
    log_str(uart, operation);
    log_str(uart, " elapsed_us=");
    log_dec_u64(uart, elapsed_us);
    log_str(uart, " rx_dma_busy=");
    log_bool(uart, diagnostics.rx_dma_busy);
    log_str(uart, " rx_dma_read_error=");
    log_bool(uart, diagnostics.rx_dma_read_error);
    log_str(uart, " rx_dma_write_error=");
    log_bool(uart, diagnostics.rx_dma_write_error);
    log_str(uart, " rx_dma_ahb_error=");
    log_bool(uart, diagnostics.rx_dma_ahb_error);
    log_str(uart, " progress_flags=0x");
    log_hex_u32(uart, diagnostics.progress_flags);
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_rx_byte_fifo_dma_status_suffix(
    uart: &mut UartTx<'static, Blocking>,
    diagnostics: TransactionPioDiagnostics,
    is_dma: bool,
) {
    if is_dma {
        log_str(uart, " rx_dma_busy=");
        log_bool(uart, diagnostics.rx_dma_busy);
        log_str(uart, " rx_dma_read_error=");
        log_bool(uart, diagnostics.rx_dma_read_error);
        log_str(uart, " rx_dma_write_error=");
        log_bool(uart, diagnostics.rx_dma_write_error);
        log_str(uart, " rx_dma_ahb_error=");
        log_bool(uart, diagnostics.rx_dma_ahb_error);
        log_str(uart, " rx_dma_status_bits=0x");
        log_hex_u8(uart, rx_dma_status_bits(diagnostics));
    }
    log_newline(uart);
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_rx_dma_error_suffix(
    uart: &mut UartTx<'static, Blocking>,
    diagnostics: TransactionPioDiagnostics,
) {
    log_str(uart, " status=error rx_dma_busy=");
    log_bool(uart, diagnostics.rx_dma_busy);
    log_str(uart, " rx_dma_read_error=");
    log_bool(uart, diagnostics.rx_dma_read_error);
    log_str(uart, " rx_dma_write_error=");
    log_bool(uart, diagnostics.rx_dma_write_error);
    log_str(uart, " rx_dma_ahb_error=");
    log_bool(uart, diagnostics.rx_dma_ahb_error);
    log_str(uart, " progress_flags=0x");
    log_hex_u32(uart, diagnostics.progress_flags);
    log_newline(uart);
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_tx_rx_dma_error_suffix(
    uart: &mut UartTx<'static, Blocking>,
    diagnostics: TransactionPioDiagnostics,
) {
    log_str(uart, " status=error tx_dma_busy=");
    log_bool(uart, diagnostics.tx_dma_busy);
    log_str(uart, " tx_dma_read_error=");
    log_bool(uart, diagnostics.tx_dma_read_error);
    log_str(uart, " tx_dma_write_error=");
    log_bool(uart, diagnostics.tx_dma_write_error);
    log_str(uart, " tx_dma_ahb_error=");
    log_bool(uart, diagnostics.tx_dma_ahb_error);
    log_str(uart, " rx_dma_busy=");
    log_bool(uart, diagnostics.rx_dma_busy);
    log_str(uart, " rx_dma_read_error=");
    log_bool(uart, diagnostics.rx_dma_read_error);
    log_str(uart, " rx_dma_write_error=");
    log_bool(uart, diagnostics.rx_dma_write_error);
    log_str(uart, " rx_dma_ahb_error=");
    log_bool(uart, diagnostics.rx_dma_ahb_error);
    log_str(uart, " progress_flags=0x");
    log_hex_u32(uart, diagnostics.progress_flags);
    log_str(uart, " timeout_during_tx_wait=");
    log_bool(
        uart,
        diagnostics.progress_flags & TransactionPioDiagnostics::STEP_TX_WAIT_START != 0
            && diagnostics.progress_flags & TransactionPioDiagnostics::STEP_TX_WAIT_DONE == 0,
    );
    log_str(uart, " timeout_during_rx_wait=");
    log_bool(
        uart,
        diagnostics.progress_flags & TransactionPioDiagnostics::STEP_RX_PULL_START != 0
            && diagnostics.progress_flags & TransactionPioDiagnostics::STEP_RX_PULL_DONE == 0,
    );
    log_newline(uart);
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_transaction_pio_mismatch(uart: &mut UartTx<'static, Blocking>, mismatch: BenchMismatch) {
    log_str(uart, " status=error fail_off=");
    log_dec_usize(uart, mismatch.offset);
    log_str(uart, " expected=0x");
    log_hex_u8(uart, mismatch.expected);
    log_str(uart, " actual=0x");
    log_hex_u8(uart, mismatch.actual);
    log_str(uart, " mismatch_addr=0x");
    log_hex_u32(uart, BENCH_ADDR + mismatch.offset as u32);
    log_newline(uart);
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_transaction_pio_tx_rx_dma_mismatch(
    uart: &mut UartTx<'static, Blocking>,
    mismatch: BenchMismatch,
    diagnostics: TransactionPioDiagnostics,
) {
    log_str(uart, " status=error fail_off=");
    log_dec_usize(uart, mismatch.offset);
    log_str(uart, " expected=0x");
    log_hex_u8(uart, mismatch.expected);
    log_str(uart, " actual=0x");
    log_hex_u8(uart, mismatch.actual);
    log_str(uart, " mismatch_addr=0x");
    log_hex_u32(uart, BENCH_ADDR + mismatch.offset as u32);
    log_str(uart, " tx_dma_busy=");
    log_bool(uart, diagnostics.tx_dma_busy);
    log_str(uart, " tx_dma_read_error=");
    log_bool(uart, diagnostics.tx_dma_read_error);
    log_str(uart, " tx_dma_write_error=");
    log_bool(uart, diagnostics.tx_dma_write_error);
    log_str(uart, " tx_dma_ahb_error=");
    log_bool(uart, diagnostics.tx_dma_ahb_error);
    log_str(uart, " rx_dma_busy=");
    log_bool(uart, diagnostics.rx_dma_busy);
    log_str(uart, " rx_dma_read_error=");
    log_bool(uart, diagnostics.rx_dma_read_error);
    log_str(uart, " rx_dma_write_error=");
    log_bool(uart, diagnostics.rx_dma_write_error);
    log_str(uart, " rx_dma_ahb_error=");
    log_bool(uart, diagnostics.rx_dma_ahb_error);
    log_str(uart, " progress_flags=0x");
    log_hex_u32(uart, diagnostics.progress_flags);
    log_newline(uart);
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_transaction_pio_rx_dma_mismatch(
    uart: &mut UartTx<'static, Blocking>,
    mismatch: BenchMismatch,
    diagnostics: TransactionPioDiagnostics,
) {
    log_str(uart, " status=error fail_off=");
    log_dec_usize(uart, mismatch.offset);
    log_str(uart, " expected=0x");
    log_hex_u8(uart, mismatch.expected);
    log_str(uart, " actual=0x");
    log_hex_u8(uart, mismatch.actual);
    log_str(uart, " mismatch_addr=0x");
    log_hex_u32(uart, BENCH_ADDR + mismatch.offset as u32);
    log_str(uart, " rx_dma_busy=");
    log_bool(uart, diagnostics.rx_dma_busy);
    log_str(uart, " rx_dma_read_error=");
    log_bool(uart, diagnostics.rx_dma_read_error);
    log_str(uart, " rx_dma_write_error=");
    log_bool(uart, diagnostics.rx_dma_write_error);
    log_str(uart, " rx_dma_ahb_error=");
    log_bool(uart, diagnostics.rx_dma_ahb_error);
    log_str(uart, " progress_flags=0x");
    log_hex_u32(uart, diagnostics.progress_flags);
    log_newline(uart);
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_transaction_pio_progress(
    uart: &mut UartTx<'static, Blocking>,
    total_bytes: usize,
    flags: u32,
) {
    if total_bytes > CHUNK_LEN {
        if flags & TransactionPioDiagnostics::STEP_CLEANUP_DONE != 0 {
            if flags & TransactionPioDiagnostics::STEP_DMA_CONFIG_DONE != 0 {
                log_line(uart, "txdma step=cleanup_done");
            } else {
                log_line(uart, "txpio step=cleanup_done");
            }
        }
        return;
    }

    if flags & TransactionPioDiagnostics::STEP_PROGRAM_CONFIG_DONE != 0 {
        log_line(uart, "txpio step=program_config_done");
    }
    if flags & TransactionPioDiagnostics::STEP_BUFFER_READY != 0 {
        log_line(uart, "txdma step=buffer_ready");
    }
    if flags & TransactionPioDiagnostics::STEP_DMA_CONFIG_START != 0 {
        log_line(uart, "txdma step=dma_config_start");
    }
    if flags & TransactionPioDiagnostics::STEP_DMA_TRANSFER_CREATE_START != 0 {
        log_line(uart, "txdma step=dma_transfer_create_start");
    }
    if flags & TransactionPioDiagnostics::STEP_DMA_CONFIG_DONE != 0 {
        log_line(uart, "txdma step=dma_transfer_create_done");
    }
    if flags & TransactionPioDiagnostics::STEP_COUNT_PUSH_DONE != 0 {
        log_line(uart, "txpio step=count_push_done");
    }
    if flags & TransactionPioDiagnostics::STEP_SM_ENABLE_START != 0 {
        log_line(uart, "txdma step=sm_enable_start");
    }
    if flags & TransactionPioDiagnostics::STEP_SM_ENABLE != 0 {
        if flags & TransactionPioDiagnostics::STEP_DMA_CONFIG_DONE != 0 {
            log_line(uart, "txdma step=sm_enable_done");
        } else {
            log_line(uart, "txpio step=sm_enable");
        }
    }
    if flags & TransactionPioDiagnostics::STEP_TX_WAIT_START != 0 {
        log_line(uart, "txdma step=tx_wait_start");
    }
    if flags & TransactionPioDiagnostics::STEP_TX_WAIT_DONE != 0 {
        log_line(uart, "txdma step=tx_wait_done");
    }
    if flags & TransactionPioDiagnostics::STEP_RX_PULL_START != 0 {
        if flags & TransactionPioDiagnostics::STEP_DMA_CONFIG_DONE != 0 {
            log_line(uart, "txdma step=rx_pull_start");
        } else {
            log_line(uart, "txpio step=rx_pull_start");
        }
    }
    if flags & TransactionPioDiagnostics::STEP_RX_PULL_DONE != 0 {
        if flags & TransactionPioDiagnostics::STEP_DMA_CONFIG_DONE != 0 {
            log_line(uart, "txdma step=rx_pull_done");
        } else {
            log_line(uart, "txpio step=rx_pull_done");
        }
    }
    if flags & TransactionPioDiagnostics::STEP_CLEANUP_DONE != 0 {
        if flags & TransactionPioDiagnostics::STEP_DMA_CONFIG_DONE != 0 {
            log_line(uart, "txdma step=cleanup_done");
        } else {
            log_line(uart, "txpio step=cleanup_done");
        }
    }
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_pattern(uart: &mut UartTx<'static, Blocking>, pattern: BenchPattern) {
    match pattern {
        BenchPattern::RepeatedAd => log_str(uart, "repeated_0xad"),
        BenchPattern::WalkingByte => log_str(uart, "walking-byte"),
        BenchPattern::AddressDerived => log_str(uart, "address-derived"),
    }
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_payload_path(uart: &mut UartTx<'static, Blocking>, path: PayloadTransferPath) {
    match path {
        PayloadTransferPath::ByteFallback => log_str(uart, "byte_fallback"),
        PayloadTransferPath::PollingBurstDiagnostic => log_str(uart, "polling_burst_diagnostic"),
        PayloadTransferPath::WordStreamPolling => log_str(uart, "word_stream_polling"),
        PayloadTransferPath::TransactionPioDiagnostic => {
            log_str(uart, "transaction_pio_diagnostic")
        }
        PayloadTransferPath::TransactionPioTxDmaDiagnostic => {
            log_str(uart, "transaction_pio_tx_dma_diagnostic")
        }
        PayloadTransferPath::TransactionPioRxDmaDiagnostic => {
            log_str(uart, "transaction_pio_rx_dma_diagnostic")
        }
        PayloadTransferPath::TransactionPioTxRxDmaDiagnostic => {
            log_str(uart, "transaction_pio_tx_rx_dma_diagnostic")
        }
        PayloadTransferPath::TransactionPioRxByteFifoDiagnostic => {
            log_str(uart, "transaction_pio_rx_byte_fifo_diagnostic")
        }
        PayloadTransferPath::TransactionPioRxByteFifoRxDmaDiagnostic => {
            log_str(uart, "transaction_pio_rx_byte_fifo_rx_dma_diagnostic")
        }
        PayloadTransferPath::TransactionPioFastRxByteFifoRxDmaDiagnostic => {
            log_str(uart, "transaction_pio_fast_rx_byte_fifo_rx_dma_diagnostic")
        }
        PayloadTransferPath::WordStreamDma => log_str(uart, "word_stream_dma"),
    }
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_fast_read_loop_variant(
    uart: &mut UartTx<'static, Blocking>,
    variant: TransactionPioFastReadLoopVariant,
) {
    match variant {
        TransactionPioFastReadLoopVariant::CurrentNoDelay => log_str(uart, "current_no_delay"),
        TransactionPioFastReadLoopVariant::OppositePolarityNoDelay => {
            log_str(uart, "opposite_polarity_no_delay")
        }
        TransactionPioFastReadLoopVariant::DelayOnIn => log_str(uart, "delay_on_in"),
        TransactionPioFastReadLoopVariant::DelayOnJmp => log_str(uart, "delay_on_jmp"),
        TransactionPioFastReadLoopVariant::FallingFudgeA => log_str(uart, "falling_fudge_a"),
        TransactionPioFastReadLoopVariant::FallingFudgeB => log_str(uart, "falling_fudge_b"),
        TransactionPioFastReadLoopVariant::FallingNoFudge => log_str(uart, "falling_no_fudge"),
        TransactionPioFastReadLoopVariant::FallingFudgeExtraLow => {
            log_str(uart, "falling_fudge_extra_low")
        }
        TransactionPioFastReadLoopVariant::FallingDiscardFirstNibble => {
            log_str(uart, "falling_discard_first_nibble")
        }
        TransactionPioFastReadLoopVariant::FallingExtraDummyHalfCycle => {
            log_str(uart, "falling_extra_dummy_half_cycle")
        }
        TransactionPioFastReadLoopVariant::FallingExtraDummyByte => {
            log_str(uart, "falling_extra_dummy_byte")
        }
    }
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_fast_falling_alignment_variant(
    uart: &mut UartTx<'static, Blocking>,
    variant: FastFallingAlignmentVariant,
) {
    match variant {
        FastFallingAlignmentVariant::DiscardFirstByte => log_str(uart, "discard_first_byte"),
        FastFallingAlignmentVariant::DiscardFirstNibble => log_str(uart, "discard_first_nibble"),
        FastFallingAlignmentVariant::ExtraDummyHalfCycle => log_str(uart, "extra_dummy_half_cycle"),
        FastFallingAlignmentVariant::ExtraDummyByte => log_str(uart, "extra_dummy_byte"),
    }
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn fast_read_loop_cycles_per_nibble(variant: TransactionPioFastReadLoopVariant) -> usize {
    match variant {
        TransactionPioFastReadLoopVariant::CurrentNoDelay
        | TransactionPioFastReadLoopVariant::OppositePolarityNoDelay
        | TransactionPioFastReadLoopVariant::FallingFudgeA
        | TransactionPioFastReadLoopVariant::FallingFudgeB
        | TransactionPioFastReadLoopVariant::FallingNoFudge
        | TransactionPioFastReadLoopVariant::FallingFudgeExtraLow
        | TransactionPioFastReadLoopVariant::FallingDiscardFirstNibble
        | TransactionPioFastReadLoopVariant::FallingExtraDummyHalfCycle
        | TransactionPioFastReadLoopVariant::FallingExtraDummyByte => 2,
        TransactionPioFastReadLoopVariant::DelayOnIn
        | TransactionPioFastReadLoopVariant::DelayOnJmp => 3,
    }
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn fast_read_loop_pre_read_fudge_cycles(variant: TransactionPioFastReadLoopVariant) -> usize {
    match variant {
        TransactionPioFastReadLoopVariant::FallingFudgeA => 1,
        TransactionPioFastReadLoopVariant::FallingFudgeB
        | TransactionPioFastReadLoopVariant::FallingFudgeExtraLow => 2,
        _ => 0,
    }
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_fast_read_loop_sample_edge(
    uart: &mut UartTx<'static, Blocking>,
    variant: TransactionPioFastReadLoopVariant,
) {
    match variant {
        TransactionPioFastReadLoopVariant::FallingFudgeA
        | TransactionPioFastReadLoopVariant::FallingFudgeB
        | TransactionPioFastReadLoopVariant::FallingNoFudge
        | TransactionPioFastReadLoopVariant::FallingFudgeExtraLow
        | TransactionPioFastReadLoopVariant::FallingDiscardFirstNibble
        | TransactionPioFastReadLoopVariant::FallingExtraDummyHalfCycle
        | TransactionPioFastReadLoopVariant::FallingExtraDummyByte => log_str(uart, "falling"),
        _ => log_str(uart, "rising/current"),
    }
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn expected_fast_read_loop_ceiling_mb_s_x100(
    variant: TransactionPioFastReadLoopVariant,
    read_clkdiv: f32,
) -> u64 {
    match fast_read_loop_cycles_per_nibble(variant) {
        2 => expected_fast_read_loop_ceiling_for_cycles_mb_s_x100(2, read_clkdiv),
        3 => expected_fast_read_loop_ceiling_for_cycles_mb_s_x100(3, read_clkdiv),
        _ => 0,
    }
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn expected_fast_current_ceiling_mb_s_x100(candidate: ReadClkdivCandidate) -> u64 {
    // 125 MHz PIO clock / clkdiv / (2 nibbles per byte * cycles per nibble).
    125_000_000u64.saturating_mul(100).saturating_mul(10)
        / (candidate.x10 as u64)
            .saturating_mul(2)
            .saturating_mul(2)
            .saturating_mul(1_000_000)
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn expected_fast_read_loop_ceiling_for_cycles_mb_s_x100(
    cycles_per_nibble: usize,
    read_clkdiv: f32,
) -> u64 {
    let clkdiv_x10 = read_clkdiv_to_x10(read_clkdiv).max(1) as u64;
    125_000_000u64.saturating_mul(100).saturating_mul(10)
        / clkdiv_x10
            .saturating_mul(2)
            .saturating_mul(cycles_per_nibble as u64)
            .saturating_mul(1_000_000)
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn rx_dma_status_bits(diagnostics: TransactionPioDiagnostics) -> u8 {
    (diagnostics.rx_dma_busy as u8)
        | ((diagnostics.rx_dma_read_error as u8) << 1)
        | ((diagnostics.rx_dma_write_error as u8) << 2)
        | ((diagnostics.rx_dma_ahb_error as u8) << 3)
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_read_clkdiv(uart: &mut UartTx<'static, Blocking>, candidate: ReadClkdivCandidate) {
    log_dec_u32(uart, candidate.x10 / 10);
    log_byte(uart, b'.');
    log_dec_u32(uart, candidate.x10 % 10);
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_read_clkdiv_from_f32(uart: &mut UartTx<'static, Blocking>, read_clkdiv: f32) {
    log_read_clkdiv(
        uart,
        ReadClkdivCandidate::new(read_clkdiv_to_x10(read_clkdiv)),
    );
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn read_clkdiv_to_x10(read_clkdiv: f32) -> u32 {
    (read_clkdiv * 10.0 + 0.5) as u32
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_bool(uart: &mut UartTx<'static, Blocking>, value: bool) {
    if value {
        log_str(uart, "true");
    } else {
        log_str(uart, "false");
    }
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn bytes_per_sec(total_bytes: usize, elapsed_us: u64) -> u64 {
    if elapsed_us == 0 {
        return 0;
    }

    (total_bytes as u64).saturating_mul(1_000_000) / elapsed_us
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_line(uart: &mut UartTx<'static, Blocking>, line: &str) {
    log_str(uart, line);
    log_newline(uart);
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_str(uart: &mut UartTx<'static, Blocking>, text: &str) {
    let _ = uart.blocking_write(text.as_bytes());
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_newline(uart: &mut UartTx<'static, Blocking>) {
    let _ = uart.blocking_write(b"\r\n");
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_byte(uart: &mut UartTx<'static, Blocking>, byte: u8) {
    let _ = uart.blocking_write(&[byte]);
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_hex_u8(uart: &mut UartTx<'static, Blocking>, value: u8) {
    log_nibble(uart, value >> 4);
    log_nibble(uart, value & 0x0f);
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_hex_u32(uart: &mut UartTx<'static, Blocking>, value: u32) {
    for shift in [28, 24, 20, 16, 12, 8, 4, 0] {
        log_nibble(uart, ((value >> shift) & 0x0f) as u8);
    }
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_bytes_hex(uart: &mut UartTx<'static, Blocking>, bytes: &[u8]) {
    for (index, byte) in bytes.iter().enumerate() {
        if index > 0 {
            log_byte(uart, b' ');
        }
        log_str(uart, "0x");
        log_hex_u8(uart, *byte);
    }
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_nibble(uart: &mut UartTx<'static, Blocking>, nibble: u8) {
    let byte = if nibble < 10 {
        b'0' + nibble
    } else {
        b'a' + (nibble - 10)
    };
    log_byte(uart, byte);
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_dec_usize(uart: &mut UartTx<'static, Blocking>, value: usize) {
    log_dec_u64(uart, value as u64);
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_dec_u32(uart: &mut UartTx<'static, Blocking>, value: u32) {
    log_dec_u64(uart, value as u64);
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_dec_u64(uart: &mut UartTx<'static, Blocking>, mut value: u64) {
    let mut buf = [0u8; 20];
    let mut len = 0;

    if value == 0 {
        log_byte(uart, b'0');
        return;
    }

    while value > 0 {
        buf[len] = b'0' + (value % 10) as u8;
        value /= 10;
        len += 1;
    }

    while len > 0 {
        len -= 1;
        log_byte(uart, buf[len]);
    }
}
