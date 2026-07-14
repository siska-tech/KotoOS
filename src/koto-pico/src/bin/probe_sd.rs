//! `probe_sd` — SD card mount and sequential-read probe (KOTO-0068).
//!
//! Brings up SPI0, mounts the FAT volume via `embedded-sdmmc` using the
//! standards-compliant 400 kHz acquisition / validated fast transfer sequence,
//! lists `apps/` long
//! filenames, and streams the first `*.kpa.json` over UART0 and, when
//! connected, USB CDC in 128-byte chunks.
//!
//! Not part of normal development: flash manually only to re-validate SD
//! storage. See `docs/hardware/PICO_HARDWARE_LOG.md`.
#![no_std]
#![no_main]

use core::fmt::{self, Write};

use embassy_executor::Spawner;
use embassy_futures::join::join;
use embassy_rp::{
    bind_interrupts,
    gpio::{Input, Level, Output, Pull},
    peripherals,
    spi::{Config as SpiConfig, Spi},
    uart::{Config as UartConfig, UartTx},
    usb::{Driver, InterruptHandler as UsbInterruptHandler},
};
use embassy_time::{Delay, Instant, Timer};
use embassy_usb::{
    class::cdc_acm::{CdcAcmClass, State},
    Builder, Config,
};
use embedded_hal_bus::spi::ExclusiveDevice;
use embedded_sdmmc::{
    LfnBuffer, Mode, SdCard, ShortFileName, TimeSource, Timestamp, VolumeIdx, VolumeManager,
};
use panic_halt as _;

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => UsbInterruptHandler<peripherals::USB>;
});

use koto_pico::firmware::config::SD_ACQUIRE_SPI_HZ;
use koto_pico::firmware::storage::initialize_sd_card;
const BANNER: &[u8] = concat!(
    "KotoOS KOTO-0068 sd-read v",
    env!("CARGO_PKG_VERSION"),
    " board=",
    env!("KOTO_BOARD_ID"),
    " mcu=",
    env!("KOTO_MCU_ID"),
    "\r\n"
)
.as_bytes();

#[embassy_executor::main(
    executor = "embassy_rp::executor::Executor",
    entry = "cortex_m_rt::entry"
)]
async fn main(_spawner: Spawner) {
    let p = koto_pico::board::split_peripherals(embassy_rp::init(Default::default()));
    let detect = Input::new(p.sd_detect, Pull::Up);

    let mut spi_config = SpiConfig::default();
    spi_config.frequency = SD_ACQUIRE_SPI_HZ;
    let spi = Spi::new_blocking(p.sd_spi, p.sd_sck, p.sd_mosi, p.sd_miso, spi_config);
    let cs = Output::new(p.sd_cs, Level::High);
    let device = ExclusiveDevice::new(spi, cs, Delay).unwrap();
    let sdcard = SdCard::new(device, Delay);

    let mut uart_config = UartConfig::default();
    uart_config.baudrate = 115_200;
    let mut uart = UartTx::new_blocking(p.uart, p.uart_tx, uart_config);

    let driver = Driver::new(p.usb, Irqs);
    let mut usb_config = Config::new(0xc0de, 0x0068);
    usb_config.manufacturer = Some("KotoOS");
    usb_config.product = Some("KotoOS SD read probe");
    usb_config.serial_number = Some("KOTO-0068");
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
        emit(&mut uart, &mut cdc, BANNER).await;
        emit(
            &mut uart,
            &mut cdc,
            b"log=uart0 tx=GP0 baud=115200 usb_cdc=optional\r\n",
        )
        .await;

        // Mounting and reading can finish before the host opens PicoCalc's
        // USB-UART COM port after a reboot. Keep the observation window open
        // before touching the card so the complete one-shot report is visible.
        for remaining in (1..=10).rev() {
            let mut countdown = TextBuffer::<64>::new();
            let _ = write!(countdown, "sd probe starts in {}s\r\n", remaining);
            emit(&mut uart, &mut cdc, countdown.as_bytes()).await;
            Timer::after_secs(1).await;
        }

        let mut line = TextBuffer::<2048>::new();
        let _ = write!(
            line,
            "sd detect={} pin=GP22 spi=SPI0 sck=GP18 mosi=GP19 miso=GP16 cs=GP17\r\n",
            if detect.is_low() { "present" } else { "absent" }
        );
        emit(&mut uart, &mut cdc, line.as_bytes()).await;

        let Some(active_hz) = initialize_sd_card(&sdcard, &mut uart) else {
            emit(&mut uart, &mut cdc, b"sd init failed\r\n").await;
            wait_forever(&mut uart, &mut cdc).await;
            return;
        };
        let card_size = match sdcard.num_bytes() {
            Ok(size) => size,
            Err(error) => {
                line.clear();
                let _ = write!(line, "sd transfer validation failed error={:?}\r\n", error);
                emit(&mut uart, &mut cdc, line.as_bytes()).await;
                wait_forever(&mut uart, &mut cdc).await;
                return;
            }
        };

        line.clear();
        let _ = write!(
            line,
            "sd init pass clock={}Hz bytes={}\r\n",
            active_hz, card_size
        );
        emit(&mut uart, &mut cdc, line.as_bytes()).await;

        let volume_mgr = VolumeManager::new(sdcard, Clock);
        let volume = match volume_mgr.open_volume(VolumeIdx(0)) {
            Ok(volume) => volume,
            Err(error) => {
                line.clear();
                let _ = write!(line, "fat mount failed error={:?}\r\n", error);
                emit(&mut uart, &mut cdc, line.as_bytes()).await;
                wait_forever(&mut uart, &mut cdc).await;
                return;
            }
        };
        let root = match volume.open_root_dir() {
            Ok(root) => root,
            Err(error) => {
                line.clear();
                let _ = write!(line, "fat root failed error={:?}\r\n", error);
                emit(&mut uart, &mut cdc, line.as_bytes()).await;
                wait_forever(&mut uart, &mut cdc).await;
                return;
            }
        };
        let apps = match root.open_dir("APPS") {
            Ok(apps) => apps,
            Err(error) => {
                line.clear();
                let _ = write!(line, "apps open failed error={:?}\r\n", error);
                emit(&mut uart, &mut cdc, line.as_bytes()).await;
                wait_forever(&mut uart, &mut cdc).await;
                return;
            }
        };

        let mut lfn_storage = [0u8; 192];
        let mut lfn = LfnBuffer::new(&mut lfn_storage);
        let mut selected: Option<ShortFileName> = None;
        let mut selected_is_text = false;
        line.clear();
        let _ = line.write_str("apps listing begin\r\n");
        let listing = apps.iterate_dir_lfn(&mut lfn, |entry, long_name| {
            let display_name = long_name.unwrap_or("");
            let _ = write!(
                line,
                "app name={} short={} bytes={} dir={}\r\n",
                if display_name.is_empty() {
                    "<short-name>"
                } else {
                    display_name
                },
                entry.name,
                entry.size,
                entry.attributes.is_directory()
            );
            if selected.is_none()
                && !entry.attributes.is_directory()
                && is_package_name(display_name)
            {
                selected = Some(entry.name.clone());
                selected_is_text = is_manifest_name(display_name);
            }
        });
        let _ = line.write_str("apps listing end\r\n");
        if let Err(error) = listing {
            line.clear();
            let _ = write!(line, "apps listing failed error={:?}\r\n", error);
        }
        emit(&mut uart, &mut cdc, line.as_bytes()).await;

        let Some(manifest_name) = selected else {
            emit(
                &mut uart,
                &mut cdc,
                b"package not found suffix=.kpa.json|.kpa\r\n",
            )
            .await;
            wait_forever(&mut uart, &mut cdc).await;
            return;
        };
        let manifest = match apps.open_file_in_dir(&manifest_name, Mode::ReadOnly) {
            Ok(file) => file,
            Err(error) => {
                line.clear();
                let _ = write!(
                    line,
                    "package open failed short={} error={:?}\r\n",
                    manifest_name, error
                );
                emit(&mut uart, &mut cdc, line.as_bytes()).await;
                wait_forever(&mut uart, &mut cdc).await;
                return;
            }
        };

        line.clear();
        let _ = write!(
            line,
            "package read begin short={} bytes={} chunk=128 text={}\r\n",
            manifest_name,
            manifest.length(),
            selected_is_text
        );
        emit(&mut uart, &mut cdc, line.as_bytes()).await;

        let mut total = 0usize;
        let mut checksum = 0x811c9dc5u32;
        let mut chunk = [0u8; 128];
        let read_started = Instant::now();
        while !manifest.is_eof() {
            match manifest.read(&mut chunk) {
                Ok(0) => break,
                Ok(count) => {
                    total += count;
                    for byte in &chunk[..count] {
                        checksum = (checksum ^ u32::from(*byte)).wrapping_mul(0x0100_0193);
                    }
                }
                Err(error) => {
                    line.clear();
                    let _ = write!(line, "\r\npackage read failed error={:?}\r\n", error);
                    emit(&mut uart, &mut cdc, line.as_bytes()).await;
                    wait_forever(&mut uart, &mut cdc).await;
                    return;
                }
            }
        }
        let elapsed_us = read_started.elapsed().as_micros().max(1);
        let kib_per_s = (total as u64)
            .saturating_mul(1_000_000)
            .saturating_div(elapsed_us)
            .saturating_div(1024);
        line.clear();
        let _ = write!(
            line,
            "\r\npackage read end bytes={} elapsed_us={} kib_per_s={} fnv1a32={:08x}\r\n",
            total, elapsed_us, kib_per_s, checksum
        );
        emit(&mut uart, &mut cdc, line.as_bytes()).await;
        emit(&mut uart, &mut cdc, b"KOTO-0068 awaiting observation\r\n").await;
        wait_forever(&mut uart, &mut cdc).await;
    };

    join(usb_task, probe_task).await;
}

fn is_manifest_name(name: &str) -> bool {
    has_suffix_ignore_ascii_case(name, b".kpa.json")
}

fn is_package_name(name: &str) -> bool {
    is_manifest_name(name) || has_suffix_ignore_ascii_case(name, b".kpa")
}

fn has_suffix_ignore_ascii_case(name: &str, suffix: &[u8]) -> bool {
    let bytes = name.as_bytes();
    bytes.len() >= suffix.len()
        && bytes[bytes.len() - suffix.len()..]
            .iter()
            .zip(suffix)
            .all(|(actual, expected)| actual.to_ascii_lowercase() == *expected)
}

async fn wait_forever<'a>(
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
    cdc: &mut CdcAcmClass<'a, Driver<'a, peripherals::USB>>,
) {
    loop {
        Timer::after_secs(5).await;
        emit(uart, cdc, b"KOTO-0068 awaiting observation\r\n").await;
    }
}

async fn emit<'a>(
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
    cdc: &mut CdcAcmClass<'a, Driver<'a, peripherals::USB>>,
    bytes: &[u8],
) {
    let _ = uart.blocking_write(bytes);
    if cdc.dtr() {
        let _ = write_packets(cdc, bytes).await;
    }
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

#[derive(Clone, Copy)]
struct Clock;

impl TimeSource for Clock {
    fn get_timestamp(&self) -> Timestamp {
        Timestamp {
            year_since_1970: 56,
            zero_indexed_month: 5,
            zero_indexed_day: 19,
            hours: 0,
            minutes: 0,
            seconds: 0,
        }
    }
}

struct TextBuffer<const N: usize> {
    bytes: [u8; N],
    len: usize,
}

impl<const N: usize> TextBuffer<N> {
    const fn new() -> Self {
        Self {
            bytes: [0; N],
            len: 0,
        }
    }

    fn clear(&mut self) {
        self.len = 0;
    }

    fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

impl<const N: usize> fmt::Write for TextBuffer<N> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let remaining = self.bytes.len().saturating_sub(self.len);
        if text.len() > remaining {
            return Err(fmt::Error);
        }
        self.bytes[self.len..self.len + text.len()].copy_from_slice(text.as_bytes());
        self.len += text.len();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{is_manifest_name, is_package_name};

    #[test]
    fn recognizes_manifest_suffix_case_insensitively() {
        assert!(is_manifest_name("memo.kpa.json"));
        assert!(is_manifest_name("MEMO.KPA.JSON"));
        assert!(!is_manifest_name("memo.json"));
        assert!(is_package_name("memo.kpa"));
        assert!(is_package_name("MEMO.KPA"));
        assert!(is_package_name("memo.kpa.json"));
        assert!(!is_package_name("memo.json"));
    }
}
