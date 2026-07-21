/// Stable caller-assigned identity for a KotoUI component.
///
/// KotoUI does not allocate IDs or attach lifetime meaning to their numeric
/// value. A component collection is responsible for keeping live IDs unique.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct WidgetId(u16);

impl WidgetId {
    pub const fn new(value: u16) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u16 {
        self.0
    }
}

/// An absolute rectangle in signed surface coordinates.
///
/// Width and height must both be positive for a rectangle to contain pixels.
/// Edge arithmetic is evaluated in `i64`, so hostile `i32` coordinates cannot
/// overflow while clipping or testing intersections.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(C)]
pub struct UiRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl UiRect {
    pub const EMPTY: Self = Self::new(0, 0, 0, 0);

    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    pub const fn is_empty(self) -> bool {
        self.w <= 0 || self.h <= 0
    }

    /// Returns the rectangle area, or `None` when it is empty or exceeds `u64`.
    pub const fn area(self) -> Option<u64> {
        if self.is_empty() {
            return None;
        }
        (self.w as u64).checked_mul(self.h as u64)
    }

    /// Intersects two rectangles without overflowing signed coordinate math.
    pub fn intersection(self, other: Self) -> Option<Self> {
        if self.is_empty() || other.is_empty() {
            return None;
        }

        let left = i64::from(self.x).max(i64::from(other.x));
        let top = i64::from(self.y).max(i64::from(other.y));
        let right = self.right_i64().min(other.right_i64());
        let bottom = self.bottom_i64().min(other.bottom_i64());
        Self::from_edges(left, top, right, bottom)
    }

    /// Returns the smallest representable rectangle containing both inputs.
    ///
    /// `None` means both rectangles are empty or the bounding width/height does
    /// not fit in `i32`. Callers can retain the two rectangles separately in the
    /// latter case.
    pub fn union(self, other: Self) -> Option<Self> {
        if self.is_empty() {
            return (!other.is_empty()).then_some(other);
        }
        if other.is_empty() {
            return Some(self);
        }

        let left = i64::from(self.x).min(i64::from(other.x));
        let top = i64::from(self.y).min(i64::from(other.y));
        let right = self.right_i64().max(other.right_i64());
        let bottom = self.bottom_i64().max(other.bottom_i64());
        Self::from_edges(left, top, right, bottom)
    }

    pub fn contains(self, other: Self) -> bool {
        if self.is_empty() || other.is_empty() {
            return false;
        }
        i64::from(self.x) <= i64::from(other.x)
            && i64::from(self.y) <= i64::from(other.y)
            && self.right_i64() >= other.right_i64()
            && self.bottom_i64() >= other.bottom_i64()
    }

    pub fn intersects_or_touches(self, other: Self) -> bool {
        if self.is_empty() || other.is_empty() {
            return false;
        }
        i64::from(self.x) <= other.right_i64()
            && i64::from(other.x) <= self.right_i64()
            && i64::from(self.y) <= other.bottom_i64()
            && i64::from(other.y) <= self.bottom_i64()
    }

    /// Insets all four edges by `amount`, returning `None` if no pixels remain.
    pub fn inset(self, amount: i32) -> Option<Self> {
        if self.is_empty() || amount < 0 {
            return None;
        }
        let amount = i64::from(amount);
        Self::from_edges(
            i64::from(self.x) + amount,
            i64::from(self.y) + amount,
            self.right_i64() - amount,
            self.bottom_i64() - amount,
        )
    }

    const fn right_i64(self) -> i64 {
        self.x as i64 + self.w as i64
    }

    const fn bottom_i64(self) -> i64 {
        self.y as i64 + self.h as i64
    }

    fn from_edges(left: i64, top: i64, right: i64, bottom: i64) -> Option<Self> {
        let width = right.checked_sub(left)?;
        let height = bottom.checked_sub(top)?;
        if width <= 0 || height <= 0 {
            return None;
        }
        Some(Self {
            x: i32::try_from(left).ok()?,
            y: i32::try_from(top).ok()?,
            w: i32::try_from(width).ok()?,
            h: i32::try_from(height).ok()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intersection_clips_extreme_coordinates_without_overflow() {
        let surface = UiRect::new(0, 0, 320, 320);
        assert_eq!(
            UiRect::new(i32::MIN, 10, i32::MAX, 20).intersection(surface),
            None
        );
        assert_eq!(
            UiRect::new(i32::MAX - 10, 10, 20, 20).intersection(surface),
            None
        );
        assert_eq!(
            UiRect::new(-10, -20, 30, 40).intersection(surface),
            Some(UiRect::new(0, 0, 20, 20))
        );
    }

    #[test]
    fn empty_rectangles_have_no_intersection_or_area() {
        let rect = UiRect::new(1, 2, 0, 10);
        assert!(rect.is_empty());
        assert_eq!(rect.area(), None);
        assert_eq!(rect.intersection(UiRect::new(0, 0, 10, 10)), None);
    }

    #[test]
    fn unrepresentable_union_is_reported() {
        let left = UiRect::new(i32::MIN, 0, 1, 1);
        let right = UiRect::new(i32::MAX - 1, 0, 1, 1);
        assert_eq!(left.union(right), None);
    }

    #[test]
    fn inset_rejects_empty_or_over_inset_rectangles() {
        assert_eq!(
            UiRect::new(0, 0, 10, 8).inset(2),
            Some(UiRect::new(2, 2, 6, 4))
        );
        assert_eq!(UiRect::new(0, 0, 4, 4).inset(2), None);
        assert_eq!(UiRect::new(0, 0, 4, 4).inset(-1), None);
    }
}
