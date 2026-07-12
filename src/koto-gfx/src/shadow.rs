//! Fingerprint shadow of the last applied static layer, and the aligned diff
//! that turns a mid-session static rebuild into bounded damage (GFX-0013).
//!
//! The retained static layer is a **single** instance (KOTO-0136: no double
//! buffer — storing a previous copy inside the diff hosts doubled its ~6 KiB and
//! hung boot), so a rebuild has no old command list to diff against and the
//! present path has always taken a whole-surface `StaticRebuild` repaint. The
//! retained-render app migrations made rebuilds a recurring per-action event
//! (KotoShogi per move, KotoRogue per room), so that ~180 ms repaint is now a
//! per-action hitch — yet across a mid-session rebuild most commands are
//! byte-identical; only a handful differ.
//!
//! [`StaticLayerShadow`] retains what a diff actually needs, at ~1/6 the cost of
//! a second layer: per command a 32-bit FNV-1a content fingerprint plus its
//! clipped on-surface footprint. [`collect_static_rebuild_dirty`] aligns a
//! rebuilt list against the shadow with the GFX-0008 prefix/suffix technique
//! (fingerprints stand in for command equality) and pushes the unmatched
//! middle's old∪new footprints into the caller's dirty set — so an identical
//! rebuild is a skip, a small edit is a few bands, and anything unalignable
//! reports itself so the caller falls back to the BUG-GFX-0012 full-repaint
//! latch. The latch is never removed; it is the escape hatch for every outcome
//! this diff does not positively bound.
//!
//! Unlike the immediate diff ([`crate::collect_immediate_dirty`]), a full-screen
//! base fill inside the unmatched region is **not** skipped: the static layer is
//! the base's home, and with a single retained instance a recolored base never
//! trips the present path's base-change check (both sides read the same layer).
//! A recolored base therefore must surface here — as whole-surface damage that
//! [`crate::FullRepaintPolicy`] escalates to a full repaint. A matched base
//! lands in the common prefix and costs nothing, like every other matched
//! command.

use crate::derive::{command_rect, union_or_either};
use crate::layer::{AppDrawCommand, GAME2D_STATIC_CMD_CAP};
use crate::{push_dirty, Rect};

/// Alignment-window cap for the static-rebuild diff: an unmatched middle wider
/// than this is a genuine relayout (title -> gameplay chrome swap, layer
/// emptied), not a localizable edit, so the caller falls back to the full
/// repaint + reveal latch exactly as before GFX-0013. Sized above the rect
/// escalation threshold ([`crate::FULL_REPAINT_RECTS`]) with the same headroom
/// rationale as the firmware's board band buffer: the damage feeds the
/// coalesce-before-decide path (GFX-0010), so a wider-than-24 but coalescible
/// edit can still be rescued; past 32 slots the rebuild is treated as a
/// relayout. Also sizes the caller's damage buffer (one union rect per slot).
pub const STATIC_DAMAGE_CAP: usize = 32;

const FNV_OFFSET: u32 = 0x811c_9dc5;
const FNV_PRIME: u32 = 0x0100_0193;

#[inline]
fn fnv1a(hash: u32, bytes: &[u8]) -> u32 {
    bytes.iter().fold(hash, |hash, byte| {
        (hash ^ u32::from(*byte)).wrapping_mul(FNV_PRIME)
    })
}

#[inline]
fn fnv_i32(hash: u32, value: i32) -> u32 {
    fnv1a(hash, &value.to_le_bytes())
}

/// 32-bit FNV-1a content fingerprint of one static command. Covers the variant
/// tag and every semantic field — for `Text`, only the live `bytes[..len]` (the
/// buffer padding is not content). Two commands with equal fingerprints are
/// treated as identical by the aligned diff; a collision would silently skip a
/// changed command's damage, so the full 32-bit hash over all fields keeps that
/// chance negligible at the 80-command layer size.
fn command_fingerprint(command: &AppDrawCommand) -> u32 {
    match command {
        AppDrawCommand::Empty => fnv1a(FNV_OFFSET, &[0]),
        AppDrawCommand::Rect { x, y, w, h, rgb565 } => {
            let hash = fnv1a(FNV_OFFSET, &[1]);
            let hash = fnv_i32(hash, *x);
            let hash = fnv_i32(hash, *y);
            let hash = fnv_i32(hash, *w);
            let hash = fnv_i32(hash, *h);
            fnv1a(hash, &rgb565.to_le_bytes())
        }
        AppDrawCommand::Text {
            x,
            y,
            rgb565,
            bytes,
            len,
        } => {
            let hash = fnv1a(FNV_OFFSET, &[2]);
            let hash = fnv_i32(hash, *x);
            let hash = fnv_i32(hash, *y);
            let hash = fnv1a(hash, &rgb565.to_le_bytes());
            let live = (*len as usize).min(bytes.len());
            fnv1a(hash, &bytes[..live])
        }
        AppDrawCommand::Pixels {
            x,
            y,
            w,
            h,
            off,
            len,
        } => {
            let hash = fnv1a(FNV_OFFSET, &[3]);
            let hash = fnv_i32(hash, *x);
            let hash = fnv_i32(hash, *y);
            let hash = fnv_i32(hash, *w);
            let hash = fnv_i32(hash, *h);
            let hash = fnv1a(hash, &off.to_le_bytes());
            fnv1a(hash, &len.to_le_bytes())
        }
    }
}

/// One shadowed command: its content fingerprint and its clipped on-surface
/// footprint, packed to i16 (surface coordinates fit; `w == 0` marks a command
/// with no on-screen footprint). 12 bytes — the whole 80-entry shadow stays
/// ~1 KiB where a second `AppStaticLayer` would cost ~6 KiB (the KOTO-0136
/// boot-stack lesson).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ShadowEntry {
    hash: u32,
    x: i16,
    y: i16,
    w: i16,
    h: i16,
}

impl ShadowEntry {
    const EMPTY: Self = Self {
        hash: 0,
        x: 0,
        y: 0,
        w: 0,
        h: 0,
    };

    fn of(command: &AppDrawCommand, surf_w: i32, surf_h: i32) -> Self {
        let (x, y, w, h) = match command_rect(*command, surf_w, surf_h) {
            Some(rect) => (rect.x as i16, rect.y as i16, rect.w as i16, rect.h as i16),
            None => (0, 0, 0, 0),
        };
        Self {
            hash: command_fingerprint(command),
            x,
            y,
            w,
            h,
        }
    }

    fn rect(&self) -> Option<Rect> {
        (self.w > 0 && self.h > 0).then_some(Rect {
            x: i32::from(self.x),
            y: i32::from(self.y),
            w: i32::from(self.w),
            h: i32::from(self.h),
        })
    }
}

/// Compact shadow of the last **applied** static layer (GFX-0013): per command a
/// content fingerprint plus its clipped footprint. The owner captures it after a
/// present that applied a (re)built layer and invalidates it at session start;
/// while invalid, [`collect_static_rebuild_dirty`] reports
/// [`StaticRebuildAlignment::NoShadow`] and the caller keeps the full-repaint
/// latch path. Retained session state — the firmware holds it in its own
/// `StaticCell` (not inside the double-buffered draw hosts), per the KOTO-0136 /
/// KOTO-0148 ownership rule.
pub struct StaticLayerShadow {
    entries: [ShadowEntry; GAME2D_STATIC_CMD_CAP],
    len: usize,
    valid: bool,
}

impl StaticLayerShadow {
    pub const fn new() -> Self {
        Self {
            entries: [ShadowEntry::EMPTY; GAME2D_STATIC_CMD_CAP],
            len: 0,
            valid: false,
        }
    }

    /// Drop the capture (session start / app switch): the next rebuild has no
    /// trusted baseline and must take the full-repaint latch path.
    pub fn invalidate(&mut self) {
        self.len = 0;
        self.valid = false;
    }

    /// Whether a capture is held (a valid baseline exists to diff against).
    pub fn is_valid(&self) -> bool {
        self.valid
    }

    /// Capture `commands` — the static layer as just applied to the panel — as
    /// the new baseline. A list past the shadow capacity cannot happen for a
    /// layer bounded by [`GAME2D_STATIC_CMD_CAP`]; if it ever does, the shadow
    /// invalidates itself rather than hold a truncated (untrustworthy) baseline.
    pub fn capture(&mut self, commands: &[AppDrawCommand], surf_w: i32, surf_h: i32) {
        if commands.len() > self.entries.len() {
            self.invalidate();
            return;
        }
        for (entry, command) in self.entries.iter_mut().zip(commands.iter()) {
            *entry = ShadowEntry::of(command, surf_w, surf_h);
        }
        self.len = commands.len();
        self.valid = true;
    }
}

impl Default for StaticLayerShadow {
    fn default() -> Self {
        Self::new()
    }
}

/// How a rebuilt static list aligned against the shadow of the last applied one.
/// Only `Identical` and `Bounded` positively bound the change; everything else
/// sends the caller to the existing full-repaint + reveal-latch path
/// (BUG-GFX-0012), which covers every pixel by construction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StaticRebuildAlignment {
    /// No valid shadow to diff against (session start, or a prior capture was
    /// dropped): fall back to the latch path.
    NoShadow,
    /// Every command fingerprint matched — the rebuild reproduced the applied
    /// layer exactly. No damage was pushed; the rebuild needs no repaint at all.
    Identical,
    /// The unmatched middle spans `region` slots (≤ the window cap); each
    /// unmatched pair's old∪new footprint was pushed into the caller's dirty
    /// set. The damage covers both the content that left and the content that
    /// arrived, so acting on it (or escalating it through the policy) always
    /// reaches GRAM with the rebuilt content.
    Bounded { region: usize },
    /// The unmatched middle exceeds the window cap — a relayout, not an edit.
    /// Nothing was pushed; fall back to the latch path.
    Wide { region: usize },
}

/// Diff a rebuilt static command list against the shadow of the last applied
/// one, pushing the unmatched middle's damage into the caller's dirty set
/// (GFX-0013). The alignment is the GFX-0008 prefix/suffix technique over
/// content fingerprints: matched ends contribute no damage (they repaint the
/// same pixels in the same relative order), and the bounded middle is paired
/// positionally with the shorter side padded absent — an insert/remove damages
/// its own footprint, a changed run damages its old∪new band.
///
/// Damage is pushed through [`push_dirty`] into `dirty`/`len`/`area`/`overflow`
/// exactly as every other layer's collection, so the caller's existing
/// truncation fail-safe applies: an `overflow` here means the damage set is
/// incomplete and the frame must not stay incremental on it. A full-screen base
/// fill is deliberately **not** skipped (see the module docs): a recolored base
/// becomes whole-surface damage the policy escalates.
#[allow(clippy::too_many_arguments)]
pub fn collect_static_rebuild_dirty(
    shadow: &StaticLayerShadow,
    cur: &[AppDrawCommand],
    surf_w: i32,
    surf_h: i32,
    max_region: usize,
    dirty: &mut [Rect],
    len: &mut usize,
    area: &mut u32,
    overflow: &mut bool,
) -> StaticRebuildAlignment {
    if !shadow.valid {
        return StaticRebuildAlignment::NoShadow;
    }
    let prev_len = shadow.len;
    let cur_len = cur.len();
    let max_common = prev_len.min(cur_len);
    let mut prefix = 0;
    while prefix < max_common && shadow.entries[prefix].hash == command_fingerprint(&cur[prefix]) {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < max_common - prefix
        && shadow.entries[prev_len - 1 - suffix].hash
            == command_fingerprint(&cur[cur_len - 1 - suffix])
    {
        suffix += 1;
    }
    let region_prev = prev_len - prefix - suffix;
    let region_cur = cur_len - prefix - suffix;
    let region = region_prev.max(region_cur);
    if region == 0 {
        return StaticRebuildAlignment::Identical;
    }
    if region > max_region {
        return StaticRebuildAlignment::Wide { region };
    }
    for k in 0..region {
        let old = (k < region_prev).then(|| shadow.entries[prefix + k]);
        let new = (k < region_cur).then(|| &cur[prefix + k]);
        // A positionally paired slot whose fingerprints match is unchanged (two
        // separated edits with identical commands between them): no damage.
        if let (Some(old_entry), Some(new_command)) = (&old, &new) {
            if old_entry.hash == command_fingerprint(new_command) {
                continue;
            }
        }
        let old_rect = old.and_then(|entry| entry.rect());
        let new_rect = new.and_then(|command| command_rect(*command, surf_w, surf_h));
        if let Some(rect) = union_or_either(old_rect, new_rect, surf_w, surf_h) {
            push_dirty(dirty, len, area, overflow, rect);
        }
    }
    StaticRebuildAlignment::Bounded { region }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::size_of;

    const SURF: i32 = 320;

    fn rect_cmd(x: i32, y: i32, rgb565: u16) -> AppDrawCommand {
        AppDrawCommand::Rect {
            x,
            y,
            w: 16,
            h: 16,
            rgb565,
        }
    }

    fn base_cmd(rgb565: u16) -> AppDrawCommand {
        AppDrawCommand::Rect {
            x: 0,
            y: 0,
            w: 320,
            h: 320,
            rgb565,
        }
    }

    fn text_cmd(x: i32, y: i32, text: &str) -> AppDrawCommand {
        let mut bytes = [0u8; crate::MAX_APP_TEXT_BYTES];
        bytes[..text.len()].copy_from_slice(text.as_bytes());
        AppDrawCommand::Text {
            x,
            y,
            rgb565: 0xFFFF,
            bytes,
            len: text.len() as u8,
        }
    }

    fn captured(commands: &[AppDrawCommand]) -> StaticLayerShadow {
        let mut shadow = StaticLayerShadow::new();
        shadow.capture(commands, SURF, SURF);
        shadow
    }

    /// Run the diff into a fresh dirty set, returning the alignment and the
    /// collected `(rects, area, overflow)`.
    fn diff(
        shadow: &StaticLayerShadow,
        cur: &[AppDrawCommand],
    ) -> (StaticRebuildAlignment, Vec<Rect>, u32, bool) {
        let mut dirty = [Rect {
            x: 0,
            y: 0,
            w: 0,
            h: 0,
        }; 256];
        let (mut len, mut area, mut overflow) = (0usize, 0u32, false);
        let alignment = collect_static_rebuild_dirty(
            shadow,
            cur,
            SURF,
            SURF,
            STATIC_DAMAGE_CAP,
            &mut dirty,
            &mut len,
            &mut area,
            &mut overflow,
        );
        (alignment, dirty[..len].to_vec(), area, overflow)
    }

    /// A typical retained-render static layer: base fill, panel chrome, rows of
    /// content runs — the KotoRogue fog / KotoShogi board shape.
    fn chrome_list() -> Vec<AppDrawCommand> {
        let mut list = vec![base_cmd(0x0001)];
        list.push(AppDrawCommand::Rect {
            x: 200,
            y: 0,
            w: 120,
            h: 320,
            rgb565: 0x2104,
        });
        for i in 0..40 {
            list.push(rect_cmd((i % 10) * 16, (i / 10) * 16, 0x8410));
        }
        list.push(text_cmd(208, 8, "HP"));
        list
    }

    #[test]
    fn shadow_is_compact() {
        // The whole 80-entry shadow must stay within the ~1.5 KiB SRAM budget
        // the issue allots (vs ~6 KiB for a second AppStaticLayer).
        assert_eq!(size_of::<ShadowEntry>(), 12);
        assert!(size_of::<StaticLayerShadow>() <= 1536);
    }

    #[test]
    fn invalid_shadow_reports_no_shadow() {
        let shadow = StaticLayerShadow::new();
        let (alignment, rects, ..) = diff(&shadow, &chrome_list());
        assert_eq!(alignment, StaticRebuildAlignment::NoShadow);
        assert!(rects.is_empty());

        let mut invalidated = captured(&chrome_list());
        invalidated.invalidate();
        let (alignment, ..) = diff(&invalidated, &chrome_list());
        assert_eq!(alignment, StaticRebuildAlignment::NoShadow);
    }

    #[test]
    fn identical_rebuild_is_a_skip() {
        // The headline Stage-1 case: an app rebuilds its static layer with the
        // exact same commands (gating missed a no-op rebuild). No damage.
        let list = chrome_list();
        let shadow = captured(&list);
        let (alignment, rects, area, overflow) = diff(&shadow, &list);
        assert_eq!(alignment, StaticRebuildAlignment::Identical);
        assert!(rects.is_empty());
        assert_eq!(area, 0);
        assert!(!overflow);
    }

    #[test]
    fn single_changed_run_damages_its_own_band_union() {
        // One fog run recolors in place: damage is exactly that band's old∪new
        // union (same rect here), not the whole panel.
        let prev = chrome_list();
        let mut cur = prev.clone();
        cur[10] = rect_cmd(128, 0, 0xFFFF); // was rect at (8*16=128, 0) col 8 row 0
        let shadow = captured(&prev);
        let (alignment, rects, ..) = diff(&shadow, &cur);
        assert_eq!(alignment, StaticRebuildAlignment::Bounded { region: 1 });
        assert_eq!(
            rects,
            vec![Rect {
                x: 128,
                y: 0,
                w: 16,
                h: 16
            }]
        );
    }

    #[test]
    fn moved_run_damages_old_and_new_footprints() {
        // A run moves: the union covers both the cells it left and entered.
        let prev = chrome_list();
        let mut cur = prev.clone();
        cur[2] = rect_cmd(48, 0, 0x8410); // slot 2 was at (0, 0)
        let shadow = captured(&prev);
        let (alignment, rects, ..) = diff(&shadow, &cur);
        assert_eq!(alignment, StaticRebuildAlignment::Bounded { region: 1 });
        assert_eq!(
            rects,
            vec![Rect {
                x: 0,
                y: 0,
                w: 64,
                h: 16
            }]
        );
    }

    #[test]
    fn inserted_command_damages_only_its_footprint() {
        // One run added mid-list (a fog cell revealed): the shifted tail is
        // fingerprint-identical suffix, so only the insert's footprint dirties.
        let prev = chrome_list();
        let mut cur = prev.clone();
        cur.insert(20, rect_cmd(100, 200, 0xBEEF));
        let shadow = captured(&prev);
        let (alignment, rects, ..) = diff(&shadow, &cur);
        assert_eq!(alignment, StaticRebuildAlignment::Bounded { region: 1 });
        assert_eq!(
            rects,
            vec![Rect {
                x: 100,
                y: 200,
                w: 16,
                h: 16
            }]
        );
    }

    #[test]
    fn removed_command_damages_its_old_footprint_from_the_shadow() {
        // One run removed (fog covering a cell): the old footprint must be
        // recomposited away, sourced from the shadow's retained rect.
        let prev = chrome_list();
        let mut cur = prev.clone();
        let removed = cur.remove(15);
        let AppDrawCommand::Rect { x, y, w, h, .. } = removed else {
            panic!("fixture slot 15 is a rect");
        };
        let shadow = captured(&prev);
        let (alignment, rects, ..) = diff(&shadow, &cur);
        assert_eq!(alignment, StaticRebuildAlignment::Bounded { region: 1 });
        assert_eq!(rects, vec![Rect { x, y, w, h }]);
    }

    #[test]
    fn separated_edits_skip_matched_middle_pairs() {
        // Two runs change with untouched commands between them (equal length):
        // the region spans the outer edits but the matched middle pairs are
        // skipped, so damage is two bands, not the whole span.
        let prev = chrome_list();
        let mut cur = prev.clone();
        cur[5] = rect_cmd(48, 0, 0xF800);
        cur[9] = rect_cmd(112, 0, 0x07E0);
        let shadow = captured(&prev);
        let (alignment, rects, ..) = diff(&shadow, &cur);
        assert_eq!(alignment, StaticRebuildAlignment::Bounded { region: 5 });
        assert_eq!(rects.len(), 2);
    }

    #[test]
    fn changed_text_damages_its_row_band() {
        let prev = chrome_list();
        let mut cur = prev.clone();
        let last = cur.len() - 1;
        cur[last] = text_cmd(208, 8, "MP");
        let shadow = captured(&prev);
        let (alignment, rects, ..) = diff(&shadow, &cur);
        assert_eq!(alignment, StaticRebuildAlignment::Bounded { region: 1 });
        // Both old and new occupy the same text band: x..surface edge, 17 tall.
        assert_eq!(
            rects,
            vec![Rect {
                x: 208,
                y: 8,
                w: 320 - 208,
                h: 17
            }]
        );
    }

    #[test]
    fn recolored_base_fill_damages_the_whole_surface() {
        // The base is NOT skipped here (module docs): with a single retained
        // layer instance the present path's base-change check cannot see a
        // recolor across a rebuild, so it must surface as whole-surface damage
        // for the policy to escalate.
        let prev = chrome_list();
        let mut cur = prev.clone();
        cur[0] = base_cmd(0x2002);
        let shadow = captured(&prev);
        let (alignment, rects, area, _) = diff(&shadow, &cur);
        assert_eq!(alignment, StaticRebuildAlignment::Bounded { region: 1 });
        assert_eq!(
            rects,
            vec![Rect {
                x: 0,
                y: 0,
                w: 320,
                h: 320
            }]
        );
        assert_eq!(area, 320 * 320);
    }

    #[test]
    fn whole_list_relayout_is_wide() {
        // Title -> gameplay chrome swap: nothing aligns, the region is the
        // whole list, well past the window — the caller keeps the latch path.
        let prev = chrome_list();
        let cur: Vec<_> = (0..40)
            .map(|i| rect_cmd((i % 10) * 16 + 1, (i / 10) * 16 + 1, 0x1111))
            .collect();
        let shadow = captured(&prev);
        let (alignment, rects, ..) = diff(&shadow, &cur);
        assert!(
            matches!(alignment, StaticRebuildAlignment::Wide { region } if region > STATIC_DAMAGE_CAP)
        );
        assert!(rects.is_empty(), "a wide relayout pushes nothing");
    }

    #[test]
    fn emptied_layer_is_wide_not_underdamaged() {
        // A rebuild to an empty layer (KotoRogue death frame): the whole old
        // list is unmatched. With 40+ old commands that exceeds the window, so
        // it falls back to the latch — never a silent skip.
        let prev = chrome_list();
        let shadow = captured(&prev);
        let (alignment, rects, ..) = diff(&shadow, &[]);
        assert!(matches!(alignment, StaticRebuildAlignment::Wide { .. }));
        assert!(rects.is_empty());
    }

    #[test]
    fn emptied_small_layer_damages_every_old_footprint() {
        // A small layer emptied within the window: every old footprint is
        // damage (erase), sourced entirely from the shadow.
        let prev = [rect_cmd(0, 0, 1), rect_cmd(32, 0, 2)];
        let shadow = captured(&prev);
        let (alignment, rects, ..) = diff(&shadow, &[]);
        assert_eq!(alignment, StaticRebuildAlignment::Bounded { region: 2 });
        assert_eq!(
            rects,
            vec![
                Rect {
                    x: 0,
                    y: 0,
                    w: 16,
                    h: 16
                },
                Rect {
                    x: 32,
                    y: 0,
                    w: 16,
                    h: 16
                },
            ]
        );
    }

    #[test]
    fn adjacent_swap_damages_both_footprints() {
        // A bounded reorder (two adjacent commands swapped): positional pairing
        // dirties both old∪new unions — covered, never under-damaged.
        let prev = [rect_cmd(0, 0, 1), rect_cmd(32, 0, 2), rect_cmd(64, 0, 3)];
        let cur = [rect_cmd(0, 0, 1), rect_cmd(64, 0, 3), rect_cmd(32, 0, 2)];
        let shadow = captured(&prev);
        let (alignment, rects, ..) = diff(&shadow, &cur);
        assert_eq!(alignment, StaticRebuildAlignment::Bounded { region: 2 });
        // Each pair's union spans (32..80, 0..16).
        assert_eq!(rects.len(), 2);
        for rect in rects {
            assert_eq!(
                rect,
                Rect {
                    x: 32,
                    y: 0,
                    w: 48,
                    h: 16
                }
            );
        }
    }

    #[test]
    fn offscreen_change_pushes_no_rect_but_reports_bounded() {
        // A command fully off-screen changes: no visible damage, but the
        // alignment still reports the bounded region (content did change).
        let prev = [rect_cmd(0, 0, 1), rect_cmd(400, 400, 2)];
        let mut shadow = StaticLayerShadow::new();
        shadow.capture(&prev, SURF, SURF);
        let cur = [rect_cmd(0, 0, 1), rect_cmd(400, 400, 3)];
        let (alignment, rects, area, overflow) = diff(&shadow, &cur);
        assert_eq!(alignment, StaticRebuildAlignment::Bounded { region: 1 });
        assert!(rects.is_empty());
        assert_eq!(area, 0);
        assert!(!overflow);
    }

    #[test]
    fn region_at_window_cap_stays_bounded() {
        // Exactly `max_region` changed slots still diff (the bail is strictly
        // greater-than), matching the immediate diff's cap semantics.
        let prev: Vec<_> = (0..STATIC_DAMAGE_CAP + 4)
            .map(|i| rect_cmd((i as i32 % 10) * 16, (i as i32 / 10) * 16, 0x1000))
            .collect();
        let mut cur = prev.clone();
        for slot in cur.iter_mut().skip(2).take(STATIC_DAMAGE_CAP) {
            let AppDrawCommand::Rect { rgb565, .. } = slot else {
                unreachable!()
            };
            *rgb565 = 0x2000;
        }
        let shadow = captured(&prev);
        let (alignment, rects, ..) = diff(&shadow, &cur);
        assert_eq!(
            alignment,
            StaticRebuildAlignment::Bounded {
                region: STATIC_DAMAGE_CAP
            }
        );
        assert_eq!(rects.len(), STATIC_DAMAGE_CAP);
    }

    #[test]
    fn overflowing_dirty_buffer_flags_overflow() {
        // A caller buffer smaller than the damage set flags overflow through
        // push_dirty — the incomplete-set signal the present path fails safe on.
        let prev: Vec<_> = (0..8).map(|i| rect_cmd(i * 40, 0, 1)).collect();
        let cur: Vec<_> = (0..8).map(|i| rect_cmd(i * 40, 0, 2)).collect();
        let shadow = captured(&prev);
        let mut dirty = [Rect {
            x: 0,
            y: 0,
            w: 0,
            h: 0,
        }; 4];
        let (mut len, mut area, mut overflow) = (0usize, 0u32, false);
        let alignment = collect_static_rebuild_dirty(
            &shadow,
            &cur,
            SURF,
            SURF,
            STATIC_DAMAGE_CAP,
            &mut dirty,
            &mut len,
            &mut area,
            &mut overflow,
        );
        assert_eq!(alignment, StaticRebuildAlignment::Bounded { region: 8 });
        assert_eq!(len, 4);
        assert!(overflow);
    }

    #[test]
    fn text_fingerprint_ignores_buffer_padding() {
        // Two Text commands with equal live bytes but different padding must
        // fingerprint equal — padding is not content and never rasterizes.
        let mut padded = text_cmd(10, 10, "OK");
        if let AppDrawCommand::Text { bytes, .. } = &mut padded {
            bytes[10] = 0xAA;
        }
        assert_eq!(
            command_fingerprint(&padded),
            command_fingerprint(&text_cmd(10, 10, "OK"))
        );
        // And different live text must differ.
        assert_ne!(
            command_fingerprint(&text_cmd(10, 10, "OK")),
            command_fingerprint(&text_cmd(10, 10, "NG"))
        );
    }

    #[test]
    fn fingerprints_distinguish_variants_and_fields() {
        // Same geometry, different variant / colour / heap ref must all differ.
        let rect = rect_cmd(0, 0, 0x1234);
        let pixels = AppDrawCommand::Pixels {
            x: 0,
            y: 0,
            w: 16,
            h: 16,
            off: 0,
            len: 512,
        };
        let pixels_moved = AppDrawCommand::Pixels {
            x: 0,
            y: 0,
            w: 16,
            h: 16,
            off: 512,
            len: 512,
        };
        assert_ne!(command_fingerprint(&rect), command_fingerprint(&pixels));
        assert_ne!(
            command_fingerprint(&pixels),
            command_fingerprint(&pixels_moved)
        );
        assert_ne!(
            command_fingerprint(&rect_cmd(0, 0, 0x1234)),
            command_fingerprint(&rect_cmd(0, 0, 0x1235))
        );
        assert_ne!(
            command_fingerprint(&AppDrawCommand::Empty),
            command_fingerprint(&rect_cmd(0, 0, 0))
        );
    }

    #[test]
    fn capture_over_capacity_invalidates() {
        let too_many = vec![rect_cmd(0, 0, 1); GAME2D_STATIC_CMD_CAP + 1];
        let mut shadow = captured(&chrome_list());
        assert!(shadow.is_valid());
        shadow.capture(&too_many, SURF, SURF);
        assert!(!shadow.is_valid());
    }
}
