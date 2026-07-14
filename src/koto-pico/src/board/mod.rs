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

#[cfg(feature = "board-picocalc-pico2w")]
mod picocalc_pico2w;
#[cfg(feature = "board-picocalc-pico2w")]
pub use picocalc_pico2w::*;

pub use picocalc::{
    split_peripherals, PicoCalcPeripherals, PicoCalcWiring, PsramCsPin, PsramPio, PsramSckPin,
    PsramSio0Pin, PsramSio1Pin, PsramSio2Pin, PsramSio3Pin, PICOCALC_WIRING,
};

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
    pub has_wireless_radio: bool,
}
