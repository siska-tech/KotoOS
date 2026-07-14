#![cfg_attr(all(feature = "rp2040-embassy", target_os = "none"), no_std)]
#![cfg_attr(all(feature = "rp2040-embassy", target_os = "none"), no_main)]

#[cfg(feature = "rp2040-embassy")]
use koto_psram::{
    addr::PsramAddr,
    bus::PsramBus,
    config::{Pins, TimingConfig},
    device::DeviceId,
    pio::blocking::{BlockingDriver, BlockingPio},
    state::PsramState,
};

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
use koto_psram::rp2040_embassy::EmbassyRpQpiBackend;

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
use embassy_rp::{
    uart::{Blocking, Config as UartConfig, UartTx},
    Peri,
};

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
const PICOCALC_UART_USB_BAUD: u32 = 115_200;

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
const UART_STAGE0_ONLY: bool = false;

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
const PRE_BACKEND_BOOT_REPEATS: usize = 5;

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
embassy_rp::bind_interrupts!(struct Irqs {
    PIO0_IRQ_0 => embassy_rp::pio::InterruptHandler<embassy_rp::peripherals::PIO0>;
});

#[cfg(feature = "rp2040-embassy")]
const COMPARE_ADDR: u32 = 0x0000_0020;
#[cfg(feature = "rp2040-embassy")]
const BOUNDARY_ADDR: u32 = 0x0000_0100;
#[cfg(feature = "rp2040-embassy")]
const MAX_COMPARE_LEN: usize = 512;
#[cfg(feature = "rp2040-embassy")]
const PREVIEW_LEN: usize = 32;

#[cfg(feature = "rp2040-embassy")]
const FIRST_COMPARE_CASE: CompareCase = CompareCase::new(COMPARE_ADDR, 32, Pattern::Repeated(0xad));

#[cfg(feature = "rp2040-embassy")]
const REPEATED_PATTERN_CASES: [CompareCase; 5] = [
    CompareCase::new(COMPARE_ADDR, 32, Pattern::Repeated(0x00)),
    CompareCase::new(COMPARE_ADDR, 32, Pattern::Repeated(0xff)),
    CompareCase::new(COMPARE_ADDR, 32, Pattern::Repeated(0xad)),
    CompareCase::new(COMPARE_ADDR, 32, Pattern::Repeated(0x55)),
    CompareCase::new(COMPARE_ADDR, 32, Pattern::Repeated(0xaa)),
];

#[cfg(feature = "rp2040-embassy")]
const BOUNDARY_CASES: [CompareCase; 7] = [
    CompareCase::new(BOUNDARY_ADDR + 0x0000, 1, Pattern::RepeatedAd),
    CompareCase::new(BOUNDARY_ADDR + 0x0200, 31, Pattern::WalkingByte),
    CompareCase::new(BOUNDARY_ADDR + 0x0400, 32, Pattern::AddressDerived),
    CompareCase::new(BOUNDARY_ADDR + 0x0600, 255, Pattern::RepeatedAd),
    CompareCase::new(BOUNDARY_ADDR + 0x0800, 256, Pattern::WalkingByte),
    CompareCase::new(BOUNDARY_ADDR + 0x0a00, 257, Pattern::AddressDerived),
    CompareCase::new(BOUNDARY_ADDR + 0x0c00, 512, Pattern::RepeatedAd),
];

#[cfg(feature = "rp2040-embassy")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareSmokeError<E> {
    Init(E),
    Bus(E),
    Mismatch(CompareMismatch),
    InvalidAddress,
}

#[cfg(feature = "rp2040-embassy")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareSmokeStep {
    BackendConfigured,
    KnownModeInitStart,
    QpiExitSent,
    QpiEnterSent,
    InitOk(DeviceId),
    InitError,
    CompareAttempted(CompareCase),
    WriteAttempted(CompareCase),
    WriteOk(CompareCase),
    WriteError {
        case: CompareCase,
        state: PsramState,
    },
    ReadAttempted(CompareCase),
    ReadOk(ReadPreview),
    ReadError {
        case: CompareCase,
        state: PsramState,
    },
    CompareOk(CompareCase),
    CompareMismatch(CompareMismatch),
    ReadProbeAttempted {
        addr: u32,
        len: usize,
    },
    ReadProbeOk(ReadPreview),
    ReadProbeError {
        addr: u32,
        len: usize,
        state: PsramState,
    },
}

#[cfg(feature = "rp2040-embassy")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompareCase {
    pub addr: u32,
    pub len: usize,
    pub pattern: Pattern,
}

#[cfg(feature = "rp2040-embassy")]
impl CompareCase {
    pub const fn new(addr: u32, len: usize, pattern: Pattern) -> Self {
        Self { addr, len, pattern }
    }
}

#[cfg(feature = "rp2040-embassy")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pattern {
    Repeated(u8),
    RepeatedAd,
    WalkingByte,
    AddressDerived,
}

#[cfg(feature = "rp2040-embassy")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadPreview {
    pub case: CompareCase,
    pub len: usize,
    pub bytes: [u8; PREVIEW_LEN],
}

#[cfg(feature = "rp2040-embassy")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompareMismatch {
    pub addr: PsramAddr,
    pub offset: u32,
    pub expected: u8,
    pub actual: u8,
    pub read_preview: ReadPreview,
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
pub type PicocalcCompareDriver<'d> = BlockingDriver<
    EmbassyRpQpiBackend<
        'd,
        embassy_rp::peripherals::PIO0,
        0,
        embassy_rp::peripherals::PIN_2,
        embassy_rp::peripherals::PIN_3,
        embassy_rp::peripherals::PIN_4,
        embassy_rp::peripherals::PIN_5,
        embassy_rp::peripherals::PIN_20,
        embassy_rp::peripherals::PIN_21,
    >,
>;

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
pub fn picocalc_compare_driver(
    pio0: Peri<'static, embassy_rp::peripherals::PIO0>,
    sio0: Peri<'static, embassy_rp::peripherals::PIN_2>,
    sio1: Peri<'static, embassy_rp::peripherals::PIN_3>,
    sio2: Peri<'static, embassy_rp::peripherals::PIN_4>,
    sio3: Peri<'static, embassy_rp::peripherals::PIN_5>,
    cs: Peri<'static, embassy_rp::peripherals::PIN_20>,
    sck: Peri<'static, embassy_rp::peripherals::PIN_21>,
    irq: impl embassy_rp::interrupt::typelevel::Binding<
        <embassy_rp::peripherals::PIO0 as embassy_rp::pio::Instance>::Interrupt,
        embassy_rp::pio::InterruptHandler<embassy_rp::peripherals::PIO0>,
    >,
) -> PicocalcCompareDriver<'static> {
    let pio = embassy_rp::pio::Pio::new(pio0, irq);
    let backend = EmbassyRpQpiBackend::new(pio.common, pio.sm0, sio0, sio1, sio2, sio3, cs, sck);

    BlockingDriver::with_config(backend, Pins::PICOCALC, TimingConfig::PICOCALC_SAFE)
}

#[cfg(feature = "rp2040-embassy")]
pub fn run_reported_compare_smoke<P>(
    pio: P,
    mut report: impl FnMut(CompareSmokeStep),
) -> Result<(DeviceId, P), CompareSmokeError<P::Error>>
where
    P: BlockingPio,
{
    run_reported_compare_smoke_with_config(
        pio,
        Pins::PICOCALC,
        TimingConfig::PICOCALC_SAFE,
        &mut report,
    )
}

#[cfg(feature = "rp2040-embassy")]
pub fn run_reported_compare_smoke_with_config<P>(
    pio: P,
    pins: Pins,
    timing: TimingConfig,
    mut report: impl FnMut(CompareSmokeStep),
) -> Result<(DeviceId, P), CompareSmokeError<P::Error>>
where
    P: BlockingPio,
{
    report(CompareSmokeStep::KnownModeInitStart);
    let mut driver = BlockingDriver::with_config(pio, pins, timing);

    let id = match driver.init() {
        Ok(id) => {
            report(CompareSmokeStep::InitOk(id));
            id
        }
        Err(error) => {
            report(CompareSmokeStep::InitError);
            return Err(CompareSmokeError::Init(error));
        }
    };

    run_compare_cases(&mut driver, &mut report)?;
    Ok((id, driver.into_inner()))
}

#[cfg(feature = "rp2040-embassy")]
fn run_compare_cases<P>(
    driver: &mut BlockingDriver<P>,
    report: &mut impl FnMut(CompareSmokeStep),
) -> Result<(), CompareSmokeError<P::Error>>
where
    P: BlockingPio,
{
    let mut expected = [0u8; MAX_COMPARE_LEN];
    let mut actual = [0u8; MAX_COMPARE_LEN];
    let mut first_mismatch = None;

    run_read_probe(driver, COMPARE_ADDR, 32, &mut actual, report)?;

    run_compare_case(
        driver,
        FIRST_COMPARE_CASE,
        &mut expected,
        &mut actual,
        report,
        &mut first_mismatch,
    )?;

    for case in REPEATED_PATTERN_CASES {
        run_compare_case(
            driver,
            case,
            &mut expected,
            &mut actual,
            report,
            &mut first_mismatch,
        )?;
    }

    for case in BOUNDARY_CASES {
        run_compare_case(
            driver,
            case,
            &mut expected,
            &mut actual,
            report,
            &mut first_mismatch,
        )?;
    }

    match first_mismatch {
        Some(mismatch) => Err(CompareSmokeError::Mismatch(mismatch)),
        None => Ok(()),
    }
}

#[cfg(feature = "rp2040-embassy")]
fn run_read_probe<P>(
    driver: &mut BlockingDriver<P>,
    addr: u32,
    len: usize,
    actual: &mut [u8; MAX_COMPARE_LEN],
    report: &mut impl FnMut(CompareSmokeStep),
) -> Result<(), CompareSmokeError<P::Error>>
where
    P: BlockingPio,
{
    let psram_addr = PsramAddr::new(addr).ok_or(CompareSmokeError::InvalidAddress)?;
    actual[..len].fill(0);

    report(CompareSmokeStep::ReadProbeAttempted { addr, len });
    if let Err(error) = driver.read_exact(psram_addr, &mut actual[..len]) {
        report(CompareSmokeStep::ReadProbeError {
            addr,
            len,
            state: driver.state(),
        });
        return Err(CompareSmokeError::Bus(error));
    }

    report(CompareSmokeStep::ReadProbeOk(preview(
        CompareCase::new(addr, len, Pattern::Repeated(0x00)),
        &actual[..len],
    )));
    Ok(())
}

#[cfg(feature = "rp2040-embassy")]
fn run_compare_case<P>(
    driver: &mut BlockingDriver<P>,
    case: CompareCase,
    expected: &mut [u8; MAX_COMPARE_LEN],
    actual: &mut [u8; MAX_COMPARE_LEN],
    report: &mut impl FnMut(CompareSmokeStep),
    first_mismatch: &mut Option<CompareMismatch>,
) -> Result<(), CompareSmokeError<P::Error>>
where
    P: BlockingPio,
{
    let addr = PsramAddr::new(case.addr).ok_or(CompareSmokeError::InvalidAddress)?;
    fill_pattern(case, addr, &mut expected[..case.len]);
    actual[..case.len].fill(0);

    report(CompareSmokeStep::CompareAttempted(case));
    report(CompareSmokeStep::WriteAttempted(case));
    if let Err(error) = driver.write_all(addr, &expected[..case.len]) {
        report(CompareSmokeStep::WriteError {
            case,
            state: driver.state(),
        });
        return Err(CompareSmokeError::Bus(error));
    }
    report(CompareSmokeStep::WriteOk(case));

    report(CompareSmokeStep::ReadAttempted(case));
    if let Err(error) = driver.read_exact(addr, &mut actual[..case.len]) {
        report(CompareSmokeStep::ReadError {
            case,
            state: driver.state(),
        });
        return Err(CompareSmokeError::Bus(error));
    }

    let preview = preview(case, &actual[..case.len]);
    report(CompareSmokeStep::ReadOk(preview));

    match compare(addr, &expected[..case.len], &actual[..case.len], preview) {
        Ok(()) => report(CompareSmokeStep::CompareOk(case)),
        Err(mismatch) => {
            report(CompareSmokeStep::CompareMismatch(mismatch));
            if first_mismatch.is_none() {
                *first_mismatch = Some(mismatch);
            }
        }
    }

    Ok(())
}

#[cfg(feature = "rp2040-embassy")]
fn fill_pattern(case: CompareCase, addr: PsramAddr, buf: &mut [u8]) {
    for (offset, byte) in buf.iter_mut().enumerate() {
        *byte = pattern_byte(case.pattern, addr, offset);
    }
}

#[cfg(feature = "rp2040-embassy")]
fn pattern_byte(pattern: Pattern, addr: PsramAddr, offset: usize) -> u8 {
    match pattern {
        Pattern::Repeated(value) => value,
        Pattern::RepeatedAd => 0xad,
        Pattern::WalkingByte => offset as u8,
        Pattern::AddressDerived => addr.get().wrapping_add(offset as u32) as u8,
    }
}

#[cfg(feature = "rp2040-embassy")]
fn compare(
    base_addr: PsramAddr,
    expected: &[u8],
    actual: &[u8],
    read_preview: ReadPreview,
) -> Result<(), CompareMismatch> {
    for (offset, (&expected, &actual)) in expected.iter().zip(actual.iter()).enumerate() {
        if expected != actual {
            let offset = offset as u32;
            return Err(CompareMismatch {
                addr: base_addr.checked_add(offset).unwrap_or(base_addr),
                offset,
                expected,
                actual,
                read_preview,
            });
        }
    }

    Ok(())
}

#[cfg(feature = "rp2040-embassy")]
fn preview(case: CompareCase, bytes: &[u8]) -> ReadPreview {
    let mut preview = [0u8; PREVIEW_LEN];
    let len = bytes.len().min(PREVIEW_LEN);
    preview[..len].copy_from_slice(&bytes[..len]);
    ReadPreview {
        case,
        len,
        bytes: preview,
    }
}

#[cfg(not(all(feature = "rp2040-embassy", target_os = "none")))]
fn main() {
    #[cfg(feature = "rp2040-embassy")]
    {
        let _ = COMPARE_ADDR;
    }
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
#[cortex_m_rt::entry]
fn embedded_main() -> ! {
    let peripherals = embassy_rp::init(Default::default());
    let mut uart = picocalc_uart_usb_tx(peripherals.UART0, peripherals.PIN_0);

    log_line(&mut uart, "boot compare");
    if UART_STAGE0_ONLY {
        loop {
            delay();
            log_line(&mut uart, "boot compare");
        }
    }

    repeat_boot_log(&mut uart);
    log_line(&mut uart, "boot compare ok");
    register_panic_uart(&mut uart);

    log_line(&mut uart, "backend configure start");
    let mut driver = picocalc_compare_driver(
        peripherals.PIO0,
        peripherals.PIN_2,
        peripherals.PIN_3,
        peripherals.PIN_4,
        peripherals.PIN_5,
        peripherals.PIN_20,
        peripherals.PIN_21,
        Irqs,
    );
    log_line(&mut uart, "backend configure ok");

    log_line(&mut uart, "driver.init start");
    match driver.init() {
        Ok(id) => {
            log_str(&mut uart, "driver.init ok id=");
            log_hex_u8(&mut uart, id.raw[0]);
            log_byte(&mut uart, b' ');
            log_hex_u8(&mut uart, id.raw[1]);
            log_byte(&mut uart, b' ');
            log_hex_u8(&mut uart, id.raw[2]);
            log_newline(&mut uart);
        }
        Err(_error) => {
            log_line(&mut uart, "driver.init error");
            loop {}
        }
    }

    log_line(&mut uart, "compare start");
    match run_compare_cases(&mut driver, &mut |step| log_step(&mut uart, step)) {
        Ok(()) => log_line(&mut uart, "compare smoke ok"),
        Err(CompareSmokeError::Mismatch(_mismatch)) => {
            log_line(&mut uart, "compare smoke mismatch")
        }
        Err(CompareSmokeError::Bus(_error)) => log_line(&mut uart, "compare smoke bus error"),
        Err(CompareSmokeError::Init(_error)) => log_line(&mut uart, "compare smoke init error"),
        Err(CompareSmokeError::InvalidAddress) => {
            log_line(&mut uart, "compare smoke invalid address")
        }
    }
    loop {}
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn repeat_boot_log(uart: &mut UartTx<'static, Blocking>) {
    for _ in 0..PRE_BACKEND_BOOT_REPEATS {
        delay();
        log_line(uart, "boot compare");
    }
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_step(uart: &mut UartTx<'static, Blocking>, step: CompareSmokeStep) {
    match step {
        CompareSmokeStep::CompareAttempted(case) => {
            log_case(uart, "compare case start", case);
        }
        CompareSmokeStep::WriteOk(case) => {
            log_case(uart, "write ok", case);
        }
        CompareSmokeStep::WriteError { case, .. } => {
            log_case(uart, "write error", case);
        }
        CompareSmokeStep::ReadOk(preview) => {
            log_case(uart, "read ok", preview.case);
            log_read_preview(uart, "read preview", preview);
        }
        CompareSmokeStep::ReadError { case, .. } => {
            log_case(uart, "read error", case);
        }
        CompareSmokeStep::CompareOk(case) => {
            log_case(uart, "compare ok", case);
        }
        CompareSmokeStep::CompareMismatch(mismatch) => {
            log_mismatch(uart, "compare fail", mismatch);
        }
        CompareSmokeStep::ReadProbeAttempted { addr, len } => {
            log_addr_len(uart, "read probe start", addr, len);
            log_newline(uart);
        }
        CompareSmokeStep::ReadProbeOk(preview) => {
            log_addr_len(uart, "read probe ok", preview.case.addr, preview.case.len);
            log_newline(uart);
            log_read_preview(uart, "read probe preview", preview);
        }
        CompareSmokeStep::ReadProbeError { addr, len, .. } => {
            log_addr_len(uart, "read probe error", addr, len);
            log_newline(uart);
        }
        _ => {}
    }
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_pattern(uart: &mut UartTx<'static, Blocking>, pattern: Pattern) {
    match pattern {
        Pattern::Repeated(value) => {
            log_str(uart, "repeated 0x");
            log_hex_u8(uart, value);
        }
        Pattern::RepeatedAd => log_str(uart, "repeated 0xad"),
        Pattern::WalkingByte => log_str(uart, "walking-byte"),
        Pattern::AddressDerived => log_str(uart, "address-derived"),
    }
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn picocalc_uart_usb_tx(
    uart0: Peri<'static, embassy_rp::peripherals::UART0>,
    tx: Peri<'static, embassy_rp::peripherals::PIN_0>,
) -> UartTx<'static, Blocking> {
    let mut config = UartConfig::default();
    config.baudrate = PICOCALC_UART_USB_BAUD;

    // PicoCalc UART-USB bridge: RP2040 UART0 TX on GP0. RX/GP1 is unused for logs.
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
fn log_case(uart: &mut UartTx<'static, Blocking>, prefix: &str, case: CompareCase) {
    log_addr_len(uart, prefix, case.addr, case.len);
    log_str(uart, " pattern=");
    log_pattern(uart, case.pattern);
    log_newline(uart);
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_mismatch(uart: &mut UartTx<'static, Blocking>, prefix: &str, mismatch: CompareMismatch) {
    log_str(uart, prefix);
    log_str(uart, " addr=0x");
    log_hex_u32(uart, mismatch.addr.get());
    log_str(uart, " offset=");
    log_dec_u32(uart, mismatch.offset);
    log_str(uart, " expected=0x");
    log_hex_u8(uart, mismatch.expected);
    log_str(uart, " actual=0x");
    log_hex_u8(uart, mismatch.actual);
    log_str(uart, " read=");
    log_preview_bytes(uart, mismatch.read_preview);
    log_newline(uart);
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_addr_len(uart: &mut UartTx<'static, Blocking>, prefix: &str, addr: u32, len: usize) {
    log_str(uart, prefix);
    log_str(uart, " addr=0x");
    log_hex_u32(uart, addr);
    log_str(uart, " len=");
    log_dec_usize(uart, len);
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_read_preview(uart: &mut UartTx<'static, Blocking>, prefix: &str, preview: ReadPreview) {
    log_str(uart, prefix);
    log_str(uart, " addr=0x");
    log_hex_u32(uart, preview.case.addr);
    log_str(uart, " len=");
    log_dec_usize(uart, preview.len);
    log_str(uart, " bytes=");
    log_preview_bytes(uart, preview);
    log_newline(uart);
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_preview_bytes(uart: &mut UartTx<'static, Blocking>, preview: ReadPreview) {
    for index in 0..preview.len {
        if index > 0 {
            log_byte(uart, b' ');
        }
        log_hex_u8(uart, preview.bytes[index]);
    }
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
fn delay() {
    for _ in 0..1_000_000 {
        core::hint::spin_loop();
    }
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_hex_u32(uart: &mut UartTx<'static, Blocking>, value: u32) {
    for shift in (0..=28).rev().step_by(4) {
        log_nibble(uart, ((value >> shift) & 0x0f) as u8);
    }
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
    log_dec_u32(uart, value as u32);
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn log_dec_u32(uart: &mut UartTx<'static, Blocking>, mut value: u32) {
    let mut buf = [0u8; 10];
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
