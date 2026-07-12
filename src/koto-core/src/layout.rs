//! Text-mode screen layout: status band, content grid, and the fixed IME line.
//!
//! Text-oriented modes (the shell list, KotoDOS-style consoles, and any app
//! backed by KotoIME) share a common vertical layout: a status band along the
//! top, a scrolling content region in the middle, and a fixed input line pinned
//! to the bottom of the screen (FR-IME-3). [`TextLayout`] turns a
//! [`RenderSurface`] plus a fixed [`CellMetrics`] into the three pixel
//! rectangles and the content character grid, so both KotoShell and KotoIME can
//! agree on where text lives without duplicating arithmetic.
//!
//! The module is `no_std` and never allocates; all results are plain values.
//! Cells are fixed-width: half-width (Latin) glyphs advance one column,
//! full-width (CJK) glyphs two. Vertical bands are sized in whole rows so the
//! status, content, and IME regions tile the screen without overlap, which is
//! the invariant the tests enforce.

use crate::font::BitmapFont;
use crate::hal::Rect;
use crate::render::RenderSurface;

/// Maximum number of text rows the fixed IME line may occupy (FR-IME-3).
pub const MAX_IME_LINES: u16 = 2;

/// Fixed dimensions of a single half-width text cell, in pixels.
///
/// `cell_width` is the advance of a half-width (Latin) glyph; full-width (CJK)
/// glyphs occupy two columns. `cell_height` is the row pitch, matching the
/// font's cell height.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CellMetrics {
    pub cell_width: u16,
    pub cell_height: u16,
}

impl CellMetrics {
    /// An 8x12 cell (compact body font).
    pub const FONT_8X12: CellMetrics = CellMetrics {
        cell_width: 8,
        cell_height: 12,
    };

    /// An 8x16 cell (taller, more legible body font).
    pub const FONT_8X16: CellMetrics = CellMetrics {
        cell_width: 8,
        cell_height: 16,
    };

    /// Derive cell metrics from a loaded bitmap font, using its half-width
    /// advance and cell height.
    pub fn from_font(font: &BitmapFont<'_>) -> Self {
        Self {
            cell_width: u16::from(font.half_width()),
            cell_height: u16::from(font.cell_height()),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LayoutError {
    /// The surface has a zero dimension.
    EmptySurface,
    /// A cell dimension is zero.
    InvalidCell,
    /// The requested IME line count is zero or exceeds [`MAX_IME_LINES`].
    InvalidImeLines,
    /// The surface cannot hold the status band, the IME line, and at least one
    /// content row at the given cell size.
    SurfaceTooSmall,
}

/// The computed regions for a text-mode screen.
///
/// The three bands stack vertically with no gaps or overlap:
/// `status` at the top (one row tall), `content` in the middle, and `ime`
/// pinned to the bottom. `content` spans every pixel between the status band
/// and the IME line; `content_rows`/`content_cols` report how many whole text
/// cells fit inside it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextLayout {
    pub surface: RenderSurface,
    pub cell: CellMetrics,
    pub status: Rect,
    pub content: Rect,
    pub ime: Rect,
    pub content_cols: u16,
    pub content_rows: u16,
}

impl TextLayout {
    /// Compute the layout for `surface` using `cell` metrics, reserving
    /// `ime_lines` rows (1 or 2) for the fixed bottom input line and one row for
    /// the status band.
    pub fn new(
        surface: RenderSurface,
        cell: CellMetrics,
        ime_lines: u16,
    ) -> Result<Self, LayoutError> {
        if surface.width == 0 || surface.height == 0 {
            return Err(LayoutError::EmptySurface);
        }
        if cell.cell_width == 0 || cell.cell_height == 0 {
            return Err(LayoutError::InvalidCell);
        }
        if ime_lines == 0 || ime_lines > MAX_IME_LINES {
            return Err(LayoutError::InvalidImeLines);
        }

        let status_h = cell.cell_height;
        let ime_h = cell.cell_height * ime_lines;

        // Status band, IME line, and at least one content row must fit.
        let reserved = status_h
            .checked_add(ime_h)
            .ok_or(LayoutError::SurfaceTooSmall)?;
        let min_height = reserved
            .checked_add(cell.cell_height)
            .ok_or(LayoutError::SurfaceTooSmall)?;
        if surface.height < min_height || surface.width < cell.cell_width {
            return Err(LayoutError::SurfaceTooSmall);
        }

        let width = i32::from(surface.width);
        let content_h = surface.height - reserved;

        let status = Rect {
            x: 0,
            y: 0,
            w: width,
            h: i32::from(status_h),
        };
        let content = Rect {
            x: 0,
            y: i32::from(status_h),
            w: width,
            h: i32::from(content_h),
        };
        let ime = Rect {
            x: 0,
            y: i32::from(status_h + content_h),
            w: width,
            h: i32::from(ime_h),
        };

        Ok(Self {
            surface,
            cell,
            status,
            content,
            ime,
            content_cols: surface.width / cell.cell_width,
            content_rows: content_h / cell.cell_height,
        })
    }

    /// Pixel rectangle of one content cell at (`col`, `row`), or `None` if it
    /// lies outside the content grid. `cols` is the number of columns the cell
    /// spans (1 for half-width, 2 for full-width glyphs).
    pub fn content_cell_rect(&self, col: u16, row: u16, cols: u16) -> Option<Rect> {
        if cols == 0 || row >= self.content_rows {
            return None;
        }
        let end_col = col.checked_add(cols)?;
        if end_col > self.content_cols {
            return None;
        }
        Some(Rect {
            x: self.content.x + i32::from(col * self.cell.cell_width),
            y: self.content.y + i32::from(row * self.cell.cell_height),
            w: i32::from(cols * self.cell.cell_width),
            h: i32::from(self.cell.cell_height),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hal::PixelFormat;

    fn surface() -> RenderSurface {
        RenderSurface::new(320, 320, PixelFormat::Rgb565)
    }

    fn overlaps(a: &Rect, b: &Rect) -> bool {
        let ax1 = a.x + a.w;
        let ay1 = a.y + a.h;
        let bx1 = b.x + b.w;
        let by1 = b.y + b.h;
        a.x < bx1 && b.x < ax1 && a.y < by1 && b.y < ay1
    }

    #[test]
    fn computes_regions_for_320_with_8x12_cells() {
        let layout = TextLayout::new(surface(), CellMetrics::FONT_8X12, 2).unwrap();

        assert_eq!(
            layout.status,
            Rect {
                x: 0,
                y: 0,
                w: 320,
                h: 12
            }
        );
        // content fills the gap: 320 - 12 (status) - 24 (two IME rows) = 284.
        assert_eq!(
            layout.content,
            Rect {
                x: 0,
                y: 12,
                w: 320,
                h: 284
            }
        );
        assert_eq!(
            layout.ime,
            Rect {
                x: 0,
                y: 296,
                w: 320,
                h: 24
            }
        );
        assert_eq!(layout.content_cols, 40);
        assert_eq!(layout.content_rows, 23); // floor(284 / 12)
    }

    #[test]
    fn supports_8x16_cells() {
        let layout = TextLayout::new(surface(), CellMetrics::FONT_8X16, 1).unwrap();
        assert_eq!(layout.status.h, 16);
        assert_eq!(layout.ime.h, 16);
        assert_eq!(layout.content.y, 16);
        assert_eq!(layout.content.h, 320 - 16 - 16);
        assert_eq!(layout.content_cols, 40);
        assert_eq!(layout.content_rows, (320 - 32) / 16);
    }

    #[test]
    fn regions_tile_the_screen_without_overlap() {
        for cell in [CellMetrics::FONT_8X12, CellMetrics::FONT_8X16] {
            for ime_lines in 1..=MAX_IME_LINES {
                let layout = TextLayout::new(surface(), cell, ime_lines).unwrap();
                let regions = [layout.status, layout.content, layout.ime];

                // Pairwise non-overlap.
                assert!(!overlaps(&layout.status, &layout.content));
                assert!(!overlaps(&layout.content, &layout.ime));
                assert!(!overlaps(&layout.status, &layout.ime));

                // Stacked top-to-bottom, flush, and within the surface.
                assert_eq!(layout.status.y, 0);
                assert_eq!(layout.content.y, layout.status.y + layout.status.h);
                assert_eq!(layout.ime.y, layout.content.y + layout.content.h);
                assert_eq!(
                    layout.ime.y + layout.ime.h,
                    i32::from(layout.surface.height)
                );
                for region in regions {
                    assert_eq!(region.x, 0);
                    assert_eq!(region.w, i32::from(layout.surface.width));
                    assert!(region.h > 0);
                }
            }
        }
    }

    #[test]
    fn ime_line_count_changes_only_the_ime_band() {
        let one = TextLayout::new(surface(), CellMetrics::FONT_8X12, 1).unwrap();
        let two = TextLayout::new(surface(), CellMetrics::FONT_8X12, 2).unwrap();
        assert_eq!(two.ime.h, one.ime.h + 12);
        // The taller IME line steals exactly one row from the content region.
        assert_eq!(two.content.h, one.content.h - 12);
        assert_eq!(two.content_rows, one.content_rows - 1);
        assert_eq!(one.status, two.status);
    }

    #[test]
    fn content_cell_rect_maps_grid_cells_and_clips() {
        let layout = TextLayout::new(surface(), CellMetrics::FONT_8X12, 2).unwrap();

        let first = layout.content_cell_rect(0, 0, 1).unwrap();
        assert_eq!(
            first,
            Rect {
                x: 0,
                y: 12,
                w: 8,
                h: 12
            }
        );

        // Full-width glyph spans two columns.
        let full = layout.content_cell_rect(2, 1, 2).unwrap();
        assert_eq!(
            full,
            Rect {
                x: 16,
                y: 24,
                w: 16,
                h: 12
            }
        );

        // Off-grid lookups return None instead of overflowing.
        assert!(layout
            .content_cell_rect(layout.content_cols, 0, 1)
            .is_none());
        assert!(layout
            .content_cell_rect(0, layout.content_rows, 1)
            .is_none());
        assert!(layout
            .content_cell_rect(layout.content_cols - 1, 0, 2)
            .is_none());
        assert!(layout.content_cell_rect(0, 0, 0).is_none());
    }

    #[test]
    fn derives_cell_metrics_from_font() {
        let mut bytes = std::vec::Vec::new();
        bytes.extend_from_slice(b"KFNT");
        bytes.extend_from_slice(&1u16.to_le_bytes()); // version
        bytes.extend_from_slice(&0u16.to_le_bytes()); // flags
        bytes.push(13); // cell_h
        bytes.push(11); // ascent
        bytes.push(6); // half_w
        bytes.push(12); // full_w
        bytes.extend_from_slice(&0u32.to_le_bytes()); // glyph_count
        let font = BitmapFont::from_bytes(&bytes).unwrap();

        let cell = CellMetrics::from_font(&font);
        assert_eq!(cell.cell_width, 6);
        assert_eq!(cell.cell_height, 13);
    }

    #[test]
    fn rejects_invalid_inputs() {
        assert_eq!(
            TextLayout::new(
                RenderSurface::new(0, 320, PixelFormat::Rgb565),
                CellMetrics::FONT_8X12,
                1
            ),
            Err(LayoutError::EmptySurface)
        );
        assert_eq!(
            TextLayout::new(
                surface(),
                CellMetrics {
                    cell_width: 0,
                    cell_height: 12
                },
                1
            ),
            Err(LayoutError::InvalidCell)
        );
        assert_eq!(
            TextLayout::new(surface(), CellMetrics::FONT_8X12, 0),
            Err(LayoutError::InvalidImeLines)
        );
        assert_eq!(
            TextLayout::new(surface(), CellMetrics::FONT_8X12, MAX_IME_LINES + 1),
            Err(LayoutError::InvalidImeLines)
        );
        // A surface too short for status + IME + one content row.
        assert_eq!(
            TextLayout::new(
                RenderSurface::new(320, 30, PixelFormat::Rgb565),
                CellMetrics::FONT_8X12,
                2
            ),
            Err(LayoutError::SurfaceTooSmall)
        );
    }
}
