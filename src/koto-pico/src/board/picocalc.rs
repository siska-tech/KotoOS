//! ClockworkPi PicoCalc carrier wiring shared by supported plug-in modules.

use embassy_rp::{peripherals, Peri, Peripherals};

pub type PsramPio = peripherals::PIO1;
pub type PsramCsPin = peripherals::PIN_20;
pub type PsramSckPin = peripherals::PIN_21;
pub type PsramSio0Pin = peripherals::PIN_2;
pub type PsramSio1Pin = peripherals::PIN_3;
pub type PsramSio2Pin = peripherals::PIN_4;
pub type PsramSio3Pin = peripherals::PIN_5;
#[cfg(any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"))]
pub type RadioPowerPin = peripherals::PIN_23;
#[cfg(any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"))]
pub type RadioDataPin = peripherals::PIN_24;
#[cfg(any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"))]
pub type RadioCsPin = peripherals::PIN_25;
#[cfg(any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"))]
pub type RadioClockPin = peripherals::PIN_29;

#[cfg(any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"))]
pub struct PicoWRadioResources {
    pub pio: Peri<'static, peripherals::PIO0>,
    pub power: Peri<'static, RadioPowerPin>,
    pub data: Peri<'static, RadioDataPin>,
    pub cs: Peri<'static, RadioCsPin>,
    pub clock: Peri<'static, RadioClockPin>,
    pub dma_tx: Peri<'static, peripherals::DMA_CH2>,
    pub dma_rx: Peri<'static, peripherals::DMA_CH3>,
}

#[cfg(all(
    any(
        feature = "wifi_residency_probe",
        all(feature = "network_service", feature = "board-picocalc-picow")
    ),
    any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w")
))]
impl PicoWRadioResources {
    /// # Safety
    ///
    /// The caller must have proven that no user of the previous alias remains
    /// alive: the arena-owned CYW43 lifecycle future must have been dropped
    /// and joined (`WifiRuntime::shutdown` returned the arena) so PIO0,
    /// DMA_CH2, DMA_CH3, and the GP23/24/25/29 pins are all dead before the
    /// returned alias is used.
    unsafe fn duplicate(&self) -> Self {
        unsafe {
            Self {
                pio: self.pio.clone_unchecked(),
                power: self.power.clone_unchecked(),
                data: self.data.clone_unchecked(),
                cs: self.cs.clone_unchecked(),
                clock: self.clock.clone_unchecked(),
                dma_tx: self.dma_tx.clone_unchecked(),
                dma_rx: self.dma_rx.clone_unchecked(),
            }
        }
    }

    /// Duplicates the radio peripheral handles for the next KOTO-0227 probe
    /// round trip.
    ///
    /// # Safety
    ///
    /// Same contract as [`Self::duplicate`]: the previous lifecycle future must
    /// have been dropped and joined and `RadioPowerOutput` must have forced
    /// GP23 low before the returned alias is used.
    #[cfg(feature = "wifi_residency_probe")]
    pub unsafe fn clone_for_probe(&self) -> Self {
        unsafe { self.duplicate() }
    }

    /// Duplicates the radio peripheral handles for the next KOTO-0251 Pico W
    /// product radio-enable cycle. RP2040's switchable residency installs a
    /// fresh arena-owned CYW43 lifecycle on every radio enable, so the product
    /// mode owner needs the same alias the probe soak used.
    ///
    /// # Safety
    ///
    /// Same contract as [`Self::duplicate`]: the previous lifecycle future must
    /// have been dropped and joined (`WifiRuntime::shutdown` returned the
    /// arena, or no lifecycle was ever installed) and GP23 forced low before
    /// the returned alias is used.
    #[cfg(all(feature = "network_service", feature = "board-picocalc-picow"))]
    pub unsafe fn clone_for_enable_cycle(&self) -> Self {
        unsafe { self.duplicate() }
    }
}

#[cfg(feature = "board-picocalc-pico2w")]
pub struct MinimalRadioProbeResources {
    pub uart: Peri<'static, peripherals::UART0>,
    pub uart_tx: Peri<'static, peripherals::PIN_0>,
    pub pio: Peri<'static, peripherals::PIO0>,
    pub power: Peri<'static, RadioPowerPin>,
    pub data: Peri<'static, RadioDataPin>,
    pub cs: Peri<'static, RadioCsPin>,
    pub clock: Peri<'static, RadioClockPin>,
    #[cfg(not(feature = "wifi_minimal_dma23_probe"))]
    pub dma_tx: Peri<'static, peripherals::DMA_CH0>,
    #[cfg(not(feature = "wifi_minimal_dma23_probe"))]
    pub dma_rx: Peri<'static, peripherals::DMA_CH1>,
    #[cfg(feature = "wifi_minimal_dma23_probe")]
    pub dma_tx: Peri<'static, peripherals::DMA_CH2>,
    #[cfg(feature = "wifi_minimal_dma23_probe")]
    pub dma_rx: Peri<'static, peripherals::DMA_CH3>,
}

#[cfg(feature = "board-picocalc-pico2w")]
pub fn split_minimal_radio_probe(p: Peripherals) -> MinimalRadioProbeResources {
    MinimalRadioProbeResources {
        uart: p.UART0,
        uart_tx: p.PIN_0,
        pio: p.PIO0,
        power: p.PIN_23,
        data: p.PIN_24,
        cs: p.PIN_25,
        clock: p.PIN_29,
        #[cfg(not(feature = "wifi_minimal_dma23_probe"))]
        dma_tx: p.DMA_CH0,
        #[cfg(not(feature = "wifi_minimal_dma23_probe"))]
        dma_rx: p.DMA_CH1,
        #[cfg(feature = "wifi_minimal_dma23_probe")]
        dma_tx: p.DMA_CH2,
        #[cfg(feature = "wifi_minimal_dma23_probe")]
        dma_rx: p.DMA_CH3,
    }
}

/// GPIO numbers and peripheral instances fixed by the PicoCalc carrier.
///
/// Embassy peripheral ownership remains type-safe at each board entry point;
/// these numeric values are the single source of truth for raw-PAC helpers,
/// diagnostics, and future board adapters.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PicoCalcWiring {
    pub uart_tx: u8,
    pub psram_sio: [u8; 4],
    pub psram_cs: u8,
    pub psram_sck: u8,
    pub keyboard_sda: u8,
    pub keyboard_scl: u8,
    pub lcd_sck: u8,
    pub lcd_mosi: u8,
    pub lcd_cs: u8,
    pub lcd_dc: u8,
    pub lcd_reset: u8,
    pub sd_miso: u8,
    pub sd_cs: u8,
    pub sd_sck: u8,
    pub sd_mosi: u8,
    pub sd_detect: u8,
    pub audio_a: u8,
    pub audio_b: u8,
}

pub const PICOCALC_WIRING: PicoCalcWiring = PicoCalcWiring {
    uart_tx: 0,
    psram_sio: [2, 3, 4, 5],
    psram_cs: 20,
    psram_sck: 21,
    keyboard_sda: 7,
    keyboard_scl: 6,
    lcd_sck: 10,
    lcd_mosi: 11,
    lcd_cs: 13,
    lcd_dc: 14,
    lcd_reset: 15,
    sd_miso: 16,
    sd_cs: 17,
    sd_sck: 18,
    sd_mosi: 19,
    sd_detect: 22,
    audio_a: 26,
    audio_b: 27,
};

/// Product-firmware peripherals named by function rather than RP GPIO number.
///
/// This is deliberately a carrier adapter: product code consumes these names,
/// while the only `PIN_n` mapping lives here. A future carrier can expose the
/// same semantic fields with different concrete Embassy peripheral types.
#[allow(missing_docs)]
pub struct PicoCalcPeripherals {
    pub uart: Peri<'static, peripherals::UART0>,
    pub uart_tx: Peri<'static, peripherals::PIN_0>,
    pub lcd_spi: Peri<'static, peripherals::SPI1>,
    pub lcd_sck: Peri<'static, peripherals::PIN_10>,
    pub lcd_mosi: Peri<'static, peripherals::PIN_11>,
    pub dma_ch0: Peri<'static, peripherals::DMA_CH0>,
    pub lcd_cs: Peri<'static, peripherals::PIN_13>,
    pub lcd_dc: Peri<'static, peripherals::PIN_14>,
    pub lcd_reset: Peri<'static, peripherals::PIN_15>,
    pub keyboard_i2c: Peri<'static, peripherals::I2C1>,
    pub keyboard_sda: Peri<'static, peripherals::PIN_7>,
    pub keyboard_scl: Peri<'static, peripherals::PIN_6>,
    pub audio_pwm: Peri<'static, peripherals::PWM_SLICE5>,
    pub audio_a: Peri<'static, peripherals::PIN_26>,
    pub audio_b: Peri<'static, peripherals::PIN_27>,
    pub core1: Peri<'static, peripherals::CORE1>,
    pub usb: Peri<'static, peripherals::USB>,
    pub sd_spi: Peri<'static, peripherals::SPI0>,
    pub sd_sck: Peri<'static, peripherals::PIN_18>,
    pub sd_mosi: Peri<'static, peripherals::PIN_19>,
    pub sd_miso: Peri<'static, peripherals::PIN_16>,
    pub sd_cs: Peri<'static, peripherals::PIN_17>,
    pub sd_detect: Peri<'static, peripherals::PIN_22>,
    pub psram_pio: Peri<'static, peripherals::PIO1>,
    pub psram_cs: Peri<'static, peripherals::PIN_20>,
    pub psram_sck: Peri<'static, peripherals::PIN_21>,
    pub psram_sio0: Peri<'static, peripherals::PIN_2>,
    pub psram_sio1: Peri<'static, peripherals::PIN_3>,
    pub psram_sio2: Peri<'static, peripherals::PIN_4>,
    pub psram_sio3: Peri<'static, peripherals::PIN_5>,
    pub psram_rx_dma: Peri<'static, peripherals::DMA_CH1>,
    #[cfg(any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"))]
    pub radio_pio: Peri<'static, peripherals::PIO0>,
    #[cfg(any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"))]
    pub radio_power: Peri<'static, peripherals::PIN_23>,
    #[cfg(any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"))]
    pub radio_data: Peri<'static, peripherals::PIN_24>,
    #[cfg(any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"))]
    pub radio_cs: Peri<'static, peripherals::PIN_25>,
    #[cfg(any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"))]
    pub radio_clock: Peri<'static, peripherals::PIN_29>,
    #[cfg(any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"))]
    pub radio_dma: Peri<'static, peripherals::DMA_CH2>,
    #[cfg(any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"))]
    pub radio_dma_rx: Peri<'static, peripherals::DMA_CH3>,
}

/// Convert Embassy's chip-oriented peripheral set into PicoCalc roles.
pub fn split_peripherals(p: Peripherals) -> PicoCalcPeripherals {
    PicoCalcPeripherals {
        uart: p.UART0,
        uart_tx: p.PIN_0,
        lcd_spi: p.SPI1,
        lcd_sck: p.PIN_10,
        lcd_mosi: p.PIN_11,
        dma_ch0: p.DMA_CH0,
        lcd_cs: p.PIN_13,
        lcd_dc: p.PIN_14,
        lcd_reset: p.PIN_15,
        keyboard_i2c: p.I2C1,
        keyboard_sda: p.PIN_7,
        keyboard_scl: p.PIN_6,
        audio_pwm: p.PWM_SLICE5,
        audio_a: p.PIN_26,
        audio_b: p.PIN_27,
        core1: p.CORE1,
        usb: p.USB,
        sd_spi: p.SPI0,
        sd_sck: p.PIN_18,
        sd_mosi: p.PIN_19,
        sd_miso: p.PIN_16,
        sd_cs: p.PIN_17,
        sd_detect: p.PIN_22,
        psram_pio: p.PIO1,
        psram_cs: p.PIN_20,
        psram_sck: p.PIN_21,
        psram_sio0: p.PIN_2,
        psram_sio1: p.PIN_3,
        psram_sio2: p.PIN_4,
        psram_sio3: p.PIN_5,
        psram_rx_dma: p.DMA_CH1,
        #[cfg(any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"))]
        radio_pio: p.PIO0,
        #[cfg(any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"))]
        radio_power: p.PIN_23,
        #[cfg(any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"))]
        radio_data: p.PIN_24,
        #[cfg(any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"))]
        radio_cs: p.PIN_25,
        #[cfg(any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"))]
        radio_clock: p.PIN_29,
        #[cfg(any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"))]
        radio_dma: p.DMA_CH2,
        #[cfg(any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"))]
        radio_dma_rx: p.DMA_CH3,
    }
}
