use crate::{DamageRects, DamageSet, Theme, UiRect, DEFAULT_DAMAGE_CAPACITY};

/// Small caller-owned context shared while updating and painting components.
///
/// It owns only theme tokens and bounded damage metadata. Component state,
/// models, font resources, framebuffers, and backend handles remain outside it.
#[derive(Clone, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct UiContext<const DAMAGE: usize = DEFAULT_DAMAGE_CAPACITY> {
    theme: Theme,
    damage: DamageSet<DAMAGE>,
}

impl<const DAMAGE: usize> UiContext<DAMAGE> {
    pub const fn new(surface: UiRect, theme: Theme) -> Self {
        Self {
            theme,
            damage: DamageSet::new(surface),
        }
    }

    pub const fn theme(&self) -> &Theme {
        &self.theme
    }

    pub fn set_theme(&mut self, theme: Theme) {
        if self.theme != theme {
            self.theme = theme;
            self.damage.push(self.damage.surface());
        }
    }

    pub const fn surface(&self) -> UiRect {
        self.damage.surface()
    }

    pub fn set_surface(&mut self, surface: UiRect) {
        self.damage.set_surface(surface);
        self.damage.push(surface);
    }

    pub fn damage(&mut self, rect: UiRect) {
        self.damage.push(rect);
    }

    pub fn damage_transition(&mut self, old: UiRect, new: UiRect) {
        self.damage.push_transition(old, new);
    }

    pub fn damaged_rects(&self) -> DamageRects<'_, DAMAGE> {
        self.damage.iter()
    }

    pub const fn has_damage(&self) -> bool {
        !self.damage.is_empty()
    }

    pub fn clear_damage(&mut self) {
        self.damage.clear();
    }
}

#[cfg(test)]
mod tests {
    use core::mem::size_of;

    use super::*;

    #[test]
    fn unchanged_theme_and_bounds_remain_idle() {
        let surface = UiRect::new(0, 0, 320, 320);
        let mut context = UiContext::<8>::new(surface, Theme::DARK);
        context.set_theme(Theme::DARK);
        context.damage_transition(surface, surface);
        assert!(!context.has_damage());
    }

    #[test]
    fn core_sizes_match_the_documented_64_bit_measurements() {
        assert_eq!(size_of::<UiRect>(), 16);
        assert_eq!(size_of::<Theme>(), 32);
        assert_eq!(size_of::<DamageSet<8>>(), 160);
        assert_eq!(size_of::<UiContext<8>>(), 192);
    }
}
