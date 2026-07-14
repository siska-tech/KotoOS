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
        EmbassyRpQpiBackend, PayloadTransferPath, QpiChunkTiming, WordStreamReadDiagnostics,
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
const TOTAL_LENGTHS: [usize; 2] = [64 * 1024, 256 * 1024];

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
const PAYLOAD_PATHS: [PayloadTransferPath; 3] = [
    PayloadTransferPath::ByteFallback,
    PayloadTransferPath::PollingBurstDiagnostic,
    PayloadTransferPath::WordStreamPolling,
];

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
const WORD_STREAM_READ_BATCH_WORDS: [usize; 4] = [4, 8, 16, 32];

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
const WORD_STREAM_READ_RX_FIFO_JOIN: [bool; 1] = [false];

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

    log_line(&mut uart, "payload path bench start");
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
    run_payload_path_bench(&mut uart, backend, write, read);
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn run_payload_path_bench(
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

    for path in PAYLOAD_PATHS {
        set_payload_path(uart, &mut driver, path);
        match path {
            PayloadTransferPath::WordStreamPolling => {
                for batch_words in WORD_STREAM_READ_BATCH_WORDS {
                    for rx_fifo_join in WORD_STREAM_READ_RX_FIFO_JOIN {
                        set_word_stream_read_diagnostics(
                            uart,
                            &mut driver,
                            WordStreamReadDiagnostics {
                                batch_words,
                                rx_fifo_join,
                            },
                        );
                        for total_bytes in TOTAL_LENGTHS {
                            for pattern in BENCH_PATTERNS {
                                run_one_payload_path_bench(
                                    uart,
                                    &mut driver,
                                    write,
                                    read,
                                    path,
                                    total_bytes,
                                    pattern,
                                );
                            }
                        }
                    }
                }
            }
            _ => {
                set_word_stream_read_diagnostics(
                    uart,
                    &mut driver,
                    WordStreamReadDiagnostics::default(),
                );
                for total_bytes in TOTAL_LENGTHS {
                    for pattern in BENCH_PATTERNS {
                        run_one_payload_path_bench(
                            uart,
                            &mut driver,
                            write,
                            read,
                            path,
                            total_bytes,
                            pattern,
                        );
                    }
                }
            }
        }
    }

    log_line(uart, "payload path bench done");
    loop {}
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
fn run_one_payload_path_bench(
    uart: &mut UartTx<'static, Blocking>,
    driver: &mut BlockingDriver<PicocalcPayloadBackend<'static>>,
    write: &mut [u8; MAX_BUFFER_BYTES],
    read: &mut [u8; MAX_BUFFER_BYTES],
    path: PayloadTransferPath,
    total_bytes: usize,
    pattern: BenchPattern,
) {
    driver
        .backend_mut_for_diagnostics()
        .reset_qpi_timing_for_diagnostics();
    let start = Instant::now();
    let write_result = write_transfer(driver, write, total_bytes, pattern);
    let elapsed_us = Instant::now().duration_since(start).as_micros();
    let timing = driver
        .backend_mut_for_diagnostics()
        .qpi_timing_for_diagnostics();
    let diagnostics = driver
        .backend_mut_for_diagnostics()
        .word_stream_read_diagnostics();
    log_result_prefix(
        uart,
        "write",
        path,
        total_bytes,
        pattern,
        elapsed_us,
        timing,
        diagnostics,
    );
    if write_result.is_ok() {
        log_line(uart, " status=ok");
    } else {
        log_line(uart, " status=error");
        return;
    }

    driver
        .backend_mut_for_diagnostics()
        .reset_qpi_timing_for_diagnostics();
    let start = Instant::now();
    let read_result = read_transfer(driver, read, total_bytes);
    let elapsed_us = Instant::now().duration_since(start).as_micros();
    let timing = driver
        .backend_mut_for_diagnostics()
        .qpi_timing_for_diagnostics();
    let diagnostics = driver
        .backend_mut_for_diagnostics()
        .word_stream_read_diagnostics();
    log_result_prefix(
        uart,
        "read",
        path,
        total_bytes,
        pattern,
        elapsed_us,
        timing,
        diagnostics,
    );
    if read_result.is_ok() {
        log_line(uart, " status=ok");
    } else {
        log_line(uart, " status=error");
        return;
    }

    driver
        .backend_mut_for_diagnostics()
        .reset_qpi_timing_for_diagnostics();
    let start = Instant::now();
    let mismatch = compare_transfer(driver, write, read, total_bytes, pattern);
    let elapsed_us = Instant::now().duration_since(start).as_micros();
    let timing = driver
        .backend_mut_for_diagnostics()
        .qpi_timing_for_diagnostics();
    let diagnostics = driver
        .backend_mut_for_diagnostics()
        .word_stream_read_diagnostics();
    log_result_prefix(
        uart,
        "compare",
        path,
        total_bytes,
        pattern,
        elapsed_us,
        timing,
        diagnostics,
    );
    match mismatch {
        Ok(Some(mismatch)) => {
            log_str(uart, " status=error mismatch_offset=");
            log_dec_usize(uart, mismatch.offset);
            log_str(uart, " expected=0x");
            log_hex_u8(uart, mismatch.expected);
            log_str(uart, " actual=0x");
            log_hex_u8(uart, mismatch.actual);
            log_newline(uart);
        }
        Ok(None) => log_line(uart, " status=ok"),
        Err(_error) => log_line(uart, " status=error"),
    }
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
fn read_transfer(
    driver: &mut BlockingDriver<PicocalcPayloadBackend<'static>>,
    read: &mut [u8; MAX_BUFFER_BYTES],
    total_bytes: usize,
) -> Result<(), PayloadBackendError> {
    let mut offset = 0;
    while offset < total_bytes {
        let len = (total_bytes - offset).min(read.len());
        read[..len].fill(0);
        driver.read_exact(bench_addr_at(offset), &mut read[..len])?;
        offset += len;
    }

    Ok(())
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn compare_transfer(
    driver: &mut BlockingDriver<PicocalcPayloadBackend<'static>>,
    expected: &mut [u8; MAX_BUFFER_BYTES],
    actual: &mut [u8; MAX_BUFFER_BYTES],
    total_bytes: usize,
    pattern: BenchPattern,
) -> Result<Option<BenchMismatch>, PayloadBackendError> {
    let mut offset = 0;
    while offset < total_bytes {
        let len = (total_bytes - offset).min(expected.len());
        fill_pattern(&mut expected[..len], offset, pattern);
        actual[..len].fill(0);
        driver.read_exact(bench_addr_at(offset), &mut actual[..len])?;
        if let Some(mut mismatch) = first_mismatch(&expected[..len], &actual[..len]) {
            mismatch.offset += offset;
            return Ok(Some(mismatch));
        }
        offset += len;
    }

    Ok(None)
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn bench_addr_at(offset: usize) -> PsramAddr {
    PsramAddr::new(BENCH_ADDR + offset as u32).unwrap()
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BenchPattern {
    RepeatedAd,
    WalkingByte,
    AddressDerived,
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
fn log_result_prefix(
    uart: &mut UartTx<'static, Blocking>,
    operation: &str,
    path: PayloadTransferPath,
    total_bytes: usize,
    pattern: BenchPattern,
    elapsed_us: u64,
    timing: QpiChunkTiming,
    diagnostics: WordStreamReadDiagnostics,
) {
    log_str(uart, "bench profile=payload_path");
    log_str(uart, " payload_path=");
    log_payload_path(uart, path);
    log_str(uart, " chunk_len=");
    log_dec_usize(uart, CHUNK_LEN);
    log_str(uart, " total_bytes=");
    log_dec_usize(uart, total_bytes);
    log_str(uart, " pattern=");
    log_pattern(uart, pattern);
    log_str(uart, " operation=");
    log_str(uart, operation);
    log_str(uart, " batch_words=");
    log_dec_usize(uart, diagnostics.batch_words);
    log_str(uart, " rx_fifo_join=");
    log_bool(uart, diagnostics.rx_fifo_join);
    log_str(uart, " elapsed_us=");
    log_dec_u64(uart, elapsed_us);
    log_str(uart, " bytes_per_sec=");
    log_dec_u64(uart, bytes_per_sec(total_bytes, elapsed_us));
    log_str(uart, " command_addr_dummy_us=");
    log_dec_u64(uart, timing.command_addr_dummy_us);
    log_str(uart, " payload_write_loop_us=");
    log_dec_u64(uart, timing.payload_write_us);
    log_str(uart, " payload_read_loop_us=");
    log_dec_u64(uart, timing.payload_read_us);
    log_str(uart, " word_stream_rx_fifo_wait_us=");
    log_dec_u64(uart, timing.word_stream_rx_fifo_wait_us);
    log_str(uart, " word_stream_rx_pull_loop_us=");
    log_dec_u64(uart, timing.word_stream_rx_pull_loop_us);
    log_str(uart, " word_stream_unpack_loop_us=");
    log_dec_u64(uart, timing.word_stream_unpack_loop_us);
    log_str(uart, " word_stream_tail_unpack_us=");
    log_dec_u64(uart, timing.word_stream_tail_unpack_us);
    log_str(uart, " flush_us=");
    log_dec_u64(uart, timing.flush_us);
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
