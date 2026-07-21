//! Raspberry Pi Pico / RP2040 installed in a PicoCalc.

use super::{BoardCapabilities, BoardProfile};

pub const PROFILE: BoardProfile = BoardProfile {
    board_id: "picocalc-pico-rp2040",
    mcu_id: "rp2040",
    flash_bytes: 2 * 1024 * 1024,
    sram_bytes: 264 * 1024,
    module_psram_bytes: 0,
    default_system_hz: 125_000_000,
    lcd_spi_hz: 62_500_000,
    psram_pio_divider: 4,
    code_window_tiles: 2,
    capabilities: BoardCapabilities::AUDIO,
};

pub const BOARD_ID: &str = PROFILE.board_id;
pub const MCU_ID: &str = PROFILE.mcu_id;
pub const FLASH_BYTES: usize = PROFILE.flash_bytes;
pub const SRAM_BYTES: usize = PROFILE.sram_bytes;
pub const MODULE_PSRAM_BYTES: usize = PROFILE.module_psram_bytes;
pub const DEFAULT_SYSTEM_HZ: u32 = PROFILE.default_system_hz;
pub const LCD_SPI_HZ: u32 = PROFILE.lcd_spi_hz;
pub const PSRAM_PIO_DIVIDER: u32 = PROFILE.psram_pio_divider;
pub const CODE_WINDOW_TILES: usize = PROFILE.code_window_tiles;
pub const HAS_WIRELESS_RADIO: bool = PROFILE.capabilities.contains(BoardCapabilities::WIFI);
pub const AUDIO_WIFI_CONCURRENT: bool = PROFILE
    .capabilities
    .contains(BoardCapabilities::AUDIO_WIFI_CONCURRENT);

const _: () = assert!(PROFILE.capabilities.contains(BoardCapabilities::AUDIO));
const _: () = assert!(!PROFILE.capabilities.contains(BoardCapabilities::WIFI));
const _: () = assert!(!AUDIO_WIFI_CONCURRENT);
