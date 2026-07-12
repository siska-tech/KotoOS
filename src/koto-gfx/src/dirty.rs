//! Coalesce dirty cells of a tile grid into a small set of band rectangles
//! (KOTO-0143), and merge scattered dirty rectangles into fewer recomposite
//! passes (KOTO-0159).
//!
//! This logic moved verbatim from `koto_core::dirty_tiles` as part of the
//! KotoGFX v0 extraction; `koto_core` re-exports it unchanged, so the present
//! path's behaviour is identical. The only internal change is that the private
//! `rect_area`/`rect_bbox` free functions are now [`Rect::area`]/[`Rect::bbox`].
//!
//! The tile algorithm is deliberately simple and deterministic: scan row-major,
//! collect each row's maximal horizontal runs of dirty cells, and merge a run
//! into the band directly above it when they share the same start column and
//! width (a vertically-adjacent identical run). Each dirty cell belongs to
//! exactly one run and thus exactly one band, so the emitted bands cover every
//! dirty cell with no overlap — recompositing their bounding rects reproduces
//! the same pixels as per-cell repaints.

use crate::rect::Rect;

/// A coalesced rectangle of dirty tile cells, in *cell* units (not pixels). The
/// caller scales by its tile size and applies its grid origin / clipping.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TileBand {
    pub col: u16,
    pub row: u16,
    pub w: u16,
    pub h: u16,
}

/// Coalesce the dirty cells of a `cols x rows` tile grid into band rectangles
/// written to `out`, returning the number of bands written.
///
/// `is_dirty(col, row)` reports whether cell `(col, row)` changed. Bands are
/// emitted in row-major order; a horizontal run is merged into a band directly
/// above it (same `col` and `w`, ending on the previous row), so contiguous
/// identical runs stack into one rect.
///
/// Returns `None` if `out` is too small to hold every band; the caller should
/// treat that as "too fragmented to stay incremental" and fall back to a full
/// repaint (which repaints everything, so no dirty cell is missed).
pub fn coalesce_dirty_tiles(
    cols: usize,
    rows: usize,
    is_dirty: impl Fn(usize, usize) -> bool,
    out: &mut [TileBand],
) -> Option<usize> {
    let mut count = 0usize;
    for row in 0..rows {
        let mut col = 0usize;
        while col < cols {
            if !is_dirty(col, row) {
                col += 1;
                continue;
            }
            // Extend this maximal horizontal run of dirty cells.
            let start = col;
            while col < cols && is_dirty(col, row) {
                col += 1;
            }
            let w = col - start;
            // Merge into the band directly above with the identical span: same
            // start column and width, and bottom edge exactly on this row (so the
            // two runs are vertically contiguous, no gap). Only one such band can
            // exist — per-row runs are disjoint, so no two open bands share a span.
            let mut extended = false;
            for band in &mut out[..count] {
                if band.col as usize == start
                    && band.w as usize == w
                    && band.row as usize + band.h as usize == row
                {
                    band.h += 1;
                    extended = true;
                    break;
                }
            }
            if !extended {
                if count >= out.len() {
                    return None;
                }
                out[count] = TileBand {
                    col: start as u16,
                    row: row as u16,
                    w: w as u16,
                    h: 1,
                };
                count += 1;
            }
        }
    }
    Some(count)
}

/// Merge a set of dirty rectangles in place into fewer rectangles, to cut the
/// number of recomposite/transfer passes a fragmented frame costs.
///
/// `rects[..len]` is the working set; the return value is the new count, with the
/// surviving rectangles packed into the front of the slice. Two rectangles are
/// merged into their bounding box when that box wastes no more than `max_waste`
/// pixels beyond the area the pair already covers (`bbox_area − area_a − area_b`):
/// nested or edge-adjacent rects qualify even at `max_waste = 0` (the box equals
/// the area they already cover, so waste saturates to 0), lightly-overlapping or
/// near rects qualify for modest budgets, and far-apart rects never do, so distinct
/// changes stay distinct passes.
///
/// Merging only ever *grows* a rectangle, and the merged set's union always covers
/// every input rect, so recompositing the scene clipped to the merged rectangles
/// reproduces the same pixels as the originals — no dirty region is dropped. This
/// is purely a pass-count optimisation: each surviving rect drives one full-scene
/// raster pass, so collapsing scattered fragments is worthwhile whenever raster
/// (CPU compose) cost dominates transfer (area) cost.
pub fn coalesce_rects(rects: &mut [Rect], len: usize, max_waste: u32) -> usize {
    let mut len = len.min(rects.len());
    // Grow each rect by absorbing every later rect that merges into it, packing the
    // survivor into the front. After a merge, `rects[i]`'s box has grown, so re-scan
    // its tail (it may now absorb a neighbour it could not before). Each merge
    // strictly reduces `len`, so this terminates; `len` is bounded by the present
    // path's dirty-rect cap, so the O(len^2) scan is cheap.
    let mut i = 0;
    while i < len {
        let mut merged_any = false;
        let mut j = i + 1;
        while j < len {
            let bbox = rects[i].bbox(rects[j]);
            let waste = bbox
                .area()
                .saturating_sub(rects[i].area())
                .saturating_sub(rects[j].area());
            if waste <= max_waste {
                rects[i] = bbox;
                // Swap the last rect into `j` and shrink; re-test the same `j` slot.
                rects[j] = rects[len - 1];
                len -= 1;
                merged_any = true;
            } else {
                j += 1;
            }
        }
        // Only advance once `rects[i]` can absorb nothing more, so its final box
        // gets a chance to sweep up rects that became reachable as it grew.
        if !merged_any {
            i += 1;
        }
    }
    len
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build an `is_dirty` predicate from an explicit set of dirty `(col, row)`
    /// cells, then coalesce a `cols x rows` grid into a fixed buffer.
    fn coalesce(cols: usize, rows: usize, dirty: &[(usize, usize)]) -> Vec<TileBand> {
        let mut out = [TileBand::default(); 256];
        let n = coalesce_dirty_tiles(cols, rows, |col, row| dirty.contains(&(col, row)), &mut out)
            .expect("buffer large enough");
        out[..n].to_vec()
    }

    /// Total cells covered by the bands, asserting no two bands overlap.
    fn covered_cells(bands: &[TileBand]) -> std::collections::BTreeSet<(usize, usize)> {
        let mut cells = std::collections::BTreeSet::new();
        for band in bands {
            for dr in 0..band.h as usize {
                for dc in 0..band.w as usize {
                    let cell = (band.col as usize + dc, band.row as usize + dr);
                    assert!(cells.insert(cell), "bands overlap at {cell:?}");
                }
            }
        }
        cells
    }

    #[test]
    fn empty_board_emits_no_bands() {
        assert!(coalesce(10, 20, &[]).is_empty());
    }

    #[test]
    fn one_dirty_row_of_ten_coalesces_to_one_band() {
        // A cleared KotoBlocks row: all 10 cells of row 5 dirty -> one 10-wide
        // band (160x16 px once scaled by the 16 px tile), not 10 cell rects.
        let dirty: Vec<_> = (0..10).map(|c| (c, 5)).collect();
        let bands = coalesce(10, 20, &dirty);
        assert_eq!(
            bands,
            vec![TileBand {
                col: 0,
                row: 5,
                w: 10,
                h: 1
            }]
        );
    }

    #[test]
    fn four_contiguous_full_rows_merge_vertically() {
        // A 4-line clear / contiguous collapse: rows 5..9 fully dirty. The
        // identical full-width runs stack into a single band, far under the
        // rect cap, instead of 40 cell rects.
        let mut dirty = Vec::new();
        for r in 5..9 {
            for c in 0..10 {
                dirty.push((c, r));
            }
        }
        let bands = coalesce(10, 20, &dirty);
        assert_eq!(
            bands,
            vec![TileBand {
                col: 0,
                row: 5,
                w: 10,
                h: 4
            }]
        );
    }

    #[test]
    fn four_separated_full_rows_stay_four_bands() {
        // Rows 2, 5, 8, 11 fully dirty but not adjacent: no vertical merge, so
        // four band rects (still a handful, not 40 cell rects).
        let mut dirty = Vec::new();
        for r in [2usize, 5, 8, 11] {
            for c in 0..10 {
                dirty.push((c, r));
            }
        }
        let bands = coalesce(10, 20, &dirty);
        assert_eq!(bands.len(), 4);
        assert!(bands.iter().all(|b| b.w == 10 && b.h == 1));
        assert_eq!(covered_cells(&bands).len(), 40);
    }

    #[test]
    fn vertical_gap_does_not_merge() {
        // Same column span on rows 0 and 2 with a clean row 1 between: two
        // separate bands, never bridged across the gap.
        let bands = coalesce(10, 20, &[(0, 0), (0, 2)]);
        assert_eq!(
            bands,
            vec![
                TileBand {
                    col: 0,
                    row: 0,
                    w: 1,
                    h: 1
                },
                TileBand {
                    col: 0,
                    row: 2,
                    w: 1,
                    h: 1
                },
            ]
        );
    }

    #[test]
    fn horizontal_gap_splits_runs() {
        // A clean cell between two dirty cells on the same row splits into two
        // single-cell bands, not one 3-wide band over the clean middle.
        let bands = coalesce(10, 20, &[(0, 0), (2, 0)]);
        assert_eq!(bands.len(), 2);
        assert!(bands.iter().all(|b| b.w == 1 && b.h == 1));
    }

    #[test]
    fn mismatched_width_runs_do_not_merge() {
        // Row 0 dirty cols 0..4, row 1 dirty cols 0..10: differing widths, so the
        // wide run starts a new band rather than extending the narrow one. All
        // dirty cells are still covered exactly once.
        let mut dirty: Vec<_> = (0..4).map(|c| (c, 0)).collect();
        dirty.extend((0..10).map(|c| (c, 1)));
        let bands = coalesce(10, 20, &dirty);
        assert_eq!(bands.len(), 2);
        assert_eq!(covered_cells(&bands).len(), 4 + 10);
        assert!(bands.contains(&TileBand {
            col: 0,
            row: 0,
            w: 4,
            h: 1
        }));
        assert!(bands.contains(&TileBand {
            col: 0,
            row: 1,
            w: 10,
            h: 1
        }));
    }

    #[test]
    fn checkerboard_does_not_overlap_and_covers_all() {
        // Pathological fragmentation: no two adjacent cells share a run or a
        // span, so every dirty cell is its own band. The point is correctness —
        // exact coverage, no overlap — the present path escalates on the count.
        let mut dirty = Vec::new();
        for r in 0..20 {
            for c in 0..10 {
                if (r + c) % 2 == 0 {
                    dirty.push((c, r));
                }
            }
        }
        let bands = coalesce(10, 20, &dirty);
        assert_eq!(covered_cells(&bands).len(), dirty.len());
        assert!(bands.iter().all(|b| b.w == 1 && b.h == 1));
    }

    #[test]
    fn overflow_returns_none() {
        // A buffer smaller than the band count signals "too fragmented" so the
        // caller can fall back to a full repaint instead of dropping bands.
        let mut out = [TileBand::default(); 2];
        let result = coalesce_dirty_tiles(10, 20, |col, row| (row + col) % 2 == 0, &mut out);
        assert_eq!(result, None);
    }

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }

    /// Every pixel covered by `rects[..len]`, so a coalesce can be asserted to
    /// preserve exact coverage (the merged set must cover at least these pixels).
    fn covered_pixels(rects: &[Rect]) -> std::collections::BTreeSet<(i32, i32)> {
        let mut px = std::collections::BTreeSet::new();
        for r in rects {
            for y in r.y..r.y + r.h {
                for x in r.x..r.x + r.w {
                    px.insert((x, y));
                }
            }
        }
        px
    }

    #[test]
    fn coalesce_empty_and_single_are_unchanged() {
        let mut none: [Rect; 0] = [];
        assert_eq!(coalesce_rects(&mut none, 0, 1024), 0);

        let mut one = [rect(3, 4, 5, 6)];
        assert_eq!(coalesce_rects(&mut one, 1, 1024), 1);
        assert_eq!(one[0], rect(3, 4, 5, 6));
    }

    #[test]
    fn coalesce_merges_nested_rect_at_zero_waste() {
        // A rect fully contained in another: the bounding box is just the outer
        // rect, so waste is 0 and they merge even at the tightest budget.
        let mut rects = [rect(0, 0, 40, 40), rect(10, 10, 8, 8)];
        let n = coalesce_rects(&mut rects, 2, 0);
        assert_eq!(n, 1);
        assert_eq!(rects[0], rect(0, 0, 40, 40));
    }

    #[test]
    fn coalesce_merges_lightly_overlapping_rects_under_budget() {
        // Partial overlap: bbox 30x30=900, areas 400+400, waste 100. Merged only
        // when the budget covers that small overhang.
        let mut rects = [rect(0, 0, 20, 20), rect(10, 10, 20, 20)];
        assert_eq!(coalesce_rects(&mut [rects[0], rects[1]], 2, 0), 2);
        let n = coalesce_rects(&mut rects, 2, 1024);
        assert_eq!(n, 1);
        assert_eq!(rects[0], rect(0, 0, 30, 30));
    }

    #[test]
    fn coalesce_merges_edge_adjacent_rects_with_zero_waste() {
        // Edge-adjacent 16x16 cells: bounding box is exactly the sum, waste 0, so
        // they merge even at max_waste = 0 — the cleared-row / neighbour case.
        let mut rects = [rect(0, 0, 16, 16), rect(16, 0, 16, 16)];
        let n = coalesce_rects(&mut rects, 2, 0);
        assert_eq!(n, 1);
        assert_eq!(rects[0], rect(0, 0, 32, 16));
    }

    #[test]
    fn coalesce_keeps_far_apart_rects_separate() {
        // A change in the well and one in a far sidebar: the bounding box wastes far
        // more than the budget, so they stay two passes.
        let mut rects = [rect(0, 0, 16, 16), rect(300, 300, 16, 16)];
        let original = covered_pixels(&rects[..2]);
        let n = coalesce_rects(&mut rects, 2, 1024);
        assert_eq!(n, 2);
        assert_eq!(covered_pixels(&rects[..n]), original);
    }

    #[test]
    fn coalesce_collapses_clustered_fragments_under_budget() {
        // Four scattered single-cell rects clustered within a small region collapse
        // toward their bounding box when the per-merge waste budget allows it — the
        // line-clear / game-over sparkle pattern this targets.
        let mut rects = [
            rect(0, 0, 16, 16),
            rect(16, 16, 16, 16),
            rect(0, 16, 16, 16),
            rect(16, 0, 16, 16),
        ];
        let original = covered_pixels(&rects[..4]);
        let n = coalesce_rects(&mut rects, 4, 16 * 16 * 4);
        assert_eq!(n, 1);
        assert_eq!(rects[0], rect(0, 0, 32, 32));
        // Coverage is preserved (a superset is acceptable; here it is exact).
        assert!(covered_pixels(&rects[..n]).is_superset(&original));
    }

    #[test]
    fn coalesce_never_drops_a_dirty_region() {
        // A pathological scatter at a tiny budget: whatever the final count, the
        // merged set must still cover every originally dirty pixel.
        let mut rects = [
            rect(0, 0, 4, 4),
            rect(40, 0, 4, 4),
            rect(0, 40, 4, 4),
            rect(40, 40, 4, 4),
            rect(20, 20, 4, 4),
        ];
        let original = covered_pixels(&rects[..5]);
        let n = coalesce_rects(&mut rects, 5, 8);
        assert!(covered_pixels(&rects[..n]).is_superset(&original));
    }
}
