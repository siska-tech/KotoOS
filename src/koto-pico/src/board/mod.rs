//! Selected hardware-board profile.
//!
//! MCU features choose the Embassy/PAC implementation. Board features choose
//! the physical module, memory capacities, clocks, and carrier wiring. Keep
//! application/runtime code dependent on this module instead of adding board
//! feature checks throughout the firmware.

mod picocalc;

#[cfg(feature = "board-picocalc-pico")]
mod picocalc_pico;
#[cfg(feature = "board-picocalc-pico")]
pub use picocalc_pico::*;

#[cfg(feature = "board-picocalc-picow")]
mod picocalc_picow;
#[cfg(feature = "board-picocalc-picow")]
pub use picocalc_picow::*;

#[cfg(feature = "board-picocalc-pico2w")]
mod picocalc_pico2w;
#[cfg(feature = "board-picocalc-pico2w")]
pub use picocalc_pico2w::*;

#[cfg(feature = "board-picocalc-pico2w")]
pub use picocalc::{split_minimal_radio_probe, MinimalRadioProbeResources};
pub use picocalc::{
    split_peripherals, PicoCalcPeripherals, PicoCalcWiring, PsramCsPin, PsramPio, PsramSckPin,
    PsramSio0Pin, PsramSio1Pin, PsramSio2Pin, PsramSio3Pin, PICOCALC_WIRING,
};
#[cfg(any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"))]
pub use picocalc::{PicoWRadioResources, RadioPowerPin};

/// Explicit service-residency capabilities declared by a board profile.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BoardCapabilities(u8);

impl BoardCapabilities {
    pub const AUDIO: Self = Self(1 << 0);
    pub const WIFI: Self = Self(1 << 1);
    pub const AUDIO_WIFI_CONCURRENT: Self = Self(1 << 2);

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn contains(self, capability: Self) -> bool {
        self.0 & capability.0 == capability.0
    }
}

/// Values that describe one complete, compile-time-selected board profile.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BoardProfile {
    pub board_id: &'static str,
    pub mcu_id: &'static str,
    pub flash_bytes: usize,
    pub sram_bytes: usize,
    pub module_psram_bytes: usize,
    pub default_system_hz: u32,
    pub lcd_spi_hz: u32,
    pub psram_pio_divider: u32,
    pub code_window_tiles: usize,
    pub capabilities: BoardCapabilities,
}
