//! PSRAM physical address handling.

use crate::error::PsramError;

/// Size of the PicoCalc PSRAM in bytes.
pub const PSRAM_SIZE: u32 = 8 * 1024 * 1024;

/// Highest valid byte address for an 8 MiB PSRAM.
pub const MAX_ADDR: u32 = PSRAM_SIZE - 1;

/// Physical address in external PSRAM.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PsramAddr(u32);

impl PsramAddr {
    /// Maximum valid address for the supported 8 MiB PSRAM.
    pub const MAX_ADDR: u32 = MAX_ADDR;

    /// Creates an address if it falls within the supported PSRAM range.
    #[inline]
    pub const fn new(addr: u32) -> Option<Self> {
        if addr <= Self::MAX_ADDR {
            Some(Self(addr))
        } else {
            None
        }
    }

    /// Creates address zero.
    #[inline]
    pub const fn zero() -> Self {
        Self(0)
    }

    /// Returns the raw byte address.
    #[inline]
    pub const fn get(self) -> u32 {
        self.0
    }

    /// Adds a byte offset while checking the PSRAM boundary.
    #[inline]
    pub const fn checked_add(self, offset: u32) -> Option<Self> {
        match self.0.checked_add(offset) {
            Some(next) => Self::new(next),
            None => None,
        }
    }

    /// Checks that a byte range starting at this address fits in PSRAM.
    #[inline]
    pub const fn checked_range_len(self, len: u32) -> Result<(), PsramError> {
        if len == 0 {
            return Ok(());
        }

        let last = match len.checked_sub(1) {
            Some(last) => last,
            None => return Err(PsramError::OutOfRange),
        };

        match self.0.checked_add(last) {
            Some(end) if end <= Self::MAX_ADDR => Ok(()),
            _ => Err(PsramError::OutOfRange),
        }
    }
}

impl From<PsramAddr> for u32 {
    #[inline]
    fn from(addr: PsramAddr) -> Self {
        addr.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_addresses_outside_8_mib() {
        assert_eq!(PsramAddr::new(MAX_ADDR).map(PsramAddr::get), Some(MAX_ADDR));
        assert_eq!(PsramAddr::new(PSRAM_SIZE), None);
    }

    #[test]
    fn checks_range_end_inclusively() {
        assert!(PsramAddr::new(MAX_ADDR)
            .unwrap()
            .checked_range_len(1)
            .is_ok());
        assert_eq!(
            PsramAddr::new(MAX_ADDR).unwrap().checked_range_len(2),
            Err(PsramError::OutOfRange)
        );
        assert!(PsramAddr::zero().checked_range_len(PSRAM_SIZE).is_ok());
    }
}
