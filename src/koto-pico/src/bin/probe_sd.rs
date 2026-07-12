//! `probe_sd` — SD card mount and sequential-read probe (KOTO-0068).
//!
//! Brings up SPI0, mounts the FAT volume via `embedded-sdmmc` using the
//! validated 12 MHz fast / 1 MHz fallback clock sequence, lists `apps/` long
//! filenames, and streams the first `*.kpa.json` over USB CDC in 128-byte
//! chunks.
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
    usb::{Driver, InterruptHandler as UsbInterruptHandler},
};
use embassy_time::{Delay, Timer};
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

const FAST_SPI_HZ: u32 = 12_000_000;
const FALLBACK_SPI_HZ: u32 = 1_000_000;
const BANNER: &[u8] = concat!(
    "KotoOS KOTO-0068 sd-read v",
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
    let detect = Input::new(p.PIN_22, Pull::Up);

    let mut spi_config = SpiConfig::default();
    spi_config.frequency = FAST_SPI_HZ;
    let spi = Spi::new_blocking(p.SPI0, p.PIN_18, p.PIN_19, p.PIN_16, spi_config);
    let cs = Output::new(p.PIN_17, Level::High);
    let device = ExclusiveDevice::new(spi, cs, Delay).unwrap();
    let sdcard = SdCard::new(device, Delay);

    let driver = Driver::new(p.USB, Irqs);
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
        cdc.wait_connection().await;
        if write_packets(&mut cdc, BANNER).await.is_err() {
            return;
        }

        let mut line = TextBuffer::<2048>::new();
        let _ = write!(
            line,
            "sd detect={} pin=GP22 spi=SPI0 sck=GP18 mosi=GP19 miso=GP16 cs=GP17\r\n",
            if detect.is_low() { "present" } else { "absent" }
        );
        let _ = write_packets(&mut cdc, line.as_bytes()).await;

        line.clear();
        let _ = write!(line, "sd init clock={}Hz\r\n", FAST_SPI_HZ);
        let _ = write_packets(&mut cdc, line.as_bytes()).await;

        let (card_size, active_hz) = match sdcard.num_bytes() {
            Ok(size) => (size, FAST_SPI_HZ),
            Err(error) => {
                line.clear();
                let _ = write!(
                    line,
                    "sd init failed clock={}Hz error={:?}; fallback={}Hz\r\n",
                    FAST_SPI_HZ, error, FALLBACK_SPI_HZ
                );
                let _ = write_packets(&mut cdc, line.as_bytes()).await;

                let mut fallback = SpiConfig::default();
                fallback.frequency = FALLBACK_SPI_HZ;
                sdcard.spi(|device| device.bus_mut().set_config(&fallback));
                sdcard.mark_card_uninit();
                match sdcard.num_bytes() {
                    Ok(size) => (size, FALLBACK_SPI_HZ),
                    Err(error) => {
                        line.clear();
                        let _ = write!(
                            line,
                            "sd init failed clock={}Hz error={:?}\r\n",
                            FALLBACK_SPI_HZ, error
                        );
                        let _ = write_packets(&mut cdc, line.as_bytes()).await;
                        wait_forever(&mut cdc).await;
                        return;
                    }
                }
            }
        };

        line.clear();
        let _ = write!(
            line,
            "sd init pass clock={}Hz bytes={}\r\n",
            active_hz, card_size
        );
        let _ = write_packets(&mut cdc, line.as_bytes()).await;

        let volume_mgr = VolumeManager::new(sdcard, Clock);
        let volume = match volume_mgr.open_volume(VolumeIdx(0)) {
            Ok(volume) => volume,
            Err(error) => {
                line.clear();
                let _ = write!(line, "fat mount failed error={:?}\r\n", error);
                let _ = write_packets(&mut cdc, line.as_bytes()).await;
                wait_forever(&mut cdc).await;
                return;
            }
        };
        let root = match volume.open_root_dir() {
            Ok(root) => root,
            Err(error) => {
                line.clear();
                let _ = write!(line, "fat root failed error={:?}\r\n", error);
                let _ = write_packets(&mut cdc, line.as_bytes()).await;
                wait_forever(&mut cdc).await;
                return;
            }
        };
        let apps = match root.open_dir("APPS") {
            Ok(apps) => apps,
            Err(error) => {
                line.clear();
                let _ = write!(line, "apps open failed error={:?}\r\n", error);
                let _ = write_packets(&mut cdc, line.as_bytes()).await;
                wait_forever(&mut cdc).await;
                return;
            }
        };

        let mut lfn_storage = [0u8; 192];
        let mut lfn = LfnBuffer::new(&mut lfn_storage);
        let mut selected: Option<ShortFileName> = None;
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
                && is_manifest_name(display_name)
            {
                selected = Some(entry.name.clone());
            }
        });
        let _ = line.write_str("apps listing end\r\n");
        if let Err(error) = listing {
            line.clear();
            let _ = write!(line, "apps listing failed error={:?}\r\n", error);
        }
        let _ = write_packets(&mut cdc, line.as_bytes()).await;

        let Some(manifest_name) = selected else {
            let _ = write_packets(&mut cdc, b"manifest not found suffix=.kpa.json\r\n").await;
            wait_forever(&mut cdc).await;
            return;
        };
        let manifest = match apps.open_file_in_dir(&manifest_name, Mode::ReadOnly) {
            Ok(file) => file,
            Err(error) => {
                line.clear();
                let _ = write!(
                    line,
                    "manifest open failed short={} error={:?}\r\n",
                    manifest_name, error
                );
                let _ = write_packets(&mut cdc, line.as_bytes()).await;
                wait_forever(&mut cdc).await;
                return;
            }
        };

        line.clear();
        let _ = write!(
            line,
            "manifest read begin short={} bytes={} chunk=128\r\n",
            manifest_name,
            manifest.length()
        );
        let _ = write_packets(&mut cdc, line.as_bytes()).await;

        let mut total = 0usize;
        let mut chunk = [0u8; 128];
        while !manifest.is_eof() {
            match manifest.read(&mut chunk) {
                Ok(0) => break,
                Ok(count) => {
                    total += count;
                    if write_terminal_text(&mut cdc, &chunk[..count])
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
                Err(error) => {
                    line.clear();
                    let _ = write!(line, "\r\nmanifest read failed error={:?}\r\n", error);
                    let _ = write_packets(&mut cdc, line.as_bytes()).await;
                    wait_forever(&mut cdc).await;
                    return;
                }
            }
        }
        line.clear();
        let _ = write!(line, "\r\nmanifest read end bytes={}\r\n", total);
        let _ = write_packets(&mut cdc, line.as_bytes()).await;
        let _ = write_packets(&mut cdc, b"KOTO-0068 awaiting observation\r\n").await;
        wait_forever(&mut cdc).await;
    };

    join(usb_task, probe_task).await;
}

fn is_manifest_name(name: &str) -> bool {
    const SUFFIX: &[u8] = b".kpa.json";
    let bytes = name.as_bytes();
    bytes.len() >= SUFFIX.len()
        && bytes[bytes.len() - SUFFIX.len()..]
            .iter()
            .zip(SUFFIX)
            .all(|(actual, expected)| actual.to_ascii_lowercase() == *expected)
}

async fn wait_forever<'a>(cdc: &mut CdcAcmClass<'a, Driver<'a, peripherals::USB>>) {
    loop {
        Timer::after_secs(5).await;
        if cdc
            .write_packet(b"KOTO-0068 awaiting reconnect\r\n")
            .await
            .is_err()
        {
            return;
        }
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

async fn write_terminal_text<'a>(
    cdc: &mut CdcAcmClass<'a, Driver<'a, peripherals::USB>>,
    bytes: &[u8],
) -> Result<(), ()> {
    let mut display = [0u8; 256];
    let mut len = 0usize;
    for byte in bytes {
        if *byte == b'\n' {
            display[len] = b'\r';
            len += 1;
        }
        display[len] = *byte;
        len += 1;
    }
    write_packets(cdc, &display[..len]).await
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
    use super::is_manifest_name;

    #[test]
    fn recognizes_manifest_suffix_case_insensitively() {
        assert!(is_manifest_name("memo.kpa.json"));
        assert!(is_manifest_name("MEMO.KPA.JSON"));
        assert!(!is_manifest_name("memo.json"));
    }
}
