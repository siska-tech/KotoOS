//! Fixed PicoCalc board wiring.
//!
//! Probe and HAL modules take their pin ownership from this map. Keeping the
//! assignments here prevents backend-specific GPIO details from leaking into
//! `koto-core`.

pub struct BoardPins;

impl BoardPins {
    /// Visible LED on the standard Raspberry Pi Pico 1 / Pico 1H.
    ///
    /// Pico W modules route their LED through the wireless chip instead, so
    /// this probe intentionally targets the project's standard Pico 1H first.
    pub const STATUS_LED: u8 = 25;
}

pub struct PicoWRadioPins;

impl PicoWRadioPins {
    pub const POWER: u8 = 23;
    pub const DATA: u8 = 24;
    pub const CS: u8 = 25;
    pub const CLOCK: u8 = 29;
    pub const LED_GPIO: u8 = 0;
}

pub struct LcdPins;

impl LcdPins {
    pub const SCK: u8 = 10;
    pub const MOSI: u8 = 11;
    pub const MISO: u8 = 12;
    pub const CS: u8 = 13;
    pub const DC: u8 = 14;
    pub const RESET: u8 = 15;
}

pub struct KeyboardPins;

impl KeyboardPins {
    pub const SDA: u8 = 6;
    pub const SCL: u8 = 7;
    pub const I2C_ADDRESS: u8 = 0x1f;
}

pub struct SdPins;

impl SdPins {
    pub const MISO: u8 = 16;
    pub const CS: u8 = 17;
    pub const SCK: u8 = 18;
    pub const MOSI: u8 = 19;
    pub const DETECT: u8 = 22;
}

pub struct PsramPins;

impl PsramPins {
    pub const MOSI: u8 = 2;
    pub const MISO: u8 = 3;
    pub const SIO0: u8 = 2;
    pub const SIO1: u8 = 3;
    pub const SIO2: u8 = 4;
    pub const SIO3: u8 = 5;
    pub const QPI_DATA_BASE: u8 = Self::SIO0;
    pub const QPI_DATA_COUNT: u8 = 4;
    pub const CS: u8 = 20;
    pub const SCK: u8 = 21;
}

pub struct AudioPins;

impl AudioPins {
    pub const LEFT: u8 = 26;
    pub const RIGHT: u8 = 27;
}
