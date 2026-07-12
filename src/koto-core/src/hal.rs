// `Rect` is owned by the KotoGFX foundation crate now (KotoGFX v0 extraction).
// Re-exported here so `koto_core::hal::Rect` and `koto_core::Rect` are unchanged.
pub use koto_gfx::Rect;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PixelFormat {
    Rgb565,
    Rgb111Packed,
    Index8,
}

pub struct Surface<'a> {
    pub width: u16,
    pub height: u16,
    pub format: PixelFormat,
    pub stride_bytes: usize,
    pub pixels: &'a [u8],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HalError {
    Unsupported,
    Busy,
    Timeout,
    Io,
    InvalidArgument,
}

pub trait VideoHal {
    fn init(
        &mut self,
        width: u16,
        height: u16,
        preferred_format: PixelFormat,
    ) -> Result<(), HalError>;
    fn begin_frame(&mut self) -> Result<(), HalError>;
    fn update_rect(&mut self, rect: Rect, surface: &Surface<'_>) -> Result<(), HalError>;
    fn update_scanlines(
        &mut self,
        y: u16,
        line_count: u16,
        surface: &Surface<'_>,
    ) -> Result<(), HalError>;
    fn end_frame(&mut self) -> Result<(), HalError>;
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Buttons {
    pub up: bool,
    pub down: bool,
    pub left: bool,
    pub right: bool,
    pub confirm: bool,
    pub cancel: bool,
    pub menu: bool,
    pub action_a: bool,
    pub action_b: bool,
    pub action_x: bool,
    pub action_y: bool,
    pub shift: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct InputState {
    pub held: Buttons,
    pub pressed: Buttons,
    pub released: Buttons,
    pub raw_keycode: Option<u32>,
    pub unicode_codepoint: Option<char>,
}

pub trait InputHal {
    fn init(&mut self) -> Result<(), HalError>;
    fn poll(&mut self) -> Result<InputState, HalError>;
}

pub struct AudioBuffer<'a> {
    pub sample_rate: u32,
    pub channels: u8,
    pub frames: usize,
    pub samples: &'a mut [i16],
}

pub trait AudioSource {
    fn fill(&mut self, buffer: &mut AudioBuffer<'_>) -> Result<(), HalError>;
}

pub trait AudioHal {
    fn init(&mut self, sample_rate: u32, channels: u8) -> Result<(), HalError>;
    fn start(&mut self) -> Result<(), HalError>;
    fn stop(&mut self) -> Result<(), HalError>;
}

pub trait FileHandle {
    fn read(&mut self, dst: &mut [u8]) -> Result<usize, HalError>;
    fn write(&mut self, src: &[u8]) -> Result<usize, HalError>;
    fn seek(&mut self, offset: u64) -> Result<(), HalError>;
}

pub trait FsHal {
    type File: FileHandle;

    fn mount(&mut self, root: &str) -> Result<(), HalError>;
    fn open(&mut self, path: &str, mode: FileMode) -> Result<Self::File, HalError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileMode {
    Read,
    Write,
    ReadWrite,
}

pub trait PsramHal {
    fn available(&self) -> bool;
    fn read(&mut self, address: u32, dst: &mut [u8]) -> Result<(), HalError>;
    fn write(&mut self, address: u32, src: &[u8]) -> Result<(), HalError>;

    /// Read `dst.len()` bytes for a [`PsramCodeWindow`](crate::psram::PsramCodeWindow)
    /// refill. Backends may route this through a faster, opt-in code-fetch path
    /// and handle their own safe fallback; the default is identical to
    /// [`PsramHal::read`], so the ordinary read/write contract is unchanged.
    fn read_code_window(&mut self, address: u32, dst: &mut [u8]) -> Result<(), HalError> {
        self.read(address, dst)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PowerState {
    #[default]
    Unsupported,
    Unknown,
    Charging {
        percent: Option<u8>,
        millivolts: Option<u16>,
    },
    Percent {
        percent: u8,
        millivolts: Option<u16>,
    },
    Voltage {
        millivolts: u16,
    },
}

impl PowerState {
    pub const fn unsupported() -> Self {
        Self::Unsupported
    }

    pub const fn unknown() -> Self {
        Self::Unknown
    }

    pub const fn charging(percent: Option<u8>, millivolts: Option<u16>) -> Self {
        Self::Charging {
            percent,
            millivolts,
        }
    }

    pub const fn percent(percent: u8, millivolts: Option<u16>) -> Self {
        Self::Percent {
            percent,
            millivolts,
        }
    }

    pub const fn voltage(millivolts: u16) -> Self {
        Self::Voltage { millivolts }
    }
}

pub trait PowerHal {
    fn poll(&mut self) -> Result<PowerState, HalError>;
}
