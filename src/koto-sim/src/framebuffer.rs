use std::fs;
use std::path::Path;

use koto_core::shell::SHELL_SURFACE;
use koto_core::{BitmapFont, BootSplash, Canvas, RenderCommand, RenderUpdate, ShellState};

use crate::SimError;

/// A host-owned RGB565 framebuffer for the simulator.
///
/// Unlike the device (which forbids a full-screen framebuffer in SRAM, see
/// NFR-MEM-3), the simulator runs on a PC and can afford the whole surface in
/// one buffer.
pub struct Framebuffer {
    width: u16,
    height: u16,
    pixels: Vec<u8>,
}

impl Framebuffer {
    pub fn new(width: u16, height: u16) -> Self {
        Self {
            width,
            height,
            pixels: vec![0u8; width as usize * height as usize * 2],
        }
    }

    pub fn width(&self) -> u16 {
        self.width
    }

    pub fn height(&self) -> u16 {
        self.height
    }

    pub fn as_canvas(&mut self) -> Canvas<'_> {
        Canvas::new(&mut self.pixels, self.width, self.height)
            .expect("framebuffer is sized width*height*2")
    }
}

pub fn load_font_bytes(path: impl AsRef<Path>) -> Result<Vec<u8>, SimError> {
    fs::read(path).map_err(|_| SimError::Io)
}

/// Render the boot splash (KOTO-0181) into a fresh shell-sized framebuffer,
/// through the same `koto_core::BootSplash` painter the device firmware uses,
/// so simulator screenshots and golden frames match the hardware pixels.
pub fn render_splash_frame(splash: &BootSplash, font: &BitmapFont<'_>) -> Framebuffer {
    let mut framebuffer = Framebuffer::new(SHELL_SURFACE.width, SHELL_SURFACE.height);
    splash.paint(&mut framebuffer.as_canvas(), font);
    framebuffer
}

pub fn write_bmp(path: impl AsRef<Path>, framebuffer: &Framebuffer) -> Result<(), SimError> {
    let width = framebuffer.width as usize;
    let height = framebuffer.height as usize;
    let row_padded = (width * 3 + 3) & !3;
    let pixel_data_size = row_padded * height;
    let file_size = 54 + pixel_data_size;

    let mut out = Vec::with_capacity(file_size);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(file_size as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(width as i32).to_le_bytes());
    out.extend_from_slice(&(height as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&24u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(pixel_data_size as u32).to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());

    let padding = [0u8; 3];
    for y in (0..height).rev() {
        for x in 0..width {
            let i = (y * width + x) * 2;
            let value = u16::from_le_bytes([framebuffer.pixels[i], framebuffer.pixels[i + 1]]);
            let (r, g, b) = rgb565_to_rgb888(value);
            out.push(b);
            out.push(g);
            out.push(r);
        }
        out.extend_from_slice(&padding[..row_padded - width * 3]);
    }

    fs::write(path, &out).map_err(|_| SimError::Io)
}

pub fn framebuffer_to_argb(framebuffer: &Framebuffer) -> Vec<u32> {
    let count = framebuffer.width as usize * framebuffer.height as usize;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let value = u16::from_le_bytes([framebuffer.pixels[i * 2], framebuffer.pixels[i * 2 + 1]]);
        let (r, g, b) = rgb565_to_rgb888(value);
        out.push((u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b));
    }
    out
}

fn rgb565_to_rgb888(value: u16) -> (u8, u8, u8) {
    let r = ((value >> 11) & 0x1F) as u8;
    let g = ((value >> 5) & 0x3F) as u8;
    let b = (value & 0x1F) as u8;
    (
        (r << 3) | (r >> 2),
        (g << 2) | (g >> 4),
        (b << 3) | (b >> 2),
    )
}

#[derive(Debug, Default, Eq, PartialEq)]
pub struct RenderRecorder {
    commands: Vec<RenderCommand>,
}

impl RenderRecorder {
    pub fn new() -> Self {
        Self {
            commands: Vec::new(),
        }
    }

    pub fn record_shell_full(&mut self, shell: &ShellState) -> Result<(), SimError> {
        let mut commands = koto_core::RenderCommandList::<4>::new();
        shell
            .render_full(&mut commands)
            .map_err(|_| SimError::InvalidRenderCommand)?;
        self.commands.extend(commands.iter().copied());
        Ok(())
    }

    pub fn record_shell_list(&mut self, shell: &ShellState) -> Result<(), SimError> {
        let mut commands =
            koto_core::RenderCommandList::<{ koto_core::shell::SHELL_LIST_COMMANDS }>::new();
        shell
            .render_list(&mut commands)
            .map_err(|_| SimError::InvalidRenderCommand)?;
        self.commands.extend(commands.iter().copied());
        Ok(())
    }

    pub fn record_shell_selection_change(
        &mut self,
        shell: &ShellState,
        previous_selected: usize,
    ) -> Result<(), SimError> {
        let mut commands = koto_core::RenderCommandList::<4>::new();
        shell
            .render_selection_change(previous_selected, &mut commands)
            .map_err(|_| SimError::InvalidRenderCommand)?;
        self.commands.extend(commands.iter().copied());
        Ok(())
    }

    pub fn commands(&self) -> &[RenderCommand] {
        &self.commands
    }
}

pub fn describe_render_command(command: &RenderCommand) -> String {
    match command.update {
        RenderUpdate::Full => format!(
            "render full {}x{} {:?}",
            command.surface.width, command.surface.height, command.surface.format
        ),
        RenderUpdate::Rect(rect) => format!(
            "render rect x={} y={} w={} h={} on {}x{} {:?}",
            rect.x,
            rect.y,
            rect.w,
            rect.h,
            command.surface.width,
            command.surface.height,
            command.surface.format
        ),
        RenderUpdate::Scanlines { y, line_count } => format!(
            "render scanlines y={} lines={} on {}x{} {:?}",
            y, line_count, command.surface.width, command.surface.height, command.surface.format
        ),
    }
}
