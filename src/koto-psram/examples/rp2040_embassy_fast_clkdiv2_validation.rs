#![cfg_attr(
    all(
        feature = "rp2040-embassy",
        feature = "psram_fast_read_clkdiv2",
        target_os = "none"
    ),
    no_std
)]
#![cfg_attr(
    all(
        feature = "rp2040-embassy",
        feature = "psram_fast_read_clkdiv2",
        target_os = "none"
    ),
    no_main
)]

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
use embassy_rp::{
    uart::{Blocking, Config as UartConfig, UartTx},
    Peri,
};

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
use embassy_time::Instant;

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
use koto_psram::{
    addr::PsramAddr,
    bus::PsramBus,
    config::{Pins, TimingConfig},
    pio::blocking::{BlockingDriver, BlockingPio},
    rp2040_embassy::{
        EmbassyRpQpiBackend, PayloadTransferPath, TransactionPioDiagnostics,
        TransactionPioFastReadLoopVariant, WordStreamReadDiagnostics,
    },
};

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
const PICOCALC_UART_USB_BAUD: u32 = 115_200;

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
const BENCH_ADDR: u32 = 0x0000_2000;

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
const CHUNK_LEN: usize = 512;

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
const MAX_BUFFER_BYTES: usize = 64 * 1024;

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
const TRANSACTION_PIO_RX_DMA_STAGING_BYTES: usize = 4096;

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
const RX_BYTE_FIFO_PROBE_TIMEOUT_POLLS: u32 = 10_000;

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
const FAST_FALLING_CLKDIV2_SIZE_SWEEP_BYTES: [usize; 11] =
    [16, 32, 64, 128, 256, 512, 1024, 2048, 4096, 8192, 16 * 1024];

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
const FAST_FALLING_CLKDIV2_BOUNDARY_ADDRS: [u32; 12] = [
    0x0000_0000,
    0x0000_0010,
    0x0000_0020,
    0x0000_0040,
    0x0000_0080,
    0x0000_0100,
    0x0000_0ff0,
    0x0000_1000,
    0x0000_1ff0,
    0x0000_2000,
    0x0000_fff0,
    0x0001_0000,
];

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
const FAST_FALLING_CLKDIV2_WRITE_READ_BYTES: [usize; 4] = [64, 512, 4096, 16 * 1024];

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
const FAST_FALLING_CLKDIV2_CHUNK_LEN: usize = 4096;

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
const FAST_FALLING_CLKDIV2_PREFILL_PATH: PayloadTransferPath = PayloadTransferPath::ByteFallback;

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
const FAST_FALLING_CLKDIV2_READ_PATH: PayloadTransferPath =
    PayloadTransferPath::TransactionPioFastRxByteFifoRxDmaDiagnostic;

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
static mut WRITE_BUF: [u8; MAX_BUFFER_BYTES] = [0; MAX_BUFFER_BYTES];

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
static mut READ_BUF: [u8; MAX_BUFFER_BYTES] = [0; MAX_BUFFER_BYTES];

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
embassy_rp::bind_interrupts!(struct Irqs {
    PIO0_IRQ_0 => embassy_rp::pio::InterruptHandler<embassy_rp::peripherals::PIO0>;
    DMA_IRQ_0 => embassy_rp::dma::InterruptHandler<embassy_rp::peripherals::DMA_CH0>, embassy_rp::dma::InterruptHandler<embassy_rp::peripherals::DMA_CH1>;
});

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
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

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
type PayloadBackendError = <PicocalcPayloadBackend<'static> as BlockingPio>::Error;

#[cfg(not(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
)))]
fn main() {}

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
#[cortex_m_rt::entry]
fn embedded_main() -> ! {
    let peripherals = embassy_rp::init(Default::default());
    let mut uart = picocalc_uart_usb_tx(peripherals.UART0, peripherals.PIN_0);
    register_panic_uart(&mut uart);

    log_line(&mut uart, "fast clkdiv2 validation start");
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
    run_fast_clkdiv2_validation_harness(&mut uart, backend, write, read);
}

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
fn run_fast_clkdiv2_validation_harness(
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

    run_fast_falling_clkdiv2_validation(uart, &mut driver, write, read);

    log_line(uart, "fast clkdiv2 validation done");
    loop {}
}

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
fn run_fast_falling_clkdiv2_validation(
    uart: &mut UartTx<'static, Blocking>,
    driver: &mut BlockingDriver<PicocalcPayloadBackend<'static>>,
    write: &mut [u8; MAX_BUFFER_BYTES],
    read: &mut [u8; MAX_BUFFER_BYTES],
) {
    log_line(uart, "fast_falling_clkdiv2_practical validation start");
    set_word_stream_read_diagnostics(uart, driver, WordStreamReadDiagnostics::default());

    let candidate = ReadClkdivCandidate::new(20);
    if configure_fast_read_loop_clkdiv_timing(
        uart,
        driver,
        candidate,
        TransactionPioFastReadLoopVariant::FallingExtraDummyByte,
    )
    .is_err()
    {
        log_fast_falling_clkdiv2_summary_prefix(
            uart,
            "configure",
            BENCH_ADDR,
            0,
            BenchPattern::AddressDerived,
            0,
            0,
            TransactionPioDiagnostics::default(),
            None,
            None,
        );
        log_line(uart, " status=error");
        log_line(uart, "fast_falling_clkdiv2_practical validation done");
        return;
    }

    for pattern in [
        BenchPattern::RepeatedAd,
        BenchPattern::WalkingByte,
        BenchPattern::AddressDerived,
    ] {
        if !run_one_fast_falling_clkdiv2_case(
            uart,
            driver,
            write,
            read,
            "basic",
            BENCH_ADDR,
            16,
            pattern,
            FAST_FALLING_CLKDIV2_CHUNK_LEN,
            None,
        ) {
            log_line(uart, "fast_falling_clkdiv2_practical validation done");
            return;
        }
    }

    for total_bytes in FAST_FALLING_CLKDIV2_SIZE_SWEEP_BYTES {
        if !run_one_fast_falling_clkdiv2_case(
            uart,
            driver,
            write,
            read,
            "size_sweep",
            BENCH_ADDR,
            total_bytes,
            BenchPattern::AddressDerived,
            FAST_FALLING_CLKDIV2_CHUNK_LEN,
            None,
        ) {
            log_line(uart, "fast_falling_clkdiv2_practical validation done");
            return;
        }
    }

    for addr in FAST_FALLING_CLKDIV2_BOUNDARY_ADDRS {
        for total_bytes in [64, 512, 4096] {
            if !fast_falling_clkdiv2_range_valid(addr, total_bytes) || total_bytes > read.len() {
                continue;
            }
            if !run_one_fast_falling_clkdiv2_case(
                uart,
                driver,
                write,
                read,
                "boundary_sweep",
                addr,
                total_bytes,
                BenchPattern::AddressDerived,
                FAST_FALLING_CLKDIV2_CHUNK_LEN,
                None,
            ) {
                log_line(uart, "fast_falling_clkdiv2_practical validation done");
                return;
            }
        }
    }

    if !run_fast_falling_clkdiv2_stress(uart, driver, write, read, 4096, 4096, 100, "stress") {
        log_line(uart, "fast_falling_clkdiv2_practical validation done");
        return;
    }
    if !run_fast_falling_clkdiv2_stress(
        uart,
        driver,
        write,
        read,
        16 * 1024,
        FAST_FALLING_CLKDIV2_CHUNK_LEN,
        100,
        "stress",
    ) {
        log_line(uart, "fast_falling_clkdiv2_practical validation done");
        return;
    }

    for total_bytes in FAST_FALLING_CLKDIV2_WRITE_READ_BYTES {
        if !run_one_fast_falling_clkdiv2_case(
            uart,
            driver,
            write,
            read,
            "write_read",
            BENCH_ADDR,
            total_bytes,
            BenchPattern::AddressDerived,
            FAST_FALLING_CLKDIV2_CHUNK_LEN,
            None,
        ) {
            log_line(uart, "fast_falling_clkdiv2_practical validation done");
            return;
        }
    }

    log_line(
        uart,
        "fast_falling_clkdiv2_practical validation status=passed",
    );
    log_line(uart, "fast_falling_clkdiv2_practical validation done");
}

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
fn run_fast_falling_clkdiv2_stress(
    uart: &mut UartTx<'static, Blocking>,
    driver: &mut BlockingDriver<PicocalcPayloadBackend<'static>>,
    write: &mut [u8; MAX_BUFFER_BYTES],
    read: &mut [u8; MAX_BUFFER_BYTES],
    total_bytes: usize,
    chunk_len: usize,
    iterations: usize,
    phase: &str,
) -> bool {
    set_payload_path_silent(driver, FAST_FALLING_CLKDIV2_PREFILL_PATH);
    if write_transfer_at_addr(
        driver,
        write,
        BENCH_ADDR,
        total_bytes,
        BenchPattern::AddressDerived,
    )
    .is_err()
    {
        log_fast_falling_clkdiv2_summary_prefix(
            uart,
            phase,
            BENCH_ADDR,
            total_bytes,
            BenchPattern::AddressDerived,
            chunk_len,
            0,
            TransactionPioDiagnostics::default(),
            None,
            Some(0),
        );
        log_fast_falling_clkdiv2_prefill_error_suffix(
            uart,
            FAST_FALLING_CLKDIV2_PREFILL_PATH,
            BENCH_ADDR,
            total_bytes,
            BenchPattern::AddressDerived,
        );
        set_payload_path_silent(driver, FAST_FALLING_CLKDIV2_READ_PATH);
        return false;
    }
    set_payload_path_silent(driver, FAST_FALLING_CLKDIV2_READ_PATH);

    let start = Instant::now();
    for iteration in 0..iterations {
        if !run_one_fast_falling_clkdiv2_read_compare(
            uart,
            driver,
            write,
            read,
            phase,
            BENCH_ADDR,
            total_bytes,
            BenchPattern::AddressDerived,
            chunk_len,
            Some(iteration),
            false,
        ) {
            return false;
        }
    }
    let elapsed_us = Instant::now().duration_since(start).as_micros();

    log_fast_falling_clkdiv2_summary_prefix(
        uart,
        phase,
        BENCH_ADDR,
        total_bytes.saturating_mul(iterations),
        BenchPattern::AddressDerived,
        chunk_len,
        elapsed_us,
        TransactionPioDiagnostics::default(),
        None,
        None,
    );
    log_line(uart, " status=ok");
    true
}

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
fn fast_falling_clkdiv2_range_valid(addr: u32, total_bytes: usize) -> bool {
    let Ok(len) = u32::try_from(total_bytes) else {
        return false;
    };
    PsramAddr::new(addr)
        .map(|addr| addr.checked_range_len(len).is_ok())
        .unwrap_or(false)
}

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
fn run_one_fast_falling_clkdiv2_case(
    uart: &mut UartTx<'static, Blocking>,
    driver: &mut BlockingDriver<PicocalcPayloadBackend<'static>>,
    write: &mut [u8; MAX_BUFFER_BYTES],
    read: &mut [u8; MAX_BUFFER_BYTES],
    phase: &str,
    addr: u32,
    total_bytes: usize,
    pattern: BenchPattern,
    chunk_len: usize,
    iteration: Option<usize>,
) -> bool {
    set_payload_path_silent(driver, FAST_FALLING_CLKDIV2_PREFILL_PATH);
    if write_transfer_at_addr(driver, write, addr, total_bytes, pattern).is_err() {
        log_fast_falling_clkdiv2_summary_prefix(
            uart,
            phase,
            addr,
            total_bytes,
            pattern,
            chunk_len,
            0,
            TransactionPioDiagnostics::default(),
            None,
            iteration,
        );
        log_fast_falling_clkdiv2_prefill_error_suffix(
            uart,
            FAST_FALLING_CLKDIV2_PREFILL_PATH,
            addr,
            total_bytes,
            pattern,
        );
        set_payload_path_silent(driver, FAST_FALLING_CLKDIV2_READ_PATH);
        return false;
    }
    set_payload_path_silent(driver, FAST_FALLING_CLKDIV2_READ_PATH);

    run_one_fast_falling_clkdiv2_read_compare(
        uart,
        driver,
        write,
        read,
        phase,
        addr,
        total_bytes,
        pattern,
        chunk_len,
        iteration,
        true,
    )
}

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
fn run_one_fast_falling_clkdiv2_read_compare(
    uart: &mut UartTx<'static, Blocking>,
    driver: &mut BlockingDriver<PicocalcPayloadBackend<'static>>,
    write: &mut [u8; MAX_BUFFER_BYTES],
    read: &mut [u8; MAX_BUFFER_BYTES],
    phase: &str,
    addr: u32,
    total_bytes: usize,
    pattern: BenchPattern,
    chunk_len: usize,
    iteration: Option<usize>,
    log_success: bool,
) -> bool {
    configure_fast_falling_clkdiv2_read_path(driver);
    driver
        .backend_mut_for_diagnostics()
        .reset_qpi_timing_for_diagnostics();
    read[..total_bytes].fill(0);

    let start = Instant::now();
    let read_result =
        read_fast_falling_clkdiv2_transfer(driver, &mut read[..total_bytes], addr, chunk_len);
    let elapsed_us = Instant::now().duration_since(start).as_micros();
    let read_result_ok = read_result.is_ok();
    let diagnostics = read_result.unwrap_or_else(|diagnostics| diagnostics);

    fill_pattern_at_addr(&mut write[..total_bytes], addr, 0, pattern);
    let mismatch = first_mismatch(&write[..total_bytes], &read[..total_bytes]);
    if read_result_ok && mismatch.is_none() && !log_success {
        set_payload_path_silent(driver, FAST_FALLING_CLKDIV2_READ_PATH);
        return true;
    }

    log_fast_falling_clkdiv2_summary_prefix(
        uart,
        phase,
        addr,
        total_bytes,
        pattern,
        chunk_len,
        elapsed_us,
        diagnostics,
        mismatch,
        iteration,
    );
    if !read_result_ok {
        log_str(uart, " status=timeout");
        log_fast_falling_clkdiv2_failure_bytes(uart, write, read, total_bytes);
        log_newline(uart);
        set_payload_path_silent(driver, FAST_FALLING_CLKDIV2_READ_PATH);
        return false;
    }
    if mismatch.is_some() {
        log_str(uart, " status=mismatch");
        log_fast_falling_clkdiv2_failure_bytes(uart, write, read, total_bytes);
        log_newline(uart);
        set_payload_path_silent(driver, FAST_FALLING_CLKDIV2_READ_PATH);
        return false;
    }

    if log_success {
        log_line(uart, " status=ok");
    }
    set_payload_path_silent(driver, FAST_FALLING_CLKDIV2_READ_PATH);
    true
}

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
fn read_fast_falling_clkdiv2_transfer(
    driver: &mut BlockingDriver<PicocalcPayloadBackend<'static>>,
    read: &mut [u8],
    addr: u32,
    chunk_len: usize,
) -> Result<TransactionPioDiagnostics, TransactionPioDiagnostics> {
    let mut offset = 0;
    let mut aggregate = TransactionPioDiagnostics::default();
    aggregate.rx_dma = true;

    while offset < read.len() {
        let len = (read.len() - offset).min(chunk_len);
        let result = driver
            .backend_mut_for_diagnostics()
            .read_qpi_chunk_transaction_pio_fast_rx_byte_fifo_rx_dma_diagnostic_for_diagnostics(
                psram_addr_at(addr, offset),
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

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
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

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
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

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
fn set_payload_path_silent(
    driver: &mut BlockingDriver<PicocalcPayloadBackend<'static>>,
    path: PayloadTransferPath,
) {
    driver
        .backend_mut_for_diagnostics()
        .set_payload_transfer_path_for_diagnostics(path);
}

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
fn configure_fast_falling_clkdiv2_read_path(
    driver: &mut BlockingDriver<PicocalcPayloadBackend<'static>>,
) {
    driver
        .backend_mut_for_diagnostics()
        .set_transaction_pio_fast_read_loop_variant_for_diagnostics(
            TransactionPioFastReadLoopVariant::FallingExtraDummyByte,
        );
    set_payload_path_silent(driver, FAST_FALLING_CLKDIV2_READ_PATH);
}

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
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

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
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

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
fn write_transfer_at_addr(
    driver: &mut BlockingDriver<PicocalcPayloadBackend<'static>>,
    write: &mut [u8; MAX_BUFFER_BYTES],
    addr: u32,
    total_bytes: usize,
    pattern: BenchPattern,
) -> Result<(), PayloadBackendError> {
    let mut offset = 0;
    while offset < total_bytes {
        let len = (total_bytes - offset).min(write.len());
        fill_pattern_at_addr(&mut write[..len], addr, offset, pattern);
        driver.write_all(psram_addr_at(addr, offset), &write[..len])?;
        offset += len;
    }

    Ok(())
}

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
fn psram_addr_at(addr: u32, offset: usize) -> PsramAddr {
    PsramAddr::new(addr + offset as u32).unwrap()
}

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BenchPattern {
    RepeatedAd,
    WalkingByte,
    AddressDerived,
}

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ReadClkdivCandidate {
    x10: u32,
}

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
impl ReadClkdivCandidate {
    const fn new(x10: u32) -> Self {
        Self { x10 }
    }

    fn as_f32(self) -> f32 {
        self.x10 as f32 / 10.0
    }
}

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
fn fill_pattern_at_addr(buf: &mut [u8], addr: u32, base_offset: usize, pattern: BenchPattern) {
    for (offset, byte) in buf.iter_mut().enumerate() {
        let absolute_offset = base_offset + offset;
        *byte = pattern_byte_at_addr(addr, absolute_offset, pattern);
    }
}

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
fn pattern_byte_at_addr(addr: u32, absolute_offset: usize, pattern: BenchPattern) -> u8 {
    match pattern {
        BenchPattern::RepeatedAd => 0xad,
        BenchPattern::WalkingByte => absolute_offset as u8,
        BenchPattern::AddressDerived => {
            let addr_byte = addr.wrapping_add(absolute_offset as u32) as u8;
            addr_byte ^ (CHUNK_LEN as u8).wrapping_mul(3) ^ ((absolute_offset >> 3) as u8)
        }
    }
}

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BenchMismatch {
    offset: usize,
    expected: u8,
    actual: u8,
}

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
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

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
fn picocalc_uart_usb_tx(
    uart0: Peri<'static, embassy_rp::peripherals::UART0>,
    tx: Peri<'static, embassy_rp::peripherals::PIN_0>,
) -> UartTx<'static, Blocking> {
    let mut config = UartConfig::default();
    config.baudrate = PICOCALC_UART_USB_BAUD;

    // PicoCalc UART-USB bridge: RP2040 UART0 TX on GP0. RX/GP1 is unused.
    UartTx::new_blocking(uart0, tx, config)
}

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
static mut PANIC_UART: *mut UartTx<'static, Blocking> = core::ptr::null_mut();

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
fn register_panic_uart(uart: &mut UartTx<'static, Blocking>) {
    unsafe {
        PANIC_UART = uart as *mut _;
    }
}

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
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

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
fn log_fast_falling_clkdiv2_summary_prefix(
    uart: &mut UartTx<'static, Blocking>,
    kind: &str,
    addr: u32,
    total_bytes: usize,
    pattern: BenchPattern,
    chunk_len: usize,
    elapsed_us: u64,
    diagnostics: TransactionPioDiagnostics,
    mismatch: Option<BenchMismatch>,
    iteration: Option<usize>,
) {
    log_str(uart, "bench profile=fast_falling_clkdiv2_practical");
    log_str(uart, " kind=");
    log_str(uart, kind);
    log_str(
        uart,
        " read_clkdiv=2.0 sample_edge=falling extra_dummy_byte=true",
    );
    log_str(uart, " rx_byte_fifo=true rx_dma_direct_u8=true dma_size=8");
    log_str(uart, " addr=0x");
    log_hex_u32(uart, addr);
    log_str(uart, " chunk_len=");
    log_dec_usize(uart, chunk_len.min(total_bytes));
    log_str(uart, " total_bytes=");
    log_dec_usize(uart, total_bytes);
    log_str(uart, " pattern=");
    log_pattern(uart, pattern);
    if let Some(iteration) = iteration {
        log_str(uart, " iteration=");
        log_dec_usize(uart, iteration);
    }
    log_str(uart, " elapsed_us=");
    log_dec_u64(uart, elapsed_us);
    log_str(uart, " bytes_per_sec=");
    log_dec_u64(uart, bytes_per_sec(total_bytes, elapsed_us));
    log_str(uart, " mb_s_x100=");
    log_dec_u64(uart, bytes_per_sec(total_bytes, elapsed_us) / 10_000);
    if let Some(mismatch) = mismatch {
        log_str(uart, " fail_off=");
        log_dec_usize(uart, mismatch.offset);
        log_str(uart, " expected=0x");
        log_hex_u8(uart, mismatch.expected);
        log_str(uart, " actual=0x");
        log_hex_u8(uart, mismatch.actual);
    }
    log_str(uart, " tx_dma_error_flags=");
    log_dma_error_flags(
        uart,
        diagnostics.tx_dma_busy,
        diagnostics.tx_dma_read_error,
        diagnostics.tx_dma_write_error,
        diagnostics.tx_dma_ahb_error,
    );
    log_str(uart, " rx_dma_error_flags=");
    log_dma_error_flags(
        uart,
        diagnostics.rx_dma_busy,
        diagnostics.rx_dma_read_error,
        diagnostics.rx_dma_write_error,
        diagnostics.rx_dma_ahb_error,
    );
    log_str(uart, " timeout=");
    log_bool(uart, fast_falling_clkdiv2_timeout_flag(diagnostics));
    log_str(uart, " progress_flags=0x");
    log_hex_u32(uart, diagnostics.progress_flags);
}

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
fn log_fast_falling_clkdiv2_failure_bytes(
    uart: &mut UartTx<'static, Blocking>,
    expected: &[u8],
    actual: &[u8],
    total_bytes: usize,
) {
    let first_len = total_bytes.min(32);
    log_str(uart, " expected_first32=");
    log_bytes_hex(uart, &expected[..first_len]);
    log_str(uart, " actual_first32=");
    log_bytes_hex(uart, &actual[..first_len]);
}

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
fn log_fast_falling_clkdiv2_prefill_error_suffix(
    uart: &mut UartTx<'static, Blocking>,
    prefill_path: PayloadTransferPath,
    addr: u32,
    len: usize,
    pattern: BenchPattern,
) {
    log_str(uart, " prefill_path=");
    log_payload_path(uart, prefill_path);
    log_str(uart, " addr=0x");
    log_hex_u32(uart, addr);
    log_str(uart, " len=");
    log_dec_usize(uart, len);
    log_str(uart, " pattern=");
    log_pattern(uart, pattern);
    log_line(uart, " status=error error=prefill");
}

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
fn log_dma_error_flags(
    uart: &mut UartTx<'static, Blocking>,
    busy: bool,
    read_error: bool,
    write_error: bool,
    ahb_error: bool,
) {
    let flags = (busy as u8)
        | ((read_error as u8) << 1)
        | ((write_error as u8) << 2)
        | ((ahb_error as u8) << 3);
    log_str(uart, "0x");
    log_hex_u8(uart, flags);
}

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
fn fast_falling_clkdiv2_timeout_flag(diagnostics: TransactionPioDiagnostics) -> bool {
    let rx_started =
        diagnostics.progress_flags & TransactionPioDiagnostics::STEP_RX_PULL_START != 0;
    let rx_done = diagnostics.progress_flags & TransactionPioDiagnostics::STEP_RX_PULL_DONE != 0;
    diagnostics.rx_dma_busy
        || diagnostics.rx_dma_read_error
        || diagnostics.rx_dma_write_error
        || diagnostics.rx_dma_ahb_error
        || (rx_started && !rx_done)
}

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
fn log_pattern(uart: &mut UartTx<'static, Blocking>, pattern: BenchPattern) {
    match pattern {
        BenchPattern::RepeatedAd => log_str(uart, "repeated_0xad"),
        BenchPattern::WalkingByte => log_str(uart, "walking-byte"),
        BenchPattern::AddressDerived => log_str(uart, "address-derived"),
    }
}

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
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

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
fn log_bool(uart: &mut UartTx<'static, Blocking>, value: bool) {
    if value {
        log_str(uart, "true");
    } else {
        log_str(uart, "false");
    }
}

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
fn bytes_per_sec(total_bytes: usize, elapsed_us: u64) -> u64 {
    if elapsed_us == 0 {
        return 0;
    }

    (total_bytes as u64).saturating_mul(1_000_000) / elapsed_us
}

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
fn log_line(uart: &mut UartTx<'static, Blocking>, line: &str) {
    log_str(uart, line);
    log_newline(uart);
}

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
fn log_str(uart: &mut UartTx<'static, Blocking>, text: &str) {
    let _ = uart.blocking_write(text.as_bytes());
}

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
fn log_newline(uart: &mut UartTx<'static, Blocking>) {
    let _ = uart.blocking_write(b"\r\n");
}

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
fn log_byte(uart: &mut UartTx<'static, Blocking>, byte: u8) {
    let _ = uart.blocking_write(&[byte]);
}

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
fn log_hex_u8(uart: &mut UartTx<'static, Blocking>, value: u8) {
    log_nibble(uart, value >> 4);
    log_nibble(uart, value & 0x0f);
}

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
fn log_hex_u32(uart: &mut UartTx<'static, Blocking>, value: u32) {
    for shift in [28, 24, 20, 16, 12, 8, 4, 0] {
        log_nibble(uart, ((value >> shift) & 0x0f) as u8);
    }
}

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
fn log_bytes_hex(uart: &mut UartTx<'static, Blocking>, bytes: &[u8]) {
    for (index, byte) in bytes.iter().enumerate() {
        if index > 0 {
            log_byte(uart, b' ');
        }
        log_str(uart, "0x");
        log_hex_u8(uart, *byte);
    }
}

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
fn log_nibble(uart: &mut UartTx<'static, Blocking>, nibble: u8) {
    let byte = if nibble < 10 {
        b'0' + nibble
    } else {
        b'a' + (nibble - 10)
    };
    log_byte(uart, byte);
}

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
fn log_dec_usize(uart: &mut UartTx<'static, Blocking>, value: usize) {
    log_dec_u64(uart, value as u64);
}

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
fn log_dec_u32(uart: &mut UartTx<'static, Blocking>, value: u32) {
    log_dec_u64(uart, value as u64);
}

#[cfg(all(
    feature = "rp2040-embassy",
    feature = "psram_fast_read_clkdiv2",
    target_os = "none"
))]
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
