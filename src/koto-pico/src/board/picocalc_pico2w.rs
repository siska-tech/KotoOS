//! Raspberry Pi Pico 2 W / RP2350A installed in a PicoCalc.

use super::BoardProfile;

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
    has_wireless_radio: true,
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
pub const HAS_WIRELESS_RADIO: bool = PROFILE.has_wireless_radio;
