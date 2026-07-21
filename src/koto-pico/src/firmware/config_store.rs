//! Bounded A/B-slot KotoConfig persistence on the root FAT volume.

use embassy_rp::uart::UartTx;
use embedded_sdmmc::{BlockDevice, Mode, VolumeIdx, VolumeManager};
use koto_core::{
    config_write_slot, newest_config_slot, ConfigService, ConfigSlot, CONFIG_FORMAT_MAX_BYTES,
};

use crate::firmware::config::FirmwareClock;
use crate::firmware::diag::uart_log;

pub const CONFIG_SLOT_A_FILE: &str = "KCFGA.BIN";
pub const CONFIG_SLOT_B_FILE: &str = "KCFGB.BIN";

fn read_slot<D>(
    volume_mgr: &VolumeManager<D, FirmwareClock>,
    name: &str,
    scratch: &mut [u8; CONFIG_FORMAT_MAX_BYTES],
) -> Option<ConfigService>
where
    D: BlockDevice,
    D::Error: core::fmt::Debug,
{
    let volume = volume_mgr.open_volume(VolumeIdx(0)).ok()?;
    let root = volume.open_root_dir().ok()?;
    let file = root.open_file_in_dir(name, Mode::ReadOnly).ok()?;
    if file.length() as usize > scratch.len() {
        return None;
    }
    let mut length = 0usize;
    while !file.is_eof() && length < scratch.len() {
        match file.read(&mut scratch[length..]) {
            Ok(0) => break,
            Ok(count) => length += count,
            Err(_) => return None,
        }
    }
    ConfigService::decode(&scratch[..length]).ok()
}

pub fn load_system_config<D>(
    volume_mgr: &VolumeManager<D, FirmwareClock>,
    scratch: &mut [u8; CONFIG_FORMAT_MAX_BYTES],
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
) -> ConfigService
where
    D: BlockDevice,
    D::Error: core::fmt::Debug,
{
    let a = read_slot(volume_mgr, CONFIG_SLOT_A_FILE, scratch);
    let b = read_slot(volume_mgr, CONFIG_SLOT_B_FILE, scratch);
    let Some((_, selected)) = newest_config_slot(a, b) else {
        uart_log(uart, "phase=341 config-default\r\n");
        return ConfigService::new();
    };
    uart_log(uart, "phase=340 config-applied\r\n");
    selected
}

pub fn save_system_config<D>(
    volume_mgr: &VolumeManager<D, FirmwareClock>,
    config: &ConfigService,
    scratch: &mut [u8; CONFIG_FORMAT_MAX_BYTES],
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
) -> bool
where
    D: BlockDevice,
    D::Error: core::fmt::Debug,
{
    let a = read_slot(volume_mgr, CONFIG_SLOT_A_FILE, scratch);
    let b = read_slot(volume_mgr, CONFIG_SLOT_B_FILE, scratch);
    let destination = match config_write_slot(a, b) {
        ConfigSlot::A => CONFIG_SLOT_A_FILE,
        ConfigSlot::B => CONFIG_SLOT_B_FILE,
    };
    let Ok(length) = config.encode(scratch) else {
        uart_log(uart, "phase=342 config-encode-error\r\n");
        return false;
    };
    let Ok(volume) = volume_mgr.open_volume(VolumeIdx(0)) else {
        uart_log(uart, "phase=343 config-volume-error\r\n");
        return false;
    };
    let Ok(root) = volume.open_root_dir() else {
        uart_log(uart, "phase=344 config-root-error\r\n");
        return false;
    };
    let Ok(file) = root.open_file_in_dir(destination, Mode::ReadWriteCreateOrTruncate) else {
        uart_log(uart, "phase=345 config-open-error\r\n");
        return false;
    };
    if file.write(&scratch[..length]).is_err() || file.flush().is_err() {
        uart_log(uart, "phase=346 config-write-error\r\n");
        return false;
    }
    uart_log(uart, "phase=347 config-saved\r\n");
    true
}
