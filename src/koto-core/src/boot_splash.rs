//! Boot splash screen shared by the device firmware and KotoSim (KOTO-0181).
//!
//! The splash is the identity moment between the panel lighting up (KOTO-0026)
//! and the shell's first paint: a dark night scene (cloaked cat mascot,
//! crescent moon, radio tower) over a large "KotoOS" wordmark, a tagline, a
//! boot checklist that mirrors the *real* `phase=` UART init milestones, and a
//! progress bar.
//!
//! Rendering follows the [`ShellState`](crate::shell::ShellState) pattern: the
//! same [`Canvas`] painter drives the simulator's full framebuffer and the
//! device's strip-at-a-time transfer path via [`BootSplash::paint_rect`], so
//! both targets show byte-identical pixels. All art is 1-bit-per-layer glyph
//! art embedded in flash (no RGB565 background asset; a full-screen bitmap
//! would cost ~150 KiB against the KOTO-0176 XIP/SRAM pressure).
//!
//! The checklist is *state*, not animation: the firmware resolves each
//! [`BootStep`] at the moment the matching init phase actually completes and
//! repaints only [`splash_step_rect`] + [`splash_progress_rect`], so the splash
//! adds no wall time of its own. Failures surface as `[ng]` lines with a short
//! note (e.g. SD missing) rather than only in UART.

use crate::font::BitmapFont;
use crate::hal::Rect;
use crate::raster::{Canvas, Rgb565};
use crate::KOTO_COPYRIGHT_NOTICE;

/// Number of checklist steps on the splash.
pub const SPLASH_STEP_COUNT: usize = 6;

/// One init milestone shown on the boot checklist, in real device boot order.
///
/// Each step corresponds to an existing `phase=` UART milestone; the hex code
/// rendered on its checklist line is that phase number (so a photo of the
/// splash can be correlated with a UART log).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BootStep {
    /// Core bring-up: clocks + UART alive (`phase=10 uart-ready`).
    Kernel,
    /// PSRAM block device (`phase=16 psram-ready`).
    Memory,
    /// CPU1 audio worker + PCM diagnostic (`phase=171 audio_pcm_diag`).
    Audio,
    /// SD mount + package catalog scan (`phase=14 catalog-ready`).
    Storage,
    /// Keyboard/power bridge first poll (`phase=146 battery`).
    Input,
    /// Shell model ready for its first paint (`phase=22 shell-render-ok`).
    Shell,
}

impl BootStep {
    pub const ALL: [BootStep; SPLASH_STEP_COUNT] = [
        BootStep::Kernel,
        BootStep::Memory,
        BootStep::Audio,
        BootStep::Storage,
        BootStep::Input,
        BootStep::Shell,
    ];

    pub fn label(self) -> &'static str {
        match self {
            BootStep::Kernel => "init kernel",
            BootStep::Memory => "init memory",
            BootStep::Audio => "init audio",
            BootStep::Storage => "mount sd",
            BootStep::Input => "init input",
            BootStep::Shell => "init shell",
        }
    }

    /// The `phase=` number of the UART milestone this step mirrors.
    pub fn phase_code(self) -> u8 {
        match self {
            BootStep::Kernel => 10,
            BootStep::Memory => 16,
            BootStep::Audio => 171,
            BootStep::Storage => 14,
            BootStep::Input => 146,
            BootStep::Shell => 22,
        }
    }

    pub fn index(self) -> usize {
        match self {
            BootStep::Kernel => 0,
            BootStep::Memory => 1,
            BootStep::Audio => 2,
            BootStep::Storage => 3,
            BootStep::Input => 4,
            BootStep::Shell => 5,
        }
    }
}

/// Outcome of one [`BootStep`]. `Failed` carries a short note rendered after
/// the label (`&'static str` so the splash stays allocation-free on device).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BootStepStatus {
    Pending,
    Ok,
    Failed(&'static str),
}

/// Splash checklist state: which init milestones have resolved and how.
#[derive(Clone, Copy, Debug)]
pub struct BootSplash {
    statuses: [BootStepStatus; SPLASH_STEP_COUNT],
}

impl Default for BootSplash {
    fn default() -> Self {
        Self::new()
    }
}

impl BootSplash {
    pub const fn new() -> Self {
        Self {
            statuses: [BootStepStatus::Pending; SPLASH_STEP_COUNT],
        }
    }

    /// A fully booted splash (every step `Ok`), for simulator screenshots and
    /// golden frames.
    pub const fn complete() -> Self {
        Self {
            statuses: [BootStepStatus::Ok; SPLASH_STEP_COUNT],
        }
    }

    pub fn resolve(&mut self, step: BootStep, status: BootStepStatus) {
        self.statuses[step.index()] = status;
    }

    pub fn status(&self, step: BootStep) -> BootStepStatus {
        self.statuses[step.index()]
    }

    /// Steps that have reached a terminal state (`Ok` or `Failed`); drives the
    /// progress bar fill.
    pub fn resolved_count(&self) -> usize {
        self.statuses
            .iter()
            .filter(|status| !matches!(status, BootStepStatus::Pending))
            .count()
    }

    pub fn any_failed(&self) -> bool {
        self.statuses
            .iter()
            .any(|status| matches!(status, BootStepStatus::Failed(_)))
    }

    /// Rasterize the whole splash using [`SplashPalette::DEFAULT`].
    pub fn paint(&self, canvas: &mut Canvas<'_>, font: &BitmapFont<'_>) {
        self.paint_rect(canvas, font, splash_surface_rect());
    }

    /// Rasterize only the portion of the splash intersecting `clip`; pixels
    /// inside `clip` are identical to a full [`paint`](Self::paint). This is
    /// the device strip/dirty-rect path (same contract as
    /// [`ShellState::paint_rect`](crate::shell::ShellState::paint_rect)).
    pub fn paint_rect(&self, canvas: &mut Canvas<'_>, font: &BitmapFont<'_>, clip: Rect) {
        let palette = &SplashPalette::DEFAULT;
        canvas.fill_rect(clip, palette.background);

        for &(x, y, size) in &STARS {
            let star = Rect {
                x,
                y,
                w: size,
                h: size,
            };
            if rects_intersect(clip, star) {
                canvas.fill_rect(star, palette.star);
            }
        }
        self.paint_art(canvas, clip, TOWER_X, TOWER_Y, ART_SCALE, &TOWER_ART);
        self.paint_art(canvas, clip, MOON_X, MOON_Y, ART_SCALE, &MOON_ART);
        self.paint_art(canvas, clip, CAT_X, CAT_Y, ART_SCALE, &CAT_ART);

        if rects_intersect(clip, wordmark_rect(font)) {
            self.paint_wordmark(canvas, font, palette);
        }
        if rects_intersect(clip, tagline_rect(font)) {
            let width = text_width(font, TAGLINE);
            canvas.draw_text(
                (SPLASH_WIDTH - width) / 2,
                TAGLINE_Y,
                font,
                TAGLINE,
                palette.tagline,
            );
        }
        if rects_intersect(clip, copyright_rect(font)) {
            let width = text_width(font, KOTO_COPYRIGHT_NOTICE);
            canvas.draw_text(
                (SPLASH_WIDTH - width) / 2,
                COPYRIGHT_Y,
                font,
                KOTO_COPYRIGHT_NOTICE,
                palette.code,
            );
        }
        for step in BootStep::ALL {
            if rects_intersect(clip, splash_step_rect(step)) {
                self.paint_step_line(canvas, font, palette, step);
            }
        }
        if rects_intersect(clip, splash_progress_rect()) {
            self.paint_progress(canvas, palette);
        }
    }

    /// Draw one glyph-art block if it intersects `clip`. The block clips per
    /// cell through [`Canvas::fill_rect`], so a partial intersection paints
    /// exactly the covered cells.
    fn paint_art(
        &self,
        canvas: &mut Canvas<'_>,
        clip: Rect,
        x: i32,
        y: i32,
        scale: i32,
        rows: &[&str],
    ) {
        if !rects_intersect(clip, art_rect(x, y, scale, rows)) {
            return;
        }
        let palette = &SplashPalette::DEFAULT;
        for (row, line) in rows.iter().enumerate() {
            for (col, cell) in line.bytes().enumerate() {
                let color = match cell {
                    b'c' => palette.cloak,
                    b'd' => palette.cloak_dark,
                    b'f' => palette.face,
                    b'e' => palette.eye,
                    b'm' => palette.moon,
                    b't' => palette.tower,
                    b'b' => palette.beacon,
                    _ => continue,
                };
                canvas.fill_rect(
                    Rect {
                        x: x + col as i32 * scale,
                        y: y + row as i32 * scale,
                        w: scale,
                        h: scale,
                    },
                    color,
                );
            }
        }
    }

    /// Large "KotoOS" wordmark: integer-scaled font glyphs, "Koto" in the
    /// light foreground and "OS" in the accent color.
    fn paint_wordmark(
        &self,
        canvas: &mut Canvas<'_>,
        font: &BitmapFont<'_>,
        palette: &SplashPalette,
    ) {
        let total = text_width_scaled(font, WORDMARK_HEAD, WORDMARK_SCALE)
            + text_width_scaled(font, WORDMARK_TAIL, WORDMARK_SCALE);
        let x = (SPLASH_WIDTH - total) / 2;
        let after_head = draw_text_scaled(
            canvas,
            x,
            WORDMARK_Y,
            font,
            WORDMARK_HEAD,
            palette.wordmark,
            WORDMARK_SCALE,
        );
        draw_text_scaled(
            canvas,
            after_head,
            WORDMARK_Y,
            font,
            WORDMARK_TAIL,
            palette.accent,
            WORDMARK_SCALE,
        );
    }

    fn paint_step_line(
        &self,
        canvas: &mut Canvas<'_>,
        font: &BitmapFont<'_>,
        palette: &SplashPalette,
        step: BootStep,
    ) {
        let y = CHECKLIST_Y + step.index() as i32 * CHECKLIST_LINE_H;
        let status = self.status(step);
        let (marker, marker_color, text_color) = match status {
            BootStepStatus::Pending => ("[--]", palette.pending, palette.pending),
            BootStepStatus::Ok => ("[ok]", palette.ok, palette.checklist),
            BootStepStatus::Failed(_) => ("[ng]", palette.fail, palette.checklist),
        };
        let mut cursor = canvas.draw_text(CHECKLIST_X, y, font, marker, marker_color);
        let space = i32::from(font.half_width());
        let mut hex = [0u8; 4];
        cursor = canvas.draw_text(
            cursor + space,
            y,
            font,
            hex_code(step.phase_code(), &mut hex),
            palette.code,
        );
        cursor = canvas.draw_text(cursor + space, y, font, step.label(), text_color);
        if let BootStepStatus::Failed(note) = status {
            canvas.draw_text(cursor + space * 2, y, font, note, palette.fail);
        }
    }

    fn paint_progress(&self, canvas: &mut Canvas<'_>, palette: &SplashPalette) {
        let outer = Rect {
            x: PROGRESS_X,
            y: PROGRESS_Y,
            w: PROGRESS_W,
            h: PROGRESS_H,
        };
        canvas.fill_rect(outer, palette.progress_border);
        canvas.fill_rect(
            Rect {
                x: outer.x + 1,
                y: outer.y + 1,
                w: outer.w - 2,
                h: outer.h - 2,
            },
            palette.progress_bg,
        );
        let fill = (outer.w - 2) * self.resolved_count() as i32 / SPLASH_STEP_COUNT as i32;
        if fill > 0 {
            canvas.fill_rect(
                Rect {
                    x: outer.x + 1,
                    y: outer.y + 1,
                    w: fill,
                    h: outer.h - 2,
                },
                palette.accent,
            );
        }
    }
}

/// Splash colors: a dark counterpart to [`ShellPalette`](crate::shell::ShellPalette)
/// (same status green/red, same light foreground as the shell header) with a
/// warm amber accent for the wordmark tail, eyes, moon, and progress fill.
pub struct SplashPalette {
    pub background: Rgb565,
    pub star: Rgb565,
    pub moon: Rgb565,
    pub tower: Rgb565,
    pub beacon: Rgb565,
    pub cloak: Rgb565,
    pub cloak_dark: Rgb565,
    pub face: Rgb565,
    pub eye: Rgb565,
    pub wordmark: Rgb565,
    pub accent: Rgb565,
    pub tagline: Rgb565,
    pub checklist: Rgb565,
    pub code: Rgb565,
    pub pending: Rgb565,
    pub ok: Rgb565,
    pub fail: Rgb565,
    pub progress_bg: Rgb565,
    pub progress_border: Rgb565,
}

impl SplashPalette {
    pub const DEFAULT: SplashPalette = SplashPalette {
        background: Rgb565::from_rgb8(10, 12, 26),
        star: Rgb565::from_rgb8(122, 132, 164),
        moon: Rgb565::from_rgb8(240, 214, 140),
        tower: Rgb565::from_rgb8(94, 104, 136),
        beacon: Rgb565::from_rgb8(224, 96, 84),
        cloak: Rgb565::from_rgb8(64, 80, 132),
        cloak_dark: Rgb565::from_rgb8(38, 48, 84),
        face: Rgb565::from_rgb8(238, 226, 200),
        eye: Rgb565::from_rgb8(240, 178, 64),
        wordmark: Rgb565::from_rgb8(236, 240, 248),
        accent: Rgb565::from_rgb8(240, 178, 64),
        tagline: Rgb565::from_rgb8(150, 160, 180),
        checklist: Rgb565::from_rgb8(198, 206, 222),
        code: Rgb565::from_rgb8(110, 120, 150),
        pending: Rgb565::from_rgb8(84, 92, 118),
        ok: Rgb565::from_rgb8(96, 200, 120),
        fail: Rgb565::from_rgb8(224, 96, 84),
        progress_bg: Rgb565::from_rgb8(28, 34, 56),
        progress_border: Rgb565::from_rgb8(60, 70, 92),
    };
}

const SPLASH_WIDTH: i32 = 320;
const SPLASH_HEIGHT: i32 = 320;

const ART_SCALE: i32 = 3;
const CAT_X: i32 = 118;
const CAT_Y: i32 = 42;
const MOON_X: i32 = 232;
const MOON_Y: i32 = 34;
const TOWER_X: i32 = 40;
const TOWER_Y: i32 = 40;

const WORDMARK_HEAD: &str = "Koto";
const WORDMARK_TAIL: &str = "OS";
const WORDMARK_Y: i32 = 154;
const WORDMARK_SCALE: i32 = 4;

const TAGLINE: &str = "tiny system. big adventure.";
const TAGLINE_Y: i32 = 206;
const COPYRIGHT_Y: i32 = 219;

const CHECKLIST_X: i32 = 72;
const CHECKLIST_Y: i32 = 234;
const CHECKLIST_LINE_H: i32 = 12;

const PROGRESS_X: i32 = 72;
const PROGRESS_Y: i32 = 309;
const PROGRESS_W: i32 = 176;
const PROGRESS_H: i32 = 7;

/// Fixed star field, kept clear of the three art blocks. `(x, y, size)`.
const STARS: [(i32, i32, i32); 12] = [
    (16, 22, 2),
    (58, 14, 1),
    (96, 30, 2),
    (150, 16, 1),
    (192, 28, 2),
    (226, 12, 1),
    (296, 24, 2),
    (306, 66, 1),
    (18, 96, 1),
    (100, 130, 1),
    (250, 120, 2),
    (288, 142, 1),
];

pub fn splash_surface_rect() -> Rect {
    Rect {
        x: 0,
        y: 0,
        w: SPLASH_WIDTH,
        h: SPLASH_HEIGHT,
    }
}

/// Full-width band of one checklist line: the device repaints exactly this
/// rect when the step resolves.
pub fn splash_step_rect(step: BootStep) -> Rect {
    Rect {
        x: 0,
        y: CHECKLIST_Y + step.index() as i32 * CHECKLIST_LINE_H,
        w: SPLASH_WIDTH,
        h: CHECKLIST_LINE_H,
    }
}

/// Full-width band around the progress bar.
pub fn splash_progress_rect() -> Rect {
    Rect {
        x: 0,
        y: PROGRESS_Y - 2,
        w: SPLASH_WIDTH,
        h: PROGRESS_H + 4,
    }
}

fn wordmark_rect(font: &BitmapFont<'_>) -> Rect {
    Rect {
        x: 0,
        y: WORDMARK_Y,
        w: SPLASH_WIDTH,
        h: i32::from(font.cell_height()) * WORDMARK_SCALE,
    }
}

fn tagline_rect(font: &BitmapFont<'_>) -> Rect {
    Rect {
        x: 0,
        y: TAGLINE_Y,
        w: SPLASH_WIDTH,
        h: i32::from(font.cell_height()),
    }
}

fn copyright_rect(font: &BitmapFont<'_>) -> Rect {
    Rect {
        x: 0,
        y: COPYRIGHT_Y,
        w: SPLASH_WIDTH,
        h: i32::from(font.cell_height()),
    }
}

fn art_rect(x: i32, y: i32, scale: i32, rows: &[&str]) -> Rect {
    let w = rows.first().map(|row| row.len()).unwrap_or(0) as i32;
    Rect {
        x,
        y,
        w: w * scale,
        h: rows.len() as i32 * scale,
    }
}

fn rects_intersect(a: Rect, b: Rect) -> bool {
    a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
}

fn text_width(font: &BitmapFont<'_>, text: &str) -> i32 {
    text.chars()
        .map(|ch| {
            font.glyph(ch)
                .map(|glyph| i32::from(glyph.width()))
                .unwrap_or_else(|| i32::from(font.half_width()))
        })
        .sum()
}

fn text_width_scaled(font: &BitmapFont<'_>, text: &str, scale: i32) -> i32 {
    text_width(font, text) * scale
}

/// Draw `text` with each glyph pixel expanded to a `scale`x`scale` block.
/// Returns the x cursor after the last glyph.
fn draw_text_scaled(
    canvas: &mut Canvas<'_>,
    x: i32,
    y: i32,
    font: &BitmapFont<'_>,
    text: &str,
    color: Rgb565,
    scale: i32,
) -> i32 {
    let mut cursor = x;
    for ch in text.chars() {
        let Some(glyph) = font.glyph(ch) else {
            cursor += i32::from(font.half_width()) * scale;
            continue;
        };
        for gy in 0..glyph.height() {
            for gx in 0..glyph.width() {
                if glyph.pixel(gx, gy) {
                    canvas.fill_rect(
                        Rect {
                            x: cursor + i32::from(gx) * scale,
                            y: y + i32::from(gy) * scale,
                            w: scale,
                            h: scale,
                        },
                        color,
                    );
                }
            }
        }
        cursor += i32::from(glyph.width()) * scale;
    }
    cursor
}

/// Format a byte as `0xNN` into `out`; the returned slice borrows it.
fn hex_code(code: u8, out: &mut [u8; 4]) -> &str {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    out[0] = b'0';
    out[1] = b'x';
    out[2] = HEX[usize::from(code >> 4)];
    out[3] = HEX[usize::from(code & 0x0F)];
    // The buffer is ASCII by construction.
    core::str::from_utf8(out).unwrap_or("0x??")
}

// --- Glyph art -------------------------------------------------------------
//
// One char per art pixel, drawn at `ART_SCALE`: `.` transparent, `c` cloak,
// `d` cloak shadow/trim, `f` face, `e` eye, `m` moon, `t` tower steel,
// `b` beacon. Rows must all be the same length (asserted in tests).

/// Cloaked cat mascot, 28x32 art pixels (84x96 on screen).
const CAT_ART: [&str; 32] = [
    "......dd..........dd........",
    "......ddd........ddd........",
    "......dcdd......ddcd........",
    "......dccdd....ddccd........",
    "......dcccdddddccccd........",
    ".....ddccccccccccccdd.......",
    "....ddccccccccccccccdd......",
    "....dccccccccccccccccd......",
    "...ddcccffffffffffcccdd.....",
    "...dcccffffffffffffcccd.....",
    "...dccffffffffffffffccd.....",
    "...dccffeeffffffeeffccd.....",
    "...dccffeeffffffeeffccd.....",
    "...dccffffffddffffffccd.....",
    "...dccffffffffffffffccd.....",
    "...dcccffffffffffffcccd.....",
    "....dcccccccccccccccd.......",
    "....ddcccccccccccccdd.......",
    "...ddcccccccccccccccdd......",
    "...dcccccccccccccccccd......",
    "..ddcccccccccccccccccdd.....",
    "..dcccccccccccccccccccd.....",
    "..dcccccccccccccccccccd.....",
    ".ddcccccccccccccccccccdd....",
    ".dcccccccccccccccccccccd....",
    ".dcccccccccccccccccccccd..d.",
    ".dcccccccccccccccccccccd.dcd",
    "dccccccccccccccccccccccd.dcd",
    "dccccccccccccccccccccccdddcd",
    "dccccccccccccccccccccccccccd",
    "dddddddddddddddddddddddddddd",
    "............................",
];

/// Crescent moon, 16x16 art pixels (48x48 on screen).
const MOON_ART: [&str; 16] = [
    ".....mmmmmm.....",
    "...mmmmmmmmmm...",
    "..mmmmmmmm......",
    ".mmmmmmm........",
    ".mmmmmm.........",
    "mmmmmm..........",
    "mmmmm...........",
    "mmmmm...........",
    "mmmmm...........",
    "mmmmm...........",
    "mmmmmm..........",
    ".mmmmmm.........",
    ".mmmmmmm........",
    "..mmmmmmmm......",
    "...mmmmmmmmmm...",
    ".....mmmmmm.....",
];

/// Radio tower with beacon, 13x30 art pixels (39x90 on screen).
const TOWER_ART: [&str; 30] = [
    "......b......",
    "......t......",
    ".....ttt.....",
    "......t......",
    ".....t.t.....",
    ".....t.t.....",
    ".....t.t.....",
    ".....ttt.....",
    "....t...t....",
    "....t...t....",
    "....t.t.t....",
    "....tt.tt....",
    "....ttttt....",
    "...t.....t...",
    "...t.....t...",
    "...t..t..t...",
    "...t.t.t.t...",
    "...tt...tt...",
    "...ttttttt...",
    "..t.......t..",
    "..t.......t..",
    "..t...t...t..",
    "..t..t.t..t..",
    "..t.t...t.t..",
    "..tt.....tt..",
    "..ttttttttt..",
    ".t.........t.",
    ".t.........t.",
    ".t.........t.",
    "tt.........tt",
];

#[cfg(test)]
mod tests {
    use super::*;

    const FONT_BYTES: &[u8] = include_bytes!("../../../assets/fonts/mplus12.kfont");

    fn font() -> BitmapFont<'static> {
        BitmapFont::from_bytes(FONT_BYTES).unwrap()
    }

    fn paint_full(splash: &BootSplash) -> Vec<u8> {
        let mut buf = vec![0u8; 320 * 320 * 2];
        splash.paint(&mut Canvas::new(&mut buf, 320, 320).unwrap(), &font());
        buf
    }

    fn pixel(buf: &[u8], x: i32, y: i32) -> u16 {
        let i = (y as usize * 320 + x as usize) * 2;
        u16::from_le_bytes([buf[i], buf[i + 1]])
    }

    fn rect_contains_color(buf: &[u8], rect: Rect, color: Rgb565) -> bool {
        for y in rect.y..rect.y + rect.h {
            for x in rect.x..rect.x + rect.w {
                if pixel(buf, x, y) == color.0 {
                    return true;
                }
            }
        }
        false
    }

    #[test]
    fn art_rows_are_rectangular_and_use_known_cells() {
        for (name, rows) in [
            ("cat", &CAT_ART[..]),
            ("moon", &MOON_ART[..]),
            ("tower", &TOWER_ART[..]),
        ] {
            let width = rows[0].len();
            for (i, row) in rows.iter().enumerate() {
                assert_eq!(row.len(), width, "{name} row {i} width");
                for cell in row.bytes() {
                    assert!(
                        matches!(cell, b'.' | b'c' | b'd' | b'f' | b'e' | b'm' | b't' | b'b'),
                        "{name} row {i} has unknown cell {:?}",
                        cell as char
                    );
                }
            }
        }
    }

    #[test]
    fn layout_stays_inside_surface_without_overlap() {
        let font = font();
        let surface = splash_surface_rect();
        let mut regions = vec![
            art_rect(CAT_X, CAT_Y, ART_SCALE, &CAT_ART),
            art_rect(MOON_X, MOON_Y, ART_SCALE, &MOON_ART),
            art_rect(TOWER_X, TOWER_Y, ART_SCALE, &TOWER_ART),
        ];
        // Art blocks must not overlap each other.
        for i in 0..regions.len() {
            for j in i + 1..regions.len() {
                assert!(
                    !rects_intersect(regions[i], regions[j]),
                    "art blocks {i} and {j} overlap"
                );
            }
        }
        regions.push(wordmark_rect(&font));
        regions.push(tagline_rect(&font));
        regions.push(copyright_rect(&font));
        for step in BootStep::ALL {
            regions.push(splash_step_rect(step));
        }
        regions.push(splash_progress_rect());
        for (i, rect) in regions.iter().enumerate() {
            assert!(rect.x >= 0 && rect.y >= 0, "region {i} origin");
            assert!(
                rect.x + rect.w <= surface.w && rect.y + rect.h <= surface.h,
                "region {i} exceeds surface: {rect:?}"
            );
        }
        // The vertical text stack must not collide.
        let wordmark = wordmark_rect(&font);
        let tagline = tagline_rect(&font);
        let copyright = copyright_rect(&font);
        assert!(wordmark.y + wordmark.h <= tagline.y);
        assert!(tagline.y + tagline.h <= copyright.y);
        assert!(copyright.y + copyright.h <= CHECKLIST_Y);
        assert!(
            CHECKLIST_Y + CHECKLIST_LINE_H * SPLASH_STEP_COUNT as i32 <= PROGRESS_Y - 2,
            "checklist runs into the progress bar"
        );
    }

    #[test]
    fn checklist_reflects_status_markers_and_notes() {
        let palette = &SplashPalette::DEFAULT;
        let mut splash = BootSplash::new();
        splash.resolve(BootStep::Kernel, BootStepStatus::Ok);
        splash.resolve(BootStep::Storage, BootStepStatus::Failed("no sd"));
        let buf = paint_full(&splash);

        assert!(
            rect_contains_color(&buf, splash_step_rect(BootStep::Kernel), palette.ok),
            "[ok] marker missing on resolved kernel line"
        );
        assert!(
            rect_contains_color(&buf, splash_step_rect(BootStep::Storage), palette.fail),
            "[ng] marker missing on failed storage line"
        );
        assert!(
            rect_contains_color(&buf, splash_step_rect(BootStep::Memory), palette.pending),
            "pending memory line missing dim marker"
        );
        assert!(
            !rect_contains_color(&buf, splash_step_rect(BootStep::Memory), palette.ok),
            "pending line must not show an ok marker"
        );
    }

    #[test]
    fn progress_bar_fill_tracks_resolved_steps() {
        let palette = &SplashPalette::DEFAULT;
        let empty = paint_full(&BootSplash::new());
        let full = paint_full(&BootSplash::complete());
        let inner_y = PROGRESS_Y + PROGRESS_H / 2;

        // Empty: no accent inside the bar; full: accent reaches the last inner column.
        for x in PROGRESS_X + 1..PROGRESS_X + PROGRESS_W - 1 {
            assert_eq!(
                pixel(&empty, x, inner_y),
                palette.progress_bg.0,
                "empty bar filled at x={x}"
            );
            assert_eq!(
                pixel(&full, x, inner_y),
                palette.accent.0,
                "full bar not filled at x={x}"
            );
        }

        // Half-resolved: fill is proportional.
        let mut half = BootSplash::new();
        half.resolve(BootStep::Kernel, BootStepStatus::Ok);
        half.resolve(BootStep::Memory, BootStepStatus::Ok);
        half.resolve(BootStep::Audio, BootStepStatus::Failed("x"));
        let half_buf = paint_full(&half);
        let fill = (PROGRESS_W - 2) * 3 / SPLASH_STEP_COUNT as i32;
        assert_eq!(
            pixel(&half_buf, PROGRESS_X + fill, inner_y),
            palette.accent.0
        );
        assert_eq!(
            pixel(&half_buf, PROGRESS_X + fill + 2, inner_y),
            palette.progress_bg.0
        );
    }

    #[test]
    fn scene_and_wordmark_render_expected_colors() {
        let palette = &SplashPalette::DEFAULT;
        let font = font();
        let buf = paint_full(&BootSplash::complete());

        assert_eq!(pixel(&buf, 0, 0), palette.background.0);
        assert_eq!(pixel(&buf, 319, 319), palette.background.0);
        assert!(rect_contains_color(
            &buf,
            art_rect(MOON_X, MOON_Y, ART_SCALE, &MOON_ART),
            palette.moon
        ));
        assert!(rect_contains_color(
            &buf,
            art_rect(TOWER_X, TOWER_Y, ART_SCALE, &TOWER_ART),
            palette.tower
        ));
        let cat = art_rect(CAT_X, CAT_Y, ART_SCALE, &CAT_ART);
        assert!(rect_contains_color(&buf, cat, palette.cloak));
        assert!(rect_contains_color(&buf, cat, palette.face));
        assert!(rect_contains_color(&buf, cat, palette.eye));
        let wordmark = wordmark_rect(&font);
        assert!(rect_contains_color(&buf, wordmark, palette.wordmark));
        assert!(rect_contains_color(&buf, wordmark, palette.accent));
        assert!(rect_contains_color(
            &buf,
            tagline_rect(&font),
            palette.tagline
        ));
        assert!(rect_contains_color(
            &buf,
            copyright_rect(&font),
            palette.code
        ));
    }

    /// The device paints the splash through `Canvas::new_viewport` strips
    /// (`paint_splash_rect`), so the contract that matters is: for any
    /// viewport, `paint_rect` produces exactly the full paint's pixels for
    /// that region. Mirrors the device path byte-for-byte, including 16-line
    /// transfer bands.
    #[test]
    fn paint_rect_matches_full_paint_inside_viewport() {
        let font = font();
        let mut splash = BootSplash::new();
        splash.resolve(BootStep::Kernel, BootStepStatus::Ok);
        splash.resolve(BootStep::Memory, BootStepStatus::Failed("psram"));

        let mut full = vec![0u8; 320 * 320 * 2];
        splash.paint(&mut Canvas::new(&mut full, 320, 320).unwrap(), &font);

        // Viewports crossing every component family: an art-block edge, the
        // wordmark, one checklist line, the progress band, and the device's
        // 16-line strip cadence over the whole surface.
        let mut viewports = vec![
            Rect {
                x: 100,
                y: 30,
                w: 160,
                h: 90,
            },
            wordmark_rect(&font),
            splash_step_rect(BootStep::Memory),
            splash_progress_rect(),
        ];
        for band in 0..20 {
            viewports.push(Rect {
                x: 0,
                y: band * 16,
                w: 320,
                h: 16,
            });
        }
        for viewport in viewports {
            let mut strip = vec![0u8; viewport.w as usize * viewport.h as usize * 2];
            let mut canvas = Canvas::new_viewport(&mut strip, 320, 320, viewport).unwrap();
            splash.paint_rect(&mut canvas, &font, viewport);
            for y in 0..viewport.h {
                for x in 0..viewport.w {
                    let i = (y as usize * viewport.w as usize + x as usize) * 2;
                    let got = u16::from_le_bytes([strip[i], strip[i + 1]]);
                    assert_eq!(
                        got,
                        pixel(&full, viewport.x + x, viewport.y + y),
                        "viewport {viewport:?} pixel ({x},{y}) must match full paint"
                    );
                }
            }
        }
    }
}
