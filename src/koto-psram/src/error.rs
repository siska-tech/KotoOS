//! Error types shared by production and diagnostic APIs.

use crate::addr::PsramAddr;

/// A byte mismatch observed during verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mismatch {
    /// Absolute PSRAM address where the mismatch was detected.
    pub addr: PsramAddr,
    /// Expected byte value.
    pub expected: u8,
    /// Actual byte value.
    pub actual: u8,
}

/// Driver-level error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PsramError {
    /// Address or byte count exceeds the supported PSRAM range.
    OutOfRange,
    /// A blocking operation did not complete before its poll budget expired.
    Timeout,
    /// The device returned data different from the expected pattern.
    Mismatch(Mismatch),
    /// Hardware returned an invalid or unsupported identity.
    UnsupportedDevice,
    /// The PIO backend reported a low-level fault.
    HardwareFault,
    /// The operation is not valid for the current driver state.
    InvalidState,
}
