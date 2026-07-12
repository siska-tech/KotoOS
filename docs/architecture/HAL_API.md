# HAL API Draft

This is the first contract between portable KotoOS code and platform backends. Names are provisional, but the ownership boundaries are not.

KotoOS is implemented primarily in Rust. The core should remain portable and `no_std` compatible where practical; host-only backends may use `std`.

## General Conventions

- HAL APIs are expressed as Rust traits.
- Core code owns policy; HAL implementations own hardware access.
- Backends return `Result<T, HalError>` rather than panicking.
- No core module should depend on backend-specific crates directly.
- C/C++ libraries, when needed, are wrapped behind small Rust FFI adapters.

## Shared Types

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

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
```

## Video

```rust
pub trait VideoHal {
    fn init(&mut self, width: u16, height: u16, preferred_format: PixelFormat) -> Result<(), HalError>;
    fn begin_frame(&mut self) -> Result<(), HalError>;
    fn update_rect(&mut self, rect: Rect, surface: &Surface<'_>) -> Result<(), HalError>;
    fn update_scanlines(&mut self, y: u16, line_count: u16, surface: &Surface<'_>) -> Result<(), HalError>;
    fn end_frame(&mut self) -> Result<(), HalError>;
}
```

Backend notes:

- Host backend may upload to a window texture.
- PicoCalc backend should use SPI DMA and avoid blocking longer than necessary.
- PicoCalc backend must respect the fixed LCD pin assignment in the requirements.
- PicoCalc backend owns LCD controller profile selection. ILI9488,
  ST7365P-compatible, and clone-panel differences are captured in
  [LCD_INIT_PROFILES.md](../hardware/LCD_INIT_PROFILES.md), not in core rendering code.

## Input

```rust
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
```

Backend notes:

- PicoCalc backend must initialize I2C to at least 100kHz when possible.
- Default game mappings require real hardware validation.
- Text input and game buttons should be separable so KotoIME can own composition.

## Audio

```rust
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
```

Backend notes:

- PicoCalc backend uses a software PCM mixer with PWM output.
- The RP2040 PWM slice constraint means stereo pitch generation cannot rely on independent hardware frequencies.

## Filesystem

```rust
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
```

Core notes:

- KotoFS must virtualize app paths before calling HAL.
- `.kpa` readers should prefer sequential reads and explicit preload windows.

## PSRAM

```rust
pub trait PsramHal {
    fn available(&self) -> bool;
    fn read(&mut self, address: u32, dst: &mut [u8]) -> Result<(), HalError>;
    fn write(&mut self, address: u32, src: &[u8]) -> Result<(), HalError>;
}
```

RP2040 rule:

- Never expose PSRAM as a directly dereferenceable slice or pointer.

Core code does not touch `PsramHal` directly. It goes through the block API in
`koto-core::psram`, which range-checks every transfer against the configured
capacity and only ever copies into caller-provided SRAM buffers:

```rust
pub const PSRAM_BLOCK_SIZE: usize = 256;

pub struct PsramBlocks<H> { /* wraps a PsramHal backend + capacity */ }

impl<H: PsramHal> PsramBlocks<H> {
    pub fn try_new(hal: H, capacity: u32) -> Result<Self, PsramError>;
    pub fn capacity(&self) -> u32;
    pub fn block_count(&self) -> u32;
    pub fn read(&mut self, address: u32, dst: &mut [u8]) -> Result<(), PsramError>;
    pub fn write(&mut self, address: u32, src: &[u8]) -> Result<(), PsramError>;
    pub fn read_block(&mut self, index: u32, dst: &mut [u8]) -> Result<(), PsramError>;
    pub fn write_block(&mut self, index: u32, src: &[u8]) -> Result<(), PsramError>;
}
```

- Out-of-range or overflowing transfers return `PsramError::OutOfRange`.
- Block transfers require buffers of exactly `PSRAM_BLOCK_SIZE` bytes.
- The device-specific PIO/DMA backend is implemented later behind `PsramHal`.

## Power

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PowerState {
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

pub trait PowerHal {
    fn poll(&mut self) -> Result<PowerState, HalError>;
}
```

Power reporting is optional. Backends should return `PowerState::Unsupported`
when the device or firmware cannot report power data, and `PowerState::Unknown`
when the capability exists but the current reading is temporarily unavailable.
KotoShell can display `Unknown`, `Charging`, `Percent`, and low-voltage-only
states, and hides the indicator for `Unsupported`.
