//! `probe_lcd` — ILI9488 SPI LCD bring-up probe (KOTO-0066).
//!
//! Drives a fixed sequence of full-screen fills, corner markers, a centered
//! rectangle, and a DMA scanline band so panel orientation, RGB color order,
//! partial-window addressing, and DMA transfer can be confirmed by eye. Output
//! is mirrored over USB CDC.
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
    "\r\n"
)
.as_bytes();

#[embassy_executor::main(
    executor = "embassy_rp::executor::Executor",
    entry = "cortex_m_rt::entry"
)]
async fn main(_spawner: Spawner) {
    let peripherals = embassy_rp::init(Default::default());

    let mut spi_config = SpiConfig::default();
    spi_config.frequency = ILI9488_SPI.spi_hz;
    let spi = Spi::new_txonly(
        peripherals.SPI1,
        peripherals.PIN_10,
        peripherals.PIN_11,
        peripherals.DMA_CH0,
        Irqs,
        spi_config,
    );
    let cs = Output::new(peripherals.PIN_13, Level::High);
    let dc = Output::new(peripherals.PIN_14, Level::High);
    let reset = Output::new(peripherals.PIN_15, Level::High);
    let mut lcd = PicoCalcLcd::new(spi, cs, dc, reset, &ILI9488_SPI);

    let driver = Driver::new(peripherals.USB, Irqs);
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
        loop {
            cdc.wait_connection().await;
            if cdc.write_packet(BANNER).await.is_err() {
                continue;
            }
            if log(&mut cdc, b"profile=ili9488-spi detection=manual\r\n")
                .await
                .is_err()
            {
                continue;
            }
            let _ = log(
                &mut cdc,
                b"spi=20000000Hz colmod=0x66 RGB666 madctl=0x48\r\n",
            )
            .await;
            let _ = log(&mut cdc, b"lcd init starting\r\n").await;

            if lcd.init().await.is_err() {
                let _ = log(&mut cdc, b"lcd init failed\r\n").await;
                continue;
            }
            let _ = log(&mut cdc, b"lcd init complete\r\n").await;

            let colors = [
                (Rgb888::RED, b"fill=red\r\n".as_slice()),
                (Rgb888::GREEN, b"fill=green\r\n".as_slice()),
                (Rgb888::BLUE, b"fill=blue\r\n".as_slice()),
                (Rgb888::BLACK, b"fill=black\r\n".as_slice()),
            ];
            for (color, message) in colors {
                if lcd.fill(color).await.is_err() {
                    let _ = log(&mut cdc, b"full fill failed\r\n").await;
                    continue;
                }
                let _ = log(&mut cdc, message).await;
                Timer::after_millis(700).await;
            }

            let _ = lcd.fill_rect(0, 0, 32, 32, Rgb888::RED).await;
            let _ = lcd.fill_rect(288, 0, 32, 32, Rgb888::GREEN).await;
            let _ = lcd.fill_rect(0, 288, 32, 32, Rgb888::BLUE).await;
            let _ = lcd.fill_rect(288, 288, 32, 32, Rgb888::WHITE).await;
            let _ = lcd.fill_rect(120, 136, 80, 48, Rgb888::YELLOW).await;
            let _ = log(&mut cdc, b"partial rectangles complete\r\n").await;

            let _ = lcd.fill_rect(0, 200, 320, 8, Rgb888::CYAN).await;
            let _ = log(&mut cdc, b"dma scanline band y=200 height=8 complete\r\n").await;
            let _ = log(&mut cdc, b"inspect orientation/color/region boundaries\r\n").await;

            loop {
                Timer::after_secs(5).await;
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
