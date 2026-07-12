#![no_std]
#![no_main]

use core::fmt::Write;

use embassy_executor::Spawner;
use embassy_futures::join::join;
use embassy_rp::{
    bind_interrupts, dma,
    gpio::{Input, Level, Output, Pull},
    i2c::{Config as I2cConfig, I2c, InterruptHandler as I2cInterruptHandler},
    peripherals,
    spi::{Config as SpiConfig, Spi},
    usb::{Driver, InterruptHandler as UsbInterruptHandler},
};
use embassy_time::Timer;
use embassy_usb::{
    class::cdc_acm::{CdcAcmClass, State},
    Builder, Config,
};
use koto_pico::{
    dashboard::{
        normalize_hid_codes, LineBuffer, ProbeStatus, KEYBOARD_RAW_CAPACITY, RP2040_SRAM_BYTES,
    },
    lcd::{PicoCalcLcd, Rgb888, ILI9488_SPI},
    pins::KeyboardPins,
};
use panic_halt as _;

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => UsbInterruptHandler<peripherals::USB>;
    DMA_IRQ_0 => dma::InterruptHandler<peripherals::DMA_CH0>;
    I2C1_IRQ => I2cInterruptHandler<peripherals::I2C1>;
});

const BANNER: &[u8] = concat!(
    "KotoOS device probe dashboard v",
    env!("CARGO_PKG_VERSION"),
    "\r\n"
)
.as_bytes();

#[embassy_executor::main(
    executor = "embassy_rp::executor::Executor",
    entry = "cortex_m_rt::entry"
)]
async fn main(_spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    let mut spi_config = SpiConfig::default();
    spi_config.frequency = ILI9488_SPI.spi_hz;
    let spi = Spi::new_txonly(p.SPI1, p.PIN_10, p.PIN_11, p.DMA_CH0, Irqs, spi_config);
    let mut lcd = PicoCalcLcd::new(
        spi,
        Output::new(p.PIN_13, Level::High),
        Output::new(p.PIN_14, Level::High),
        Output::new(p.PIN_15, Level::High),
        &ILI9488_SPI,
    );
    let lcd_status = if lcd.init().await.is_ok() {
        ProbeStatus::Pass
    } else {
        ProbeStatus::Error
    };

    let mut i2c_config = I2cConfig::default();
    i2c_config.frequency = 100_000;
    let mut keyboard = I2c::new_async(p.I2C1, p.PIN_7, p.PIN_6, Irqs, i2c_config);
    let sd_detect = Input::new(p.PIN_22, Pull::Up);

    let driver = Driver::new(p.USB, Irqs);
    let mut usb_config = Config::new(0xc0de, 0x0108);
    usb_config.manufacturer = Some("KotoOS");
    usb_config.product = Some("KotoOS device probe dashboard");
    usb_config.serial_number = Some("DEVICE-PROBE");
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
        let mut raw = [0u8; KEYBOARD_RAW_CAPACITY];
        let mut line = LineBuffer::new();
        let mut sequence = 0u32;

        if lcd_status == ProbeStatus::Pass {
            paint_static_dashboard(&mut lcd, lcd_status, sd_status(&sd_detect)).await;
        }

        loop {
            cdc.wait_connection().await;
            if write_packets(&mut cdc, BANNER).await.is_err() {
                continue;
            }
            write_summary(&mut cdc, &mut line, lcd_status, sd_status(&sd_detect)).await;

            loop {
                raw.fill(0);
                let keyboard_status = if keyboard
                    .read_async(KeyboardPins::I2C_ADDRESS, &mut raw)
                    .await
                    .is_ok()
                {
                    ProbeStatus::Pass
                } else {
                    ProbeStatus::Error
                };
                let normalized = normalize_hid_codes(&raw);
                let sd = sd_status(&sd_detect);

                line.clear();
                let _ = write!(
                    line,
                    "probe seq={} keyboard={} raw=",
                    sequence,
                    keyboard_status.label()
                );
                for byte in raw {
                    let _ = write!(line, "{:02x}", byte);
                }
                let _ = write!(line, " normalized=");
                normalized.compact(&mut line);
                let _ = write!(line, " sd={}\r\n", sd.label());
                if write_packets(&mut cdc, line.as_bytes()).await.is_err() {
                    break;
                }

                if lcd_status == ProbeStatus::Pass {
                    paint_live_dashboard(&mut lcd, keyboard_status, &raw, normalized, sd).await;
                }
                sequence = sequence.wrapping_add(1);
                Timer::after_millis(100).await;
            }
        }
    };

    join(usb_task, probe_task).await;
}

fn sd_status(detect: &Input<'_>) -> ProbeStatus {
    if detect.is_low() {
        ProbeStatus::Present
    } else {
        ProbeStatus::Absent
    }
}

async fn write_summary<'a>(
    cdc: &mut CdcAcmClass<'a, Driver<'a, peripherals::USB>>,
    line: &mut LineBuffer,
    lcd: ProbeStatus,
    sd: ProbeStatus,
) {
    for (name, status) in [
        ("lcd", lcd),
        ("keyboard", ProbeStatus::Pending),
        ("sd_detect", sd),
        ("psram", ProbeStatus::Pending),
    ] {
        line.clear();
        let _ = write!(line, "probe {}={}\r\n", name, status.label());
        let _ = write_packets(cdc, line.as_bytes()).await;
    }
    line.clear();
    let _ = write!(
        line,
        "memory sram_total={} dashboard_static={} framebuffer=0\r\n",
        RP2040_SRAM_BYTES,
        core::mem::size_of::<LineBuffer>() + KEYBOARD_RAW_CAPACITY
    );
    let _ = write_packets(cdc, line.as_bytes()).await;
}

async fn write_packets<'a>(
    cdc: &mut CdcAcmClass<'a, Driver<'a, peripherals::USB>>,
    bytes: &[u8],
) -> Result<(), ()> {
    for chunk in bytes.chunks(64) {
        cdc.write_packet(chunk).await.map_err(|_| ())?;
    }
    Ok(())
}

async fn paint_static_dashboard(
    lcd: &mut PicoCalcLcd<'_>,
    lcd_status: ProbeStatus,
    sd: ProbeStatus,
) {
    let _ = lcd.fill(Rgb888::new(10, 14, 22)).await;
    let _ = lcd.fill_rect(0, 0, 320, 28, Rgb888::new(38, 74, 130)).await;
    paint_status_row(lcd, 36, lcd_status).await;
    paint_status_row(lcd, 76, ProbeStatus::Pending).await;
    paint_status_row(lcd, 116, sd).await;
    paint_status_row(lcd, 156, ProbeStatus::Pending).await;
    paint_status_row(lcd, 196, ProbeStatus::Pass).await;
    // SRAM budget bar: 264 KiB total, tiny fixed dashboard state, no framebuffer.
    let _ = lcd
        .fill_rect(18, 244, 284, 18, Rgb888::new(45, 52, 66))
        .await;
    let _ = lcd.fill_rect(18, 244, 12, 18, Rgb888::CYAN).await;
}

async fn paint_live_dashboard(
    lcd: &mut PicoCalcLcd<'_>,
    keyboard: ProbeStatus,
    raw: &[u8; KEYBOARD_RAW_CAPACITY],
    normalized: koto_pico::dashboard::NormalizedKeys,
    sd: ProbeStatus,
) {
    paint_status_row(lcd, 76, keyboard).await;
    paint_status_row(lcd, 116, sd).await;
    let _ = lcd
        .fill_rect(96, 84, 208, 20, Rgb888::new(10, 14, 22))
        .await;
    for (index, byte) in raw.iter().enumerate() {
        let color = if *byte == 0 {
            Rgb888::new(48, 54, 66)
        } else {
            Rgb888::YELLOW
        };
        let _ = lcd
            .fill_rect(100 + index as u16 * 24, 86, 18, 16, color)
            .await;
    }
    for (index, pressed) in [
        normalized.up,
        normalized.down,
        normalized.left,
        normalized.right,
        normalized.action_a,
        normalized.action_b,
        normalized.action_x,
        normalized.action_y,
    ]
    .into_iter()
    .enumerate()
    {
        let color = if pressed {
            Rgb888::CYAN
        } else {
            Rgb888::new(34, 40, 52)
        };
        let _ = lcd
            .fill_rect(100 + index as u16 * 24, 106, 18, 5, color)
            .await;
    }
}

async fn paint_status_row(lcd: &mut PicoCalcLcd<'_>, y: u16, status: ProbeStatus) {
    let color = match status {
        ProbeStatus::Pass | ProbeStatus::Present => Rgb888::GREEN,
        ProbeStatus::Pending => Rgb888::YELLOW,
        ProbeStatus::Absent => Rgb888::new(100, 108, 122),
        ProbeStatus::Error => Rgb888::RED,
    };
    let _ = lcd.fill_rect(18, y, 284, 28, Rgb888::new(28, 34, 46)).await;
    let _ = lcd.fill_rect(18, y, 10, 28, color).await;
    let _ = lcd.fill_rect(38, y + 10, 46, 8, color).await;
}
