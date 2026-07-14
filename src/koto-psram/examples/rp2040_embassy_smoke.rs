#![cfg_attr(all(feature = "rp2040-embassy", target_os = "none"), no_std)]
#![cfg_attr(all(feature = "rp2040-embassy", target_os = "none"), no_main)]

#[cfg(feature = "rp2040-embassy")]
use koto_psram::{
    addr::PsramAddr,
    bus::PsramBus,
    config::{Pins, TimingConfig},
    device::DeviceId,
    error::Mismatch,
    pio::blocking::{BlockingDriver, BlockingPio},
    protocol::QpiTransaction,
};

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
use embassy_rp::uart::{Blocking, Config as UartConfig, UartTx};

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
const PICOCALC_UART_USB_BAUD: u32 = 115_200;

#[cfg(feature = "rp2040-embassy")]
const SMOKE_ADDR: u32 = 0x0000_0020;
#[cfg(feature = "rp2040-embassy")]
const BOUNDARY_ADDR: u32 = 0x0000_0100;
#[cfg(feature = "rp2040-embassy")]
const BOUNDARY_LENGTHS: [usize; 7] = [1, 31, 32, 255, 256, 257, 512];

#[cfg(feature = "rp2040-embassy")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmokeError<E> {
    Init(E),
    Bus(E),
    Mismatch(Mismatch),
    InvalidAddress,
}

#[cfg(feature = "rp2040-embassy")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitSmokeStep {
    BackendConfigured,
    KnownModeInitStart,
    QpiExitSent,
    QpiEnterSent,
    InitOk(DeviceId),
    InitError,
    QpiWrite32BAttempted,
    QpiWriteOk,
    QpiWriteError,
    QpiRead32BAttempted,
    QpiReadOk([u8; 16]),
    QpiReadError,
}

#[cfg(feature = "rp2040-embassy")]
pub fn run_reported_init_smoke<P>(
    pio: P,
    mut report: impl FnMut(InitSmokeStep),
) -> Result<(DeviceId, P), SmokeError<P::Error>>
where
    P: BlockingPio,
{
    run_reported_init_smoke_with_config(pio, Pins::PICOCALC, TimingConfig::DEFAULT, &mut report)
}

#[cfg(feature = "rp2040-embassy")]
pub fn run_reported_init_smoke_with_config<P>(
    pio: P,
    pins: Pins,
    timing: TimingConfig,
    mut report: impl FnMut(InitSmokeStep),
) -> Result<(DeviceId, P), SmokeError<P::Error>>
where
    P: BlockingPio,
{
    report(InitSmokeStep::KnownModeInitStart);
    let reporting_pio = ReportingPio { inner: pio, report };
    let mut driver = BlockingDriver::with_config(reporting_pio, pins, timing);

    match driver.init() {
        Ok(id) => {
            let mut reporting_pio = driver.into_inner();
            reporting_pio.report(InitSmokeStep::InitOk(id));
            Ok((id, reporting_pio.inner))
        }
        Err(error) => {
            let mut reporting_pio = driver.into_inner();
            reporting_pio.report(InitSmokeStep::InitError);
            Err(SmokeError::Init(error))
        }
    }
}

#[cfg(feature = "rp2040-embassy")]
pub fn run_reported_write_only_smoke<P>(
    pio: P,
    mut report: impl FnMut(InitSmokeStep),
) -> Result<(DeviceId, P), SmokeError<P::Error>>
where
    P: BlockingPio,
{
    run_reported_write_only_smoke_with_config(
        pio,
        Pins::PICOCALC,
        TimingConfig::DEFAULT,
        &mut report,
    )
}

#[cfg(feature = "rp2040-embassy")]
pub fn run_reported_write_only_smoke_with_config<P>(
    pio: P,
    pins: Pins,
    timing: TimingConfig,
    mut report: impl FnMut(InitSmokeStep),
) -> Result<(DeviceId, P), SmokeError<P::Error>>
where
    P: BlockingPio,
{
    report(InitSmokeStep::KnownModeInitStart);
    let reporting_pio = ReportingPio { inner: pio, report };
    let mut driver = BlockingDriver::with_config(reporting_pio, pins, timing);

    let id = match driver.init() {
        Ok(id) => id,
        Err(error) => {
            let mut reporting_pio = driver.into_inner();
            reporting_pio.report(InitSmokeStep::InitError);
            return Err(SmokeError::Init(error));
        }
    };

    let mut reporting_pio = driver.into_inner();
    reporting_pio.report(InitSmokeStep::InitOk(id));

    let addr = PsramAddr::new(SMOKE_ADDR).ok_or(SmokeError::InvalidAddress)?;
    let tx = QpiTransaction::write(addr, 32, timing);
    let mut data = [0u8; 32];
    for (index, byte) in data.iter_mut().enumerate() {
        *byte = (index as u8).wrapping_mul(17).wrapping_add(0xa5);
    }

    reporting_pio.report(InitSmokeStep::QpiWrite32BAttempted);
    match reporting_pio.write_qpi_chunk(tx, &data) {
        Ok(()) => {
            reporting_pio.report(InitSmokeStep::QpiWriteOk);
            Ok((id, reporting_pio.inner))
        }
        Err(error) => {
            reporting_pio.report(InitSmokeStep::QpiWriteError);
            Err(SmokeError::Bus(error))
        }
    }
}

#[cfg(feature = "rp2040-embassy")]
pub fn run_reported_read_only_smoke<P>(
    pio: P,
    mut report: impl FnMut(InitSmokeStep),
) -> Result<(DeviceId, P), SmokeError<P::Error>>
where
    P: BlockingPio,
{
    run_reported_read_only_smoke_with_config(
        pio,
        Pins::PICOCALC,
        TimingConfig::DEFAULT,
        &mut report,
    )
}

#[cfg(feature = "rp2040-embassy")]
pub fn run_reported_read_only_smoke_with_config<P>(
    pio: P,
    pins: Pins,
    timing: TimingConfig,
    mut report: impl FnMut(InitSmokeStep),
) -> Result<(DeviceId, P), SmokeError<P::Error>>
where
    P: BlockingPio,
{
    report(InitSmokeStep::KnownModeInitStart);
    let reporting_pio = ReportingPio { inner: pio, report };
    let mut driver = BlockingDriver::with_config(reporting_pio, pins, timing);

    let id = match driver.init() {
        Ok(id) => id,
        Err(error) => {
            let mut reporting_pio = driver.into_inner();
            reporting_pio.report(InitSmokeStep::InitError);
            return Err(SmokeError::Init(error));
        }
    };

    let mut reporting_pio = driver.into_inner();
    reporting_pio.report(InitSmokeStep::InitOk(id));

    let addr = PsramAddr::new(SMOKE_ADDR).ok_or(SmokeError::InvalidAddress)?;
    let tx = QpiTransaction::read(addr, 32, timing);
    let mut data = [0u8; 32];

    reporting_pio.report(InitSmokeStep::QpiRead32BAttempted);
    match reporting_pio.read_qpi_chunk(tx, &mut data) {
        Ok(()) => {
            let mut prefix = [0u8; 16];
            prefix.copy_from_slice(&data[..16]);
            reporting_pio.report(InitSmokeStep::QpiReadOk(prefix));
            Ok((id, reporting_pio.inner))
        }
        Err(error) => {
            reporting_pio.report(InitSmokeStep::QpiReadError);
            Err(SmokeError::Bus(error))
        }
    }
}

#[cfg(feature = "rp2040-embassy")]
struct ReportingPio<P, R> {
    inner: P,
    report: R,
}

#[cfg(feature = "rp2040-embassy")]
impl<P, R> ReportingPio<P, R>
where
    R: FnMut(InitSmokeStep),
{
    fn report(&mut self, step: InitSmokeStep) {
        (self.report)(step);
    }
}

#[cfg(feature = "rp2040-embassy")]
impl<P, R> BlockingPio for ReportingPio<P, R>
where
    P: BlockingPio,
    R: FnMut(InitSmokeStep),
{
    type Error = P::Error;

    fn configure(&mut self, pins: Pins, timing: TimingConfig) -> Result<(), Self::Error> {
        self.inner.configure(pins, timing)?;
        self.report(InitSmokeStep::BackendConfigured);
        Ok(())
    }

    fn exit_qpi_quad(&mut self) -> Result<(), Self::Error> {
        self.inner.exit_qpi_quad()
    }

    fn exit_qpi_spi(&mut self) -> Result<(), Self::Error> {
        self.inner.exit_qpi_spi()?;
        self.report(InitSmokeStep::QpiExitSent);
        Ok(())
    }

    fn read_id_spi(&mut self) -> Result<DeviceId, Self::Error> {
        self.inner.read_id_spi()
    }

    fn enter_qpi_spi(&mut self) -> Result<(), Self::Error> {
        self.inner.enter_qpi_spi()?;
        self.report(InitSmokeStep::QpiEnterSent);
        Ok(())
    }

    fn read_qpi_chunk(
        &mut self,
        transaction: koto_psram::protocol::QpiTransaction,
        buf: &mut [u8],
    ) -> Result<(), Self::Error> {
        self.inner.read_qpi_chunk(transaction, buf)
    }

    fn write_qpi_chunk(
        &mut self,
        transaction: koto_psram::protocol::QpiTransaction,
        data: &[u8],
    ) -> Result<(), Self::Error> {
        self.inner.write_qpi_chunk(transaction, data)
    }
}

#[cfg(feature = "rp2040-embassy")]
pub fn run_smoke<P>(driver: &mut BlockingDriver<P>) -> Result<(), SmokeError<P::Error>>
where
    P: BlockingPio,
{
    driver.init().map_err(SmokeError::Init)?;

    let mut expected = [0u8; 512];
    for (index, byte) in expected.iter_mut().enumerate() {
        *byte = (index as u8).wrapping_mul(17).wrapping_add(0xa5);
    }

    let addr = PsramAddr::new(SMOKE_ADDR).ok_or(SmokeError::InvalidAddress)?;
    let mut readback = [0u8; 512];

    driver
        .write_all(addr, &expected[..32])
        .map_err(SmokeError::Bus)?;
    driver
        .read_exact(addr, &mut readback[..32])
        .map_err(SmokeError::Bus)?;
    compare(addr, &expected[..32], &readback[..32])?;

    for len in BOUNDARY_LENGTHS {
        let addr = PsramAddr::new(BOUNDARY_ADDR).ok_or(SmokeError::InvalidAddress)?;
        driver
            .write_all(addr, &expected[..len])
            .map_err(SmokeError::Bus)?;
        driver
            .read_exact(addr, &mut readback[..len])
            .map_err(SmokeError::Bus)?;
        compare(addr, &expected[..len], &readback[..len])?;
    }

    Ok(())
}

#[cfg(feature = "rp2040-embassy")]
fn compare<E>(addr: PsramAddr, expected: &[u8], actual: &[u8]) -> Result<(), SmokeError<E>> {
    for (offset, (&expected, &actual)) in expected.iter().zip(actual.iter()).enumerate() {
        if expected != actual {
            let offset = u32::try_from(offset).map_err(|_| SmokeError::InvalidAddress)?;
            let addr = addr.checked_add(offset).ok_or(SmokeError::InvalidAddress)?;
            return Err(SmokeError::Mismatch(Mismatch {
                addr,
                expected,
                actual,
            }));
        }
    }

    Ok(())
}

#[cfg(not(all(feature = "rp2040-embassy", target_os = "none")))]
fn main() {
    #[cfg(feature = "rp2040-embassy")]
    {
        let _ = SMOKE_ADDR;
    }
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
#[cortex_m_rt::entry]
fn embedded_main() -> ! {
    let peripherals = embassy_rp::init(Default::default());
    let mut uart = picocalc_uart_usb_tx(peripherals.UART0, peripherals.PIN_0);
    register_panic_uart(&mut uart);
    log_line(&mut uart, "boot via PicoCalc UART-USB");
    log_line(&mut uart, "embassy_rp::init start/ok");
    loop {}
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn picocalc_uart_usb_tx(
    uart0: embassy_rp::Peri<'static, embassy_rp::peripherals::UART0>,
    tx: embassy_rp::Peri<'static, embassy_rp::peripherals::PIN_0>,
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
                log_str(uart, ":");
                log_dec_u32(uart, location.line());
            }
            log_newline(uart);
        }
    }
    loop {}
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
fn log_dec_u32(uart: &mut UartTx<'static, Blocking>, mut value: u32) {
    let mut buf = [0u8; 10];
    let mut len = 0;

    if value == 0 {
        let _ = uart.blocking_write(b"0");
        return;
    }

    while value > 0 {
        buf[len] = b'0' + (value % 10) as u8;
        value /= 10;
        len += 1;
    }

    while len > 0 {
        len -= 1;
        let _ = uart.blocking_write(&[buf[len]]);
    }
}
