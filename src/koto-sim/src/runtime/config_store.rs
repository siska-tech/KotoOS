use std::fs;
use std::path::{Path, PathBuf};

use koto_core::{
    config_write_slot, newest_config_slot, ConfigService, ConfigSlot, CONFIG_FORMAT_MAX_BYTES,
};

use super::{SimError, SAVE_DATA_ROOT};

pub const CONFIG_APP_ID: &str = "dev.koto.config";
const SLOT_A: &str = "config-a.bin";
const SLOT_B: &str = "config-b.bin";

fn config_dir(root: &Path) -> PathBuf {
    root.join(SAVE_DATA_ROOT).join(CONFIG_APP_ID)
}

fn slot_path(root: &Path, slot: &str) -> PathBuf {
    config_dir(root).join(slot)
}

fn load_slot(path: &Path) -> Option<ConfigService> {
    let len = usize::try_from(fs::metadata(path).ok()?.len()).ok()?;
    if len > CONFIG_FORMAT_MAX_BYTES {
        return None;
    }
    let bytes = fs::read(path).ok()?;
    ConfigService::decode(&bytes).ok()
}

/// Loads the newest valid slot, or safe English defaults when both are absent
/// or invalid. A torn write can invalidate at most the slot being replaced.
pub fn load_system_config(root: impl AsRef<Path>) -> ConfigService {
    let root = root.as_ref();
    let a = load_slot(&slot_path(root, SLOT_A));
    let b = load_slot(&slot_path(root, SLOT_B));
    newest_config_slot(a, b)
        .map(|(_, config)| config)
        .unwrap_or_default()
}

/// Writes one complete checksummed snapshot to the older/invalid slot. The
/// previously newest slot remains available if this write is interrupted.
pub fn save_system_config(config: &ConfigService, root: impl AsRef<Path>) -> Result<(), SimError> {
    let root = root.as_ref();
    let dir = config_dir(root);
    fs::create_dir_all(&dir).map_err(|_| SimError::Io)?;

    let a = load_slot(&slot_path(root, SLOT_A));
    let b = load_slot(&slot_path(root, SLOT_B));
    let destination = match config_write_slot(a, b) {
        ConfigSlot::A => SLOT_A,
        ConfigSlot::B => SLOT_B,
    };

    let mut bytes = [0; CONFIG_FORMAT_MAX_BYTES];
    let len = config.encode(&mut bytes).map_err(|_| SimError::Io)?;
    fs::write(slot_path(root, destination), &bytes[..len]).map_err(|_| SimError::Io)
}
