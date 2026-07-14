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
    rp2040_embassy::EmbassyRpQpiBackend,
};

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
const PICOCALC_UART_USB_BAUD: u32 = 115_200;
#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
const PROFILE: &str = "clkdiv_sweep";
#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
const BENCH_ADDR: u32 = 0x0000_2000;
#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
const CHUNK_LEN: usize = 512;
#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
const SHORT_TOTAL_BYTES: usize = 64 * 1024;
#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
const LONG_TOTAL_BYTES: usize = 256 * 1024;
#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
const MAX_BUFFER_BYTES: usize = SHORT_TOTAL_BYTES;
#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
const CANDIDATES: [ClkdivCandidate; 5] = [
    ClkdivCandidate::new(8, 8),
    ClkdivCandidate::new(6, 6),
    ClkdivCandidate::new(4, 4),
    ClkdivCandidate::new(3, 3),
    ClkdivCandidate::new(2, 2),
];

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
static mut WRITE_BUF: [u8; MAX_BUFFER_BYTES] = [0; MAX_BUFFER_BYTES];
#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
static mut READ_BUF: [u8; MAX_BUFFER_BYTES] = [0; MAX_BUFFER_BYTES];

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ClkdivCandidate {
    read_clkdiv: u32,
    write_clkdiv: u32,
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
impl ClkdivCandidate {
    const fn new(read_clkdiv: u32, write_clkdiv: u32) -> Self {
        Self {
            read_clkdiv,
            write_clkdiv,
        }
    }

    const fn timing(self) -> TimingConfig {
        TimingConfig {
            read_clkdiv: self.read_clkdiv as f32,
            write_clkdiv: self.write_clkdiv as f32,
            max_chunk_len: CHUNK_LEN,
            ..TimingConfig::PICOCALC_SAFE
        }
    }
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SweepOutcome {
    Pass,
    RecoverableFail,
    UnrecoverableFail,
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
embassy_rp::bind_interrupts!(struct Irqs {
    PIO0_IRQ_0 => embassy_rp::pio::InterruptHandler<embassy_rp::peripherals::PIO0>;
});

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
type PicocalcSweepBackend<'d> = EmbassyRpQpiBackend<
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

#[cfg(not(all(feature = "rp2040-embassy", target_os = "none")))]
fn main() {}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
#[cortex_m_rt::entry]
fn embedded_main() -> ! {
    let peripherals = embassy_rp::init(Default::default());
    let mut uart = picocalc_uart_usb_tx(peripherals.UART0, peripherals.PIN_0);
    register_panic_uart(&mut uart);

    log_line(&mut uart, "clkdiv sweep boot");
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
    log_line(&mut uart, "backend configure ok");

    let write = unsafe { &mut *core::ptr::addr_of_mut!(WRITE_BUF) };
    let read = unsafe { &mut *core::ptr::addr_of_mut!(READ_BUF) };
    run_clkdiv_sweep(&mut uart, backend, write, read);
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn run_clkdiv_sweep(
    uart: &mut UartTx<'static, Blocking>,
    backend: PicocalcSweepBackend<'static>,
    write: &mut [u8; MAX_BUFFER_BYTES],
    read: &mut [u8; MAX_BUFFER_BYTES],
) -> ! {
    let mut driver =
        BlockingDriver::with_config(backend, Pins::PICOCALC, TimingConfig::PICOCALC_SAFE);

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
            log_result_prefix(uart, "init", ClkdivCandidate::new(0, 0), 0, 0);
            log_line(uart, " status=error");
            loop {}
        }
    }

    for candidate in CANDIDATES {
        match run_candidate(uart, &mut driver, write, read, candidate) {
            SweepOutcome::Pass | SweepOutcome::RecoverableFail => {}
            SweepOutcome::UnrecoverableFail => {
                log_line(uart, "clkdiv sweep stopped");
                loop {}
            }
        }
    }

    log_line(uart, "clkdiv sweep done");
    loop {}
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn run_candidate<P>(
    uart: &mut UartTx<'static, Blocking>,
    driver: &mut BlockingDriver<P>,
    write: &mut [u8; MAX_BUFFER_BYTES],
    read: &mut [u8; MAX_BUFFER_BYTES],
    candidate: ClkdivCandidate,
) -> SweepOutcome
where
    P: BlockingPio,
{
    log_result_prefix(uart, "configure", candidate, 0, 0);
    match driver.configure_timing(candidate.timing()) {
        Ok(()) => log_line(uart, " status=ok"),
        Err(_error) => {
            log_line(uart, " status=error");
            return SweepOutcome::UnrecoverableFail;
        }
    }

    match run_one_total(uart, driver, write, read, candidate, SHORT_TOTAL_BYTES) {
        SweepOutcome::Pass => run_one_total(uart, driver, write, read, candidate, LONG_TOTAL_BYTES),
        other => other,
    }
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn run_one_total<P>(
    uart: &mut UartTx<'static, Blocking>,
    driver: &mut BlockingDriver<P>,
    write: &mut [u8; MAX_BUFFER_BYTES],
    read: &mut [u8; MAX_BUFFER_BYTES],
    candidate: ClkdivCandidate,
    total_bytes: usize,
) -> SweepOutcome
where
    P: BlockingPio,
{
    let start = Instant::now();
    let write_result = write_transfer(driver, write, candidate, total_bytes);
    let elapsed_us = Instant::now().duration_since(start).as_micros();
    log_result_prefix(uart, "write", candidate, total_bytes, elapsed_us);
    if write_result.is_ok() {
        log_line(uart, " status=ok");
    } else {
        log_line(uart, " status=error recovery=safe_no");
        return SweepOutcome::UnrecoverableFail;
    }

    let start = Instant::now();
    let read_result = read_transfer(driver, read, total_bytes);
    let elapsed_us = Instant::now().duration_since(start).as_micros();
    log_result_prefix(uart, "read", candidate, total_bytes, elapsed_us);
    if read_result.is_ok() {
        log_line(uart, " status=ok");
    } else {
        log_line(uart, " status=error recovery=safe_no");
        return SweepOutcome::UnrecoverableFail;
    }

    let start = Instant::now();
    let mismatch = compare_transfer(driver, write, read, candidate, total_bytes);
    let elapsed_us = Instant::now().duration_since(start).as_micros();
    log_result_prefix(uart, "compare", candidate, total_bytes, elapsed_us);
    match mismatch {
        Ok(Some(mismatch)) => {
            log_str(uart, " status=error recovery=safe_yes mismatch_offset=");
            log_dec_usize(uart, mismatch.offset);
            log_str(uart, " expected=0x");
            log_hex_u8(uart, mismatch.expected);
            log_str(uart, " actual=0x");
            log_hex_u8(uart, mismatch.actual);
            log_newline(uart);
            SweepOutcome::RecoverableFail
        }
        Ok(None) => {
            log_line(uart, " status=ok");
            SweepOutcome::Pass
        }
        Err(_error) => {
            log_line(uart, " status=error recovery=safe_no");
            SweepOutcome::UnrecoverableFail
        }
    }
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn write_transfer<P>(
    driver: &mut BlockingDriver<P>,
    write: &mut [u8; MAX_BUFFER_BYTES],
    candidate: ClkdivCandidate,
    total_bytes: usize,
) -> Result<(), P::Error>
where
    P: BlockingPio,
{
    let mut offset = 0;
    while offset < total_bytes {
        let len = (total_bytes - offset).min(write.len());
        fill_pattern(&mut write[..len], offset, candidate);
        driver.write_all(bench_addr_at(offset), &write[..len])?;
        offset += len;
    }

    Ok(())
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn read_transfer<P>(
    driver: &mut BlockingDriver<P>,
    read: &mut [u8; MAX_BUFFER_BYTES],
    total_bytes: usize,
) -> Result<(), P::Error>
where
    P: BlockingPio,
{
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
fn compare_transfer<P>(
    driver: &mut BlockingDriver<P>,
    expected: &mut [u8; MAX_BUFFER_BYTES],
    actual: &mut [u8; MAX_BUFFER_BYTES],
    candidate: ClkdivCandidate,
    total_bytes: usize,
) -> Result<Option<BenchMismatch>, P::Error>
where
    P: BlockingPio,
{
    let mut offset = 0;
    while offset < total_bytes {
        let len = (total_bytes - offset).min(expected.len());
        fill_pattern(&mut expected[..len], offset, candidate);
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
fn fill_pattern(buf: &mut [u8], base_offset: usize, candidate: ClkdivCandidate) {
    for (offset, byte) in buf.iter_mut().enumerate() {
        let absolute_offset = base_offset + offset;
        let addr_byte = BENCH_ADDR.wrapping_add(absolute_offset as u32) as u8;
        *byte = addr_byte
            ^ (CHUNK_LEN as u8).wrapping_mul(3)
            ^ (candidate.read_clkdiv as u8).wrapping_mul(5)
            ^ (candidate.write_clkdiv as u8).wrapping_mul(7)
            ^ ((absolute_offset >> 3) as u8);
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
    candidate: ClkdivCandidate,
    total_bytes: usize,
    elapsed_us: u64,
) {
    log_str(uart, "bench profile=");
    log_str(uart, PROFILE);
    log_str(uart, " read_clkdiv=");
    log_dec_u32(uart, candidate.read_clkdiv);
    log_str(uart, " write_clkdiv=");
    log_dec_u32(uart, candidate.write_clkdiv);
    log_str(uart, " chunk_len=");
    log_dec_usize(uart, CHUNK_LEN);
    log_str(uart, " total_bytes=");
    log_dec_usize(uart, total_bytes);
    log_str(uart, " operation=");
    log_str(uart, operation);
    log_str(uart, " elapsed_us=");
    log_dec_u64(uart, elapsed_us);
    log_str(uart, " bytes_per_sec=");
    log_dec_u64(uart, bytes_per_sec(total_bytes, elapsed_us));
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
