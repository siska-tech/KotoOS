//! Raspberry Pi Pico 2 W / RP2350A installed in a PicoCalc.

use super::{BoardCapabilities, BoardProfile};

pub const PROFILE: BoardProfile = BoardProfile {
    board_id: "picocalc-pico2w-rp2350a",
    mcu_id: "rp2350a",
    flash_bytes: 4 * 1024 * 1024,
    sram_bytes: 520 * 1024,
    module_psram_bytes: 0,
    default_system_hz: 150_000_000,
    lcd_spi_hz: 37_500_000,
    psram_pio_divider: 5,
    code_window_tiles: 3,
    capabilities: BoardCapabilities::AUDIO
        .union(BoardCapabilities::WIFI)
        .union(BoardCapabilities::AUDIO_WIFI_CONCURRENT),
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
const _: () = assert!(PROFILE.capabilities.contains(BoardCapabilities::WIFI));
const _: () = assert!(AUDIO_WIFI_CONCURRENT);
