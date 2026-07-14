//! PicoCalc LCD profile and DMA-backed probe driver.
//!
//! The first profile follows ClockworkPi's ILI9488 reference firmware. It uses
//! RGB666 on the wire because that is the controller's reliable SPI format.

use embassy_rp::{
    gpio::Output,
    peripherals,
    spi::{Async, Error as SpiError, Spi},
};
use embassy_time::Timer;

const CASET: u8 = 0x2a;
const PASET: u8 = 0x2b;
const RAMWR: u8 = 0x2c;
const MADCTL: u8 = 0x36;
const COLMOD: u8 = 0x3a;
const SLPOUT: u8 = 0x11;
const DISPON: u8 = 0x29;

/// One LCD initialization command and its optional post-command delay.
pub struct InitCommand {
    pub command: u8,
    pub data: &'static [u8],
    pub delay_ms: u64,
}

/// Static controller behavior selected by the embedded backend.
pub struct LcdProfile {
    pub name: &'static str,
    pub width: u16,
    pub height: u16,
    pub x_offset: u16,
    pub y_offset: u16,
    pub reset_low_us: u64,
    pub reset_high_us: u64,
    pub spi_hz: u32,
    pub madctl: u8,
    pub colmod: u8,
    pub init_commands: &'static [InitCommand],
}

static ILI9488_INIT: &[InitCommand] = &[
    InitCommand {
        command: 0xe0,
        data: &[
            0x00, 0x03, 0x09, 0x08, 0x16, 0x0a, 0x3f, 0x78, 0x4c, 0x09, 0x0a, 0x08, 0x16, 0x1a,
            0x0f,
        ],
        delay_ms: 0,
    },
    InitCommand {
        command: 0xe1,
        data: &[
            0x00, 0x16, 0x19, 0x03, 0x0f, 0x05, 0x32, 0x45, 0x46, 0x04, 0x0e, 0x0d, 0x35, 0x37,
            0x0f,
        ],
        delay_ms: 0,
    },
    InitCommand {
        command: 0xc0,
        data: &[0x17, 0x15],
        delay_ms: 0,
    },
    InitCommand {
        command: 0xc1,
        data: &[0x41],
        delay_ms: 0,
    },
    InitCommand {
        command: 0xc5,
        data: &[0x00, 0x12, 0x80],
        delay_ms: 0,
    },
    InitCommand {
        command: MADCTL,
        data: &[0x48],
        delay_ms: 0,
    },
    InitCommand {
        command: COLMOD,
        data: &[0x66],
        delay_ms: 0,
    },
    InitCommand {
        command: 0xb0,
        data: &[0x00],
        delay_ms: 0,
    },
    InitCommand {
        command: 0xb1,
        data: &[0xa0],
        delay_ms: 0,
    },
    InitCommand {
        command: 0x21,
        data: &[],
        delay_ms: 0,
    },
    InitCommand {
        command: 0xb4,
        data: &[0x02],
        delay_ms: 0,
    },
    InitCommand {
        command: 0xb6,
        data: &[0x02, 0x02, 0x3b],
        delay_ms: 0,
    },
    InitCommand {
        command: 0xb7,
        data: &[0xc6],
        delay_ms: 0,
    },
    InitCommand {
        command: 0xe9,
        data: &[0x00],
        delay_ms: 0,
    },
    InitCommand {
        command: 0xf7,
        data: &[0xa9, 0x51, 0x2c, 0x82],
        delay_ms: 0,
    },
    InitCommand {
        command: SLPOUT,
        data: &[],
        delay_ms: 120,
    },
    InitCommand {
        command: DISPON,
        data: &[],
        delay_ms: 120,
    },
];

/// Initial manual profile for the common PicoCalc ILI9488 panel.
pub static ILI9488_SPI: LcdProfile = LcdProfile {
    name: "ili9488-spi",
    width: 320,
    height: 320,
    x_offset: 0,
    y_offset: 0,
    reset_low_us: 10_000,
    reset_high_us: 200_000,
    // RP2040 keeps its hardware-validated 62.5 MHz rate. RP2350A begins at a
    // conservative, exactly-derived 37.5 MHz until KOTO-0205 device captures
    // establish the panel/DMA ceiling for that MCU.
    spi_hz: crate::board::LCD_SPI_HZ,
    madctl: 0x48,
    colmod: 0x66,
    init_commands: ILI9488_INIT,
};

#[derive(Clone, Copy)]
pub struct Rgb888 {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

impl Rgb888 {
    pub const BLACK: Self = Self::new(0x00, 0x00, 0x00);
    pub const WHITE: Self = Self::new(0xff, 0xff, 0xff);
    pub const RED: Self = Self::new(0xff, 0x00, 0x00);
    pub const GREEN: Self = Self::new(0x00, 0xff, 0x00);
    pub const BLUE: Self = Self::new(0x00, 0x00, 0xff);
    pub const YELLOW: Self = Self::new(0xff, 0xff, 0x00);
    pub const CYAN: Self = Self::new(0x00, 0xff, 0xff);

    pub const fn new(red: u8, green: u8, blue: u8) -> Self {
        Self { red, green, blue }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LcdError {
    InvalidRectangle,
    Spi,
}

impl From<SpiError> for LcdError {
    fn from(_: SpiError) -> Self {
        Self::Spi
    }
}

/// DMA-backed LCD transport for PicoCalc SPI1.
pub struct PicoCalcLcd<'d> {
    spi: Spi<'d, peripherals::SPI1, Async>,
    cs: Output<'d>,
    dc: Output<'d>,
    reset: Output<'d>,
    profile: &'static LcdProfile,
}

impl<'d> PicoCalcLcd<'d> {
    pub fn new(
        spi: Spi<'d, peripherals::SPI1, Async>,
        cs: Output<'d>,
        dc: Output<'d>,
        reset: Output<'d>,
        profile: &'static LcdProfile,
    ) -> Self {
        Self {
            spi,
            cs,
            dc,
            reset,
            profile,
        }
    }

    pub fn profile(&self) -> &'static LcdProfile {
        self.profile
    }

    pub async fn init(&mut self) -> Result<(), LcdError> {
        self.cs.set_high();
        self.dc.set_high();
        self.reset.set_high();
        Timer::after_micros(10_000).await;
        self.reset.set_low();
        Timer::after_micros(self.profile.reset_low_us).await;
        self.reset.set_high();
        Timer::after_micros(self.profile.reset_high_us).await;

        for entry in self.profile.init_commands {
            self.command(entry.command, entry.data).await?;
            if entry.delay_ms != 0 {
                Timer::after_millis(entry.delay_ms).await;
            }
        }
        Ok(())
    }

    pub async fn fill(&mut self, color: Rgb888) -> Result<(), LcdError> {
        self.fill_rect(0, 0, self.profile.width, self.profile.height, color)
            .await
    }

    /// Fill one bounded address window. Pixel rows are transferred with DMA.
    pub async fn fill_rect(
        &mut self,
        x: u16,
        y: u16,
        width: u16,
        height: u16,
        color: Rgb888,
    ) -> Result<(), LcdError> {
        if width == 0
            || height == 0
            || x.checked_add(width).is_none()
            || y.checked_add(height).is_none()
            || x + width > self.profile.width
            || y + height > self.profile.height
        {
            return Err(LcdError::InvalidRectangle);
        }

        let mut scanline = [0u8; 320 * 3];
        let row_bytes = width as usize * 3;
        for pixel in scanline[..row_bytes].chunks_exact_mut(3) {
            pixel[0] = color.red;
            pixel[1] = color.green;
            pixel[2] = color.blue;
        }

        self.set_window(x, y, width, height).await?;
        self.cs.set_low();
        self.dc.set_low();
        self.spi.write(&[RAMWR]).await?;
        self.dc.set_high();
        for _ in 0..height {
            self.spi.write(&scanline[..row_bytes]).await?;
        }
        self.cs.set_high();
        Ok(())
    }

    /// Transfer a bounded little-endian RGB565 window, converting it to the
    /// ILI9488's validated RGB666 byte stream.
    ///
    /// The whole window is converted into `scratch` and pushed with a single
    /// DMA transfer. A per-scanline write was measured at ~96 µs/row of fixed
    /// DMA-setup overhead on the PicoCalc; batching the window removes that cost
    /// for dirty-rectangle redraws (KOTO-0120). `scratch` must hold at least
    /// `width * height * 3` bytes.
    pub async fn write_rgb565_rect(
        &mut self,
        x: u16,
        y: u16,
        width: u16,
        height: u16,
        pixels: &[u8],
        scratch: &mut [u8],
    ) -> Result<(), LcdError> {
        // The convert (CPU) and transfer (SPI DMA) halves are split out
        // (KOTO-0174 H-A) so the present path can time each and, in the pipeline
        // variant, convert rect N+1 while rect N's RGB666 is on the DMA. This
        // wrapper keeps the original one-shot semantics for the boot / shell /
        // banded callers.
        if convert_rgb565_to_rgb666(pixels, scratch, width, height).is_none() {
            return Err(LcdError::InvalidRectangle);
        }
        self.transfer_rgb666(x, y, width, height, scratch).await
    }

    /// Push an already-converted RGB666 window (`rgb666[..w*h*3]`) to GRAM with a
    /// single DMA transfer. The SPI (`set_window` + `RAMWR` + data) is the only
    /// half that touches the bus, so the pipeline can run it concurrently with a
    /// CPU raster+convert of the next rect (KOTO-0174 H-A).
    pub async fn transfer_rgb666(
        &mut self,
        x: u16,
        y: u16,
        width: u16,
        height: u16,
        rgb666: &[u8],
    ) -> Result<(), LcdError> {
        let target_bytes = width as usize * height as usize * 3;
        if width == 0
            || height == 0
            || x.checked_add(width).is_none()
            || y.checked_add(height).is_none()
            || x + width > self.profile.width
            || y + height > self.profile.height
            || rgb666.len() < target_bytes
        {
            return Err(LcdError::InvalidRectangle);
        }
        self.set_window(x, y, width, height).await?;
        self.cs.set_low();
        self.dc.set_low();
        self.spi.write(&[RAMWR]).await?;
        self.dc.set_high();
        self.spi.write(&rgb666[..target_bytes]).await?;
        self.cs.set_high();
        Ok(())
    }

    /// Diag-only re-split of [`transfer_rgb666`](Self::transfer_rgb666)
    /// (KOTO-0174 re-investigation): open the GRAM window, issue `RAMWR`, and
    /// leave CS asserted with DC=data so [`write_rgb666_data`](Self::write_rgb666_data)
    /// can stream the pixel bytes as a separate future. This is the H-A
    /// pipeline's `begin` primitive, re-added for the boot-time SPI microbench
    /// (`phase=178`/`phase=179`) — the production present path still uses the
    /// fused `transfer_rgb666`.
    pub async fn begin_rgb666(
        &mut self,
        x: u16,
        y: u16,
        width: u16,
        height: u16,
    ) -> Result<(), LcdError> {
        if width == 0
            || height == 0
            || x.checked_add(width).is_none()
            || y.checked_add(height).is_none()
            || x + width > self.profile.width
            || y + height > self.profile.height
        {
            return Err(LcdError::InvalidRectangle);
        }
        self.set_window(x, y, width, height).await?;
        self.cs.set_low();
        self.dc.set_low();
        self.spi.write(&[RAMWR]).await?;
        self.dc.set_high();
        Ok(())
    }

    /// Stream already-converted RGB666 bytes into the window opened by
    /// [`begin_rgb666`](Self::begin_rgb666). Exactly one `Spi::write` DMA
    /// future — the thing the `phase=178` overlap bench races against CPU
    /// work. Call [`end_rgb666`](Self::end_rgb666) when the window is done.
    pub async fn write_rgb666_data(&mut self, rgb666: &[u8]) -> Result<(), LcdError> {
        self.spi.write(rgb666).await?;
        Ok(())
    }

    /// Close the window opened by [`begin_rgb666`](Self::begin_rgb666).
    pub fn end_rgb666(&mut self) {
        self.cs.set_high();
    }

    async fn set_window(
        &mut self,
        x: u16,
        y: u16,
        width: u16,
        height: u16,
    ) -> Result<(), LcdError> {
        let x0 = x + self.profile.x_offset;
        let x1 = x0 + width - 1;
        let y0 = y + self.profile.y_offset;
        let y1 = y0 + height - 1;
        self.command(
            CASET,
            &[(x0 >> 8) as u8, x0 as u8, (x1 >> 8) as u8, x1 as u8],
        )
        .await?;
        self.command(
            PASET,
            &[(y0 >> 8) as u8, y0 as u8, (y1 >> 8) as u8, y1 as u8],
        )
        .await
    }

    async fn command(&mut self, command: u8, data: &[u8]) -> Result<(), LcdError> {
        self.cs.set_low();
        self.dc.set_low();
        self.spi.write(&[command]).await?;
        if !data.is_empty() {
            self.dc.set_high();
            self.spi.write(data).await?;
        }
        self.cs.set_high();
        Ok(())
    }
}

// The CPU half of `write_rgb565_rect` (KOTO-0174 H-A split), overlapped with
// the previous band's DMA by the H-A2 pipeline. The implementation moved to
// koto-gfx for the H-D byte-algebra optimization so its byte-exactness proof
// is host-testable (koto-pico cannot run host tests); re-exported here so the
// firmware call sites are unchanged.
pub use koto_gfx::convert_rgb565_to_rgb666;
