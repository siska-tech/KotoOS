//! Clock and dummy-cycle sweep descriptions.

use crate::config::TimingConfig;

/// Inclusive sweep range.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SweepRange {
    /// First clock divider to test.
    pub min_clkdiv: f32,
    /// Last clock divider to test.
    pub max_clkdiv: f32,
    /// Clock divider step.
    pub clkdiv_step: f32,
    /// First dummy-cycle count.
    pub min_dummy_cycles: u8,
    /// Last dummy-cycle count.
    pub max_dummy_cycles: u8,
}

impl SweepRange {
    /// Design-document default sweep.
    pub const DEFAULT: Self = Self {
        min_clkdiv: 2.0,
        max_clkdiv: 10.0,
        clkdiv_step: 1.0,
        min_dummy_cycles: 2,
        max_dummy_cycles: 8,
    };

    /// Builds a timing config for a read sweep point.
    pub fn read_timing(self, clkdiv: f32, dummy_cycles: u8) -> TimingConfig {
        TimingConfig {
            read_clkdiv: clkdiv,
            fallback_read_clkdiv: (clkdiv * 2.0).max(TimingConfig::DEFAULT.fallback_read_clkdiv),
            read_dummy_cycles: dummy_cycles,
            ..TimingConfig::DEFAULT
        }
    }
}

/// Result for one sweep point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SweepPoint {
    /// Tested read clock divider multiplied by 10.
    pub clkdiv_x10: u16,
    /// Tested read dummy cycles.
    pub dummy_cycles: u8,
    /// Number of byte mismatches observed.
    pub errors: u32,
}
