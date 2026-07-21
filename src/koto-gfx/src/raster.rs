//! Minimal software rasterizer for RGB565 surfaces.
//!
//! [`Canvas`] borrows a caller-owned pixel buffer (`2` bytes per pixel, native
//! `u16` little-endian) and offers clipped primitives: [`fill_rect`] and glyph
//! blitting via [`draw_glyph`] / [`draw_text`]. All primitives clip to the
//! canvas bounds, so the same code can target a full host framebuffer (KotoSim)
//! or a small on-device tile without changes.
//!
//! Lifted verbatim from `koto_core::raster` (KotoGFX migration Stage 4,
//! GFX-0004 — the R14 rasterizer + font + colour group); re-exported from
//! `koto_core` so `shell_render` and every other `Canvas`/`Rgb565` consumer is
//! unchanged. It now sits in `koto-gfx`, below the compositor that uses it,
//! resolving the dependency inversion that previously kept the layer compositor
//! in the firmware.
//!
//! [`fill_rect`]: Canvas::fill_rect
//! [`draw_glyph`]: Canvas::draw_glyph
//! [`draw_text`]: Canvas::draw_text

use crate::font::{BitmapFont, Glyph};
use crate::Rect;

/// A 16-bit RGB565 color.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rgb565(pub u16);

impl Rgb565 {
    pub const fn from_rgb8(r: u8, g: u8, b: u8) -> Self {
        let value = ((r as u16 & 0xF8) << 8) | ((g as u16 & 0xFC) << 3) | (b as u16 >> 3);
        Rgb565(value)
    }
}

/// A mutable RGB565 drawing surface over a borrowed pixel buffer.
pub struct Canvas<'a> {
    pixels: &'a mut [u8],
    width: u16,
    height: u16,
    viewport: Rect,
}

impl<'a> Canvas<'a> {
    /// Wrap a buffer that must hold at least `width * height * 2` bytes.
    pub fn new(pixels: &'a mut [u8], width: u16, height: u16) -> Option<Self> {
        let needed = width as usize * height as usize * 2;
        if pixels.len() < needed {
            return None;
        }
        Some(Self {
            pixels,
            width,
            height,
            viewport: Rect {
                x: 0,
                y: 0,
                w: i32::from(width),
                h: i32::from(height),
            },
        })
    }

    /// Wrap a bounded pixel window positioned within a larger logical surface.
    ///
    /// Drawing coordinates remain relative to the logical surface. Pixels
    /// outside `viewport` are clipped, while pixels inside it are translated
    /// into the caller's compact `viewport.w * viewport.h * 2` buffer.
    pub fn new_viewport(
        pixels: &'a mut [u8],
        width: u16,
        height: u16,
        viewport: Rect,
    ) -> Option<Self> {
        if viewport.x < 0
            || viewport.y < 0
            || viewport.w <= 0
            || viewport.h <= 0
            || viewport.x + viewport.w > i32::from(width)
            || viewport.y + viewport.h > i32::from(height)
        {
            return None;
        }
        let needed = viewport.w as usize * viewport.h as usize * 2;
        if pixels.len() < needed {
            return None;
        }
        Some(Self {
            pixels,
            width,
            height,
            viewport,
        })
    }

    pub fn width(&self) -> u16 {
        self.width
    }

    pub fn height(&self) -> u16 {
        self.height
    }

    pub fn pixels(&self) -> &[u8] {
        self.pixels
    }

    /// Set a single pixel, ignoring out-of-bounds coordinates.
    pub fn put_pixel(&mut self, x: i32, y: i32, color: Rgb565) {
        if x < self.viewport.x
            || y < self.viewport.y
            || x >= self.viewport.x + self.viewport.w
            || y >= self.viewport.y + self.viewport.h
        {
            return;
        }
        let local_x = (x - self.viewport.x) as usize;
        let local_y = (y - self.viewport.y) as usize;
        let index = (local_y * self.viewport.w as usize + local_x) * 2;
        let [lo, hi] = color.0.to_le_bytes();
        self.pixels[index] = lo;
        self.pixels[index + 1] = hi;
    }

    /// Fill the canvas with a solid color.
    pub fn clear(&mut self, color: Rgb565) {
        let full = Rect {
            x: 0,
            y: 0,
            w: self.width as i32,
            h: self.height as i32,
        };
        self.fill_rect(full, color);
    }

    /// Fill a rectangle, clipped to the canvas bounds.
    ///
    /// The clip is computed once (KOTO-0174 H-P): each row's clipped span is
    /// contiguous in the compact viewport buffer, so the inner loop writes the
    /// 2-byte colour straight into the row slice instead of routing every pixel
    /// through [`Self::put_pixel`]'s per-pixel viewport re-check. Byte-identical
    /// output; the base clear and every rect fill get the win.
    pub fn fill_rect(&mut self, rect: Rect, color: Rgb565) {
        let x0 = rect.x.max(self.viewport.x);
        let y0 = rect.y.max(self.viewport.y);
        let x1 = (rect.x + rect.w).min(self.viewport.x + self.viewport.w);
        let y1 = (rect.y + rect.h).min(self.viewport.y + self.viewport.h);
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        let [lo, hi] = color.0.to_le_bytes();
        let vw = self.viewport.w as usize;
        let local_x0 = (x0 - self.viewport.x) as usize;
        let span = (x1 - x0) as usize;
        for y in y0..y1 {
            let local_y = (y - self.viewport.y) as usize;
            let start = (local_y * vw + local_x0) * 2;
            for px in self.pixels[start..start + span * 2].chunks_exact_mut(2) {
                px[0] = lo;
                px[1] = hi;
            }
        }
    }

    /// Blit a `w`x`h` block of little-endian RGB565 pixels (row-major, exactly
    /// `w * h * 2` bytes) with its top-left at (`x`, `y`). Out-of-bounds pixels and
    /// any tail shorter than `w * h * 2` bytes are skipped. The blit is opaque;
    /// callers wanting sprite transparency simply omit transparent cells.
    pub fn blit_rgb565(&mut self, x: i32, y: i32, w: i32, h: i32, pixels: &[u8]) {
        if w <= 0 || h <= 0 {
            return;
        }
        // Clip the destination to the viewport once (KOTO-0174 H-P). Each visible
        // destination row maps to a contiguous source span and a contiguous
        // destination span, so the row is one `copy_from_slice` instead of a
        // per-pixel `put_pixel`. A short source tail still stops the blit safely
        // (the row `get` returns `None`), matching the old per-pixel guard for
        // the full-tile inputs every caller passes.
        let x0 = x.max(self.viewport.x);
        let y0 = y.max(self.viewport.y);
        let x1 = (x + w).min(self.viewport.x + self.viewport.w);
        let y1 = (y + h).min(self.viewport.y + self.viewport.h);
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        let src_w = w as usize;
        let copy_bytes = (x1 - x0) as usize * 2;
        let vw = self.viewport.w as usize;
        let local_x0 = (x0 - self.viewport.x) as usize;
        let src_col0 = (x0 - x) as usize;
        for dy in y0..y1 {
            let src_row = (dy - y) as usize;
            let src_start = (src_row * src_w + src_col0) * 2;
            let Some(src) = pixels.get(src_start..src_start + copy_bytes) else {
                return;
            };
            let local_y = (dy - self.viewport.y) as usize;
            let dst_start = (local_y * vw + local_x0) * 2;
            self.pixels[dst_start..dst_start + copy_bytes].copy_from_slice(src);
        }
    }

    /// Blit a glyph with its set pixels painted in `fg`; unset pixels are left
    /// untouched (paint a background rect first if needed).
    ///
    /// The glyph box is clipped to the viewport once (KOTO-0174 H-P), then only
    /// the visible cells are scanned and each set bit is written by direct index
    /// — no per-pixel viewport re-check or `put_pixel` call. Glyph rasterization
    /// is the single largest present-path cost (Stage 0c: text ≈ 2.6× the rect
    /// fills), so this is the hottest inner loop. Byte-identical output.
    pub fn draw_glyph(&mut self, x: i32, y: i32, glyph: &Glyph<'_>, fg: Rgb565) {
        let gw = i32::from(glyph.width());
        let gh = i32::from(glyph.height());
        let gx0 = (self.viewport.x - x).max(0);
        let gy0 = (self.viewport.y - y).max(0);
        let gx1 = (self.viewport.x + self.viewport.w - x).min(gw);
        let gy1 = (self.viewport.y + self.viewport.h - y).min(gh);
        if gx1 <= gx0 || gy1 <= gy0 {
            return;
        }
        let [lo, hi] = fg.0.to_le_bytes();
        let vw = self.viewport.w as usize;
        for gy in gy0..gy1 {
            let row_base = (y + gy - self.viewport.y) as usize * vw;
            for gx in gx0..gx1 {
                if glyph.pixel(gx as u8, gy as u8) {
                    let idx = (row_base + (x + gx - self.viewport.x) as usize) * 2;
                    self.pixels[idx] = lo;
                    self.pixels[idx + 1] = hi;
                }
            }
        }
    }

    /// Draw a UTF-8 string starting at (`x`, `y`) (glyph top-left). Newlines
    /// return to the original x and advance by one font cell height. Missing
    /// glyphs advance by the font's half width. Returns the x cursor after the
    /// last glyph on the final line.
    pub fn draw_text(
        &mut self,
        x: i32,
        y: i32,
        font: &BitmapFont<'_>,
        text: &str,
        fg: Rgb565,
    ) -> i32 {
        let mut cursor = x;
        let mut baseline_y = y;
        for ch in text.chars() {
            if ch == '\n' {
                cursor = x;
                baseline_y += i32::from(font.cell_height());
                continue;
            }
            match font
                .glyph(ch)
                .or_else(|| font.glyph('\u{fffd}'))
                .or_else(|| font.glyph('?'))
            {
                Some(glyph) => {
                    self.draw_glyph(cursor, baseline_y, &glyph, fg);
                    cursor += glyph.width() as i32;
                }
                None => cursor += font.half_width() as i32,
            }
        }
        cursor
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canvas_buf(w: u16, h: u16) -> std::vec::Vec<u8> {
        std::vec![0u8; w as usize * h as usize * 2]
    }

    fn pixel_at(buf: &[u8], w: u16, x: u16, y: u16) -> u16 {
        let i = (y as usize * w as usize + x as usize) * 2;
        u16::from_le_bytes([buf[i], buf[i + 1]])
    }

    // KOTO-0174 H-P byte-identity: the hoisted-clip inner loops must produce the
    // exact same buffer as the original per-pixel `put_pixel` reference, for
    // every clipping case (negative origin, right/bottom straddle, offset
    // viewport). `put_pixel` is unchanged, so replaying the old loop through it
    // is the ground truth.

    fn ref_fill(canvas: &mut Canvas<'_>, rect: Rect, color: Rgb565) {
        for y in rect.y..rect.y + rect.h {
            for x in rect.x..rect.x + rect.w {
                canvas.put_pixel(x, y, color);
            }
        }
    }

    fn ref_blit(canvas: &mut Canvas<'_>, x: i32, y: i32, w: i32, h: i32, pixels: &[u8]) {
        for row in 0..h {
            for col in 0..w {
                let index = ((row * w + col) * 2) as usize;
                let Some(bytes) = pixels.get(index..index + 2) else {
                    return;
                };
                canvas.put_pixel(
                    x + col,
                    y + row,
                    Rgb565(u16::from_le_bytes([bytes[0], bytes[1]])),
                );
            }
        }
    }

    const VIEWPORTS: [Rect; 3] = [
        Rect {
            x: 0,
            y: 0,
            w: 12,
            h: 10,
        },
        Rect {
            x: 3,
            y: 2,
            w: 7,
            h: 6,
        },
        Rect {
            x: 5,
            y: 4,
            w: 4,
            h: 3,
        },
    ];

    #[test]
    fn hp_fill_rect_matches_put_pixel_reference() {
        let rects = [
            Rect {
                x: -2,
                y: 1,
                w: 6,
                h: 5,
            },
            Rect {
                x: 8,
                y: -1,
                w: 6,
                h: 8,
            },
            Rect {
                x: 1,
                y: 1,
                w: 3,
                h: 3,
            },
            Rect {
                x: 0,
                y: 0,
                w: 12,
                h: 10,
            },
        ];
        for vp in VIEWPORTS {
            for rect in rects {
                let mut a = canvas_buf(12, 10);
                let mut b = canvas_buf(12, 10);
                let mut fast = Canvas::new_viewport(&mut a, 12, 10, vp).unwrap();
                let mut refc = Canvas::new_viewport(&mut b, 12, 10, vp).unwrap();
                fast.fill_rect(rect, Rgb565(0xABCD));
                ref_fill(&mut refc, rect, Rgb565(0xABCD));
                assert_eq!(a, b, "fill_rect {rect:?} vp {vp:?}");
            }
        }
    }

    /// A one-glyph `.kfont` blob: codepoint `'X'`, `width`x`rows` cell, one
    /// `row_bytes` per row (MSB = column 0). `bitmap` is one byte per row.
    fn one_glyph_font(width: u8, rows: u8, bitmap: &[u8]) -> Vec<u8> {
        let mut d = Vec::new();
        d.extend_from_slice(b"KFNT");
        d.extend_from_slice(&1u16.to_le_bytes()); // version
        d.extend_from_slice(&0u16.to_le_bytes());
        d.push(rows); // cell_h
        d.push(rows); // ascent
        d.push(width); // half_w
        d.push(width); // full_w
        d.extend_from_slice(&1u32.to_le_bytes()); // glyph_count
        d.extend_from_slice(&(u32::from('X')).to_le_bytes());
        d.push(width);
        d.push(1); // row_bytes
        d.extend_from_slice(&0u32.to_le_bytes()); // bitmap off
        d.extend_from_slice(bitmap);
        d
    }

    fn ref_glyph(canvas: &mut Canvas<'_>, x: i32, y: i32, glyph: &Glyph<'_>, fg: Rgb565) {
        for gy in 0..glyph.height() {
            for gx in 0..glyph.width() {
                if glyph.pixel(gx, gy) {
                    canvas.put_pixel(x + i32::from(gx), y + i32::from(gy), fg);
                }
            }
        }
    }

    #[test]
    fn hp_draw_glyph_matches_put_pixel_reference() {
        // 4x5 glyph with set bits in the corners and centre, so every clip edge
        // touches painted pixels.
        let bitmap = [
            0b1001_0000,
            0b0110_0000,
            0b0110_0000,
            0b0110_0000,
            0b1001_0000,
        ];
        let blob = one_glyph_font(4, 5, &bitmap);
        let font = BitmapFont::from_bytes(&blob).unwrap();
        let glyph = font.glyph('X').unwrap();
        let placements = [(-2i32, 1i32), (10, -1), (2, 2), (0, 0), (11, 9)];
        for vp in VIEWPORTS {
            for (x, y) in placements {
                let mut a = canvas_buf(12, 10);
                let mut b = canvas_buf(12, 10);
                let mut fast = Canvas::new_viewport(&mut a, 12, 10, vp).unwrap();
                let mut refc = Canvas::new_viewport(&mut b, 12, 10, vp).unwrap();
                fast.draw_glyph(x, y, &glyph, Rgb565(0x7BEF));
                ref_glyph(&mut refc, x, y, &glyph, Rgb565(0x7BEF));
                assert_eq!(a, b, "glyph at ({x},{y}) vp {vp:?}");
            }
        }
    }

    #[test]
    fn hp_blit_matches_put_pixel_reference() {
        // A 5x4 source with distinct little-endian values per pixel.
        let (sw, sh) = (5i32, 4i32);
        let src: Vec<u8> = (0..(sw * sh))
            .flat_map(|i| (0x1000u16 + i as u16).to_le_bytes())
            .collect();
        let placements = [(-2i32, 1i32), (9, -1), (2, 2), (0, 0), (10, 8)];
        for vp in VIEWPORTS {
            for (x, y) in placements {
                let mut a = canvas_buf(12, 10);
                let mut b = canvas_buf(12, 10);
                let mut fast = Canvas::new_viewport(&mut a, 12, 10, vp).unwrap();
                let mut refc = Canvas::new_viewport(&mut b, 12, 10, vp).unwrap();
                fast.blit_rgb565(x, y, sw, sh, &src);
                ref_blit(&mut refc, x, y, sw, sh, &src);
                assert_eq!(a, b, "blit at ({x},{y}) vp {vp:?}");
            }
        }
    }

    #[test]
    fn rgb565_packs_channels() {
        assert_eq!(Rgb565::from_rgb8(0xFF, 0x00, 0x00), Rgb565(0xF800));
        assert_eq!(Rgb565::from_rgb8(0x00, 0xFF, 0x00), Rgb565(0x07E0));
        assert_eq!(Rgb565::from_rgb8(0x00, 0x00, 0xFF), Rgb565(0x001F));
    }

    #[test]
    fn fill_rect_clips_to_bounds() {
        let mut buf = canvas_buf(4, 4);
        let mut canvas = Canvas::new(&mut buf, 4, 4).unwrap();
        // Rectangle straddling the right/bottom edges.
        canvas.fill_rect(
            Rect {
                x: 2,
                y: 2,
                w: 10,
                h: 10,
            },
            Rgb565(0xFFFF),
        );
        assert_eq!(pixel_at(&buf, 4, 3, 3), 0xFFFF);
        assert_eq!(pixel_at(&buf, 4, 1, 1), 0x0000);
    }

    #[test]
    fn negative_origin_rect_clips() {
        let mut buf = canvas_buf(4, 4);
        let mut canvas = Canvas::new(&mut buf, 4, 4).unwrap();
        canvas.fill_rect(
            Rect {
                x: -2,
                y: -2,
                w: 4,
                h: 4,
            },
            Rgb565(0x1234),
        );
        assert_eq!(pixel_at(&buf, 4, 0, 0), 0x1234);
        assert_eq!(pixel_at(&buf, 4, 1, 1), 0x1234);
        assert_eq!(pixel_at(&buf, 4, 2, 2), 0x0000);
    }

    #[test]
    fn draw_text_wraps_on_newline() {
        let mut data = std::vec::Vec::new();
        data.extend_from_slice(b"KFNT");
        data.extend_from_slice(&1u16.to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes());
        data.push(2);
        data.push(2);
        data.push(1);
        data.push(1);
        data.extend_from_slice(&1u32.to_le_bytes());
        data.extend_from_slice(&(b'A' as u32).to_le_bytes());
        data.push(1);
        data.push(1);
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&[0b1000_0000, 0b1000_0000]);
        let font = BitmapFont::from_bytes(&data).unwrap();
        let mut buf = canvas_buf(4, 4);
        let mut canvas = Canvas::new(&mut buf, 4, 4).unwrap();

        canvas.draw_text(1, 0, &font, "A\nA", Rgb565(0xFFFF));

        assert_eq!(pixel_at(&buf, 4, 1, 0), 0xFFFF);
        assert_eq!(pixel_at(&buf, 4, 1, 2), 0xFFFF);
        assert_eq!(pixel_at(&buf, 4, 2, 0), 0x0000);
    }

    #[test]
    fn missing_text_glyph_prefers_replacement_then_question_mark() {
        let mut data = std::vec::Vec::new();
        data.extend_from_slice(b"KFNT");
        data.extend_from_slice(&1u16.to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes());
        data.extend_from_slice(&[1, 1, 1, 1]);
        data.extend_from_slice(&2u32.to_le_bytes());
        // Sorted index: '?' is blank, U+FFFD is set. A missing 'Z' must render
        // U+FFFD rather than silently advancing or selecting '?'.
        data.extend_from_slice(&(b'?' as u32).to_le_bytes());
        data.extend_from_slice(&[1, 1]);
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0xfffdu32.to_le_bytes());
        data.extend_from_slice(&[1, 1]);
        data.extend_from_slice(&1u32.to_le_bytes());
        data.extend_from_slice(&[0, 0b1000_0000]);
        let font = BitmapFont::from_bytes(&data).unwrap();
        let mut buf = canvas_buf(2, 1);
        Canvas::new(&mut buf, 2, 1)
            .unwrap()
            .draw_text(0, 0, &font, "Z", Rgb565(0xffff));
        assert_eq!(pixel_at(&buf, 2, 0, 0), 0xffff);

        // With no replacement glyph, the same path falls back to '?'.
        data[12..16].copy_from_slice(&1u32.to_le_bytes());
        data.truncate(16 + 10 + 1);
        data[16..20].copy_from_slice(&(b'?' as u32).to_le_bytes());
        data[20..22].copy_from_slice(&[1, 1]);
        data[22..26].copy_from_slice(&0u32.to_le_bytes());
        data[26] = 0b1000_0000;
        let font = BitmapFont::from_bytes(&data).unwrap();
        let mut buf = canvas_buf(2, 1);
        Canvas::new(&mut buf, 2, 1)
            .unwrap()
            .draw_text(0, 0, &font, "Z", Rgb565(0xffff));
        assert_eq!(pixel_at(&buf, 2, 0, 0), 0xffff);
    }

    #[test]
    fn viewport_uses_logical_coordinates_and_compact_storage() {
        let mut buf = canvas_buf(4, 2);
        let mut canvas = Canvas::new_viewport(
            &mut buf,
            8,
            8,
            Rect {
                x: 2,
                y: 3,
                w: 4,
                h: 2,
            },
        )
        .unwrap();
        canvas.fill_rect(
            Rect {
                x: 3,
                y: 4,
                w: 2,
                h: 1,
            },
            Rgb565(0x55aa),
        );
        assert_eq!(pixel_at(&buf, 4, 1, 1), 0x55aa);
        assert_eq!(pixel_at(&buf, 4, 2, 1), 0x55aa);
        assert_eq!(pixel_at(&buf, 4, 0, 0), 0x0000);
    }
}
