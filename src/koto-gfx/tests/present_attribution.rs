//! KOTO-0174 Stage 0c: host attribution microbench for the present path.
//!
//! Replays a KotoRun-shaped frame (the `phase=160` command mix: 8 static
//! commands, 58 immediate rects, 10 immediate text items, no board/sprites)
//! through the *real* koto-gfx compositor, clipped to two dirty rects totaling
//! ~4,184 px like the device baseline, and attributes the wall time per phase:
//! clear, static list, immediate list, and the separate RGB565->RGB666
//! conversion the device's `write_rgb565_rect` runs before the SPI transfer.
//!
//! Host absolute times are NOT device times (x86 at GHz vs Cortex-M0+ at
//! 125 MHz running fills from flash XIP), but two things transfer:
//!   * the *ratios* between phases (all run the same compositor), and
//!   * the *pixel-pass count* (how many times each dirty pixel is written) —
//!     which is exact and is the fork between a per-pixel lever (H-A/H-B) and a
//!     per-command lever (H-C).
//!
//! Run: `cargo test -p koto-gfx --test present_attribution -- --ignored --nocapture`

use std::time::Instant;

use koto_gfx::{
    paint_app_commands, paint_command_list, AppDrawCommand, BitmapFont, Canvas, Game2dSprite,
    Game2dStampDef, Game2dText, Game2dTilemap, Rect, Rgb565,
};

const FONT_BYTES: &[u8] = include_bytes!("../../../assets/fonts/mplus12.kfont");

const SURFACE: u16 = 320;

/// The two device-baseline dirty rects (a status text row + a small readout),
/// ~4,184 px total, matching the `dirty_px=4184 rects=2` KotoRun sample.
const DIRTY: [Rect; 2] = [
    Rect {
        x: 8,
        y: 40,
        w: 200,
        h: 16,
    }, // 3,200 px
    Rect {
        x: 240,
        y: 60,
        w: 66,
        h: 15,
    }, // 990 px
];

/// Build the KotoRun-shaped command stack: a full-screen base clear plus 7
/// chrome rects as the static layer, and 58 rects + 10 short text items as the
/// immediate list. Geometry is representative (scattered across the surface so
/// most commands clip-reject against a given dirty rect, exactly like the real
/// per-rect recomposite), not pixel-identical to the app.
fn static_layer() -> Vec<AppDrawCommand> {
    let mut v = vec![AppDrawCommand::Rect {
        x: 0,
        y: 0,
        w: 320,
        h: 320,
        rgb565: 0x0000,
    }];
    for i in 0..7 {
        v.push(AppDrawCommand::Rect {
            x: 4 + i * 44,
            y: 4,
            w: 40,
            h: 300,
            rgb565: 0x2104,
        });
    }
    v
}

fn text_command(x: i32, y: i32, s: &str, rgb565: u16) -> AppDrawCommand {
    let mut bytes = [0u8; 64];
    let b = s.as_bytes();
    let len = b.len().min(64);
    bytes[..len].copy_from_slice(&b[..len]);
    AppDrawCommand::Text {
        x,
        y,
        rgb565,
        bytes,
        len: len as u8,
    }
}

fn immediate_rects() -> Vec<AppDrawCommand> {
    // 58 small scattered rects (HUD cells, gauges, tokens).
    (0..58)
        .map(|i| {
            let col = i % 10;
            let row = i / 10;
            AppDrawCommand::Rect {
                x: 8 + col * 30,
                y: 40 + row * 24,
                w: 26,
                h: 14,
                rgb565: 0xFFE0u16.wrapping_add(i as u16 * 7),
            }
        })
        .collect()
}

fn immediate_texts() -> Vec<AppDrawCommand> {
    // 10 short status strings, several overlapping dirty rect A's row band.
    let strings = [
        "SCORE 001280",
        "HP 12/12",
        "LV 3",
        "GOLD 47",
        "FLOOR 2",
        "TURN 214",
        "STR 8",
        "DEX 6",
        "MAG 4",
        "def 5",
    ];
    strings
        .iter()
        .enumerate()
        .map(|(i, s)| text_command(8, 40 + (i as i32 % 3) * 16, s, 0xFFFF))
        .collect()
}

fn immediate_list() -> Vec<AppDrawCommand> {
    let mut v = immediate_rects();
    v.extend(immediate_texts());
    v
}

fn empty_board() -> Game2dTilemap {
    Game2dTilemap::legacy()
}

/// One dirty-rect recomposite exactly as `present_app_delta` does it: a fresh
/// viewport canvas over the rect, cleared to base, then the whole stack.
#[allow(clippy::too_many_arguments)]
fn recomposite(
    buf: &mut [u8],
    rect: Rect,
    font: &BitmapFont<'_>,
    statics: &[AppDrawCommand],
    board: &Game2dTilemap,
    sprites: &[Game2dSprite],
    stamps: &[Game2dStampDef],
    texts: &[Game2dText],
    immediate: &[AppDrawCommand],
) {
    let used = rect.w as usize * rect.h as usize * 2;
    let mut canvas = Canvas::new_viewport(&mut buf[..used], SURFACE, SURFACE, rect).unwrap();
    canvas.clear(Rgb565(0x0000));
    paint_app_commands(
        &mut canvas,
        font,
        statics,
        board,
        sprites,
        stamps,
        texts,
        immediate,
        &[],
    );
}

/// Replicate the device `write_rgb565_rect` CPU RGB565->RGB666 conversion so the
/// bench can weigh it against the raster passes (its cost currently hides inside
/// `transfer_us`).
fn convert_565_to_666(src: &[u8], dst: &mut [u8]) {
    for (rgb565, rgb666) in src.chunks_exact(2).zip(dst.chunks_exact_mut(3)) {
        let value = u16::from_le_bytes([rgb565[0], rgb565[1]]);
        rgb666[0] = ((value >> 11) as u8 & 0x1f) << 3;
        rgb666[1] = ((value >> 5) as u8 & 0x3f) << 2;
        rgb666[2] = (value as u8 & 0x1f) << 3;
    }
}

#[test]
#[ignore = "timing microbench; run with --ignored --nocapture"]
fn present_path_phase_attribution() {
    let font = BitmapFont::from_bytes(FONT_BYTES).expect("mplus12 font");
    let statics = static_layer();
    let immediate = immediate_list();
    let imm_rects = immediate_rects();
    let imm_texts = immediate_texts();
    let board = empty_board();
    let sprites: [Game2dSprite; 0] = [];
    let stamps: [Game2dStampDef; 0] = [];
    let texts: [Game2dText; 0] = [];

    let dirty_px: usize = DIRTY.iter().map(|r| (r.w * r.h) as usize).sum();
    let max_used = DIRTY
        .iter()
        .map(|r| r.w as usize * r.h as usize * 2)
        .max()
        .unwrap();
    let mut buf = vec![0u8; max_used];
    let mut rgb666 = vec![0u8; max_used / 2 * 3];

    const ITERS: u32 = 20_000;

    // Warm up (page in code, prime caches) so the timed loop is steady-state.
    for _ in 0..500 {
        for &rect in &DIRTY {
            recomposite(
                &mut buf, rect, &font, &statics, &board, &sprites, &stamps, &texts, &immediate,
            );
        }
    }

    // Phase: full recomposite (clear + whole stack), the device `raster_us`.
    let t = Instant::now();
    for _ in 0..ITERS {
        for &rect in &DIRTY {
            recomposite(
                &mut buf, rect, &font, &statics, &board, &sprites, &stamps, &texts, &immediate,
            );
        }
    }
    let full_ns = t.elapsed().as_nanos() as f64 / ITERS as f64;

    // Phase: clear only (pure fill_rect over the dirty area — the raw per-pixel
    // put_pixel cost, one pass).
    let t = Instant::now();
    for _ in 0..ITERS {
        for &rect in &DIRTY {
            let used = rect.w as usize * rect.h as usize * 2;
            let mut canvas =
                Canvas::new_viewport(&mut buf[..used], SURFACE, SURFACE, rect).unwrap();
            canvas.clear(Rgb565(0x0000));
            std::hint::black_box(&canvas);
        }
    }
    let clear_ns = t.elapsed().as_nanos() as f64 / ITERS as f64;

    // Phase: static list only (8 commands, clipped) over a pre-cleared canvas.
    let t = Instant::now();
    for _ in 0..ITERS {
        for &rect in &DIRTY {
            let used = rect.w as usize * rect.h as usize * 2;
            let mut canvas =
                Canvas::new_viewport(&mut buf[..used], SURFACE, SURFACE, rect).unwrap();
            paint_command_list(&mut canvas, &font, &[], &statics);
            std::hint::black_box(&canvas);
        }
    }
    let static_ns = t.elapsed().as_nanos() as f64 / ITERS as f64;

    // Phase: immediate list only (58 rects + 10 text, clipped).
    let t = Instant::now();
    for _ in 0..ITERS {
        for &rect in &DIRTY {
            let used = rect.w as usize * rect.h as usize * 2;
            let mut canvas =
                Canvas::new_viewport(&mut buf[..used], SURFACE, SURFACE, rect).unwrap();
            paint_command_list(&mut canvas, &font, &[], &immediate);
            std::hint::black_box(&canvas);
        }
    }
    let immediate_ns = t.elapsed().as_nanos() as f64 / ITERS as f64;

    // Split the immediate list: 58 rect fills vs 10 text (glyph raster) — the
    // fill-loop lever and the glyph-loop lever target different code.
    let mut bench_list = |list: &[AppDrawCommand]| {
        let t = Instant::now();
        for _ in 0..ITERS {
            for &rect in &DIRTY {
                let used = rect.w as usize * rect.h as usize * 2;
                let mut canvas =
                    Canvas::new_viewport(&mut buf[..used], SURFACE, SURFACE, rect).unwrap();
                paint_command_list(&mut canvas, &font, &[], list);
                std::hint::black_box(&canvas);
            }
        }
        t.elapsed().as_nanos() as f64 / ITERS as f64
    };
    let imm_rects_ns = bench_list(&imm_rects);
    let imm_texts_ns = bench_list(&imm_texts);

    // Phase: pure per-command CLIP-REJECT floor. Recomposite the whole stack
    // clipped to a 1x1 corner rect no command paints into — isolating the
    // fixed cost of *walking* 76 commands (match dispatch + clip math, and
    // critically the per-glyph-pixel clip-reject inside `draw_text` for the 10
    // text items). This is the ceiling on what per-command culling (H-C) could
    // remove; the remainder of FULL is genuine per-pixel paint (H-A/H-B).
    let corner = Rect {
        x: 319,
        y: 319,
        w: 1,
        h: 1,
    };
    let t = Instant::now();
    for _ in 0..ITERS {
        let mut canvas = Canvas::new_viewport(&mut buf[..2], SURFACE, SURFACE, corner).unwrap();
        canvas.clear(Rgb565(0x0000));
        paint_app_commands(
            &mut canvas,
            &font,
            &statics,
            &board,
            &sprites,
            &stamps,
            &texts,
            &immediate,
            &[],
        );
        std::hint::black_box(&canvas);
    }
    let reject_ns = t.elapsed().as_nanos() as f64 / ITERS as f64;

    // Phase: RGB565->RGB666 conversion over the dirty area (device transfer CPU cost).
    let t = Instant::now();
    for _ in 0..ITERS {
        for &rect in &DIRTY {
            let used = rect.w as usize * rect.h as usize * 2;
            convert_565_to_666(&buf[..used], &mut rgb666[..used / 2 * 3]);
            std::hint::black_box(&rgb666);
        }
    }
    let convert_ns = t.elapsed().as_nanos() as f64 / ITERS as f64;

    let per_px = |ns: f64| ns / dirty_px as f64;
    println!("\n=== KOTO-0174 Stage 0c present-path attribution (host) ===");
    println!(
        "frame shape: 2 dirty rects, {dirty_px} px; static=8 immediate=68 (58 rect + 10 text)"
    );
    println!("{:<22} {:>10} {:>12}", "phase", "ns/frame", "ns/dirty_px");
    println!(
        "{:<22} {:>10.0} {:>12.3}",
        "clear (1 fill pass)",
        clear_ns,
        per_px(clear_ns)
    );
    println!(
        "{:<22} {:>10.0} {:>12.3}",
        "static list (8)",
        static_ns,
        per_px(static_ns)
    );
    println!(
        "{:<22} {:>10.0} {:>12.3}",
        "immediate list (68)",
        immediate_ns,
        per_px(immediate_ns)
    );
    println!(
        "{:<22} {:>10.0} {:>12}",
        "  imm rects (58)", imm_rects_ns, "-"
    );
    println!(
        "{:<22} {:>10.0} {:>12}",
        "  imm text (10)", imm_texts_ns, "-"
    );
    println!(
        "{:<22} {:>10.0} {:>12.3}",
        "FULL recomposite",
        full_ns,
        per_px(full_ns)
    );
    println!(
        "{:<22} {:>10.0} {:>12}",
        "clip-reject floor (76)", reject_ns, "-"
    );
    println!(
        "{:<22} {:>10.0} {:>12.3}",
        "convert 565->666",
        convert_ns,
        per_px(convert_ns)
    );
    println!(
        "sum(clear+static+immediate) = {:.0} ns vs FULL {:.0} ns ({:.0}% accounted)",
        clear_ns + static_ns + immediate_ns,
        full_ns,
        (clear_ns + static_ns + immediate_ns) / full_ns * 100.0
    );
    println!(
        "clear share of FULL: {:.0}%   immediate share: {:.0}%",
        clear_ns / full_ns * 100.0,
        immediate_ns / full_ns * 100.0
    );
    println!(
        "per-command floor (H-C ceiling): {:.0}% of FULL; per-pixel paint remainder (H-A/H-B): {:.0}%",
        reject_ns / full_ns * 100.0,
        (full_ns - reject_ns) / full_ns * 100.0
    );
}
