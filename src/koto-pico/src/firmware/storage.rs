//! SD-card bring-up and the `APPS`/`ICONS` catalog loader (KOTO-0121 / KOTO-0122).

use core::fmt::Write;

use embassy_embedded_hal::SetConfig;
use embassy_rp::spi::Config as SpiConfig;
use embassy_rp::uart::UartTx;
use embassy_time::{block_for, Delay, Duration, Instant};
use embedded_hal_bus::spi::ExclusiveDevice;
use embedded_sdmmc::{
    Block, BlockDevice, BlockIdx, LfnBuffer, Mode, SdCard, ShortFileName, VolumeIdx, VolumeManager,
};
use koto_core::shell::StorageStatus;
use koto_core::{PackageIcon, PackageInfo, PackageList, MAX_PACKAGES};

use crate::dashboard::LineBuffer;
use crate::firmware::config::{
    FirmwareClock, KICON_BYTES, MANIFEST_LFN_BYTES, MAX_MANIFEST_BYTES, SD_ACQUIRE_ATTEMPTS,
    SD_ACQUIRE_SPI_HZ, SD_ACQUIRE_TIMEOUT_MS, SD_FALLBACK_SPI_HZ, SD_IDLE_CLOCK_BYTES,
    SD_POWER_UP_DELAY_MS, SD_TRANSFER_SPI_HZ,
};
use crate::firmware::diag::{uart_log, uart_write_line};
use crate::firmware::parse_package_summary;

/// Acquire the SD card at the SPI-mode initialization clock, then select the
/// fastest data clock that can still read the card's CSD. The card remains
/// owned by the caller so its `VolumeManager` can stay alive for catalog reads
/// and later preference writes.
pub fn initialize_sd_card<SPI, CS>(
    sdcard: &SdCard<ExclusiveDevice<SPI, CS, Delay>, Delay>,
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
) -> Option<u32>
where
    SPI: embedded_hal::spi::SpiBus<u8> + SetConfig<Config = SpiConfig>,
    CS: embedded_hal::digital::OutputPin,
{
    let mut line = LineBuffer::new();
    let _ = write!(
        line,
        "phase=131 sd-card-init-start acquire_clock={}\r\n",
        SD_ACQUIRE_SPI_HZ
    );
    uart_write_line(uart, &line);

    // Make the power-up allowance local and explicit instead of relying on
    // splash/probe delays that happen to precede this call.
    block_for(Duration::from_millis(SD_POWER_UP_DELAY_MS));

    // SD Physical Layer SPI mode requires at least 74 clocks with CS high
    // before CMD0. Use 160 clocks for margin. ExclusiveDevice leaves CS high
    // between transactions, so bypass its transaction wrapper for the bytes.
    let idle = [0xff; SD_IDLE_CLOCK_BYTES];
    let mut acquire_at = |clock: u32| -> bool {
        let mut config = SpiConfig::default();
        config.frequency = clock;
        if !sdcard.spi(|device| device.bus_mut().set_config(&config).is_ok()) {
            return false;
        }
        let deadline_started = Instant::now();
        for attempt in 1..=SD_ACQUIRE_ATTEMPTS {
            if attempt > 1 {
                block_for(Duration::from_millis(5));
            }
            sdcard.mark_card_uninit();
            if !sdcard.spi(|device| device.bus_mut().write(&idle).is_ok()) {
                return false;
            }
            match sdcard.num_bytes() {
                Ok(_) => {
                    line.clear();
                    let _ = write!(
                        line,
                        "phase=183 sd-acquire-ok clock={} attempt={} elapsed_ms={}\r\n",
                        clock,
                        attempt,
                        deadline_started.elapsed().as_millis()
                    );
                    uart_write_line(uart, &line);
                    return true;
                }
                Err(error) => {
                    line.clear();
                    let _ = write!(
                        line,
                        "phase=181 sd-acquire-attempt-failed clock={} attempt={} elapsed_ms={} error={:?}\r\n",
                        clock,
                        attempt,
                        deadline_started.elapsed().as_millis(),
                        error
                    );
                    uart_write_line(uart, &line);
                }
            }
            if deadline_started.elapsed() >= Duration::from_millis(SD_ACQUIRE_TIMEOUT_MS) {
                line.clear();
                let _ = write!(
                    line,
                    "phase=181 sd-acquire-timeout clock={} limit_ms={}\r\n",
                    clock, SD_ACQUIRE_TIMEOUT_MS
                );
                uart_write_line(uart, &line);
                break;
            }
        }
        false
    };

    let mut acquire_hz = SD_ACQUIRE_SPI_HZ;
    let acquired = acquire_at(SD_ACQUIRE_SPI_HZ);
    if !acquired {
        // The on-hand PicoCalc SD path has historically acquired reliably at
        // 1 MHz even when 400 kHz and pre-acquisition 12 MHz both report
        // CardNotFound. Keep that compatibility acquisition, then promote the
        // already initialized card to a fast data clock below.
        if !acquire_at(SD_FALLBACK_SPI_HZ) {
            uart_log(
                uart,
                "phase=191 sd-card-init-error stage=compat-acquire\r\n",
            );
            return None;
        }
        acquire_hz = SD_FALLBACK_SPI_HZ;
    }

    let mut active_hz = acquire_hz;
    let mut validation_block = [Block::new()];
    for candidate in SD_TRANSFER_SPI_HZ {
        let mut config = SpiConfig::default();
        config.frequency = candidate;
        let configured = sdcard.spi(|device| device.bus_mut().set_config(&config).is_ok());
        let csd_ok = configured && sdcard.num_bytes().is_ok();
        // A short CSD response can pass on a marginal clock that corrupts a
        // full data sector. SdCard's default CRC checking makes this 512-byte
        // boot-sector read the acceptance gate for the selected clock.
        let sector_ok = csd_ok && sdcard.read(&mut validation_block, BlockIdx(0)).is_ok();
        if sector_ok {
            active_hz = candidate;
            break;
        }
        line.clear();
        let _ = write!(
            line,
            "phase=181 sd-transfer-clock-rejected clock={}\r\n",
            candidate
        );
        uart_write_line(uart, &line);
    }

    // If every promoted clock failed, restore and validate the acquisition
    // clock so the compatibility path remains usable.
    if active_hz == acquire_hz {
        let mut config = SpiConfig::default();
        config.frequency = acquire_hz;
        let restored = sdcard.spi(|device| device.bus_mut().set_config(&config).is_ok());
        if !restored || sdcard.num_bytes().is_err() {
            uart_log(uart, "phase=191 sd-card-transfer-error\r\n");
            return None;
        }
    }
    line.clear();
    let _ = write!(
        line,
        "phase=132 sd-card-init-ok acquire_clock={} transfer_clock={}\r\n",
        acquire_hz, active_hz
    );
    uart_write_line(uart, &line);
    Some(active_hz)
}

/// Scan binary `APPS/*.kpa` archives into `packages`, returning the storage status. The
/// caller owns `packages` and the scan scratch buffers (`names`, `manifest`,
/// `lfn_storage`, `kicon`), all of which live in static or owned storage so this
/// loader runs in a small call frame (KOTO-0121). After the manifest pass it
/// reads each archive's embedded manifest and icon entries in place. Every
/// failure mode — card absent, mount/root/APPS error, list error, malformed
/// manifests, missing/invalid icons, or an empty catalog — leaves a usable shell
/// state and emits a UART diagnostic.
pub fn load_packages<D>(
    volume_mgr: &VolumeManager<D, FirmwareClock>,
    packages: &mut PackageList,
    names: &mut [Option<ShortFileName>; MAX_PACKAGES],
    manifest: &mut [u8; MAX_MANIFEST_BYTES],
    lfn_storage: &mut [u8; MANIFEST_LFN_BYTES],
    kicon: &mut [u8; KICON_BYTES],
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
) -> StorageStatus
where
    D: BlockDevice,
    D::Error: core::fmt::Debug,
{
    uart_log(uart, "phase=133 fat-volume-open-start\r\n");
    let Ok(volume) = volume_mgr.open_volume(VolumeIdx(0)) else {
        uart_log(uart, "phase=193 fat-volume-open-error\r\n");
        fill_fallback(packages, "dev.koto.storage-mount-error", "SD mount error");
        return StorageStatus::Unknown;
    };
    uart_log(uart, "phase=134 fat-volume-open-ok\r\n");
    let Ok(root) = volume.open_root_dir() else {
        uart_log(uart, "phase=194 fat-root-open-error\r\n");
        fill_fallback(packages, "dev.koto.storage-root-error", "SD root error");
        return StorageStatus::Unknown;
    };
    uart_log(uart, "phase=135 fat-root-open-ok\r\n");
    let Ok(apps) = root.open_dir("APPS") else {
        uart_log(uart, "phase=195 apps-dir-open-error\r\n");
        fill_fallback(packages, "dev.koto.storage-apps-missing", "APPS missing");
        return StorageStatus::Unknown;
    };
    uart_log(uart, "phase=136 apps-dir-open-ok\r\n");

    let mut name_count = 0usize;
    let mut lfn = LfnBuffer::new(lfn_storage);
    if apps
        .iterate_dir_lfn(&mut lfn, |entry, long_name| {
            if name_count < names.len()
                && !entry.attributes.is_directory()
                && is_package_name(long_name, &entry.name)
            {
                names[name_count] = Some(entry.name.clone());
                name_count += 1;
            }
        })
        .is_err()
    {
        uart_log(uart, "phase=196 apps-list-error\r\n");
        fill_fallback(packages, "dev.koto.storage-list-error", "APPS read error");
        return StorageStatus::Unknown;
    }
    let mut line = LineBuffer::new();
    let _ = write!(line, "phase=137 apps-list-ok packages={}\r\n", name_count);
    uart_write_line(uart, &line);

    let mut icon_count = 0usize;
    for (index, name) in names[..name_count].iter().flatten().enumerate() {
        line.clear();
        let _ = write!(line, "phase=138 manifest-read-start index={}\r\n", index);
        uart_write_line(uart, &line);
        let Ok(file) = apps.open_file_in_dir(name, Mode::ReadOnly) else {
            continue;
        };
        let mut header = [0u8; 64];
        let mut header_len = 0usize;
        while header_len < header.len() {
            match file.read(&mut header[header_len..]) {
                Ok(0) => break,
                Ok(count) => header_len += count,
                Err(_) => break,
            }
        }
        if header_len != header.len() || &header[..4] != b"KPA1" {
            continue;
        }
        let metadata_offset = u32::from_le_bytes(header[32..36].try_into().unwrap_or([0; 4]));
        let metadata_size =
            u32::from_le_bytes(header[36..40].try_into().unwrap_or([0; 4])) as usize;
        if metadata_size > MAX_MANIFEST_BYTES || file.seek_from_start(metadata_offset).is_err() {
            continue;
        }
        let mut length = 0usize;
        while length < metadata_size {
            match file.read(&mut manifest[length..metadata_size]) {
                Ok(0) => break,
                Ok(count) => length += count,
                Err(_) => {
                    length = 0;
                    break;
                }
            }
        }
        if let Some(mut package) = parse_package_summary(&manifest[..length]) {
            if let Some(icon_path) = package.icon_path() {
                let entry_count = u32::from_le_bytes(header[16..20].try_into().unwrap_or([0; 4]));
                let table_offset = u32::from_le_bytes(header[20..24].try_into().unwrap_or([0; 4]));
                let strings_offset =
                    u32::from_le_bytes(header[24..28].try_into().unwrap_or([0; 4]));
                let mut record = [0u8; 64];
                let mut icon_range = None;
                for entry_index in 0..entry_count {
                    if file
                        .seek_from_start(
                            table_offset.saturating_add(entry_index.saturating_mul(64)),
                        )
                        .is_err()
                    {
                        break;
                    }
                    let mut got = 0usize;
                    while got < record.len() {
                        match file.read(&mut record[got..]) {
                            Ok(0) | Err(_) => break,
                            Ok(count) => got += count,
                        }
                    }
                    if got != record.len() {
                        break;
                    }
                    let path_offset = u32::from_le_bytes(record[0..4].try_into().unwrap_or([0; 4]));
                    let path_len =
                        u32::from_le_bytes(record[4..8].try_into().unwrap_or([0; 4])) as usize;
                    if path_len > lfn_storage.len() {
                        continue;
                    }
                    if file
                        .seek_from_start(strings_offset.saturating_add(path_offset))
                        .is_err()
                    {
                        break;
                    }
                    let mut path_got = 0usize;
                    while path_got < path_len {
                        match file.read(&mut lfn_storage[path_got..path_len]) {
                            Ok(0) | Err(_) => break,
                            Ok(count) => path_got += count,
                        }
                    }
                    if path_got == path_len && lfn_storage[..path_len] == *icon_path.as_bytes() {
                        icon_range = Some((
                            u32::from_le_bytes(record[16..20].try_into().unwrap_or([0; 4])),
                            u32::from_le_bytes(record[20..24].try_into().unwrap_or([0; 4]))
                                as usize,
                        ));
                        break;
                    }
                }
                if let Some((offset, size)) = icon_range.filter(|(_, size)| *size <= kicon.len()) {
                    if file.seek_from_start(offset).is_ok() {
                        let mut got = 0usize;
                        while got < size {
                            match file.read(&mut kicon[got..size]) {
                                Ok(0) | Err(_) => break,
                                Ok(count) => got += count,
                            }
                        }
                        if got == size {
                            if let Ok(icon) = PackageIcon::from_kicon_text(&kicon[..size]) {
                                package.set_icon(icon);
                                icon_count += 1;
                            }
                        }
                    }
                }
            }
            packages.push(package);
        }
    }
    line.clear();
    let _ = write!(
        line,
        "phase=139 manifest-read-done accepted={}\r\n",
        packages.len()
    );
    uart_write_line(uart, &line);

    line.clear();
    let _ = write!(line, "phase=140 icons-loaded count={}\r\n", icon_count);
    uart_write_line(uart, &line);

    if packages.is_empty() {
        fill_fallback(packages, "dev.koto.storage-empty", "No valid apps");
    }
    StorageStatus::Present
}

/// Push a single placeholder package so the shell always has a usable entry.
pub fn fill_fallback(packages: &mut PackageList, app_id: &str, name: &str) {
    if let Some(package) = PackageInfo::new(app_id, name) {
        packages.push(package);
    }
}

fn is_manifest_name(name: &str) -> bool {
    const SUFFIX: &[u8] = b".kpa";
    let bytes = name.as_bytes();
    bytes.len() >= SUFFIX.len()
        && bytes[bytes.len() - SUFFIX.len()..]
            .iter()
            .zip(SUFFIX)
            .all(|(actual, expected)| actual.to_ascii_lowercase() == *expected)
}

fn is_package_name(long_name: Option<&str>, short_name: &ShortFileName) -> bool {
    long_name.is_some_and(is_manifest_name) || short_name.extension().eq_ignore_ascii_case(b"KPA")
}
