use crate::hal::{PixelFormat, Rect};
use crate::render::{RenderCommand, RenderError, RenderSurface};

pub const KOTODOS_SCREEN_WIDTH: u16 = 320;
pub const KOTODOS_SCREEN_HEIGHT: u16 = 320;
pub const KOTODOS_GAME_HEIGHT: u16 = 200;
pub const KOTODOS_UI_HEIGHT: u16 = 120;

pub const KOTODOS_SURFACE: RenderSurface = RenderSurface {
    width: KOTODOS_SCREEN_WIDTH,
    height: KOTODOS_SCREEN_HEIGHT,
    format: PixelFormat::Rgb565,
};

pub const KOTODOS_GAME_REGION: Rect = Rect {
    x: 0,
    y: 0,
    w: KOTODOS_SCREEN_WIDTH as i32,
    h: KOTODOS_GAME_HEIGHT as i32,
};

pub const KOTODOS_UI_REGION: Rect = Rect {
    x: 0,
    y: KOTODOS_GAME_HEIGHT as i32,
    w: KOTODOS_SCREEN_WIDTH as i32,
    h: KOTODOS_UI_HEIGHT as i32,
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct KotoDosMode;

impl KotoDosMode {
    pub const fn surface(self) -> RenderSurface {
        KOTODOS_SURFACE
    }

    pub const fn game_region(self) -> Rect {
        KOTODOS_GAME_REGION
    }

    pub const fn ui_region(self) -> Rect {
        KOTODOS_UI_REGION
    }

    pub fn game_frame(self) -> Result<RenderCommand, RenderError> {
        RenderCommand::rect(self.surface(), self.game_region())
    }

    pub fn game_rect(self, rect: Rect) -> Result<RenderCommand, RenderError> {
        if !rect_within(rect, self.game_region()) {
            return Err(RenderError::InvalidRect);
        }
        RenderCommand::rect(self.surface(), rect)
    }

    pub fn game_scanlines(self, y: u16, line_count: u16) -> Result<RenderCommand, RenderError> {
        let bottom = y
            .checked_add(line_count)
            .ok_or(RenderError::InvalidScanlines)?;
        if line_count == 0 || bottom > KOTODOS_GAME_HEIGHT {
            return Err(RenderError::InvalidScanlines);
        }
        RenderCommand::scanlines(self.surface(), y, line_count)
    }
}

fn rect_within(rect: Rect, bounds: Rect) -> bool {
    if rect.x < bounds.x || rect.y < bounds.y || rect.w <= 0 || rect.h <= 0 {
        return false;
    }

    let Some(right) = rect.x.checked_add(rect.w) else {
        return false;
    };
    let Some(bottom) = rect.y.checked_add(rect.h) else {
        return false;
    };
    let Some(bounds_right) = bounds.x.checked_add(bounds.w) else {
        return false;
    };
    let Some(bounds_bottom) = bounds.y.checked_add(bounds.h) else {
        return false;
    };

    right <= bounds_right && bottom <= bounds_bottom
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::RenderUpdate;

    #[test]
    fn constants_partition_the_320_square_screen() {
        assert_eq!(KOTODOS_SURFACE.width, 320);
        assert_eq!(KOTODOS_SURFACE.height, 320);
        assert_eq!(KOTODOS_GAME_REGION.w, 320);
        assert_eq!(KOTODOS_GAME_REGION.h, 200);
        assert_eq!(KOTODOS_UI_REGION.y, 200);
        assert_eq!(KOTODOS_UI_REGION.w, 320);
        assert_eq!(KOTODOS_UI_REGION.h, 120);
        assert_eq!(
            KOTODOS_GAME_HEIGHT + KOTODOS_UI_HEIGHT,
            KOTODOS_SCREEN_HEIGHT
        );
    }

    #[test]
    fn game_commands_target_only_the_game_region() {
        let mode = KotoDosMode;

        assert_eq!(
            mode.game_frame().unwrap().update,
            RenderUpdate::Rect(KOTODOS_GAME_REGION)
        );
        assert_eq!(
            mode.game_rect(Rect {
                x: 8,
                y: 16,
                w: 24,
                h: 32
            })
            .unwrap()
            .update,
            RenderUpdate::Rect(Rect {
                x: 8,
                y: 16,
                w: 24,
                h: 32
            })
        );
        assert_eq!(
            mode.game_scanlines(198, 2).unwrap().update,
            RenderUpdate::Scanlines {
                y: 198,
                line_count: 2
            }
        );
    }

    #[test]
    fn game_commands_reject_ui_or_offscreen_targets() {
        let mode = KotoDosMode;

        for rect in [
            Rect {
                x: 0,
                y: 200,
                w: 1,
                h: 1,
            },
            Rect {
                x: 0,
                y: 199,
                w: 1,
                h: 2,
            },
            Rect {
                x: -1,
                y: 0,
                w: 1,
                h: 1,
            },
            Rect {
                x: 319,
                y: 0,
                w: 2,
                h: 1,
            },
        ] {
            assert_eq!(mode.game_rect(rect), Err(RenderError::InvalidRect));
        }

        assert_eq!(
            mode.game_scanlines(199, 2),
            Err(RenderError::InvalidScanlines)
        );
        assert_eq!(
            mode.game_scanlines(200, 1),
            Err(RenderError::InvalidScanlines)
        );
    }
}
