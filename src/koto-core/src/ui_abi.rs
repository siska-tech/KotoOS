//! Wire-level codecs shared by the KotoUI VM host adapters.
//!
//! These types deliberately depend on configuration snapshots rather than the
//! mutable configuration service. A VM host copies the snapshot at the session
//! boundary and never retains a pointer into system configuration storage.

use crate::{
    ConfigSnapshot, UI_DAMAGE_CAPACITY, UI_DATA_CAPACITY, UI_EVENT_QUEUE_CAPACITY,
    UI_MAX_LIST_ROWS, UI_MAX_MOUNT_BYTES, UI_MAX_NODES, UI_MAX_OPEN_MODALS, UI_MAX_TEXT_FIELDS,
    UI_MAX_TEXT_FIELD_BYTES, UI_MAX_UPDATE_BYTES,
};

pub const UI_CAPABILITIES_BYTES: usize = 64;
pub const UI_ABI_HOST_MINOR: u16 = 18;

const UI_CAPABILITIES_MAGIC: &[u8; 4] = b"KUC1";
const NODE_KIND_MASK_V1: u32 = 0x0000_00fe;
const UI_CAPABILITIES_FLAGS_V1: u16 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiAbiError {
    BufferTooSmall,
}

/// Dynamic KotoUI capability snapshot encoded by `ui_capabilities`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiCapabilities {
    config: ConfigSnapshot,
}

impl UiCapabilities {
    pub const fn from_config(config: ConfigSnapshot) -> Self {
        Self { config }
    }

    pub const fn config(self) -> ConfigSnapshot {
        self.config
    }

    /// Encodes the frozen KUC1 v1.0 record and returns its exact byte length.
    pub fn encode(self, dst: &mut [u8]) -> Result<usize, UiAbiError> {
        if dst.len() < UI_CAPABILITIES_BYTES {
            return Err(UiAbiError::BufferTooSmall);
        }

        let out = &mut dst[..UI_CAPABILITIES_BYTES];
        out.fill(0);
        out[0..4].copy_from_slice(UI_CAPABILITIES_MAGIC);
        put_u16(out, 4, 1);
        put_u16(out, 6, 0);
        put_u32(out, 8, NODE_KIND_MASK_V1);
        put_u16(out, 12, UI_MAX_NODES as u16);
        out[14] = UI_MAX_TEXT_FIELDS as u8;
        out[15] = UI_EVENT_QUEUE_CAPACITY as u8;
        put_u16(out, 16, UI_MAX_MOUNT_BYTES as u16);
        put_u16(out, 18, UI_MAX_UPDATE_BYTES as u16);
        put_u16(out, 20, UI_DATA_CAPACITY as u16);
        put_u16(out, 22, UI_MAX_TEXT_FIELD_BYTES as u16);
        put_u16(out, 24, UI_MAX_LIST_ROWS as u16);
        out[26] = UI_DAMAGE_CAPACITY as u8;
        out[27] = UI_MAX_OPEN_MODALS as u8;
        put_u16(out, 28, UI_ABI_HOST_MINOR);
        put_u16(out, 30, UI_CAPABILITIES_FLAGS_V1);

        let locale = self.config.locale.tag().as_bytes();
        debug_assert!(!locale.is_empty() && locale.len() <= 23);
        out[32] = locale.len() as u8;
        out[33] = 0; // LTR; RTL remains reserved in v1.
        put_u32(out, 36, self.config.locale_generation);
        out[40..40 + locale.len()].copy_from_slice(locale);
        Ok(UI_CAPABILITIES_BYTES)
    }
}

fn put_u16(dst: &mut [u8], offset: usize, value: u16) {
    dst[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(dst: &mut [u8], offset: usize, value: u32) {
    dst[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ConfigService, Locale};

    #[test]
    fn encodes_canonical_english_capability_record() {
        let mut bytes = [0xa5; UI_CAPABILITIES_BYTES];
        let written = UiCapabilities::from_config(ConfigService::new().snapshot())
            .encode(&mut bytes)
            .unwrap();

        assert_eq!(written, 64);
        assert_eq!(&bytes[0..4], b"KUC1");
        assert_eq!(u32::from_le_bytes(bytes[8..12].try_into().unwrap()), 0xfe);
        assert_eq!(u16::from_le_bytes(bytes[28..30].try_into().unwrap()), 18);
        assert_eq!(bytes[32], 5);
        assert_eq!(bytes[33], 0);
        assert_eq!(u32::from_le_bytes(bytes[36..40].try_into().unwrap()), 1);
        assert_eq!(&bytes[40..45], b"en-US");
        assert!(bytes[45..64].iter().all(|byte| *byte == 0));

        let fixture =
            include_str!("../../../harness/fixtures/koto_ui_abi/valid_en_us_capabilities.hex")
                .trim()
                .as_bytes();
        assert_eq!(fixture.len(), UI_CAPABILITIES_BYTES * 2);
        for (index, actual) in bytes.iter().enumerate() {
            let high = hex_nibble(fixture[index * 2]);
            let low = hex_nibble(fixture[index * 2 + 1]);
            assert_eq!(*actual, (high << 4) | low, "fixture byte {index}");
        }
    }

    #[test]
    fn reflects_locale_snapshot_and_rejects_short_destination() {
        let mut config = ConfigService::new();
        assert!(config.set_locale(Locale::JaJp));
        let capabilities = UiCapabilities::from_config(config.snapshot());
        let mut bytes = [0; UI_CAPABILITIES_BYTES];
        capabilities.encode(&mut bytes).unwrap();

        assert_eq!(bytes[32], 5);
        assert_eq!(&bytes[40..45], b"ja-JP");
        assert_eq!(u32::from_le_bytes(bytes[36..40].try_into().unwrap()), 2);
        assert_eq!(
            capabilities.encode(&mut bytes[..63]),
            Err(UiAbiError::BufferTooSmall)
        );
    }

    fn hex_nibble(byte: u8) -> u8 {
        match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            _ => panic!("invalid fixture hex"),
        }
    }
}
