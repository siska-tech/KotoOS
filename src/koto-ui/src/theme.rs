/// An RGB565 color value independent of any concrete raster backend.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(transparent)]
pub struct Rgb565(pub u16);

impl Rgb565 {
    pub const BLACK: Self = Self(0x0000);
    pub const WHITE: Self = Self(0xffff);

    pub const fn from_rgb8(red: u8, green: u8, blue: u8) -> Self {
        Self(((red as u16 & 0xf8) << 8) | ((green as u16 & 0xfc) << 3) | (blue as u16 >> 3))
    }
}

/// Colors used to paint one control interaction state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct ControlStyle {
    pub background: Rgb565,
    pub foreground: Rgb565,
    pub border: Rgb565,
}

impl ControlStyle {
    pub const fn new(background: Rgb565, foreground: Rgb565, border: Rgb565) -> Self {
        Self {
            background,
            foreground,
            border,
        }
    }
}

/// Compact built-in theme tokens shared by KotoUI controls.
///
/// Spacing values are pixels. The reserved byte keeps the structure's layout
/// explicit while leaving room for a compatible small token in a later issue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct Theme {
    pub normal: ControlStyle,
    pub focused: ControlStyle,
    pub pressed: ControlStyle,
    pub disabled: ControlStyle,
    pub accent: Rgb565,
    pub focus: Rgb565,
    pub spacing: u8,
    pub border_width: u8,
    pub focus_width: u8,
    pub reserved: u8,
}

impl Theme {
    pub const DARK: Self = Self {
        normal: ControlStyle::new(
            Rgb565::from_rgb8(26, 34, 52),
            Rgb565::from_rgb8(236, 240, 248),
            Rgb565::from_rgb8(96, 108, 132),
        ),
        focused: ControlStyle::new(
            Rgb565::from_rgb8(40, 96, 176),
            Rgb565::WHITE,
            Rgb565::from_rgb8(126, 196, 255),
        ),
        pressed: ControlStyle::new(
            Rgb565::from_rgb8(28, 58, 110),
            Rgb565::WHITE,
            Rgb565::from_rgb8(180, 220, 255),
        ),
        disabled: ControlStyle::new(
            Rgb565::from_rgb8(38, 44, 58),
            Rgb565::from_rgb8(126, 132, 144),
            Rgb565::from_rgb8(70, 76, 88),
        ),
        accent: Rgb565::from_rgb8(126, 196, 255),
        focus: Rgb565::from_rgb8(255, 220, 80),
        spacing: 4,
        border_width: 1,
        focus_width: 1,
        reserved: 0,
    };
}

impl Default for Theme {
    fn default() -> Self {
        Self::DARK
    }
}
