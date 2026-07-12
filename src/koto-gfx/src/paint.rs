//! The CPU layer compositor — the chunk compositor over the retained layer model.
//!
//! These functions were lifted verbatim from the Pico firmware's `app_render.rs`
//! (KotoGFX migration Stage 4, GFX-0004): the per-layer paint passes and the
//! fixed-z-order orchestration that composites the static chrome, the retained
//! board tilemap, the retained sprite layer, the retained text layer, and the
//! per-frame immediate command list into a [`Canvas`]. They operate on the
//! GFX-0002 POD layer model (board/sprite/stamp/text slices + the immediate and
//! static command slices) plus the app heap slice — no `DeviceRuntimeHost`, no
//! timing, no transfer. The firmware keeps identically-signed adapters that
//! unpack its host into these slices (the Stage 1/3 methodology) and still owns
//! the caller-supplied strip and the clear-to-base/banding around each call.
//!
//! The `Canvas` viewport clips every blit and glyph to the current strip/rect, so
//! only layers intersecting it cost anything; the firmware's banded present path
//! drives one whole-stack recomposite per surviving dirty rect, clipped to it.

use crate::derive::stamp_cell;
use crate::font::BitmapFont;
use crate::layer::{
    AppDrawCommand, Game2dBoard, Game2dSprite, Game2dStampDef, Game2dText, GAME2D_BOARD_COLS,
    GAME2D_ORIGIN_X, GAME2D_ORIGIN_Y, GAME2D_TILE_BYTES, GAME2D_TILE_PX,
};
use crate::raster::{Canvas, Rgb565};
use crate::Rect;

/// Composite the retained Game2D board layer into `canvas`. Each non-empty cell
/// holds the app-heap byte offset of a 16x16 RGB565 tile; the canvas viewport
/// clips blits outside the current rect, so off-strip cells are no-ops
/// (KOTO-0135).
pub fn paint_board_layer(canvas: &mut Canvas<'_>, board: &Game2dBoard, heap: &[u8]) {
    for (index, &tile_ref) in board.iter().enumerate() {
        let Ok(off) = usize::try_from(tile_ref) else {
            continue; // empty (`-1`)
        };
        let Some(src) = heap.get(off..off.saturating_add(GAME2D_TILE_BYTES)) else {
            continue;
        };
        let cx = (index % GAME2D_BOARD_COLS) as i32;
        let cy = (index / GAME2D_BOARD_COLS) as i32;
        canvas.blit_rgb565(
            GAME2D_ORIGIN_X + cx * GAME2D_TILE_PX,
            GAME2D_ORIGIN_Y + cy * GAME2D_TILE_PX,
            GAME2D_TILE_PX,
            GAME2D_TILE_PX,
            src,
        );
    }
}

/// Composite the retained Game2D sprite layer into `canvas` (KOTO-0140). Each
/// visible sprite draws its stamp's cells — the 16x16 tile at `tile_ref` blitted
/// at `(x + dcol*16, y + drow*16)`. The canvas viewport clips blits outside the
/// current rect, so off-strip sprites are no-ops.
pub fn paint_sprite_layer(
    canvas: &mut Canvas<'_>,
    sprites: &[Game2dSprite],
    stamps: &[Game2dStampDef],
    heap: &[u8],
) {
    for sprite in sprites {
        if !sprite.visible {
            continue;
        }
        let Ok(tile_off) = usize::try_from(sprite.tile_ref) else {
            continue;
        };
        let Some(src) = heap.get(tile_off..tile_off.saturating_add(GAME2D_TILE_BYTES)) else {
            continue;
        };
        let Some(stamp) = stamps.get(sprite.stamp_id as usize) else {
            continue;
        };
        for cell in 0..stamp.count as usize {
            let Some((dcol, drow)) = stamp_cell(heap, stamp.cells_off, cell) else {
                continue;
            };
            canvas.blit_rgb565(
                sprite.x as i32 + dcol * GAME2D_TILE_PX,
                sprite.y as i32 + drow * GAME2D_TILE_PX,
                GAME2D_TILE_PX,
                GAME2D_TILE_PX,
                src,
            );
        }
    }
}

/// Composite the retained Game2D text layer into `canvas` (KOTO-0141). Each visible
/// item draws its UTF-8 string at `(x, y)` in its colour. The canvas viewport clips
/// glyphs outside the current rect, so off-strip items are no-ops.
pub fn paint_text_layer(canvas: &mut Canvas<'_>, font: &BitmapFont<'_>, text_items: &[Game2dText]) {
    for item in text_items {
        if !item.visible {
            continue;
        }
        if let Ok(text) = core::str::from_utf8(&item.bytes[..item.len as usize]) {
            canvas.draw_text(
                item.x as i32,
                item.y as i32,
                font,
                text,
                Rgb565(item.rgb565),
            );
        }
    }
}

/// Composite a command list (the static chrome layer or the per-frame immediate
/// list) into `canvas`. Rect fills, heap-referenced pixel blits, and text are
/// painted in list order; the canvas viewport clips each to the current rect.
pub fn paint_command_list(
    canvas: &mut Canvas<'_>,
    font: &BitmapFont<'_>,
    heap: &[u8],
    commands: &[AppDrawCommand],
) {
    for command in commands {
        match command {
            AppDrawCommand::Empty => {}
            AppDrawCommand::Rect { x, y, w, h, rgb565 } => canvas.fill_rect(
                Rect {
                    x: *x,
                    y: *y,
                    w: *w,
                    h: *h,
                },
                Rgb565(*rgb565),
            ),
            AppDrawCommand::Pixels {
                x,
                y,
                w,
                h,
                off,
                len,
            } => {
                if let Some(src) =
                    heap.get(*off as usize..(*off as usize).saturating_add(*len as usize))
                {
                    canvas.blit_rgb565(*x, *y, *w, *h, src);
                }
            }
            AppDrawCommand::Text {
                x,
                y,
                rgb565,
                bytes,
                len,
            } => {
                if let Ok(text) = core::str::from_utf8(&bytes[..*len as usize]) {
                    canvas.draw_text(*x, *y, font, text, Rgb565(*rgb565));
                }
            }
        }
    }
}

/// Composite the whole layer stack into `canvas` in the fixed z-order (KOTO-0140):
/// static/background chrome (KOTO-0136), then the retained board tilemap
/// (KOTO-0135), then the retained sprite layer, then the retained text layer, then
/// the per-frame immediate list (debug/overlay/transition/text). This replaces the
/// KOTO-0135 `game2d_present` stream marker with a fixed layer ordering — simpler
/// and app-agnostic. The canvas viewport clips each blit, so only layers
/// intersecting the current strip/rect cost anything. The caller clears `canvas`
/// to the base colour first; this paints over that base.
#[allow(clippy::too_many_arguments)]
pub fn paint_app_commands(
    canvas: &mut Canvas<'_>,
    font: &BitmapFont<'_>,
    static_commands: &[AppDrawCommand],
    board: &Game2dBoard,
    sprites: &[Game2dSprite],
    stamps: &[Game2dStampDef],
    text_items: &[Game2dText],
    immediate_commands: &[AppDrawCommand],
    heap: &[u8],
) {
    paint_command_list(canvas, font, heap, static_commands);
    paint_board_layer(canvas, board, heap);
    paint_sprite_layer(canvas, sprites, stamps, heap);
    paint_text_layer(canvas, font, text_items);
    paint_command_list(canvas, font, heap, immediate_commands);
}
