//! Bounded A/B-slot Wi-Fi secret persistence (KOTO-0240).
//!
//! This is the platform adapter behind `koto_core::SecretMedium`. It stores the
//! two credential slots in files (`KWSA.BIN` / `KWSB.BIN`) whose names are
//! **distinct** from the public KotoConfig settings slots (`KCFGA.BIN` /
//! `KCFGB.BIN`), keeping the secret namespace separate from `KCF1` on the same
//! root volume. It provides no hardware-backed confidentiality: physical access
//! to the SD card can recover credentials despite logical two-slot erasure. See
//! [`koto_core::wifi_secrets`] for the threat model and disclosure limits.
//!
//! A zero-length or missing slot file reads back as [`SlotRead::Absent`] (a
//! fresh or erased slot); a wrong-length file reads back as an invalid record so
//! the store fails closed rather than trusting a partial write.

use embedded_sdmmc::{BlockDevice, Mode, VolumeIdx, VolumeManager};
use koto_core::{
    MediumFault, SecretMedium, SecretSlot, SlotRead, WifiSecretStore, WIFI_SECRET_RECORD_BYTES,
};

use crate::firmware::config::FirmwareClock;

pub const SECRET_SLOT_A_FILE: &str = "KWSA.BIN";
pub const SECRET_SLOT_B_FILE: &str = "KWSB.BIN";

const fn slot_file(slot: SecretSlot) -> &'static str {
    match slot {
        SecretSlot::A => SECRET_SLOT_A_FILE,
        SecretSlot::B => SECRET_SLOT_B_FILE,
    }
}

/// A [`SecretMedium`] backed by two root-volume files, separate from the public
/// settings slots. Borrows the shared [`VolumeManager`]; every operation opens
/// the volume for the duration of a single read/write/erase, mirroring the
/// KotoConfig persistence adapter.
pub struct SdSecretMedium<'a, D>
where
    D: BlockDevice,
    D::Error: core::fmt::Debug,
{
    volume_mgr: &'a VolumeManager<D, FirmwareClock>,
}

impl<'a, D> SdSecretMedium<'a, D>
where
    D: BlockDevice,
    D::Error: core::fmt::Debug,
{
    pub fn new(volume_mgr: &'a VolumeManager<D, FirmwareClock>) -> Self {
        Self { volume_mgr }
    }

    /// Truncates a slot file to zero length so it reads back as absent.
    fn truncate(&self, name: &str) -> Result<(), MediumFault> {
        let volume = self.volume_mgr.open_volume(VolumeIdx(0)).map_err(fault)?;
        let root = volume.open_root_dir().map_err(fault)?;
        let file = root
            .open_file_in_dir(name, Mode::ReadWriteCreateOrTruncate)
            .map_err(fault)?;
        file.flush().map_err(fault)?;
        Ok(())
    }
}

fn fault<E>(_error: E) -> MediumFault {
    MediumFault
}

impl<D> SecretMedium for SdSecretMedium<'_, D>
where
    D: BlockDevice,
    D::Error: core::fmt::Debug,
{
    fn read_slot(&self, slot: SecretSlot, dst: &mut [u8; WIFI_SECRET_RECORD_BYTES]) -> SlotRead {
        // Leave dst zeroed unless a full, correctly sized record is read. An
        // all-zero buffer decodes as bad magic, so a wrong-length file fails
        // closed rather than being trusted as a partial write.
        dst.fill(0);
        let Ok(volume) = self.volume_mgr.open_volume(VolumeIdx(0)) else {
            return SlotRead::Absent;
        };
        let Ok(root) = volume.open_root_dir() else {
            return SlotRead::Absent;
        };
        let Ok(file) = root.open_file_in_dir(slot_file(slot), Mode::ReadOnly) else {
            // Missing file: the slot has never been written.
            return SlotRead::Absent;
        };
        let length = file.length() as usize;
        if length == 0 {
            // Zero-length file: erased or never populated.
            return SlotRead::Absent;
        }
        if length != WIFI_SECRET_RECORD_BYTES {
            // Wrong size: present but unusable. Keep dst zeroed (invalid record).
            return SlotRead::Present;
        }
        let mut read = 0usize;
        while read < WIFI_SECRET_RECORD_BYTES {
            match file.read(&mut dst[read..]) {
                Ok(0) => break,
                Ok(count) => read += count,
                Err(_) => {
                    dst.fill(0);
                    return SlotRead::Present;
                }
            }
        }
        // A short read leaves trailing zeros, which decode rejects.
        SlotRead::Present
    }

    fn write_slot(
        &mut self,
        slot: SecretSlot,
        src: &[u8; WIFI_SECRET_RECORD_BYTES],
    ) -> Result<(), MediumFault> {
        let volume = self.volume_mgr.open_volume(VolumeIdx(0)).map_err(fault)?;
        let root = volume.open_root_dir().map_err(fault)?;
        let file = root
            .open_file_in_dir(slot_file(slot), Mode::ReadWriteCreateOrTruncate)
            .map_err(fault)?;
        file.write(src).map_err(fault)?;
        file.flush().map_err(fault)?;
        Ok(())
    }

    fn erase_slot(&mut self, slot: SecretSlot) -> Result<(), MediumFault> {
        self.truncate(slot_file(slot))
    }
}

/// Loads a credential store from the two root-volume secret slots. The returned
/// store owns its medium (borrowing `volume_mgr`) and reports availability via
/// [`WifiSecretStore::available`]. Never blocks boot; corruption disables Wi-Fi
/// profiles without falling back to public settings.
pub fn load_wifi_secret_store<D>(
    volume_mgr: &VolumeManager<D, FirmwareClock>,
) -> WifiSecretStore<SdSecretMedium<'_, D>>
where
    D: BlockDevice,
    D::Error: core::fmt::Debug,
{
    let (store, _outcome) = WifiSecretStore::load(SdSecretMedium::new(volume_mgr));
    store
}
