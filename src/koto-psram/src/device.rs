//! PSRAM device identity helpers.

/// JEDEC-like device identity captured during SPI probing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceId {
    /// Raw identity bytes returned by the device.
    pub raw: [u8; 3],
}

impl DeviceId {
    /// Creates a device identity from raw bytes.
    #[inline]
    pub const fn new(raw: [u8; 3]) -> Self {
        Self { raw }
    }

    /// Returns true when the ID is non-zero and non-erased-looking.
    #[inline]
    pub const fn looks_present(self) -> bool {
        let all_zero = self.raw[0] == 0x00 && self.raw[1] == 0x00 && self.raw[2] == 0x00;
        let all_erased = self.raw[0] == 0xff && self.raw[1] == 0xff && self.raw[2] == 0xff;
        !all_zero && !all_erased
    }
}

impl From<[u8; 3]> for DeviceId {
    #[inline]
    fn from(raw: [u8; 3]) -> Self {
        Self::new(raw)
    }
}

impl From<DeviceId> for [u8; 3] {
    #[inline]
    fn from(id: DeviceId) -> Self {
        id.raw
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_floating_or_silent_bus_ids() {
        assert!(!DeviceId::new([0x00; 3]).looks_present());
        assert!(!DeviceId::new([0xff; 3]).looks_present());
        assert!(DeviceId::new([0x0d, 0x5d, 0x5d]).looks_present());
    }
}
