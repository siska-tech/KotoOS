use crate::hal::{PixelFormat, Rect};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RenderSurface {
    pub width: u16,
    pub height: u16,
    pub format: PixelFormat,
}

impl RenderSurface {
    pub const fn new(width: u16, height: u16, format: PixelFormat) -> Self {
        Self {
            width,
            height,
            format,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RenderUpdate {
    Full,
    Rect(Rect),
    Scanlines { y: u16, line_count: u16 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RenderCommand {
    pub surface: RenderSurface,
    pub update: RenderUpdate,
}

impl RenderCommand {
    pub fn full(surface: RenderSurface) -> Result<Self, RenderError> {
        validate_surface(surface)?;
        Ok(Self {
            surface,
            update: RenderUpdate::Full,
        })
    }

    pub fn rect(surface: RenderSurface, rect: Rect) -> Result<Self, RenderError> {
        validate_surface(surface)?;
        validate_rect(surface, rect)?;
        Ok(Self {
            surface,
            update: RenderUpdate::Rect(rect),
        })
    }

    pub fn scanlines(surface: RenderSurface, y: u16, line_count: u16) -> Result<Self, RenderError> {
        validate_surface(surface)?;
        validate_scanlines(surface, y, line_count)?;
        Ok(Self {
            surface,
            update: RenderUpdate::Scanlines { y, line_count },
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RenderError {
    EmptySurface,
    InvalidRect,
    InvalidScanlines,
    CommandListFull,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderCommandList<const N: usize> {
    items: [Option<RenderCommand>; N],
    len: usize,
}

impl<const N: usize> RenderCommandList<N> {
    pub const fn new() -> Self {
        Self {
            items: [None; N],
            len: 0,
        }
    }

    pub fn push(&mut self, command: RenderCommand) -> Result<(), RenderError> {
        if self.len >= N {
            return Err(RenderError::CommandListFull);
        }
        self.items[self.len] = Some(command);
        self.len += 1;
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn iter(&self) -> RenderCommandIter<'_, N> {
        RenderCommandIter {
            list: self,
            index: 0,
        }
    }
}

impl<const N: usize> Default for RenderCommandList<N> {
    fn default() -> Self {
        Self::new()
    }
}

pub struct RenderCommandIter<'a, const N: usize> {
    list: &'a RenderCommandList<N>,
    index: usize,
}

impl<'a, const N: usize> Iterator for RenderCommandIter<'a, N> {
    type Item = &'a RenderCommand;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.list.len {
            return None;
        }
        let item = self.list.items[self.index].as_ref();
        self.index += 1;
        item
    }
}

fn validate_surface(surface: RenderSurface) -> Result<(), RenderError> {
    if surface.width == 0 || surface.height == 0 {
        return Err(RenderError::EmptySurface);
    }
    Ok(())
}

fn validate_rect(surface: RenderSurface, rect: Rect) -> Result<(), RenderError> {
    if rect.x < 0 || rect.y < 0 || rect.w <= 0 || rect.h <= 0 {
        return Err(RenderError::InvalidRect);
    }

    let right = rect.x.checked_add(rect.w).ok_or(RenderError::InvalidRect)?;
    let bottom = rect.y.checked_add(rect.h).ok_or(RenderError::InvalidRect)?;
    if right > i32::from(surface.width) || bottom > i32::from(surface.height) {
        return Err(RenderError::InvalidRect);
    }

    Ok(())
}

fn validate_scanlines(surface: RenderSurface, y: u16, line_count: u16) -> Result<(), RenderError> {
    if line_count == 0 {
        return Err(RenderError::InvalidScanlines);
    }

    let bottom = y
        .checked_add(line_count)
        .ok_or(RenderError::InvalidScanlines)?;
    if bottom > surface.height {
        return Err(RenderError::InvalidScanlines);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn surface() -> RenderSurface {
        RenderSurface::new(320, 320, PixelFormat::Rgb565)
    }

    #[test]
    fn represents_full_rect_and_scanline_updates() {
        assert_eq!(
            RenderCommand::full(surface()).unwrap().update,
            RenderUpdate::Full
        );
        assert_eq!(
            RenderCommand::rect(
                surface(),
                Rect {
                    x: 4,
                    y: 8,
                    w: 16,
                    h: 12
                }
            )
            .unwrap()
            .update,
            RenderUpdate::Rect(Rect {
                x: 4,
                y: 8,
                w: 16,
                h: 12
            })
        );
        assert_eq!(
            RenderCommand::scanlines(surface(), 10, 3).unwrap().update,
            RenderUpdate::Scanlines {
                y: 10,
                line_count: 3
            }
        );
    }

    #[test]
    fn rejects_invalid_rectangles() {
        for rect in [
            Rect {
                x: -1,
                y: 0,
                w: 1,
                h: 1,
            },
            Rect {
                x: 0,
                y: 0,
                w: 0,
                h: 1,
            },
            Rect {
                x: 319,
                y: 0,
                w: 2,
                h: 1,
            },
            Rect {
                x: 0,
                y: 319,
                w: 1,
                h: 2,
            },
        ] {
            assert_eq!(
                RenderCommand::rect(surface(), rect),
                Err(RenderError::InvalidRect)
            );
        }
    }

    #[test]
    fn rejects_invalid_scanlines() {
        assert_eq!(
            RenderCommand::scanlines(surface(), 0, 0),
            Err(RenderError::InvalidScanlines)
        );
        assert_eq!(
            RenderCommand::scanlines(surface(), 319, 2),
            Err(RenderError::InvalidScanlines)
        );
    }
}
