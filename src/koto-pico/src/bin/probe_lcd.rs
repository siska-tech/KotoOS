//! `probe_lcd` — ILI9488 SPI LCD bring-up probe (KOTO-0066).
//!
//! Drives a fixed sequence of full-screen fills, corner markers, a centered
//! rectangle, and a DMA scanline band so panel orientation, RGB color order,
//! partial-window addressing, and DMA transfer can be confirmed by eye. Output
//! is emitted over UART0 immediately and mirrored over USB CDC when connected.
//!
//! Not part of normal development: flash manually only to re-validate the LCD on
//! new hardware. See `docs/hardware/PICO_HARDWARE_LOG.md`.
#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_futures::join::join;
use embassy_rp::{
    bind_interrupts, dma,
    gpio::{Level, Output},
    peripherals,
    spi::{Config as SpiConfig, Spi},
    uart::{Config as UartConfig, UartTx},
    usb::{Driver, InterruptHandler as UsbInterruptHandler},
};
use embassy_time::Timer;
use embassy_usb::{
    class::cdc_acm::{CdcAcmClass, State},
    Builder, Config,
};
use koto_pico::lcd::{PicoCalcLcd, Rgb888, ILI9488_SPI};
use panic_halt as _;

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => UsbInterruptHandler<peripherals::USB>;
    DMA_IRQ_0 => dma::InterruptHandler<peripherals::DMA_CH0>;
});

const BANNER: &[u8] = concat!(
    "KotoOS KOTO-0066 lcd-fill v",
    env!("CARGO_PKG_VERSION"),
    " board=",
    env!("KOTO_BOARD_ID"),
    " mcu=",
    env!("KOTO_MCU_ID"),
    "\r\n"
)
.as_bytes();

#[cfg(feature = "board-picocalc-pico")]
const TRANSPORT_PROFILE: &[u8] =
    b"spi_requested_hz=62500000 dma=DMA_CH0 dreq=SPI1_TX profile=rp2040-validated\r\n";
#[cfg(feature = "board-picocalc-pico2w")]
const TRANSPORT_PROFILE: &[u8] =
    b"spi_requested_hz=37500000 dma=DMA_CH0 dreq=SPI1_TX profile=rp2350a-conservative\r\n";

#[embassy_executor::main(
    executor = "embassy_rp::executor::Executor",
    entry = "cortex_m_rt::entry"
)]
async fn main(_spawner: Spawner) {
    let peripherals = koto_pico::board::split_peripherals(embassy_rp::init(Default::default()));

    let mut spi_config = SpiConfig::default();
    spi_config.frequency = ILI9488_SPI.spi_hz;
    let spi = Spi::new_txonly(
        peripherals.lcd_spi,
        peripherals.lcd_sck,
        peripherals.lcd_mosi,
        peripherals.dma_ch0,
        Irqs,
        spi_config,
    );
    let cs = Output::new(peripherals.lcd_cs, Level::High);
    let dc = Output::new(peripherals.lcd_dc, Level::High);
    let reset = Output::new(peripherals.lcd_reset, Level::High);
    let mut lcd = PicoCalcLcd::new(spi, cs, dc, reset, &ILI9488_SPI);

    let mut uart_config = UartConfig::default();
    uart_config.baudrate = 115_200;
    let mut uart = UartTx::new_blocking(peripherals.uart, peripherals.uart_tx, uart_config);

    let driver = Driver::new(peripherals.usb, Irqs);
    let mut usb_config = Config::new(0xc0de, 0x0066);
    usb_config.manufacturer = Some("KotoOS");
    usb_config.product = Some("KotoOS LCD fill probe");
    usb_config.serial_number = Some("KOTO-0066");
    usb_config.max_power = 100;

    let mut config_descriptor = [0; 256];
    let mut bos_descriptor = [0; 256];
    let mut msos_descriptor = [0; 128];
    let mut control_buf = [0; 64];
    let mut cdc_state = State::new();
    let mut builder = Builder::new(
        driver,
        usb_config,
        &mut config_descriptor,
        &mut bos_descriptor,
        &mut msos_descriptor,
        &mut control_buf,
    );
    let mut cdc = CdcAcmClass::new(&mut builder, &mut cdc_state, 64);
    let mut usb = builder.build();

    let usb_task = usb.run();
    let probe_task = async {
        // The PicoCalc mainboard Type-C connector exposes UART0, not the Pico
        // module's native USB. LCD bring-up must therefore run before and
        // independently of any USB CDC host connection.
        let _ = uart.blocking_write(BANNER);
        let _ = uart.blocking_write(b"log=uart0 tx=GP0 baud=115200 format=8N1\r\n");
        let _ = uart.blocking_write(b"profile=ili9488-spi detection=manual\r\n");
        let _ = uart.blocking_write(TRANSPORT_PROFILE);
        let _ = uart.blocking_write(b"colmod=0x66 RGB666 madctl=0x48\r\n");
        let _ = uart.blocking_write(b"lcd init starting\r\n");

        let mut result = "pass";
        if lcd.init().await.is_err() {
            result = "init-fail";
            let _ = uart.blocking_write(b"lcd init failed\r\n");
        } else {
            let _ = uart.blocking_write(b"lcd init complete\r\n");
            let colors = [
                (Rgb888::RED, b"fill=red\r\n".as_slice()),
                (Rgb888::GREEN, b"fill=green\r\n".as_slice()),
                (Rgb888::BLUE, b"fill=blue\r\n".as_slice()),
                (Rgb888::BLACK, b"fill=black\r\n".as_slice()),
            ];
            for (color, message) in colors {
                if lcd.fill(color).await.is_err() {
                    result = "draw-fail";
                    let _ = uart.blocking_write(b"full fill failed\r\n");
                    continue;
                }
                let _ = uart.blocking_write(message);
                Timer::after_millis(700).await;
            }

            let _ = lcd.fill_rect(0, 0, 32, 32, Rgb888::RED).await;
            let _ = lcd.fill_rect(288, 0, 32, 32, Rgb888::GREEN).await;
            let _ = lcd.fill_rect(0, 288, 32, 32, Rgb888::BLUE).await;
            let _ = lcd.fill_rect(288, 288, 32, 32, Rgb888::WHITE).await;
            let _ = lcd.fill_rect(120, 136, 80, 48, Rgb888::YELLOW).await;
            let _ = uart.blocking_write(b"partial rectangles complete\r\n");

            let _ = lcd.fill_rect(0, 200, 320, 8, Rgb888::CYAN).await;
            let _ = uart.blocking_write(b"dma scanline band y=200 height=8 complete\r\n");
            let _ = uart.blocking_write(b"inspect orientation/color/region boundaries\r\n");
        }

        // Native USB is optional. If connected later, replay a compact result
        // without gating the already-completed LCD sequence.
        loop {
            cdc.wait_connection().await;
            let _ = cdc.write_packet(BANNER).await;
            let _ = log(&mut cdc, b"profile=ili9488-spi detection=manual\r\n").await;
            let _ = log(&mut cdc, TRANSPORT_PROFILE).await;
            let summary = if result == "pass" {
                b"lcd sequence complete result=pass\r\n".as_slice()
            } else if result == "init-fail" {
                b"lcd sequence complete result=init-fail\r\n".as_slice()
            } else {
                b"lcd sequence complete result=draw-fail\r\n".as_slice()
            };
            let _ = log(&mut cdc, summary).await;
            loop {
                Timer::after_secs(5).await;
                let _ = uart.blocking_write(b"KOTO-0066 awaiting observation\r\n");
                if log(&mut cdc, b"KOTO-0066 awaiting observation\r\n")
                    .await
                    .is_err()
                {
                    break;
                }
            }
        }
    };

    join(usb_task, probe_task).await;
}

async fn log<'a>(
    cdc: &mut CdcAcmClass<'a, Driver<'a, peripherals::USB>>,
    message: &[u8],
) -> Result<(), ()> {
    cdc.write_packet(message).await.map_err(|_| ())
}
