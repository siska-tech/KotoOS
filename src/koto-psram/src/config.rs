//! Pin and timing configuration.

/// PicoCalc PSRAM pin assignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pins {
    /// QPI SIO0 / SPI MOSI.
    pub sio0: u8,
    /// QPI SIO1 / SPI MISO.
    pub sio1: u8,
    /// QPI SIO2.
    pub sio2: u8,
    /// QPI SIO3.
    pub sio3: u8,
    /// Active-low chip select.
    pub cs: u8,
    /// Serial clock.
    pub sck: u8,
}

impl Pins {
    /// PicoCalc GPIO mapping documented for RP2040 boards.
    pub const PICOCALC: Self = Self {
        sio0: 2,
        sio1: 3,
        sio2: 4,
        sio3: 5,
        cs: 20,
        sck: 21,
    };

    /// Validates that data and side-set pins are contiguous as expected by PIO.
    #[inline]
    pub const fn validate(self) -> bool {
        self.sio1 == self.sio0 + 1
            && self.sio2 == self.sio0 + 2
            && self.sio3 == self.sio0 + 3
            && self.sck == self.cs + 1
    }
}

/// Clock and dummy-cycle configuration for blocking PIO transfers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimingConfig {
    /// Clock divider for reads.
    pub read_clkdiv: f32,
    /// Clock divider for writes.
    pub write_clkdiv: f32,
    /// Conservative read divider used after a transfer error.
    pub fallback_read_clkdiv: f32,
    /// QPI read dummy cycles.
    pub read_dummy_cycles: u8,
    /// QPI write dummy cycles.
    pub write_dummy_cycles: u8,
    /// Poll budget for blocking FIFO loops.
    pub timeout_polls: u32,
    /// Maximum byte count per blocking QPI transaction.
    pub max_chunk_len: usize,
}

impl TimingConfig {
    /// PicoCalc hardware-passing safe profile from Phase 3 MVP bring-up.
    ///
    /// This profile passed hardware compare on PicoCalc with lengths 1, 31,
    /// 32, 255, 256, 257, and 512 using repeated, walking-byte, and
    /// address-derived patterns. It also preserves driver-level chunk splitting
    /// through `max_chunk_len`.
    pub const PICOCALC_SAFE: Self = Self {
        read_clkdiv: 4.0,
        write_clkdiv: 2.0,
        fallback_read_clkdiv: 8.0,
        read_dummy_cycles: 6,
        write_dummy_cycles: 0,
        timeout_polls: 100_000,
        max_chunk_len: 256,
    };

    /// Candidate profile for extended PicoCalc chunk-length benchmarking.
    ///
    /// This keeps the Phase 3 safe clocking and dummy-cycle values unchanged,
    /// but raises `max_chunk_len` to the Phase 4 hardware-passing candidate.
    pub const PICOCALC_FAST_CANDIDATE: Self = Self {
        max_chunk_len: 512,
        ..Self::PICOCALC_SAFE
    };

    /// Conservative defaults for early PicoCalc bring-up.
    pub const DEFAULT: Self = Self {
        ..Self::PICOCALC_SAFE
    };

    /// Validates basic timing invariants.
    #[inline]
    pub const fn validate(self) -> bool {
        self.read_clkdiv >= 1.0
            && self.write_clkdiv >= 1.0
            && self.fallback_read_clkdiv >= self.read_clkdiv
            && self.read_dummy_cycles <= 16
            && self.write_dummy_cycles <= 16
            && self.timeout_polls > 0
            && self.max_chunk_len > 0
    }
}

impl Default for TimingConfig {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picocalc_pins_match_pio_contiguity_assumption() {
        assert!(Pins::PICOCALC.validate());
    }

    #[test]
    fn default_timing_is_valid() {
        assert!(TimingConfig::DEFAULT.validate());
    }

    #[test]
    fn default_chunk_matches_mvp_granularity() {
        assert_eq!(TimingConfig::DEFAULT.max_chunk_len, 256);
    }

    #[test]
    fn picocalc_safe_matches_phase3_default() {
        assert_eq!(TimingConfig::DEFAULT, TimingConfig::PICOCALC_SAFE);
        assert!(TimingConfig::PICOCALC_SAFE.validate());
    }

    #[test]
    fn picocalc_fast_candidate_only_raises_chunk_len() {
        assert_eq!(TimingConfig::PICOCALC_FAST_CANDIDATE.max_chunk_len, 512);
        assert!(TimingConfig::PICOCALC_FAST_CANDIDATE.validate());

        let safe_with_fast_chunk = TimingConfig {
            max_chunk_len: 512,
            ..TimingConfig::PICOCALC_SAFE
        };
        assert_eq!(TimingConfig::PICOCALC_FAST_CANDIDATE, safe_with_fast_chunk);
    }
}
