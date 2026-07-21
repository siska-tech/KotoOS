use crate::UiRect;

/// Default number of independently damaged regions retained before fallback.
pub const DEFAULT_DAMAGE_CAPACITY: usize = 8;

/// Fixed-capacity, surface-clipped dirty-region accumulator.
///
/// Intersecting or edge-touching regions are coalesced. If more than `N`
/// independent regions are needed, iteration yields the complete surface as a
/// single conservative fallback. `N == 0` is supported and always falls back
/// on the first non-empty damage request.
#[derive(Clone, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct DamageSet<const N: usize = DEFAULT_DAMAGE_CAPACITY> {
    surface: UiRect,
    rects: [UiRect; N],
    len: usize,
    fallback: bool,
}

impl<const N: usize> DamageSet<N> {
    pub const fn new(surface: UiRect) -> Self {
        Self {
            surface,
            rects: [UiRect::EMPTY; N],
            len: 0,
            fallback: false,
        }
    }

    pub const fn surface(&self) -> UiRect {
        self.surface
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0 && !self.fallback
    }

    pub const fn is_fallback(&self) -> bool {
        self.fallback
    }

    pub const fn len(&self) -> usize {
        if self.fallback {
            1
        } else {
            self.len
        }
    }

    pub fn clear(&mut self) {
        self.len = 0;
        self.fallback = false;
    }

    pub fn set_surface(&mut self, surface: UiRect) {
        self.surface = surface;
        self.clear();
    }

    pub fn push(&mut self, rect: UiRect) {
        if self.fallback {
            return;
        }
        let Some(mut incoming) = rect.intersection(self.surface) else {
            return;
        };

        let mut index = 0;
        while index < self.len {
            let current = self.rects[index];
            if current.contains(incoming) {
                return;
            }
            if incoming.contains(current) || current.intersects_or_touches(incoming) {
                let Some(joined) = current.union(incoming) else {
                    index += 1;
                    continue;
                };
                incoming = joined;
                self.remove(index);
                index = 0;
                continue;
            }
            index += 1;
        }

        if self.len < N {
            self.rects[self.len] = incoming;
            self.len += 1;
        } else {
            self.len = 0;
            self.fallback = true;
        }
    }

    /// Damages the union of old and new bounds when visual state changes.
    pub fn push_transition(&mut self, old: UiRect, new: UiRect) {
        if old == new {
            return;
        }
        if let Some(union) = old.union(new) {
            self.push(union);
        } else {
            self.push(old);
            self.push(new);
        }
    }

    pub fn iter(&self) -> DamageRects<'_, N> {
        DamageRects {
            set: self,
            index: 0,
        }
    }

    fn remove(&mut self, index: usize) {
        let mut cursor = index;
        while cursor + 1 < self.len {
            self.rects[cursor] = self.rects[cursor + 1];
            cursor += 1;
        }
        self.len -= 1;
        self.rects[self.len] = UiRect::EMPTY;
    }
}

pub struct DamageRects<'a, const N: usize> {
    set: &'a DamageSet<N>,
    index: usize,
}

impl<const N: usize> Iterator for DamageRects<'_, N> {
    type Item = UiRect;

    fn next(&mut self) -> Option<Self::Item> {
        if self.set.fallback {
            if self.index == 0 {
                self.index = 1;
                return (!self.set.surface.is_empty()).then_some(self.set.surface);
            }
            return None;
        }
        if self.index >= self.set.len {
            return None;
        }
        let rect = self.set.rects[self.index];
        self.index += 1;
        Some(rect)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.set.len().saturating_sub(self.index);
        (remaining, Some(remaining))
    }
}

impl<const N: usize> ExactSizeIterator for DamageRects<'_, N> {}

#[cfg(test)]
mod tests {
    use super::*;

    const SURFACE: UiRect = UiRect::new(0, 0, 100, 100);

    #[test]
    fn clips_deduplicates_and_coalesces_damage() {
        let mut damage = DamageSet::<4>::new(SURFACE);
        damage.push(UiRect::new(-10, 10, 20, 20));
        damage.push(UiRect::new(2, 12, 2, 2));
        damage.push(UiRect::new(10, 10, 5, 20));
        assert_eq!(
            damage.iter().collect::<std::vec::Vec<_>>(),
            [UiRect::new(0, 10, 15, 20)]
        );
    }

    #[test]
    fn ignores_empty_and_off_surface_damage() {
        let mut damage = DamageSet::<2>::new(SURFACE);
        damage.push(UiRect::new(0, 0, 0, 10));
        damage.push(UiRect::new(200, 200, 10, 10));
        assert!(damage.is_empty());
    }

    #[test]
    fn capacity_overflow_falls_back_to_surface() {
        let mut damage = DamageSet::<2>::new(SURFACE);
        damage.push(UiRect::new(0, 0, 5, 5));
        damage.push(UiRect::new(20, 20, 5, 5));
        damage.push(UiRect::new(40, 40, 5, 5));
        assert!(damage.is_fallback());
        assert_eq!(damage.iter().collect::<std::vec::Vec<_>>(), [SURFACE]);
    }

    #[test]
    fn zero_capacity_has_a_valid_fallback_iterator() {
        let mut damage = DamageSet::<0>::new(SURFACE);
        damage.push(UiRect::new(1, 1, 1, 1));
        assert_eq!(damage.len(), 1);
        assert_eq!(damage.iter().next(), Some(SURFACE));
    }

    #[test]
    fn transition_uses_union_and_unchanged_transition_is_idle() {
        let mut damage = DamageSet::<2>::new(SURFACE);
        damage.push_transition(UiRect::new(10, 10, 5, 5), UiRect::new(20, 10, 5, 5));
        assert_eq!(damage.iter().next(), Some(UiRect::new(10, 10, 15, 5)));
        damage.clear();
        damage.push_transition(UiRect::new(10, 10, 5, 5), UiRect::new(10, 10, 5, 5));
        assert!(damage.is_empty());
    }
}
