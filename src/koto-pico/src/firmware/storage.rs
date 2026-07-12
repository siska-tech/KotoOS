//! SD-card bring-up and the `APPS`/`ICONS` catalog loader (KOTO-0121 / KOTO-0122).

use core::fmt::Write;

use embassy_embedded_hal::SetConfig;
use embassy_rp::spi::Config as SpiConfig;
use embassy_rp::uart::UartTx;
use embassy_time::Delay;
use embedded_hal_bus::spi::ExclusiveDevice;
use embedded_sdmmc::{
    BlockDevice, LfnBuffer, Mode, SdCard, ShortFileName, VolumeIdx, VolumeManager,
};
use koto_core::shell::StorageStatus;
use koto_core::{PackageIcon, PackageInfo, PackageList, MAX_ICON_PATH_LEN, MAX_PACKAGES};

use crate::dashboard::LineBuffer;
use crate::firmware::config::{
    FirmwareClock, KICON_BYTES, MANIFEST_LFN_BYTES, MAX_MANIFEST_BYTES, SD_FALLBACK_SPI_HZ,
    SD_FAST_SPI_HZ,
};
use crate::firmware::diag::{uart_log, uart_write_line};
use crate::firmware::parse_package_summary;

/// Initialize the SD card using the hardware-validated fast/fallback clock
/// sequence. The card remains owned by the caller so its `VolumeManager` can
/// stay alive for catalog reads and later preference writes.
pub fn initialize_sd_card<SPI, CS>(
    sdcard: &SdCard<ExclusiveDevice<SPI, CS, Delay>, Delay>,
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
) -> Option<u32>
where
    SPI: embedded_hal::spi::SpiBus<u8> + SetConfig<Config = SpiConfig>,
    CS: embedded_hal::digital::OutputPin,
{
    uart_log(uart, "phase=131 sd-card-init-start clock=12000000\r\n");
    let active_hz = match sdcard.num_bytes() {
        Ok(_) => SD_FAST_SPI_HZ,
        Err(_) => {
            uart_log(uart, "phase=181 sd-fast-init-failed fallback=1000000\r\n");
            let mut fallback = SpiConfig::default();
            fallback.frequency = SD_FALLBACK_SPI_HZ;
            sdcard.spi(|device| {
                let _ = device.bus_mut().set_config(&fallback);
            });
            sdcard.mark_card_uninit();
            match sdcard.num_bytes() {
                Ok(_) => SD_FALLBACK_SPI_HZ,
                Err(_) => {
                    uart_log(uart, "phase=191 sd-card-init-error\r\n");
                    return None;
                }
            }
        }
    };
    let mut line = LineBuffer::new();
    let _ = write!(line, "phase=132 sd-card-init-ok clock={}\r\n", active_hz);
    uart_write_line(uart, &line);
    Some(active_hz)
}

/// Scan `APPS/*.kpa.json` into `packages`, returning the storage status. The
/// caller owns `packages` and the scan scratch buffers (`names`, `manifest`,
/// `lfn_storage`, `kicon`), all of which live in static or owned storage so this
/// loader runs in a small call frame (KOTO-0121). After the manifest pass it
/// attaches `ICONS/*.kicon` assets to the matching packages (KOTO-0122). Every
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
                && long_name.is_some_and(is_manifest_name)
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
    let _ = write!(line, "phase=137 apps-list-ok manifests={}\r\n", name_count);
    uart_write_line(uart, &line);

    for (index, name) in names[..name_count].iter().flatten().enumerate() {
        line.clear();
        let _ = write!(line, "phase=138 manifest-read-start index={}\r\n", index);
        uart_write_line(uart, &line);
        let Ok(file) = apps.open_file_in_dir(name, Mode::ReadOnly) else {
            continue;
        };
        if file.length() as usize > MAX_MANIFEST_BYTES {
            continue;
        }
        let mut length = 0usize;
        while !file.is_eof() && length < manifest.len() {
            match file.read(&mut manifest[length..]) {
                Ok(0) => break,
                Ok(count) => length += count,
                Err(_) => {
                    length = 0;
                    break;
                }
            }
        }
        if let Some(package) = parse_package_summary(&manifest[..length]) {
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

    // Attach ICONS/*.kicon assets to the discovered packages (KOTO-0122). Reads
    // are bounded and sequential into the shared `kicon` scratch; the icon theme
    // already rode in on the manifest summary. A missing or invalid asset keeps
    // the deterministic fallback icon from the portable shell.
    let mut icon_count = 0usize;
    if let Ok(icons_dir) = root.open_dir("ICONS") {
        for index in 0..packages.len() {
            // Copy the icon path out so the list can be re-borrowed mutably.
            let mut path_buf = [0u8; MAX_ICON_PATH_LEN];
            let Some(path_len) = packages
                .get(index)
                .and_then(PackageInfo::icon_path)
                .map(|path| {
                    let bytes = path.as_bytes();
                    path_buf[..bytes.len()].copy_from_slice(bytes);
                    bytes.len()
                })
            else {
                continue;
            };
            let Ok(file_name) = core::str::from_utf8(&path_buf[..path_len]) else {
                continue;
            };
            let file_name = file_name.rsplit('/').next().unwrap_or(file_name);

            // Match the asset's long name to its 8.3 short name, then read it.
            let mut short: Option<ShortFileName> = None;
            let mut icon_lfn = LfnBuffer::new(lfn_storage);
            let _ = icons_dir.iterate_dir_lfn(&mut icon_lfn, |entry, long_name| {
                if short.is_none()
                    && !entry.attributes.is_directory()
                    && long_name.is_some_and(|name| name.eq_ignore_ascii_case(file_name))
                {
                    short = Some(entry.name.clone());
                }
            });
            let Some(short) = short else {
                continue;
            };
            let Ok(file) = icons_dir.open_file_in_dir(&short, Mode::ReadOnly) else {
                continue;
            };
            if file.length() as usize > KICON_BYTES {
                continue;
            }
            let mut length = 0usize;
            while !file.is_eof() && length < kicon.len() {
                match file.read(&mut kicon[length..]) {
                    Ok(0) => break,
                    Ok(count) => length += count,
                    Err(_) => {
                        length = 0;
                        break;
                    }
                }
            }
            drop(file);
            if let Ok(icon) = PackageIcon::from_kicon_text(&kicon[..length]) {
                if let Some(package) = packages.get_mut(index) {
                    package.set_icon(icon);
                    icon_count += 1;
                }
            }
        }
    }
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
    const SUFFIX: &[u8] = b".kpa.json";
    let bytes = name.as_bytes();
    bytes.len() >= SUFFIX.len()
        && bytes[bytes.len() - SUFFIX.len()..]
            .iter()
            .zip(SUFFIX)
            .all(|(actual, expected)| actual.to_ascii_lowercase() == *expected)
}
