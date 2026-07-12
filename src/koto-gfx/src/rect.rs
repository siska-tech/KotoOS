//! The axis-aligned rectangle type shared across KotoOS rendering, plus the
//! pure rectangle helpers the coalescing logic needs.
//!
//! `Rect` lived in `koto_core::hal` and is re-exported there unchanged; this is
//! its canonical home now (KotoGFX v0). The `area`/`bbox` methods were private
//! free functions in `koto_core::dirty_tiles` (`rect_area`/`rect_bbox`); they
//! keep their exact arithmetic so coalescing behaviour is unchanged.

/// An axis-aligned rectangle in surface pixel coordinates. `w`/`h` may be
/// negative for degenerate rects; the helpers below treat those as empty.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    /// Pixel area of the rectangle (saturating; a degenerate rect is zero).
    pub fn area(self) -> u32 {
        let w = self.w.max(0) as u32;
        let h = self.h.max(0) as u32;
        w.saturating_mul(h)
    }

    /// Smallest rectangle covering both `self` and `other` (their bounding box).
    pub fn bbox(self, other: Rect) -> Rect {
        let x0 = self.x.min(other.x);
        let y0 = self.y.min(other.y);
        let x1 = self
            .x
            .saturating_add(self.w)
            .max(other.x.saturating_add(other.w));
        let y1 = self
            .y
            .saturating_add(self.h)
            .max(other.y.saturating_add(other.h));
        Rect {
            x: x0,
            y: y0,
            w: x1 - x0,
            h: y1 - y0,
        }
    }

    /// Clip `(x, y, w, h)` to a `surf_w`×`surf_h` surface anchored at the origin,
    /// returning the on-surface rectangle or `None` if it has zero area there.
    ///
    /// This is the surface-dimension-parameterised form of the firmware's
    /// `clip_app_rect` (KotoGFX migration Stage 1, GFX-0001): the body is
    /// identical with the hardcoded `320` replaced by `surf_w`/`surf_h`. The
    /// arithmetic — `clamp(0, surf)` on each edge, saturating right/bottom edges,
    /// and the `x1 > x0 && y1 > y0` emptiness test — is preserved verbatim so the
    /// firmware delegation changes no pixels.
    pub fn clip(x: i32, y: i32, w: i32, h: i32, surf_w: i32, surf_h: i32) -> Option<Rect> {
        let x0 = x.clamp(0, surf_w);
        let y0 = y.clamp(0, surf_h);
        let x1 = x.saturating_add(w).clamp(0, surf_w);
        let y1 = y.saturating_add(h).clamp(0, surf_h);
        (x1 > x0 && y1 > y0).then_some(Rect {
            x: x0,
            y: y0,
            w: x1 - x0,
            h: y1 - y0,
        })
    }

    /// Union of `a` and `b`, clipped to the `surf_w`×`surf_h` surface — the
    /// surface-parameterised form of the firmware's `union_rect`. Takes the
    /// bounding box of the two rects (with saturating right/bottom edges) and
    /// delegates the final clip to [`Rect::clip`], matching the firmware body.
    pub fn union_clipped(a: Rect, b: Rect, surf_w: i32, surf_h: i32) -> Option<Rect> {
        let x0 = a.x.min(b.x);
        let y0 = a.y.min(b.y);
        let x1 = a.x.saturating_add(a.w).max(b.x.saturating_add(b.w));
        let y1 = a.y.saturating_add(a.h).max(b.y.saturating_add(b.h));
        Rect::clip(x0, y0, x1 - x0, y1 - y0, surf_w, surf_h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn area_of_normal_rect() {
        assert_eq!(
            Rect {
                x: 3,
                y: 4,
                w: 16,
                h: 16
            }
            .area(),
            256
        );
    }

    #[test]
    fn area_of_degenerate_rect_is_zero() {
        assert_eq!(
            Rect {
                x: 0,
                y: 0,
                w: 0,
                h: 10
            }
            .area(),
            0
        );
        assert_eq!(
            Rect {
                x: 0,
                y: 0,
                w: -5,
                h: 10
            }
            .area(),
            0
        );
    }

    #[test]
    fn bbox_of_disjoint_rects_covers_both() {
        let a = Rect {
            x: 0,
            y: 0,
            w: 16,
            h: 16,
        };
        let b = Rect {
            x: 100,
            y: 100,
            w: 16,
            h: 16,
        };
        assert_eq!(
            a.bbox(b),
            Rect {
                x: 0,
                y: 0,
                w: 116,
                h: 116
            }
        );
    }

    #[test]
    fn bbox_of_nested_rect_is_the_outer_rect() {
        let outer = Rect {
            x: 0,
            y: 0,
            w: 40,
            h: 40,
        };
        let inner = Rect {
            x: 10,
            y: 10,
            w: 8,
            h: 8,
        };
        assert_eq!(outer.bbox(inner), outer);
    }

    #[test]
    fn bbox_of_edge_adjacent_cells_is_their_sum() {
        let a = Rect {
            x: 0,
            y: 0,
            w: 16,
            h: 16,
        };
        let b = Rect {
            x: 16,
            y: 0,
            w: 16,
            h: 16,
        };
        assert_eq!(
            a.bbox(b),
            Rect {
                x: 0,
                y: 0,
                w: 32,
                h: 16
            }
        );
    }

    #[test]
    fn clip_of_interior_rect_is_unchanged() {
        assert_eq!(
            Rect::clip(10, 20, 30, 40, 320, 320),
            Some(Rect {
                x: 10,
                y: 20,
                w: 30,
                h: 40
            }),
        );
    }

    #[test]
    fn clip_of_fully_offscreen_rect_is_none() {
        // Past the right/bottom edge.
        assert_eq!(Rect::clip(320, 0, 16, 16, 320, 320), None);
        assert_eq!(Rect::clip(0, 320, 16, 16, 320, 320), None);
        // Entirely left/above the origin.
        assert_eq!(Rect::clip(-16, 0, 16, 16, 320, 320), None);
        assert_eq!(Rect::clip(0, -16, 16, 16, 320, 320), None);
    }

    #[test]
    fn clip_of_partially_overlapping_rect_keeps_the_visible_part() {
        // Straddling the top-left corner: negative origin clamps to 0 and the
        // width/height shrink by the off-surface amount.
        assert_eq!(
            Rect::clip(-4, -6, 20, 20, 320, 320),
            Some(Rect {
                x: 0,
                y: 0,
                w: 16,
                h: 14
            }),
        );
        // Straddling the bottom-right corner.
        assert_eq!(
            Rect::clip(310, 300, 40, 40, 320, 320),
            Some(Rect {
                x: 310,
                y: 300,
                w: 10,
                h: 20
            }),
        );
    }

    #[test]
    fn clip_of_zero_area_rect_is_none() {
        assert_eq!(Rect::clip(10, 10, 0, 16, 320, 320), None);
        assert_eq!(Rect::clip(10, 10, 16, 0, 320, 320), None);
        assert_eq!(Rect::clip(10, 10, -5, 16, 320, 320), None);
    }

    #[test]
    fn clip_saturates_on_overflowing_bounds() {
        // x + w overflows i32; the saturating add pins the right edge to i32::MAX,
        // which clamps to surf_w rather than wrapping negative.
        assert_eq!(
            Rect::clip(100, 100, i32::MAX, i32::MAX, 320, 320),
            Some(Rect {
                x: 100,
                y: 100,
                w: 220,
                h: 220
            }),
        );
    }

    #[test]
    fn clip_respects_surface_dims_other_than_320() {
        // Smaller surface clips earlier.
        assert_eq!(
            Rect::clip(60, 60, 40, 40, 80, 100),
            Some(Rect {
                x: 60,
                y: 60,
                w: 20,
                h: 40
            }),
        );
        // Larger surface lets the rect through unclipped.
        assert_eq!(
            Rect::clip(300, 300, 40, 40, 480, 480),
            Some(Rect {
                x: 300,
                y: 300,
                w: 40,
                h: 40
            }),
        );
        // Non-square surface clips x and y independently.
        assert_eq!(
            Rect::clip(0, 0, 200, 200, 128, 64),
            Some(Rect {
                x: 0,
                y: 0,
                w: 128,
                h: 64
            }),
        );
    }

    #[test]
    fn union_clipped_covers_both_rects() {
        let a = Rect {
            x: 10,
            y: 10,
            w: 16,
            h: 16,
        };
        let b = Rect {
            x: 50,
            y: 60,
            w: 16,
            h: 16,
        };
        assert_eq!(
            Rect::union_clipped(a, b, 320, 320),
            Some(Rect {
                x: 10,
                y: 10,
                w: 56,
                h: 66
            }),
        );
    }

    #[test]
    fn union_clipped_clips_the_combined_box_to_the_surface() {
        let a = Rect {
            x: -10,
            y: -10,
            w: 20,
            h: 20,
        };
        let b = Rect {
            x: 300,
            y: 300,
            w: 40,
            h: 40,
        };
        // Bounding box is (-10,-10)..(340,340); clipped to 320 on every edge.
        assert_eq!(
            Rect::union_clipped(a, b, 320, 320),
            Some(Rect {
                x: 0,
                y: 0,
                w: 320,
                h: 320
            }),
        );
    }

    #[test]
    fn union_clipped_of_fully_offscreen_rects_is_none() {
        let a = Rect {
            x: 400,
            y: 400,
            w: 16,
            h: 16,
        };
        let b = Rect {
            x: 500,
            y: 500,
            w: 16,
            h: 16,
        };
        assert_eq!(Rect::union_clipped(a, b, 320, 320), None);
    }

    #[test]
    fn union_clipped_respects_surface_dims_other_than_320() {
        let a = Rect {
            x: 0,
            y: 0,
            w: 16,
            h: 16,
        };
        let b = Rect {
            x: 40,
            y: 40,
            w: 60,
            h: 60,
        };
        // Box is (0,0)..(100,100); a 64x80 surface clips it.
        assert_eq!(
            Rect::union_clipped(a, b, 64, 80),
            Some(Rect {
                x: 0,
                y: 0,
                w: 64,
                h: 80
            }),
        );
    }
}
