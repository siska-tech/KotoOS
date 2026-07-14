//! Present app draw-command lists to the panel: full compose, KOTO-0128 delta,
//! and the KOTO-0129 pixel-blit diagnostic. KOTO-0131 added per-frame paint
//! metrics, a banded fallback for oversize dirty rectangles, and a full-repaint
//! threshold so a misaligned positional diff can no longer death-spiral into a
//! flurry of near-full-screen partial transfers.

use core::fmt::Write;

use embassy_futures::join::join;
use embassy_rp::uart::UartTx;
use embassy_time::{Instant, Timer};
use koto_core::{coalesce_dirty_tiles, BitmapFont, Canvas, Rect, TileBand};
use koto_gfx::{
    coalesce_then_decide, collect_immediate_dirty, collect_initial_scene_dirty, decision_snapshot,
    has_retained_scene_content, push_dirty, DeltaDecision, EditRegionShape, FullRepaintPolicy,
    Game2dTilemap, FULL_REPAINT_RECTS, MAX_EDIT_REGION, STATIC_DAMAGE_CAP,
};

use crate::dashboard::LineBuffer;
use crate::firmware::app_host::{AppDrawCommand, AppStaticLayer, DeviceRuntimeHost};
use crate::firmware::config::{
    GAME2D_MAX_SPRITES, GAME2D_MAX_TEXT_ITEMS, GAME2D_TILE_PX, PRESENT_PIPELINE,
    RASTER_STRIP_BYTES, RASTER_STRIP_LINES, RGB666_STRIP_BYTES,
};
use crate::firmware::diag::{
    uart_log, uart_write_line, DirtyRectGeometry, FullRepaintReason, PaintMetrics,
};
use crate::lcd::{PicoCalcLcd, Rgb888};

// The delta full-repaint thresholds (KOTO-0131) and the escalation/attribution
// logic now live in the KotoGFX foundation crate (`koto_gfx::FullRepaintPolicy`
// / `FULL_REPAINT_AREA` / `FULL_REPAINT_RECTS`); this path feeds the collected
// diff into `FullRepaintPolicy::decide`. The local `DIRTY_RECT_CAP` below still
// sizes its working set off `FULL_REPAINT_RECTS`.

// Coalesced board dirty-band buffer (KOTO-0143). The board's per-cell dirty set is
// merged into horizontal/vertical bands before sizing and transfer, so a line clear
// is a few bands instead of dozens of cell rects. A fragmented board could in
// principle exceed this, but any band count past `FULL_REPAINT_RECTS` already
// escalates to a full repaint, so the buffer only needs headroom above that cap;
// an overflow is itself treated as a rect-count escalation (it cannot drop a band).
const BOARD_BAND_CAP: usize = 32;

// Dirty-rect coalescing (KOTO-0159). Each surviving dirty rect drives one full
// recomposite pass over the whole layer stack (static -> board -> sprites -> text
// -> commands) clipped to it; that scan dominates `raster_us` regardless of how
// few pixels the rect actually covers. So on a fragmented event frame (line clear,
// hard drop, game over) where the dirty *area* is tiny but the rect *count* is
// high, collapsing scattered rects into a handful of bounding rects cuts the pass
// count — and `raster_us` with it — at the cost of a little redundant transfer.
//
// `DIRTY_RECT_CAP` bounds the collected working set at one past the full-repaint
// rect threshold: overflowing it means the frame already has more dirty rects than
// would stay incremental, so it escalates to a full repaint (which covers
// everything) and the extra rects are never needed.
//
// `DIRTY_COALESCE_MAX_WASTE` is the per-merge wasted-area budget: two dirty rects
// merge when their bounding box covers no more than this many pixels beyond what
// the pair already dirties. Sized to ~four tiles (1024 px) so neighbouring well
// cells and adjacent effect rects collapse while a well change and a far sidebar
// stay separate passes. Conservative and tuned against hardware UART (`phase=164`);
// raster is per-pass, not per-pixel, so trading a little transfer area is a net win.
const DIRTY_RECT_CAP: usize = FULL_REPAINT_RECTS as usize + 1;
const DIRTY_COALESCE_MAX_WASTE: u32 = (GAME2D_TILE_PX * GAME2D_TILE_PX * 4) as u32;

// Expanded dirty-rect collection cap (GFX-0010 Stage 1A/1B, GFX-0011 Stage 1).
// `DIRTY_RECT_CAP` is sized to one past the rect threshold because the *original*
// decision was pre-coalesce — past 25 rects the frame already escalated, so storing
// more bought the decision nothing. GFX-0010 Stage 1B moved the decision *after*
// coalescing, so the working set must now admit the full raw fragmentation the
// coalescer needs as input, not just the first 25.
//
// The present path collects the *full structural* dirty set into this larger buffer
// and feeds it to `coalesce_then_decide`, which decides on the post-coalesce count and
// re-summed area. A raw set that overflows even this cap (or a layer that overflowed
// upstream) is truncated and fails safe to a full repaint. The cap is the sum of every
// layer's own worst case (app-agnostic, no per-app constant; it tracks those caps if
// they move):
//   - `MAX_EDIT_REGION`     immediate command edit-region rects (length-shift path)
//   - `BOARD_BAND_CAP`      coalesced board bands
//   - `GAME2D_MAX_SPRITES`  one rect per changed sprite
//   - `GAME2D_MAX_TEXT_ITEMS` one row-band per changed text item
//   - `STATIC_DAMAGE_CAP`   static-rebuild shadow-diff union rects (GFX-0013)
// GFX-0011 Stage 1 promotes this cap onto the *immediate command diff* too: the
// length-shift edit region is now collected up to this bound instead of bailing at the
// smaller `MAX_EDIT_REGION` (which structurally excluded wide count-shift frames from
// the coalesce-before-decide rescue). A region wider than this cap still bails — case
// #4, the truncation fail-safe — so only the boundary moved (24 → the structural sum),
// not the fail-safe. `pub(crate)` so the `phase=169` diagnostic can report the live cap.
pub(crate) const DIRTY_RECT_PROBE_CAP: usize = MAX_EDIT_REGION
    + BOARD_BAND_CAP
    + GAME2D_MAX_SPRITES
    + GAME2D_MAX_TEXT_ITEMS
    + STATIC_DAMAGE_CAP;

/// Drive a known 16x16 RGB565 tile through the real `draw_pixels` command and
/// present path, isolating the blit pipeline from app/game logic for hardware
/// bring-up (KOTO-0129). The tile is four 8x8 quadrants (red / green / blue /
/// white) at a fixed position, so a correct blit is unmistakable on the panel.
pub async fn present_pixel_diagnostic(
    lcd: &mut PicoCalcLcd<'_>,
    font: &BitmapFont<'_>,
    host: &mut DeviceRuntimeHost,
    static_layer: &AppStaticLayer,
    strip: &mut [u8; RASTER_STRIP_BYTES],
    scratch: &mut [u8; RGB666_STRIP_BYTES],
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
) {
    const TILE: usize = 16;
    const DIAG_X: i32 = 152;
    const DIAG_Y: i32 = 152;
    let mut tile = [0u8; TILE * TILE * 2];
    for row in 0..TILE {
        for col in 0..TILE {
            let color: u16 = match (col < TILE / 2, row < TILE / 2) {
                (true, true) => 0xF800,   // top-left: red
                (false, true) => 0x07E0,  // top-right: green
                (true, false) => 0x001F,  // bottom-left: blue
                (false, false) => 0xFFFF, // bottom-right: white
            };
            let index = (row * TILE + col) * 2;
            tile[index..index + 2].copy_from_slice(&color.to_le_bytes());
        }
    }
    host.clear_frame();
    let _ = host.push(AppDrawCommand::Pixels {
        x: DIAG_X,
        y: DIAG_Y,
        w: TILE as i32,
        h: TILE as i32,
        off: 0,
        len: tile.len() as u32,
    });
    // The boot diagnostic borrows the empty `APP_STATIC` layer (it composites
    // nothing); the app path supplies the populated one.
    let mut metrics = PaintMetrics::default();
    if present_app_commands(
        lcd,
        font,
        host,
        static_layer,
        &tile,
        strip,
        scratch,
        // The bring-up diagnostic draws one tile over the empty static layer (no
        // full-screen base), so this reason is never recorded; pass the initial-
        // build reason for completeness.
        FullRepaintReason::StaticRebuild,
        &mut metrics,
    )
    .await
    .is_ok()
    {
        uart_log(
            uart,
            "phase=157 pixel-diagnostic ok x=152 y=152 w=16 h=16\r\n",
        );
        // Hold the known tile on the panel long enough to read by eye before the
        // shell redraw paints over it (the boot self-tests already pause here).
        Timer::after_millis(1_000).await;
    } else {
        uart_log(uart, "phase=257 pixel-diagnostic-error\r\n");
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn present_app_commands(
    lcd: &mut PicoCalcLcd<'_>,
    font: &BitmapFont<'_>,
    host: &DeviceRuntimeHost,
    static_layer: &AppStaticLayer,
    heap: &[u8],
    strip: &mut [u8; RASTER_STRIP_BYTES],
    scratch: &mut [u8; RGB666_STRIP_BYTES],
    // Why this present was reached as a full repaint (KOTO-0143). Only recorded if
    // the frame actually whole-surface composites (it has a full-screen base); an
    // app with no base transfers per-command and is not a full repaint.
    reason: FullRepaintReason,
    metrics: &mut PaintMetrics,
) -> Result<(), ()> {
    if let Some(base_color) = full_screen_base_color(host, static_layer) {
        // Whole-surface compose: a true full repaint. Flag it (with its reason) so
        // the per-frame metric can show how often, and why, the delta could not
        // stay incremental. The band stream is pipelined like the incremental
        // path (KOTO-0174 H-A2): the whole surface is one rect banded to the
        // half-scratch budget, so ~45 ms of full-surface DMA hides under the
        // next band's raster+convert.
        metrics.mark_full_repaint(reason);
        let full = [Rect {
            x: 0,
            y: 0,
            w: 320,
            h: 320,
        }];
        return present_rects_pipelined(
            lcd,
            font,
            host,
            static_layer,
            heap,
            strip,
            scratch,
            &full,
            base_color,
            metrics,
        )
        .await;
    }

    // A missing full-screen base does not make a retained scene empty. The old
    // first-present fallback below replays only immediate commands, so accepting
    // a retained board/sprite/text/static state through it would omit those
    // pixels and let the frame loop snapshot an image that never reached GRAM.
    // Derive damage from the host-owned empty black surface and run the normal
    // fixed-order compositor before the frame becomes the delta baseline.
    if has_retained_scene_content(
        &static_layer.commands[..static_layer.len],
        &host.board,
        &host.sprites,
        &host.text_items,
    ) {
        return present_initial_retained_scene(
            lcd,
            font,
            host,
            static_layer,
            heap,
            strip,
            scratch,
            metrics,
        )
        .await;
    }

    // Preserve the legacy no-base immediate-only path. Its commands are already
    // bounded partial transfers and do not need retained whole-stack composition.
    for command in &host.commands[..host.len] {
        match command {
            AppDrawCommand::Empty => {}
            AppDrawCommand::Pixels {
                x,
                y,
                w,
                h,
                off,
                len,
            } => {
                let Some(rect) = clip_app_rect(*x, *y, *w, *h) else {
                    continue;
                };
                let off = koto_gfx::pixel_heap_offset(*off) as usize;
                let Some(src) = heap.get(off..off.saturating_add(*len as usize)) else {
                    continue;
                };
                // Compose the blit off-screen into a strip sized to the clipped
                // rectangle, then ship it in one transfer (mirrors the Text arm and
                // keeps every present within the bounded strip budget).
                let used = rect.w as usize * rect.h as usize * 2;
                if used > strip.len() {
                    continue;
                }
                let raster_started = Instant::now();
                strip[..used].fill(0);
                let mut canvas =
                    Canvas::new_viewport(&mut strip[..used], 320, 320, rect).ok_or(())?;
                canvas.blit_rgb565(*x, *y, *w, *h, src);
                metrics.record_raster(raster_started);

                let transfer_started = Instant::now();
                lcd.write_rgb565_rect(
                    rect.x as u16,
                    rect.y as u16,
                    rect.w as u16,
                    rect.h as u16,
                    &strip[..used],
                    scratch,
                )
                .await
                .map_err(|_| ())?;
                metrics.record_transfer(transfer_started, (rect.w * rect.h) as u32);
            }
            AppDrawCommand::Rect { x, y, w, h, rgb565 } => {
                let Some(rect) = clip_app_rect(*x, *y, *w, *h) else {
                    continue;
                };
                let transfer_started = Instant::now();
                lcd.fill_rect(
                    rect.x as u16,
                    rect.y as u16,
                    rect.w as u16,
                    rect.h as u16,
                    rgb565_to_rgb888(*rgb565),
                )
                .await
                .map_err(|_| ())?;
                metrics.record_transfer(transfer_started, (rect.w * rect.h) as u32);
            }
            AppDrawCommand::Text {
                x,
                y,
                rgb565,
                bytes,
                len,
            } => {
                let Ok(text) = core::str::from_utf8(&bytes[..*len as usize]) else {
                    continue;
                };
                // The 17-line text band (16 glyph rows + descender line) is one
                // line taller than the RASTER_STRIP_LINES(16) strip since 520db8b
                // halved the strip, so it is rastered and shipped in strip-sized
                // slices — the viewport clips glyph rows to each slice. Indexing
                // the whole band into `strip` was an out-of-bounds panic, and
                // panic-halt froze the device on the only path that reaches this
                // arm: a no-base app's first present (KOTO-0178, Dirty Rects).
                let band_y = (*y).clamp(0, 303);
                let mut row = band_y;
                while row < band_y + 17 {
                    let slice_h = (band_y + 17 - row).min(RASTER_STRIP_LINES as i32);
                    let rect = Rect {
                        x: 0,
                        y: row,
                        w: 320,
                        h: slice_h,
                    };
                    let used = rect.w as usize * rect.h as usize * 2;
                    let raster_started = Instant::now();
                    strip[..used].fill(0);
                    let mut canvas =
                        Canvas::new_viewport(&mut strip[..used], 320, 320, rect).ok_or(())?;
                    canvas.draw_text(*x, *y, font, text, koto_core::Rgb565(*rgb565));
                    metrics.record_raster(raster_started);

                    let transfer_started = Instant::now();
                    lcd.write_rgb565_rect(
                        rect.x as u16,
                        rect.y as u16,
                        rect.w as u16,
                        rect.h as u16,
                        &strip[..used],
                        scratch,
                    )
                    .await
                    .map_err(|_| ())?;
                    metrics.record_transfer(transfer_started, (rect.w * rect.h) as u32);
                    row += slice_h;
                }
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn present_initial_retained_scene(
    lcd: &mut PicoCalcLcd<'_>,
    font: &BitmapFont<'_>,
    host: &DeviceRuntimeHost,
    static_layer: &AppStaticLayer,
    heap: &[u8],
    strip: &mut [u8; RASTER_STRIP_BYTES],
    scratch: &mut [u8; RGB666_STRIP_BYTES],
    metrics: &mut PaintMetrics,
) -> Result<(), ()> {
    let mut dirty = [Rect {
        x: 0,
        y: 0,
        w: 0,
        h: 0,
    }; DIRTY_RECT_PROBE_CAP];
    let (mut len, mut area, mut overflow) = (0usize, 0u32, false);
    collect_initial_scene_dirty(
        &static_layer.commands[..static_layer.len],
        &host.board,
        &host.sprites,
        &host.stamps,
        &host.text_items,
        &host.commands[..host.len],
        heap,
        320,
        320,
        &mut dirty,
        &mut len,
        &mut area,
        &mut overflow,
    );

    let (decision_len, decision_overflow) = decision_snapshot(len, overflow, DIRTY_RECT_CAP);
    let outcome = coalesce_then_decide(
        FullRepaintPolicy::default(),
        &mut dirty,
        len,
        area,
        decision_len,
        decision_overflow,
        false,
        overflow,
        false,
        DIRTY_COALESCE_MAX_WASTE,
    );
    match outcome.decision {
        DeltaDecision::Skip => Ok(()),
        DeltaDecision::Incremental => {
            present_rects_pipelined(
                lcd,
                font,
                host,
                static_layer,
                heap,
                strip,
                scratch,
                &dirty[..outcome.coalesced_len],
                0,
                metrics,
            )
            .await
        }
        DeltaDecision::FullRepaint(reason) => {
            metrics.mark_full_repaint(reason);
            let full = [Rect {
                x: 0,
                y: 0,
                w: 320,
                h: 320,
            }];
            present_rects_pipelined(
                lcd,
                font,
                host,
                static_layer,
                heap,
                strip,
                scratch,
                &full,
                0,
                metrics,
            )
            .await
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn present_app_delta(
    lcd: &mut PicoCalcLcd<'_>,
    font: &BitmapFont<'_>,
    previous: &DeviceRuntimeHost,
    current: &DeviceRuntimeHost,
    static_layer: &AppStaticLayer,
    heap: &[u8],
    strip: &mut [u8; RASTER_STRIP_BYTES],
    scratch: &mut [u8; RGB666_STRIP_BYTES],
    // GFX-0013: how this frame's static rebuild (if any) presents. The frame loop
    // owns the shadow diff; this path only acts on its outcome. `static_rebuild_full`
    // is the fallback (no shadow / wide relayout / base-overdraw hazard): take the
    // whole-surface StaticRebuild repaint exactly as every rebuild did pre-GFX-0013.
    // `static_damage` is a *bounded* rebuild's damage — the unmatched commands'
    // old∪new union rects — fed into the same working set as every other layer, so
    // the coalesce-before-decide policy owns escalation. Both empty/false on a
    // steady or identical-rebuild frame.
    static_rebuild_full: bool,
    static_damage: &[Rect],
    metrics: &mut PaintMetrics,
) -> Result<(), ()> {
    // Treat the previous frame's composited pixels as retained GRAM (KOTO-0128).
    // A change in the full-screen base — including one appearing or disappearing
    // — means the whole frame must be repainted; otherwise only the rectangles
    // whose composed pixels changed are updated. Apps without a full-screen base
    // (e.g. a partial background band like the Dirty Rects sample) use black as
    // the retained baseline, matching the launch-time clear, so their changed
    // regions still composite off-screen and transfer atomically instead of
    // erasing GRAM in place — which was the visible flicker.
    let previous_base = full_screen_base_color(previous, static_layer);
    if full_screen_base_color(current, static_layer) != previous_base {
        return present_app_commands(
            lcd,
            font,
            current,
            static_layer,
            heap,
            strip,
            scratch,
            FullRepaintReason::BaseChange,
            metrics,
        )
        .await;
    }
    // The retained static/background layer (KOTO-0136) is a layout-stable base; a
    // rebuild this frame (`game2d_static_begin`) means the app changed its chrome
    // (e.g. entering gameplay), which the positional command diff cannot track.
    // Pre-GFX-0013 every rebuild took this whole-surface repaint; now only the
    // fallback does (the frame loop's shadow diff could not positively bound the
    // change). A bounded rebuild instead feeds `static_damage` into the working
    // set below, and an identical rebuild feeds nothing at all.
    if static_rebuild_full {
        return present_app_commands(
            lcd,
            font,
            current,
            static_layer,
            heap,
            strip,
            scratch,
            FullRepaintReason::StaticRebuild,
            metrics,
        )
        .await;
    }
    let base = previous_base.unwrap_or(0);

    // Coalesce the board's changed cells into bands *once* (KOTO-0143), shared by
    // the sizing pass and the transfer pass below. Without this a 10-cell line
    // clear is 10 cell rects and a 4-line collapse ~40 — enough to trip the rect
    // cap and escalate an incremental change to a full repaint. Merging adjacent
    // dirty cells into horizontal/vertical bands keeps a cleared row one band and a
    // contiguous collapse a handful, so line clears stay incremental.
    let board_geometry_changed = !previous.board.geometry_eq(&current.board);
    let mut board_bands = [TileBand::default(); BOARD_BAND_CAP];
    let board_band_count = if board_geometry_changed {
        Some(0)
    } else {
        coalesce_dirty_tiles(
            usize::from(current.board.columns),
            usize::from(current.board.rows),
            |col, row| {
                let index = Game2dTilemap::cell_index(col, row);
                previous.board.cells[index] != current.board.cells[index]
            },
            &mut board_bands,
        )
    };
    // `None` means the board fragmented past the band buffer; it cannot drop a
    // band, so treat it as a rect-count escalation (a full repaint covers all).
    let board_overflow = board_band_count.is_none();
    let board_bands = &board_bands[..board_band_count.unwrap_or(0)];

    // Collect every changed region into one working set (KOTO-0159), then size,
    // coalesce, and transfer it. Summing the union-rect areas and counting the
    // rectangles lets a frame whose positional diff has gone wide (board grow /
    // line collapse) take one clean full compose instead of dozens of overlapping
    // near-full-screen partial transfers that would cost more in total (KOTO-0131);
    // collecting them (instead of counting twice) also lets fragmented event frames
    // be coalesced into far fewer recomposite passes before transfer.
    // GFX-0010 Stage 1A: the working set is sized to the *expanded probe* cap so the
    // observe-only coalesce-pressure dry-run sees the full structural dirty set, not the
    // first 25. The live decision still reads a 25-cap snapshot derived below
    // (`decision_len`/`decision_rect_overflow`), so no pixel and no attribution changes —
    // `push_dirty` accumulates `dirty_area` independent of the buffer length, so the
    // summed area the policy decides on is identical whatever the cap.
    let mut dirty = [Rect {
        x: 0,
        y: 0,
        w: 0,
        h: 0,
    }; DIRTY_RECT_PROBE_CAP];
    let mut probe_len = 0usize;
    let mut dirty_area: u32 = 0;
    // Overflow past the *probe* cap (or a layer overflowing upstream) means the probe
    // set is truncated — far rarer than the 25-cap truncation it replaces. No collected
    // rect is ever dropped silently (the full repaint covers everything).
    let mut probe_overflow = false;
    // Immediate command diff (KOTO-0131/0141, GFX-0008): a changed/moved/disappeared
    // command dirties the union of its old and new footprints. When the list length
    // shifts, the diff anchors on the common prefix/suffix and diffs only the bounded
    // edit region between them, so a single insert/remove localizes to its own slot
    // instead of misaligning the whole tail into a flurry of spurious rects — the
    // false-positive `CommandCountShift` full repaints. GFX-0011 Stage 1: the edit
    // region is now collected up to the expanded `DIRTY_RECT_PROBE_CAP` (as every other
    // layer already is) instead of bailing at the smaller `MAX_EDIT_REGION`, so a wide
    // count shift (the whole immediate list shifting one slot — a snake body moving)
    // feeds its union rects into the coalesce-before-decide path below rather than being
    // skipped and force-escalated. A region wider than even this cap still bails and
    // flags overflow (case #4, the truncation fail-safe), so the policy full-repaints it
    // as `CommandCountShift` (the count having changed); only the boundary moved. The
    // logical lists are `commands[..len]`, so a slot past `len` is simply absent —
    // replacing the old `command_at` Empty-padding (KOTO-0141 stale-overlay fix).
    collect_immediate_dirty(
        &previous.commands[..previous.len],
        &current.commands[..current.len],
        320,
        320,
        DIRTY_RECT_PROBE_CAP,
        &mut dirty,
        &mut probe_len,
        &mut dirty_area,
        &mut probe_overflow,
    );
    // Static-rebuild shadow-diff damage (GFX-0013): a bounded rebuild's unmatched
    // commands contribute their old∪new union rects, collected by the frame loop's
    // `collect_static_rebuild_dirty` against the fingerprint shadow of the last
    // applied layer. They join the same working set as every other layer, so a
    // room-transition rebuild is a few fog bands through the normal coalesce ->
    // decide -> transfer flow — and a genuinely wide one escalates through
    // `FullRepaintPolicy` on its own area/rect merit, exactly as immediate damage
    // does. Empty on steady frames, identical rebuilds, and fallback rebuilds
    // (which took the `static_rebuild_full` early return above).
    for rect in static_damage.iter().copied() {
        push_dirty(
            &mut dirty,
            &mut probe_len,
            &mut dirty_area,
            &mut probe_overflow,
            rect,
        );
    }
    // Game2D board layer (KOTO-0135): the retained tilemap is part of the
    // `DeviceRuntimeHost`, so the previous-frame copy is the dirty reference. Its
    // changed cells are coalesced into bands (KOTO-0143) above, so a lock or line
    // clear feeds a few band rects into the set instead of one rect per cell. A
    // board with no changes adds nothing, so free-fall frames stay incremental on
    // the command/sprite diff alone.
    if board_geometry_changed {
        for tilemap in [&previous.board, &current.board] {
            if let Some(rect) = koto_gfx::tilemap_bounds_rect(tilemap, 320, 320) {
                push_dirty(
                    &mut dirty,
                    &mut probe_len,
                    &mut dirty_area,
                    &mut probe_overflow,
                    rect,
                );
            }
        }
    }
    for band in board_bands {
        if let Some(rect) = board_band_rect(*band, &current.board) {
            push_dirty(
                &mut dirty,
                &mut probe_len,
                &mut dirty_area,
                &mut probe_overflow,
                rect,
            );
        }
    }
    // Game2D sprite layer (KOTO-0140): each sprite whose `(stamp, x, y, tile_ref,
    // visible)` changed contributes one dirty rect — the union of its old and new
    // footprints — so a piece moving down is one small stable band, not a
    // positional-diff balloon. Sprites diff by stable index, like board cells.
    for index in 0..GAME2D_MAX_SPRITES {
        if previous.sprites[index] != current.sprites[index] {
            if let Some(rect) = sprite_dirty_rect(previous, current, heap, index) {
                push_dirty(
                    &mut dirty,
                    &mut probe_len,
                    &mut dirty_area,
                    &mut probe_overflow,
                    rect,
                );
            }
        }
    }
    // Game2D text layer (KOTO-0141): each item whose `(x, y, rgb565, bytes, visible)`
    // changed contributes one dirty rect — the union of its old and new row bands —
    // so a status value that changes repaints only its own row and an unchanged
    // value (the common case) costs nothing. Text items diff by stable index, like
    // sprites and board cells, so they never shift the positional command count.
    for index in 0..GAME2D_MAX_TEXT_ITEMS {
        if previous.text_items[index] != current.text_items[index] {
            if let Some(rect) = text_dirty_rect(previous, current, index) {
                push_dirty(
                    &mut dirty,
                    &mut probe_len,
                    &mut dirty_area,
                    &mut probe_overflow,
                    rect,
                );
            }
        }
    }
    // GFX-0010 Stage 1B: coalesce the full expanded raw dirty set *before* deciding, so a
    // fragmented-but-coalescible frame stays incremental at its true post-coalesce pass
    // count instead of escalating on the raw rect count (the coalesce-ordering defect this
    // issue addresses). The decision now reads the post-coalesce count and a re-summed
    // post-coalesce area. The 25-cap snapshot (`decision_snapshot`) is computed only so the
    // `phase=171` diagnostic can report what the *pre-coalesce* order would have decided
    // (`old_reason` / `converted_to_incremental`); it no longer drives the present.
    //
    // Truncation fail-safe: if the expanded probe overflowed (`probe_overflow`, e.g. a wide
    // command restructure that bailed) or the board fragmented past its band buffer
    // (`board_overflow`), the raw set is incomplete, so `coalesce_then_decide` forces a full
    // repaint regardless of the coalesced count — preserving the pre-coalesce overflow /
    // `CommandCountShift` safety unless the dirty set is fully derived and non-truncated.
    // Coalescing only grows rects and never drops a region, so recompositing the survivors
    // reproduces the same pixels (KOTO-0159): a rescued frame is pixel-identical to the
    // full repaint it replaces.
    let (decision_len, decision_rect_overflow) =
        decision_snapshot(probe_len, probe_overflow, DIRTY_RECT_CAP);
    // Pre-coalesce fragmentation geometry, captured before `coalesce_then_decide` mutates
    // `dirty` in place. `rects_pre` / `bbox` / `sample` are now over the *full* raw set (no
    // longer the 25-cap prefix), so the `phase=164` line reports the true fragmentation of a
    // rescued frame; on an unchanged incremental frame `probe_len <= FULL_REPAINT_RECTS`, so
    // this is identical to the pre-Stage-1B geometry.
    let mut geom = DirtyRectGeometry::from_rects(&dirty[..probe_len], dirty_area);
    let outcome = coalesce_then_decide(
        FullRepaintPolicy::default(),
        &mut dirty,
        probe_len,
        dirty_area,
        decision_len,
        decision_rect_overflow,
        board_overflow,
        probe_overflow || board_overflow,
        previous.len != current.len,
        DIRTY_COALESCE_MAX_WASTE,
    );
    if matches!(outcome.decision, DeltaDecision::Skip) {
        return Ok(());
    }
    geom.rects_post = outcome.coalesced_len as u16;
    metrics.record_dirty_geometry(geom);
    if let DeltaDecision::FullRepaint(reason) = outcome.decision {
        // Still escalated after coalescing (irreducible scatter, post-coalesce area over
        // threshold, a truncated raw set, or a wide command restructure). For the
        // rect/area escalations record the coalesce-pressure diagnostic so `phase=171` can
        // classify it, then whole-surface recompose. The collected rects are dead on this
        // branch (the full repaint ignores them); `rects_post` already carries the
        // post-coalesce pass count the frame *would* have cost.
        if matches!(
            reason,
            FullRepaintReason::RectsExceeded | FullRepaintReason::AreaExceeded
        ) {
            metrics.record_coalesce_pressure(outcome.pressure);
        } else if matches!(reason, FullRepaintReason::CommandCountShift) {
            // GFX-0011 Stage 1: the wide edit region is now *collected* (above) into the
            // expanded cap and run through the same `coalesce_then_decide` path as every
            // other layer, so `outcome.pressure` already holds the real post-coalesce
            // contrast for this frame — no separate observe-only dry-run is needed. A frame
            // that stays a `CommandCountShift` full repaint here did so because the raw set
            // was truncated (region past the cap, or a layer overflowed upstream) or the
            // coalesced damage is genuinely wide-area; the alignment shape (computed against
            // the live cap) plus the real pressure classify which. `phase=169`/`phase=174`
            // report it; a rescued (coalescible) count shift is no longer a full repaint and
            // is reported on `phase=171` via `converted_to_incremental` below.
            let shape = EditRegionShape::of(
                &previous.commands[..previous.len],
                &current.commands[..current.len],
                DIRTY_RECT_PROBE_CAP,
            );
            metrics.record_command_shift(shape, outcome.pressure);
        }
        return present_app_commands(
            lcd,
            font,
            current,
            static_layer,
            heap,
            strip,
            scratch,
            reason,
            metrics,
        )
        .await;
    }
    // Incremental: present the coalesced survivors packed into `dirty[..coalesced_len]`.
    // When this frame was *converted* from a pre-coalesce full repaint, record the
    // coalesce-pressure diagnostic too, so `phase=171` reports the rescue even though the
    // frame is no longer a full repaint.
    if outcome.pressure.converted_to_incremental {
        metrics.record_coalesce_pressure(outcome.pressure);
    }
    let dirty_len = outcome.coalesced_len;

    // Transfer the coalesced dirty rects as one pipelined band stream (KOTO-0174
    // H-A2): each rect composes the full layer stack (static -> board -> sprites
    // -> text -> commands over `base`) clipped to it, banded to the half-scratch
    // budget so a rect too tall for one band repaints its own column, not
    // 320x320 (KOTO-0131) — and band N's data DMA runs under band N+1's
    // raster+convert. A coalesced rect that overlaps several layers is harmless —
    // the recomposite reproduces exactly those pixels (KOTO-0135/0140/41).
    present_rects_pipelined(
        lcd,
        font,
        current,
        static_layer,
        heap,
        strip,
        scratch,
        &dirty[..dirty_len],
        base,
        metrics,
    )
    .await
}

/// Half of the RGB666 scratch: the pipelined present (KOTO-0174 H-A2) ping-pongs
/// the existing strip-sized scratch as two halves — zero extra SRAM — so one
/// half can be on the SPI data DMA while the other receives the next band's
/// convert. Caps a band at 8 full-width rows (2,560 px); narrower rects get
/// proportionally taller bands, still within the RGB565 strip.
const RGB666_HALF_BYTES: usize = RGB666_STRIP_BYTES / 2;
const PIPELINE_BAND_PX: usize = RGB666_HALF_BYTES / 3;

/// The synchronous CPU half of one present band (KOTO-0174 H-A2): compose the
/// band off-screen (clear to `base`, paint the full command stack clipped to
/// it), then convert it to the panel's RGB666 into `out`. Runs joined against
/// the previous band's in-flight data DMA, so it must not touch the LCD.
#[allow(clippy::too_many_arguments)]
fn raster_convert_band(
    font: &BitmapFont<'_>,
    host: &DeviceRuntimeHost,
    static_layer: &AppStaticLayer,
    heap: &[u8],
    base: u16,
    band: Rect,
    strip: &mut [u8; RASTER_STRIP_BYTES],
    out: &mut [u8],
    metrics: &mut PaintMetrics,
) -> Result<(), ()> {
    // band pixels <= PIPELINE_BAND_PX, so used <= strip.len().
    let used = band.w as usize * band.h as usize * 2;
    let raster_started = Instant::now();
    let mut canvas = Canvas::new_viewport(&mut strip[..used], 320, 320, band).ok_or(())?;
    let clear_started = Instant::now();
    canvas.clear(koto_core::Rgb565(base));
    metrics.record_clear(clear_started);
    paint_app_commands(&mut canvas, font, host, static_layer, heap);
    metrics.record_raster(raster_started);

    let convert_started = Instant::now();
    crate::lcd::convert_rgb565_to_rgb666(&strip[..used], out, band.w as u16, band.h as u16)
        .ok_or(())?;
    metrics.record_convert(convert_started);
    Ok(())
}

/// Present a set of composed dirty rects as one software-pipelined band stream
/// (KOTO-0174 H-A2). Rects are walked in order and split into bands of at most
/// [`PIPELINE_BAND_PX`] pixels; while band N's RGB666 bytes are on the SPI data
/// DMA (window already open, `write_rgb666_data` polled first inside the join —
/// the exact shape the `phase=178` boot bench proved overlaps completely), band
/// N+1 rasters and converts into the other scratch half. The window prologue
/// (`begin_rgb666`) must wait for the previous band's data to drain — the SPI
/// bus is shared — so the exposed per-band transfer cost is ~74 µs of prologue
/// plus whatever DMA outlives the overlapped CPU work. With
/// [`PRESENT_PIPELINE`] off, the same stream drains serially (identical pixels,
/// identical band geometry; only the wait structure changes) for A/B.
#[allow(clippy::too_many_arguments)]
async fn present_rects_pipelined(
    lcd: &mut PicoCalcLcd<'_>,
    font: &BitmapFont<'_>,
    host: &DeviceRuntimeHost,
    static_layer: &AppStaticLayer,
    heap: &[u8],
    strip: &mut [u8; RASTER_STRIP_BYTES],
    scratch: &mut [u8; RGB666_STRIP_BYTES],
    rects: &[Rect],
    base: u16,
    metrics: &mut PaintMetrics,
) -> Result<(), ()> {
    // `front` holds the pending band's converted bytes (the DMA source); `back`
    // is free for the current band's convert. Swapped after each window opens.
    let (mut front, mut back) = scratch.split_at_mut(RGB666_HALF_BYTES);
    let mut pending: Option<(Rect, usize)> = None;
    let mut rect_index = 0usize;
    let mut band_y: Option<i32> = None;
    loop {
        // Derive the next band of the stream: rects in order, each split into
        // <= PIPELINE_BAND_PX bands (full width -> 8 rows per band).
        let mut next: Option<Rect> = None;
        while rect_index < rects.len() {
            let rect = rects[rect_index];
            let y = band_y.unwrap_or(rect.y);
            let y_end = rect.y.saturating_add(rect.h);
            if y >= y_end || rect.w <= 0 {
                rect_index += 1;
                band_y = None;
                continue;
            }
            let max_lines = ((PIPELINE_BAND_PX / rect.w as usize).max(1)) as i32;
            let lines = (y_end - y).min(max_lines);
            next = Some(Rect {
                x: rect.x,
                y,
                w: rect.w,
                h: lines,
            });
            band_y = Some(y + lines);
            break;
        }

        let Some(band) = next else {
            // Stream exhausted: drain the last band's data and close its window.
            if let Some((prev, prev_bytes)) = pending.take() {
                let dma_started = Instant::now();
                let result = lcd.write_rgb666_data(&front[..prev_bytes]).await;
                lcd.end_rgb666();
                result.map_err(|_| ())?;
                metrics.record_dma_exposed(
                    dma_started.elapsed().as_micros(),
                    (prev.w * prev.h) as u32,
                );
            }
            return Ok(());
        };
        let band_bytes = band.w as usize * band.h as usize * 3;

        if let Some((prev, prev_bytes)) = pending.take() {
            if PRESENT_PIPELINE {
                // The race the phase=178 bench validated: polling the data-DMA
                // future first starts the transfer, then the raster+convert of
                // this band runs to completion underneath it. `raster_us` /
                // `convert_us` record the true CPU cost inside the join; only
                // the *exposed* remainder counts as transfer.
                let join_started = Instant::now();
                let mut cpu_result = Ok(());
                let mut cpu_us = 0u64;
                let (write_result, ()) = join(lcd.write_rgb666_data(&front[..prev_bytes]), async {
                    let cpu_started = Instant::now();
                    cpu_result = raster_convert_band(
                        font,
                        host,
                        static_layer,
                        heap,
                        base,
                        band,
                        strip,
                        &mut back[..],
                        metrics,
                    );
                    cpu_us = cpu_started.elapsed().as_micros();
                })
                .await;
                lcd.end_rgb666();
                write_result.map_err(|_| ())?;
                cpu_result?;
                metrics.record_dma_exposed(
                    join_started.elapsed().as_micros().saturating_sub(cpu_us),
                    (prev.w * prev.h) as u32,
                );
            } else {
                let dma_started = Instant::now();
                let result = lcd.write_rgb666_data(&front[..prev_bytes]).await;
                lcd.end_rgb666();
                result.map_err(|_| ())?;
                metrics.record_dma_exposed(
                    dma_started.elapsed().as_micros(),
                    (prev.w * prev.h) as u32,
                );
                raster_convert_band(
                    font,
                    host,
                    static_layer,
                    heap,
                    base,
                    band,
                    strip,
                    &mut back[..],
                    metrics,
                )?;
            }
        } else {
            // First band of the stream: nothing in flight to hide it under.
            raster_convert_band(
                font,
                host,
                static_layer,
                heap,
                base,
                band,
                strip,
                &mut back[..],
                metrics,
            )?;
        }

        // The bus is free (previous data drained above): open this band's
        // window now, so the next iteration's join starts its data DMA on the
        // first poll.
        let begin_started = Instant::now();
        lcd.begin_rgb666(band.x as u16, band.y as u16, band.w as u16, band.h as u16)
            .await
            .map_err(|_| ())?;
        metrics.record_window_open(begin_started);
        core::mem::swap(&mut front, &mut back);
        pending = Some((band, band_bytes));
    }
}

/// Log a bounded sample of one frame's command list — each command's requested
/// geometry and clipped on-screen rect, plus whether a full-screen base clear
/// leads the frame — for the first-frame draw-pattern audit (KOTO-0131). Capped
/// at `MAX_SAMPLE` lines so a busy board never floods UART or stalls the frame.
pub(crate) fn log_command_sample(
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
    host: &DeviceRuntimeHost,
    static_layer: &AppStaticLayer,
    frame: u32,
) {
    const MAX_SAMPLE: usize = 16;
    let mut line = LineBuffer::new();
    let count = host.len.min(MAX_SAMPLE);
    for index in 0..count {
        let (cx, cy, cw, ch) = match command_at(host, index) {
            AppDrawCommand::Empty => (0, 0, 0, 0),
            AppDrawCommand::Rect { x, y, w, h, .. } | AppDrawCommand::Pixels { x, y, w, h, .. } => {
                match clip_app_rect(x, y, w, h) {
                    Some(r) => (r.x, r.y, r.w, r.h),
                    None => (0, 0, 0, 0),
                }
            }
            AppDrawCommand::Text { x, y, .. } => match clip_app_rect(x, y, 320 - x.max(0), 17) {
                Some(r) => (r.x, r.y, r.w, r.h),
                None => (0, 0, 0, 0),
            },
        };
        line.clear();
        match command_at(host, index) {
            AppDrawCommand::Empty => continue,
            AppDrawCommand::Rect { x, y, w, h, rgb565 } => {
                let _ = write!(
                    line,
                    "phase=161 hc f={} i={} op=rect x={} y={} w={} h={} rgb={} clip={},{},{},{}\r\n",
                    frame, index, x, y, w, h, rgb565, cx, cy, cw, ch
                );
            }
            AppDrawCommand::Pixels {
                x,
                y,
                w,
                h,
                off,
                len,
            } => {
                let _ = write!(
                    line,
                    "phase=161 hc f={} i={} op=pixels x={} y={} w={} h={} off={} len={} clip={},{},{},{}\r\n",
                    frame, index, x, y, w, h, off, len, cx, cy, cw, ch
                );
            }
            AppDrawCommand::Text { x, y, len, .. } => {
                let _ = write!(
                    line,
                    "phase=161 hc f={} i={} op=text x={} y={} len={} clip={},{},{},{}\r\n",
                    frame, index, x, y, len, cx, cy, cw, ch
                );
            }
        }
        uart_write_line(uart, &line);
    }
    line.clear();
    let base = full_screen_base_color(host, static_layer);
    let _ = write!(
        line,
        "phase=161 hc f={} summary len={} sampled={} fullscreen_base={}\r\n",
        frame,
        host.len,
        count,
        base.map(i32::from).unwrap_or(-1),
    );
    uart_write_line(uart, &line);
}

fn command_at(host: &DeviceRuntimeHost, index: usize) -> AppDrawCommand {
    // `clear_frame` resets only `len`, not the backing array, so slots in
    // `[len, commands.len())` still hold this double-buffer's commands from two
    // frames ago. `len` is the logical boundary: anything at or beyond it is *gone*
    // this frame and must read back as `Empty`, so the positional diff dirties a
    // disappeared command's old footprint (`command[i]` old-vs-new where new is
    // `Empty`) instead of comparing against stale bytes that may equal the old
    // command and silently skip the erase (KOTO-0141 stale-overlay fix).
    if index >= host.len {
        return AppDrawCommand::Empty;
    }
    host.commands
        .get(index)
        .copied()
        .unwrap_or(AppDrawCommand::Empty)
}

fn full_screen_base_color(host: &DeviceRuntimeHost, static_layer: &AppStaticLayer) -> Option<u16> {
    // The retained static layer (KOTO-0136) holds the app's full-screen page clear
    // now, so check it first; fall back to the immediate list for apps that still
    // emit their base every frame (and the title screen, before the layer exists).
    full_screen_base_in(&static_layer.commands[..static_layer.len])
        .or_else(|| full_screen_base_in(&host.commands[..host.len]))
}

fn full_screen_base_in(commands: &[AppDrawCommand]) -> Option<u16> {
    commands.iter().find_map(|command| {
        if let AppDrawCommand::Rect { x, y, w, h, rgb565 } = command {
            if *x <= 0 && *y <= 0 && x.saturating_add(*w) >= 320 && y.saturating_add(*h) >= 320 {
                return Some(*rgb565);
            }
        }
        None
    })
}

/// Pixel rect of a coalesced board band on the 320x320 surface, or `None` if it
/// falls fully off-screen (KOTO-0135/0143). Dirty-derivation logic moved to
/// koto-gfx (GFX-0003); the app surface is the fixed 320x320 panel.
fn board_band_rect(band: TileBand, tilemap: &Game2dTilemap) -> Option<Rect> {
    koto_gfx::board_band_rect(band, tilemap, 320, 320)
}

/// Dirty rect for sprite `index`: the union of its old (previous-frame) and new
/// (current-frame) footprints, so a moving instance repaints both the cells it
/// left and the cells it entered (KOTO-0140). Dirty-derivation logic moved to
/// koto-gfx (GFX-0003); each footprint resolves against its own frame's stamps.
fn sprite_dirty_rect(
    previous: &DeviceRuntimeHost,
    current: &DeviceRuntimeHost,
    heap: &[u8],
    index: usize,
) -> Option<Rect> {
    koto_gfx::sprite_dirty_rect(
        &previous.sprites[index],
        &previous.stamps,
        &current.sprites[index],
        &current.stamps,
        heap,
        320,
        320,
    )
}

/// Composite the whole layer stack into `canvas` in the fixed z-order. The
/// compositor moved into koto-gfx (GFX-0004); this firmware adapter keeps the same
/// signature and unpacks `DeviceRuntimeHost`/`AppStaticLayer` into the POD layer
/// slices koto-gfx composites (the GFX-0003 adapter methodology). The fixed
/// static -> board -> sprites -> text -> immediate ordering and the clear-to-base
/// (the caller's `canvas.clear(base)` before this) are unchanged.
fn paint_app_commands(
    canvas: &mut Canvas<'_>,
    font: &BitmapFont<'_>,
    host: &DeviceRuntimeHost,
    static_layer: &AppStaticLayer,
    heap: &[u8],
) {
    koto_gfx::paint_app_commands(
        canvas,
        font,
        &static_layer.commands[..static_layer.len],
        &host.board,
        &host.sprites,
        &host.stamps,
        &host.text_items,
        &host.commands[..host.len],
        heap,
    );
}

/// Dirty rect for text item `index`: the union of its old (previous-frame) and new
/// (current-frame) footprints, so a value that moves or shrinks repaints both the
/// row it left and the row it entered (KOTO-0141). Dirty-derivation logic moved to
/// koto-gfx (GFX-0003); the app surface is the fixed 320x320 panel.
fn text_dirty_rect(
    previous: &DeviceRuntimeHost,
    current: &DeviceRuntimeHost,
    index: usize,
) -> Option<Rect> {
    koto_gfx::text_dirty_rect(
        &previous.text_items[index],
        &current.text_items[index],
        320,
        320,
    )
}

fn clip_app_rect(x: i32, y: i32, w: i32, h: i32) -> Option<Rect> {
    // Delegates to the surface-parameterised koto-gfx helper at the fixed 320x320
    // app surface (KotoGFX migration Stage 1, GFX-0001). Signature unchanged.
    Rect::clip(x, y, w, h, 320, 320)
}

fn rgb565_to_rgb888(color: u16) -> Rgb888 {
    Rgb888 {
        red: (((color >> 11) & 0x1f) as u8) << 3,
        green: (((color >> 5) & 0x3f) as u8) << 2,
        blue: ((color & 0x1f) as u8) << 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::firmware::config::MAX_APP_TEXT_BYTES;

    fn text_cmd(x: i32, y: i32) -> AppDrawCommand {
        AppDrawCommand::Text {
            x,
            y,
            rgb565: 0xFFFF,
            bytes: [b'9'; MAX_APP_TEXT_BYTES],
            len: 3,
        }
    }

    /// Collect the immediate-command diff's dirty rects between two frames, exactly
    /// as `present_app_delta` does — through `koto_gfx::collect_immediate_dirty`
    /// (GFX-0008 prefix/suffix aligned diff), over the `commands[..len]` logical
    /// lists. Lets the invalidation invariants be asserted without the panel.
    fn immediate_dirty_rects(
        previous: &DeviceRuntimeHost,
        current: &DeviceRuntimeHost,
    ) -> Vec<Rect> {
        let mut dirty = [Rect {
            x: 0,
            y: 0,
            w: 0,
            h: 0,
        }; DIRTY_RECT_CAP];
        let (mut len, mut area, mut overflow) = (0usize, 0u32, false);
        collect_immediate_dirty(
            &previous.commands[..previous.len],
            &current.commands[..current.len],
            320,
            320,
            MAX_EDIT_REGION,
            &mut dirty,
            &mut len,
            &mut area,
            &mut overflow,
        );
        dirty[..len].to_vec()
    }

    /// A disappeared immediate command (a transient overlay/highlight cleared from
    /// the list) must dirty its old footprint, even though `clear_frame` leaves the
    /// command bytes in the buffer past `len` — the KOTO-0141 stale-overlay bug:
    /// without `command_at` bounding by `len`, the leftover bytes (here identical to
    /// the previous overlay) compared equal and the erase was silently skipped.
    #[test]
    fn disappeared_immediate_rect_dirties_its_old_footprint() {
        let overlay = AppDrawCommand::Rect {
            x: 20,
            y: 92,
            w: 136,
            h: 30,
            rgb565: 0xABCD,
        };
        let mut previous = DeviceRuntimeHost::new();
        previous.commands[0] = overlay;
        previous.len = 1;
        // The double-buffer this frame reused still holds the overlay bytes from two
        // frames ago, but the app emitted nothing: len = 0.
        let mut current = DeviceRuntimeHost::new();
        current.commands[0] = overlay;
        current.len = 0;

        assert_eq!(
            immediate_dirty_rects(&previous, &current),
            vec![Rect {
                x: 20,
                y: 92,
                w: 136,
                h: 30,
            }]
        );
    }

    /// Same invariant for an immediate `Text` command (a status/banner string): when
    /// it disappears, its old row-band footprint must be dirtied so the stale glyphs
    /// are recomposited away (KOTO-0141).
    #[test]
    fn disappeared_immediate_text_dirties_its_old_footprint() {
        let mut previous = DeviceRuntimeHost::new();
        previous.commands[0] = text_cmd(46, 100);
        previous.len = 1;
        let mut current = DeviceRuntimeHost::new();
        current.commands[0] = text_cmd(46, 100); // stale leftover
        current.len = 0;

        assert_eq!(
            immediate_dirty_rects(&previous, &current),
            vec![Rect {
                x: 46,
                y: 100,
                w: 320 - 46,
                h: 17,
            }]
        );
    }

    /// A moving immediate command dirties the union of its old and new footprints, so
    /// both the cells it left and the cells it entered are recomposited.
    #[test]
    fn moved_immediate_command_dirties_union_of_footprints() {
        let mut previous = DeviceRuntimeHost::new();
        previous.commands[0] = AppDrawCommand::Rect {
            x: 10,
            y: 10,
            w: 16,
            h: 16,
            rgb565: 1,
        };
        previous.len = 1;
        let mut current = DeviceRuntimeHost::new();
        current.commands[0] = AppDrawCommand::Rect {
            x: 40,
            y: 10,
            w: 16,
            h: 16,
            rgb565: 1,
        };
        current.len = 1;

        assert_eq!(
            immediate_dirty_rects(&previous, &current),
            vec![Rect {
                x: 10,
                y: 10,
                w: 46,
                h: 16,
            }]
        );
    }

    /// A normal falling frame (empty immediate list both sides, ignoring any stale
    /// buffer bytes) produces no immediate dirty rects — the KOTO-0141 steady-state.
    #[test]
    fn stable_empty_immediate_list_is_clean() {
        let mut previous = DeviceRuntimeHost::new();
        // Stale leftovers past len from an earlier overlay must be ignored.
        previous.commands[0] = AppDrawCommand::Rect {
            x: 1,
            y: 2,
            w: 3,
            h: 4,
            rgb565: 5,
        };
        previous.len = 0;
        let current = DeviceRuntimeHost::new();
        assert!(immediate_dirty_rects(&previous, &current).is_empty());
    }
}
