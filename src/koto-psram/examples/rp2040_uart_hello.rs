#![cfg_attr(all(feature = "rp2040-embassy", target_os = "none"), no_std)]
#![cfg_attr(all(feature = "rp2040-embassy", target_os = "none"), no_main)]

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
use embassy_rp::uart::{Blocking, Config as UartConfig, UartTx};

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
const PICOCALC_UART_USB_BAUD: u32 = 115_200;
#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
const HELLO: &[u8] = b"hello from koto-psram uart hello\r\n";

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
#[cortex_m_rt::entry]
fn embedded_main() -> ! {
    let peripherals = embassy_rp::init(Default::default());
    let mut uart = picocalc_uart_usb_tx(peripherals.UART0, peripherals.PIN_0);

    loop {
        let _ = uart.blocking_write(HELLO);
        delay();
    }
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn picocalc_uart_usb_tx(
    uart0: embassy_rp::Peri<'static, embassy_rp::peripherals::UART0>,
    tx: embassy_rp::Peri<'static, embassy_rp::peripherals::PIN_0>,
) -> UartTx<'static, Blocking> {
    let mut config = UartConfig::default();
    config.baudrate = PICOCALC_UART_USB_BAUD;

    UartTx::new_blocking(uart0, tx, config)
}

#[cfg(not(all(feature = "rp2040-embassy", target_os = "none")))]
fn main() {}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
fn delay() {
    for _ in 0..1_000_000 {
        core::hint::spin_loop();
    }
}

#[cfg(all(feature = "rp2040-embassy", target_os = "none"))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    loop {}
}
