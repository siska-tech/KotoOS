//! Dirty-rect fragmentation diagnostics model (KOTO-0159).
//!
//! Pure data describing one frame's dirty-rectangle fragmentation, used by the
//! firmware's `phase=164` triage line to tell whether a slow event frame is
//! dominated by *many small scattered rects* (each one full-scene recomposite
//! pass) rather than transfer area.
//!
//! [`DirtyRectGeometry`] and [`DIRTY_SAMPLE_QUADS`] moved here verbatim from the
//! Pico firmware's `diag` module (which re-exports them unchanged). The
//! [`DirtyRectGeometry::from_rects`] summarizer was the private `dirty_geometry`
//! free function in `app_render`; its arithmetic is identical, so the emitted
//! geometry is unchanged. `rects_post` is filled in by the caller after
//! coalescing.

use crate::rect::Rect;

/// Pre-coalesce dirty rects sampled into [`DirtyRectGeometry::sample`].
pub const DIRTY_SAMPLE_QUADS: usize = 4;

/// One frame's dirty-rectangle fragmentation snapshot (KOTO-0159). Each surviving
/// dirty rect drives a full-scene recomposite pass, so a high `rects_pre` over a
/// tiny `area_pre` is the per-rect raster overhead this targets; `rects_post` is
/// the count after coalescing (the passes actually rastered/transferred).
#[derive(Clone, Copy, Default)]
pub struct DirtyRectGeometry {
    /// Dirty rects before coalescing (the raw fragmentation).
    pub rects_pre: u16,
    /// Dirty rects after coalescing == recomposite/transfer passes run.
    pub rects_post: u16,
    /// Summed area of the pre-coalesce rects (overlaps double-counted).
    pub area_pre: u32,
    /// Area of the single bounding box over every pre-coalesce dirty rect.
    pub bbox_area: u32,
    /// Largest single pre-coalesce dirty-rect area.
    pub max_area: u32,
    /// Smallest single pre-coalesce dirty-rect area.
    pub min_area: u32,
    /// First few pre-coalesce rects as flat `(x, y, w, h)` quads, for spotting the
    /// effect (well-local vs sidebar) that fragmented the frame.
    pub sample: [i32; DIRTY_SAMPLE_QUADS * 4],
    /// Valid quads in `sample`.
    pub sample_len: u8,
}

impl DirtyRectGeometry {
    /// Summarize the pre-coalesce dirty set for the `phase=164` triage line
    /// (KOTO-0159): rect count, the bounding box over all rects (so a high count
    /// over a box far larger than `area_pre` reads as "scattered"), per-rect
    /// min/max area, and a sample of the first few rects' geometry. `rects_post` is
    /// left at its default; the caller fills it in after coalescing.
    pub fn from_rects(rects: &[Rect], area_pre: u32) -> Self {
        let mut geom = DirtyRectGeometry {
            rects_pre: rects.len() as u16,
            area_pre,
            ..DirtyRectGeometry::default()
        };
        if rects.is_empty() {
            return geom;
        }
        let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        geom.min_area = u32::MAX;
        for (i, rect) in rects.iter().enumerate() {
            let a = rect.w as u32 * rect.h as u32;
            geom.max_area = geom.max_area.max(a);
            geom.min_area = geom.min_area.min(a);
            x0 = x0.min(rect.x);
            y0 = y0.min(rect.y);
            x1 = x1.max(rect.x.saturating_add(rect.w));
            y1 = y1.max(rect.y.saturating_add(rect.h));
            if i < DIRTY_SAMPLE_QUADS {
                let base = i * 4;
                geom.sample[base] = rect.x;
                geom.sample[base + 1] = rect.y;
                geom.sample[base + 2] = rect.w;
                geom.sample[base + 3] = rect.h;
                geom.sample_len = (i + 1) as u8;
            }
        }
        geom.bbox_area = ((x1 - x0) as u32).saturating_mul((y1 - y0) as u32);
        geom
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn empty_set_is_all_zero() {
        let geom = DirtyRectGeometry::from_rects(&[], 0);
        assert_eq!(geom.rects_pre, 0);
        assert_eq!(geom.area_pre, 0);
        assert_eq!(geom.bbox_area, 0);
        assert_eq!(geom.sample_len, 0);
        // `min_area` stays at its default (0) for an empty set, not `u32::MAX`.
        assert_eq!(geom.min_area, 0);
    }

    #[test]
    fn single_rect_summary() {
        let geom = DirtyRectGeometry::from_rects(&[rect(10, 20, 16, 16)], 256);
        assert_eq!(geom.rects_pre, 1);
        assert_eq!(geom.area_pre, 256);
        assert_eq!(geom.bbox_area, 256);
        assert_eq!(geom.min_area, 256);
        assert_eq!(geom.max_area, 256);
        assert_eq!(geom.sample_len, 1);
        assert_eq!(&geom.sample[..4], &[10, 20, 16, 16]);
    }

    #[test]
    fn scattered_rects_report_bbox_and_extremes() {
        // Two small rects far apart: tiny summed area, but a large bounding box —
        // the "scattered" signature the triage line looks for.
        let rects = [rect(0, 0, 4, 4), rect(300, 300, 8, 8)];
        let geom = DirtyRectGeometry::from_rects(&rects, 16 + 64);
        assert_eq!(geom.rects_pre, 2);
        assert_eq!(geom.area_pre, 80);
        assert_eq!(geom.min_area, 16);
        assert_eq!(geom.max_area, 64);
        // bbox spans (0,0)..(308,308) = 308 * 308.
        assert_eq!(geom.bbox_area, 308 * 308);
        assert_eq!(geom.sample_len, 2);
    }

    #[test]
    fn sample_is_capped_at_quad_limit() {
        // More rects than sample slots: the count is exact but only the first
        // `DIRTY_SAMPLE_QUADS` are sampled.
        let many: Vec<Rect> = (0..(DIRTY_SAMPLE_QUADS as i32 + 3))
            .map(|i| rect(i, i, 2, 2))
            .collect();
        let geom = DirtyRectGeometry::from_rects(&many, 4 * many.len() as u32);
        assert_eq!(geom.rects_pre as usize, many.len());
        assert_eq!(geom.sample_len as usize, DIRTY_SAMPLE_QUADS);
    }
}
