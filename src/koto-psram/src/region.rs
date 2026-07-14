//! Safe PSRAM sub-region descriptors.

use crate::{addr::PsramAddr, bus::PsramBus, error::PsramError};

/// A bounded PSRAM sub-region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PsramRegion {
    start: PsramAddr,
    len: u32,
}

impl PsramRegion {
    /// Creates a region if the full range fits in PSRAM.
    pub const fn new(start: PsramAddr, len: u32) -> Result<Self, PsramError> {
        match start.checked_range_len(len) {
            Ok(()) => Ok(Self { start, len }),
            Err(err) => Err(err),
        }
    }

    /// Creates a descriptor for the entire 8 MiB PSRAM.
    pub const fn whole() -> Self {
        Self {
            start: PsramAddr::zero(),
            len: crate::addr::PSRAM_SIZE,
        }
    }

    /// Region start address.
    #[inline]
    pub const fn start(self) -> PsramAddr {
        self.start
    }

    /// Region length in bytes.
    #[inline]
    pub const fn len(self) -> u32 {
        self.len
    }

    /// Returns true when the region is empty.
    #[inline]
    pub const fn is_empty(self) -> bool {
        self.len == 0
    }

    /// Splits the region into two descriptors at `offset`.
    pub const fn split_at(self, offset: u32) -> Option<(Self, Self)> {
        if offset > self.len {
            return None;
        }

        let right_start = match self.start.checked_add(offset) {
            Some(addr) => addr,
            None => return None,
        };

        Some((
            Self {
                start: self.start,
                len: offset,
            },
            Self {
                start: right_start,
                len: self.len - offset,
            },
        ))
    }

    /// Converts a region-relative offset to an absolute PSRAM address.
    pub const fn addr_at(self, offset: u32, len: u32) -> Result<PsramAddr, PsramError> {
        if offset > self.len {
            return Err(PsramError::OutOfRange);
        }

        let remaining = self.len - offset;
        if len > remaining {
            return Err(PsramError::OutOfRange);
        }

        match self.start.checked_add(offset) {
            Some(addr) => Ok(addr),
            None => Err(PsramError::OutOfRange),
        }
    }

    /// Reads bytes through a production bus using a region-relative offset.
    pub fn read_exact<B: PsramBus>(
        self,
        bus: &mut B,
        offset: u32,
        buf: &mut [u8],
    ) -> Result<(), B::Error>
    where
        B::Error: From<PsramError>,
    {
        let len = u32::try_from(buf.len()).map_err(|_| PsramError::OutOfRange)?;
        let addr = self.addr_at(offset, len)?;
        bus.read_exact(addr, buf)
    }

    /// Writes bytes through a production bus using a region-relative offset.
    pub fn write_all<B: PsramBus>(
        self,
        bus: &mut B,
        offset: u32,
        data: &[u8],
    ) -> Result<(), B::Error>
    where
        B::Error: From<PsramError>,
    {
        let len = u32::try_from(data.len()).map_err(|_| PsramError::OutOfRange)?;
        let addr = self.addr_at(offset, len)?;
        bus.write_all(addr, data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::tests::MockBus;

    #[test]
    fn split_preserves_bounds() {
        let region = PsramRegion::new(PsramAddr::new(16).unwrap(), 16).unwrap();
        let (left, right) = region.split_at(6).unwrap();
        assert_eq!(left.start().get(), 16);
        assert_eq!(left.len(), 6);
        assert_eq!(right.start().get(), 22);
        assert_eq!(right.len(), 10);
        assert_eq!(region.split_at(17), None);
    }

    #[test]
    fn region_access_uses_relative_offsets() {
        let mut bus = MockBus::new();
        let region = PsramRegion::new(PsramAddr::new(4).unwrap(), 8).unwrap();
        region.write_all(&mut bus, 2, &[9, 8, 7]).unwrap();
        let mut out = [0; 3];
        region.read_exact(&mut bus, 2, &mut out).unwrap();
        assert_eq!(out, [9, 8, 7]);
    }
}
