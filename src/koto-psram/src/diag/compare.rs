//! Byte-pattern compare helpers for diagnostic binaries.

use crate::{
    addr::PsramAddr,
    bus::{check_access, PsramBus},
    error::Mismatch,
};

/// Error returned by diagnostic compare helpers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareError<E> {
    /// The caller-provided scratch buffer cannot hold the expected pattern.
    ScratchTooSmall,
    /// The requested compare range does not fit in PSRAM.
    InvalidRange,
    /// The underlying bus returned an error.
    Bus(E),
    /// Read-back data did not match the expected pattern.
    Mismatch(Mismatch),
}

/// Writes `expected`, reads it back into `scratch`, and reports the first mismatch.
pub fn compare_pattern<B>(
    bus: &mut B,
    addr: PsramAddr,
    expected: &[u8],
    scratch: &mut [u8],
) -> Result<(), CompareError<B::Error>>
where
    B: PsramBus,
{
    if scratch.len() < expected.len() {
        return Err(CompareError::ScratchTooSmall);
    }
    check_access(addr, expected.len()).map_err(|_| CompareError::InvalidRange)?;

    let scratch = &mut scratch[..expected.len()];
    bus.write_all(addr, expected).map_err(CompareError::Bus)?;
    bus.read_exact(addr, scratch).map_err(CompareError::Bus)?;

    for (offset, (&expected, &actual)) in expected.iter().zip(scratch.iter()).enumerate() {
        if expected != actual {
            let offset = u32::try_from(offset).map_err(|_| CompareError::InvalidRange)?;
            let addr = addr.checked_add(offset).ok_or(CompareError::InvalidRange)?;
            return Err(CompareError::Mismatch(Mismatch {
                addr,
                expected,
                actual,
            }));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{bus::tests::MockBus, error::PsramError};

    struct CorruptingBus {
        inner: MockBus,
    }

    impl PsramBus for CorruptingBus {
        type Error = PsramError;

        fn read_exact(&mut self, addr: PsramAddr, buf: &mut [u8]) -> Result<(), Self::Error> {
            self.inner.read_exact(addr, buf)?;
            if let Some(byte) = buf.get_mut(1) {
                *byte ^= 0xff;
            }
            Ok(())
        }

        fn write_all(&mut self, addr: PsramAddr, data: &[u8]) -> Result<(), Self::Error> {
            self.inner.write_all(addr, data)
        }
    }

    #[test]
    fn compare_pattern_round_trips_expected_bytes() {
        let mut bus = MockBus::new();
        let mut scratch = [0; 4];
        assert_eq!(
            compare_pattern(
                &mut bus,
                PsramAddr::new(8).unwrap(),
                &[0xaa, 0x55, 0x80, 0x01],
                &mut scratch
            ),
            Ok(())
        );
    }

    #[test]
    fn compare_pattern_reports_first_mismatch() {
        let mut bus = CorruptingBus {
            inner: MockBus::new(),
        };
        let mut scratch = [0; 4];
        assert_eq!(
            compare_pattern(
                &mut bus,
                PsramAddr::new(8).unwrap(),
                &[0xaa, 0x55, 0x80, 0x01],
                &mut scratch
            ),
            Err(CompareError::Mismatch(Mismatch {
                addr: PsramAddr::new(9).unwrap(),
                expected: 0x55,
                actual: 0xaa,
            }))
        );
    }

    #[test]
    fn compare_pattern_requires_enough_scratch() {
        let mut bus = MockBus::new();
        let mut scratch = [0; 1];
        assert_eq!(
            compare_pattern(&mut bus, PsramAddr::zero(), &[0xaa, 0x55], &mut scratch),
            Err(CompareError::ScratchTooSmall)
        );
    }
}
