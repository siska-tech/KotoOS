//! Production PSRAM byte-slice access.

use crate::{addr::PsramAddr, error::PsramError};

/// Blocking production access to PSRAM.
pub trait PsramBus {
    /// Backend-specific error type.
    type Error;

    /// Reads exactly `buf.len()` bytes starting at `addr`.
    fn read_exact(&mut self, addr: PsramAddr, buf: &mut [u8]) -> Result<(), Self::Error>;

    /// Writes all bytes in `data` starting at `addr`.
    fn write_all(&mut self, addr: PsramAddr, data: &[u8]) -> Result<(), Self::Error>;
}

/// Shared bounds check for byte-slice operations.
#[inline]
pub fn check_access(addr: PsramAddr, len: usize) -> Result<(), PsramError> {
    let len = u32::try_from(len).map_err(|_| PsramError::OutOfRange)?;
    addr.checked_range_len(len)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::addr::PSRAM_SIZE;

    pub struct MockBus {
        memory: [u8; 64],
    }

    impl MockBus {
        pub fn new() -> Self {
            Self { memory: [0; 64] }
        }
    }

    impl PsramBus for MockBus {
        type Error = PsramError;

        fn read_exact(&mut self, addr: PsramAddr, buf: &mut [u8]) -> Result<(), Self::Error> {
            check_access(addr, buf.len())?;
            let start = usize::try_from(addr.get()).map_err(|_| PsramError::OutOfRange)?;
            let end = start.checked_add(buf.len()).ok_or(PsramError::OutOfRange)?;
            let src = self.memory.get(start..end).ok_or(PsramError::OutOfRange)?;
            buf.copy_from_slice(src);
            Ok(())
        }

        fn write_all(&mut self, addr: PsramAddr, data: &[u8]) -> Result<(), Self::Error> {
            check_access(addr, data.len())?;
            let start = usize::try_from(addr.get()).map_err(|_| PsramError::OutOfRange)?;
            let end = start
                .checked_add(data.len())
                .ok_or(PsramError::OutOfRange)?;
            let dst = self
                .memory
                .get_mut(start..end)
                .ok_or(PsramError::OutOfRange)?;
            dst.copy_from_slice(data);
            Ok(())
        }
    }

    #[test]
    fn check_access_rejects_wrap_past_end() {
        let addr = PsramAddr::new(PSRAM_SIZE - 2).unwrap();
        assert_eq!(check_access(addr, 3), Err(PsramError::OutOfRange));
    }

    #[test]
    fn mock_bus_round_trips_bytes() {
        let mut bus = MockBus::new();
        let addr = PsramAddr::new(8).unwrap();
        bus.write_all(addr, &[1, 2, 3, 4]).unwrap();
        let mut out = [0; 4];
        bus.read_exact(addr, &mut out).unwrap();
        assert_eq!(out, [1, 2, 3, 4]);
    }
}
