//! Device probing helpers.

pub use crate::device::DeviceId;

/// Result of a diagnostic probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProbeReport {
    /// Device identity observed over SPI.
    pub id: DeviceId,
    /// Whether the identity looked electrically plausible.
    pub present: bool,
    /// Whether QPI entry was attempted after probing.
    pub qpi_entered: bool,
}

impl ProbeReport {
    /// Creates a report from a raw ID and QPI entry status.
    pub const fn new(id: DeviceId, qpi_entered: bool) -> Self {
        Self {
            id,
            present: id.looks_present(),
            qpi_entered,
        }
    }
}
