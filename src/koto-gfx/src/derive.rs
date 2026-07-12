//! Dirty-region derivation over the retained layer POD model.
//!
//! These functions were lifted verbatim from the Pico firmware's `app_render.rs`
//! (KotoGFX migration Stage 3, GFX-0003): the per-command / per-sprite / per-text
//! footprints, their old-vs-new dirty unions, the board-band rect, the stamp-cell
//! decoder, and the dirty-set accumulator. They are pure integer geometry over the
//! GFX-0002 POD types plus the app heap slice — no `Canvas`, no host state, no
//! timing, no transfer — so the present path's *collection* of dirty rects now
//! lives here alongside the policy that consumes it (`FullRepaintPolicy`,
//! `coalesce_rects`, `DirtyRectGeometry`).
//!
//! The surface is parameterised (`surf_w`, `surf_h`) exactly as the Stage 1
//! [`Rect::clip`]/[`Rect::union_clipped`] helpers are; the firmware passes its
//! fixed `320, 320` panel. The escalation thresholds and reason-code priority stay
//! in [`crate::FullRepaintPolicy`] — this module only *collects* dirty rects.

use crate::layer::{
    AppDrawCommand, Game2dSprite, Game2dStampDef, Game2dText, GAME2D_ORIGIN_X, GAME2D_ORIGIN_Y,
    GAME2D_TILE_PX,
};
use crate::{coalesce_rects, FullRepaintPolicy, Rect, TileBand};

/// Height in pixels of the row band a `Text` command / retained text item dirties.
/// v1 uses a fixed band from `x` to the right surface edge (no tight CJK metrics),
/// matching the immediate `draw_text` footprint for pixel parity (KOTO-0141).
const TEXT_BAND_PX: i32 = 17;

/// Append `rect` to the working dirty set, accumulating its summed area and
/// flagging overflow past the cap (KOTO-0159). Overflow forces a full repaint
/// (which repaints everything), so a rect not stored here is never visually lost.
pub fn push_dirty(
    dirty: &mut [Rect],
    len: &mut usize,
    area: &mut u32,
    overflow: &mut bool,
    rect: Rect,
) {
    *area = area.saturating_add(rect.w as u32 * rect.h as u32);
    if *len < dirty.len() {
        dirty[*len] = rect;
        *len += 1;
    } else {
        *overflow = true;
    }
}

/// Union of an old and a new footprint: both present → clipped bounding box; one
/// present → that one (an appear/disappear erases or paints its own rect); neither
/// → clean. The shared core of the `*_dirty_rect` functions (KOTO-0140/0141) and
/// the static-rebuild shadow diff (GFX-0013).
pub(crate) fn union_or_either(
    old: Option<Rect>,
    new: Option<Rect>,
    surf_w: i32,
    surf_h: i32,
) -> Option<Rect> {
    match (old, new) {
        (Some(a), Some(b)) => Rect::union_clipped(a, b, surf_w, surf_h),
        (Some(rect), None) | (None, Some(rect)) => Some(rect),
        (None, None) => None,
    }
}

/// The clipped row band a text string at `(x, y)` occupies: from `x` to the right
/// surface edge, `TEXT_BAND_PX` tall. Shared by `Text` commands and retained text.
fn text_band_rect(x: i32, y: i32, surf_w: i32, surf_h: i32) -> Option<Rect> {
    Rect::clip(x, y, surf_w - x.max(0), TEXT_BAND_PX, surf_w, surf_h)
}

/// On-surface footprint of one draw command (its clipped pixel rect), or `None`
/// if it is empty or fully off-screen. Shared with the static-rebuild shadow
/// (GFX-0013), which retains these rects per captured command.
pub(crate) fn command_rect(command: AppDrawCommand, surf_w: i32, surf_h: i32) -> Option<Rect> {
    match command {
        AppDrawCommand::Empty => None,
        AppDrawCommand::Rect { x, y, w, h, .. } => Rect::clip(x, y, w, h, surf_w, surf_h),
        AppDrawCommand::Pixels { x, y, w, h, .. } => Rect::clip(x, y, w, h, surf_w, surf_h),
        AppDrawCommand::Text { x, y, .. } => text_band_rect(x, y, surf_w, surf_h),
    }
}

/// Dirty rect for one immediate command slot across two frames: the union of its
/// old and new footprints, so a moved/changed command repaints both (KOTO-0128).
pub fn command_dirty_rect(
    old: AppDrawCommand,
    new: AppDrawCommand,
    surf_w: i32,
    surf_h: i32,
) -> Option<Rect> {
    union_or_either(
        command_rect(old, surf_w, surf_h),
        command_rect(new, surf_w, surf_h),
        surf_w,
        surf_h,
    )
}

/// Cap on the aligned immediate-diff edit region (GFX-0008). A length shift whose
/// changed span exceeds this is a genuinely wide restructure, not a localizable
/// single edit, so the present path full-repaints (attributed `CommandCountShift`,
/// the count having changed) rather than paying to diff the whole misaligned span.
/// Sized at the rect-escalation order of magnitude ([`crate::FULL_REPAINT_RECTS`]):
/// past this many changed commands the bounded diff cannot stay under the rect
/// threshold anyway, so escalating is the same decision the thresholds would reach.
pub const MAX_EDIT_REGION: usize = crate::FULL_REPAINT_RECTS as usize;

/// The prefix/suffix alignment shape of an immediate-command diff (GFX-0011 Stage 0).
///
/// This is the geometry [`collect_immediate_dirty`] computes internally to localize a
/// length shift; it is surfaced here (observe-only) so the `phase=169` diagnostic can
/// report *why* a `CommandCountShift` frame fell back — a short common prefix/suffix with
/// a wide `region` is a smoothly-shifting list (the whole body moved), not a localizable
/// single edit. `bailed` is the exact live-path decision: on a length shift whose `region`
/// exceeds `max_edit_region`, [`collect_immediate_dirty`] sets its `overflow` flag and
/// returns *without diffing the span*, which is why such a frame reports `rects_pre=0`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EditRegionShape {
    /// Longest common command prefix of the two frames.
    pub prefix_len: usize,
    /// Longest common command suffix (not overlapping the prefix).
    pub suffix_len: usize,
    /// Changed span on the previous side: `prev.len() - suffix - prefix`.
    pub edit_region_prev: usize,
    /// Changed span on the current side: `cur.len() - suffix - prefix`.
    pub edit_region_cur: usize,
    /// The wider of the two spans — the number of slots the edit-region diff walks.
    pub region: usize,
    /// The immediate command-list length changed this frame (the length-shift path).
    pub length_shifted: bool,
    /// The live path skipped the edit-region diff and escalated: a length shift whose
    /// `region > max_edit_region`. (Equal-length frames never consult the cap.)
    pub bailed: bool,
}

impl EditRegionShape {
    /// Compute the alignment shape of `prev` vs `cur` under `max_edit_region` (the live
    /// bail cap). Pure integer work over the two command slices — the same prefix/suffix
    /// scan [`collect_immediate_dirty`] runs, factored out so the live path and the
    /// diagnostic share one source of truth.
    pub fn of(prev: &[AppDrawCommand], cur: &[AppDrawCommand], max_edit_region: usize) -> Self {
        let prev_len = prev.len();
        let cur_len = cur.len();
        let max_common = prev_len.min(cur_len);
        let mut prefix = 0;
        while prefix < max_common && prev[prefix] == cur[prefix] {
            prefix += 1;
        }
        let mut suffix = 0;
        while suffix < max_common - prefix
            && prev[prev_len - 1 - suffix] == cur[cur_len - 1 - suffix]
        {
            suffix += 1;
        }
        let edit_region_prev = prev_len - suffix - prefix;
        let edit_region_cur = cur_len - suffix - prefix;
        let region = edit_region_prev.max(edit_region_cur);
        let length_shifted = prev_len != cur_len;
        Self {
            prefix_len: prefix,
            suffix_len: suffix,
            edit_region_prev,
            edit_region_cur,
            region,
            length_shifted,
            // The live path bails only on the length-shift branch; an equal-length frame
            // diffs every slot without consulting the cap.
            bailed: length_shifted && region > max_edit_region,
        }
    }
}

/// Observe-only coalesce-pressure probe for a `CommandCountShift` frame (GFX-0011 Stage 0).
///
/// The result of collecting the immediate diff's edit region into the *expanded* probe
/// buffer (no `MAX_EDIT_REGION` bail — only a buffer-capacity overflow) and batch-coalescing
/// it, so a hardware log can tell whether a wide count-shift frame *would* have stayed
/// incremental had it been collected before the decision (the GFX-0011 Stage 1 hypothesis),
/// or is genuinely truncated / wide-area. Measurement only — it changes no decision.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CommandShiftProbe {
    /// Edit-region union rects collected before coalescing (capped at the probe buffer).
    pub rects_pre: u16,
    /// Rects surviving [`coalesce_rects`] — the passes the frame would cost incrementally.
    pub rects_coalesced: u16,
    /// Summed area of the pre-coalesce rects (overlaps double-counted).
    pub area_pre: u32,
    /// Re-summed area of the coalesced survivors (grown to their bounding boxes).
    pub area_coalesced: u32,
    /// Bounding box over all pre-coalesce rects — far above `area_pre` reads as scattered.
    pub bbox_area: u32,
    /// The edit region overflowed the probe buffer: the collected set is truncated, so
    /// `would_incremental` is forced false (an incomplete set cannot be trusted). Far rarer
    /// than the live `MAX_EDIT_REGION` bail this probe measures against.
    pub probe_truncated: bool,
    /// The post-coalesce set would stay incremental (count ≤ rect threshold, area under the
    /// area threshold, and not truncated) — the Stage-1 rescue candidate signal.
    pub would_incremental: bool,
}

/// Collect the immediate diff's edit region into `dirty` **without** the live
/// `MAX_EDIT_REGION` bail, batch-coalesce it, and report whether it would have stayed
/// incremental (GFX-0011 Stage 0, observe-only).
///
/// This is the measurement the live path refuses to make: [`collect_immediate_dirty`]
/// escalates a wide count-shift frame *before* diffing the span (so it reports
/// `rects_pre=0`), which structurally excludes the frame from the GFX-0010 coalesce-before-
/// decide rescue. Here the same positional edit-region diff runs with the cap raised to the
/// buffer capacity, so the only truncation is a genuine probe-buffer overflow
/// (`probe_truncated`). The positional pairing over a shifted list is *pixel-correct*, only
/// wasteful (GFX-0008): each slot's union rect covers its old ∪ new footprint, so the
/// collected set covers the symmetric difference — coalescing it and recompositing would
/// reproduce the same pixels as the full repaint it might replace.
///
/// `dirty` is used as scratch (collected into, then coalesced in place); the caller passes
/// a buffer it does not otherwise need (e.g. the dead dirty set on the full-repaint branch).
/// `policy` supplies the thresholds `would_incremental` is judged against. This changes no
/// rendering decision — it only computes a [`CommandShiftProbe`] for the diagnostic.
pub fn probe_command_shift_coalesce(
    policy: FullRepaintPolicy,
    prev: &[AppDrawCommand],
    cur: &[AppDrawCommand],
    surf_w: i32,
    surf_h: i32,
    dirty: &mut [Rect],
    max_waste: u32,
) -> CommandShiftProbe {
    let (mut len, mut area, mut overflow) = (0usize, 0u32, false);
    // `usize::MAX` disables the edit-region bail, so `collect_immediate_dirty` diffs the
    // whole region and overflows only when the probe buffer fills — the honest truncation.
    collect_immediate_dirty(
        prev,
        cur,
        surf_w,
        surf_h,
        usize::MAX,
        dirty,
        &mut len,
        &mut area,
        &mut overflow,
    );
    let bbox_area = match dirty[..len].split_first() {
        None => 0,
        Some((first, rest)) => rest.iter().fold(*first, |acc, r| acc.bbox(*r)).area(),
    };
    let coalesced_len = coalesce_rects(dirty, len, max_waste);
    let area_coalesced: u32 = dirty[..coalesced_len].iter().map(|r| r.area()).sum();
    // Incremental iff the coalesced set is within both thresholds (matching the strict
    // `> rect_threshold` / `>= area_threshold` escalation in `FullRepaintPolicy::decide`)
    // and the raw set was fully collected.
    let would_incremental = !overflow
        && coalesced_len as u32 <= policy.rect_threshold
        && area_coalesced < policy.area_threshold;
    CommandShiftProbe {
        rects_pre: len as u16,
        rects_coalesced: coalesced_len as u16,
        area_pre: area,
        area_coalesced,
        bbox_area,
        probe_truncated: overflow,
        would_incremental,
    }
}

/// True if `command` fills (at least) the whole surface — the full-screen page-clear
/// base. The present path handles the base via its clear-to-base step (KOTO-0128),
/// so a base fill never feeds a dirty rect into the command diff. Moved here from the
/// firmware with the surface parameterised (GFX-0008); the `<= 0` / `>=` bounds make
/// an over-large or negatively-offset clear still count as the base.
pub fn is_full_screen_base(command: AppDrawCommand, surf_w: i32, surf_h: i32) -> bool {
    matches!(
        command,
        AppDrawCommand::Rect { x, y, w, h, .. }
            if x <= 0 && y <= 0 && x.saturating_add(w) >= surf_w && y.saturating_add(h) >= surf_h
    )
}

/// Diff one immediate-command slot pair into the working dirty set: skip unchanged
/// pairs and full-screen-base fills (handled by the clear-to-base), else push the
/// union of the old and new footprints (KOTO-0128/0131). Shared by the equal-length
/// and edit-region passes of [`collect_immediate_dirty`].
#[allow(clippy::too_many_arguments)]
fn diff_command_pair(
    old: AppDrawCommand,
    new: AppDrawCommand,
    surf_w: i32,
    surf_h: i32,
    dirty: &mut [Rect],
    len: &mut usize,
    area: &mut u32,
    overflow: &mut bool,
) {
    if old == new
        || is_full_screen_base(old, surf_w, surf_h)
        || is_full_screen_base(new, surf_w, surf_h)
    {
        return;
    }
    if let Some(rect) = command_dirty_rect(old, new, surf_w, surf_h) {
        push_dirty(dirty, len, area, overflow, rect);
    }
}

/// Collect the immediate command list's dirty rects into the working set (GFX-0008).
///
/// `prev` / `cur` are the two frames' immediate command lists already sliced to their
/// logical length (so a slot past `len` is simply absent, replacing the firmware's
/// `command_at` Empty-padding — KOTO-0141). Each changed/moved/disappeared command
/// dirties the union of its old and new footprints.
///
/// When the list length is unchanged the diff is positional (`prev[i]` vs `cur[i]`),
/// byte-identical to the pre-GFX-0008 walk. When the length **shifts**, a positional
/// walk would misalign every slot after the inserted/removed command and balloon the
/// dirty set into spurious near-arbitrary union rects — the false-positive
/// `CommandCountShift` full repaints. Instead the diff anchors on the longest common
/// **prefix** and **suffix** and diffs only the bounded **edit region** between them,
/// so a single insert/remove localizes to its own footprint. Commands outside the
/// region are byte-identical in the same relative paint order, so they contribute no
/// damage; every changed immediate pixel is covered by an edit-region footprint.
///
/// A region wider than `max_edit_region` is a genuinely wide restructure, not a
/// localizable edit: it flags `overflow` (the rect-count escalation signal) and
/// returns without diffing the span, so [`crate::FullRepaintPolicy`] full-repaints it
/// exactly as the pre-GFX-0008 misaligned diff did — `CommandCountShift`, since the
/// caller still reports the count change — without paying for the wide walk.
#[allow(clippy::too_many_arguments)]
pub fn collect_immediate_dirty(
    prev: &[AppDrawCommand],
    cur: &[AppDrawCommand],
    surf_w: i32,
    surf_h: i32,
    max_edit_region: usize,
    dirty: &mut [Rect],
    len: &mut usize,
    area: &mut u32,
    overflow: &mut bool,
) {
    let prev_len = prev.len();
    let cur_len = cur.len();
    // Equal-length lists never misalign: slot i pairs with slot i. Diff every slot,
    // exactly as the pre-GFX-0008 positional walk did (no edit-region cap — an
    // equal-length frame is not a count shift, so the area/rect thresholds, not this
    // cap, own its escalation and its `AreaExceeded`/`RectsExceeded` attribution).
    if prev_len == cur_len {
        for i in 0..prev_len {
            diff_command_pair(prev[i], cur[i], surf_w, surf_h, dirty, len, area, overflow);
        }
        return;
    }
    // Length shifted: anchor on the common prefix and suffix so a single insert or
    // remove collapses to its own slot instead of misaligning the tail. The alignment
    // geometry is computed by `EditRegionShape::of` (shared with the GFX-0011 diagnostic).
    let shape = EditRegionShape::of(prev, cur, max_edit_region);
    if shape.bailed {
        // Wide restructure — escalate via the existing rect-overflow signal rather
        // than diffing the whole misaligned span (the work the cap exists to avoid).
        *overflow = true;
        return;
    }
    let prefix = shape.prefix_len;
    let prev_end = prefix + shape.edit_region_prev;
    let cur_end = prefix + shape.edit_region_cur;
    // Diff the bounded edit region by positional pairing, treating the shorter side's
    // missing slots as `Empty` (an appear/disappear erases/paints its own footprint)
    // — the `command_at` out-of-range semantics, confined to the region.
    for k in 0..shape.region {
        let old = if prefix + k < prev_end {
            prev[prefix + k]
        } else {
            AppDrawCommand::Empty
        };
        let new = if prefix + k < cur_end {
            cur[prefix + k]
        } else {
            AppDrawCommand::Empty
        };
        diff_command_pair(old, new, surf_w, surf_h, dirty, len, area, overflow);
    }
}

/// Pixel rect of a coalesced board band on the app surface, or `None` if it falls
/// fully off-screen (KOTO-0135/0143). The band is in cell units; scale by the tile
/// size and offset by the board origin, then clip.
pub fn board_band_rect(band: TileBand, surf_w: i32, surf_h: i32) -> Option<Rect> {
    Rect::clip(
        GAME2D_ORIGIN_X + band.col as i32 * GAME2D_TILE_PX,
        GAME2D_ORIGIN_Y + band.row as i32 * GAME2D_TILE_PX,
        band.w as i32 * GAME2D_TILE_PX,
        band.h as i32 * GAME2D_TILE_PX,
        surf_w,
        surf_h,
    )
}

/// Resolve cell `index` of a `format 0` stamp to its `(dcol, drow)` offset. Each
/// cell is one nibble at heap byte `cells_off + index/2` (low nibble for even
/// `index`, high for odd), encoding `nibble = drow*4 + dcol` — the KOTO-0138 cell
/// layout. Returns `None` if the byte lies outside the heap (KOTO-0140).
pub fn stamp_cell(heap: &[u8], cells_off: u32, index: usize) -> Option<(i32, i32)> {
    let byte = *heap.get(cells_off as usize + (index >> 1))?;
    let nibble = if index & 1 == 0 {
        byte & 0x0f
    } else {
        byte >> 4
    };
    Some(((nibble & 3) as i32, (nibble >> 2) as i32))
}

/// Bounding rect of a visible sprite's whole footprint on the app surface, or
/// `None` if it is hidden, undefined, or fully off-screen (KOTO-0140). The cell
/// offsets come from `stamps[sprite.stamp_id]` resolved against the app heap.
pub fn sprite_footprint_rect(
    sprite: &Game2dSprite,
    stamps: &[Game2dStampDef],
    heap: &[u8],
    surf_w: i32,
    surf_h: i32,
) -> Option<Rect> {
    if !sprite.visible {
        return None;
    }
    let stamp = stamps.get(sprite.stamp_id as usize)?;
    if stamp.count == 0 {
        return None;
    }
    let (mut min_col, mut min_row, mut max_col, mut max_row) = (i32::MAX, i32::MAX, 0, 0);
    for cell in 0..stamp.count as usize {
        let Some((dcol, drow)) = stamp_cell(heap, stamp.cells_off, cell) else {
            continue;
        };
        min_col = min_col.min(dcol);
        min_row = min_row.min(drow);
        max_col = max_col.max(dcol);
        max_row = max_row.max(drow);
    }
    if min_col == i32::MAX {
        return None;
    }
    let x = sprite.x as i32 + min_col * GAME2D_TILE_PX;
    let y = sprite.y as i32 + min_row * GAME2D_TILE_PX;
    let w = (max_col - min_col + 1) * GAME2D_TILE_PX;
    let h = (max_row - min_row + 1) * GAME2D_TILE_PX;
    Rect::clip(x, y, w, h, surf_w, surf_h)
}

/// Dirty rect for one sprite slot: the union of its old (previous-frame) and new
/// (current-frame) footprints, so a moving instance repaints both the cells it
/// left and the cells it entered (KOTO-0140). Each footprint resolves against the
/// stamp table of its own frame.
#[allow(clippy::too_many_arguments)]
pub fn sprite_dirty_rect(
    prev_sprite: &Game2dSprite,
    prev_stamps: &[Game2dStampDef],
    cur_sprite: &Game2dSprite,
    cur_stamps: &[Game2dStampDef],
    heap: &[u8],
    surf_w: i32,
    surf_h: i32,
) -> Option<Rect> {
    let old = sprite_footprint_rect(prev_sprite, prev_stamps, heap, surf_w, surf_h);
    let new = sprite_footprint_rect(cur_sprite, cur_stamps, heap, surf_w, surf_h);
    union_or_either(old, new, surf_w, surf_h)
}

/// Footprint rect of a visible retained text item on the app surface, or `None` if
/// it is hidden or fully off-screen (KOTO-0141). v1 uses the same from-`x`,
/// `TEXT_BAND_PX`-tall row band the immediate `Text` command uses, so a migrated
/// value repaints exactly the region the old `draw_text` did — pixel parity.
pub fn text_footprint_rect(item: &Game2dText, surf_w: i32, surf_h: i32) -> Option<Rect> {
    if !item.visible {
        return None;
    }
    text_band_rect(item.x as i32, item.y as i32, surf_w, surf_h)
}

/// Dirty rect for one text slot: the union of its old (previous-frame) and new
/// (current-frame) footprints, so a value that moves or shrinks repaints both the
/// row it left and the row it entered (KOTO-0141).
pub fn text_dirty_rect(
    prev_item: &Game2dText,
    cur_item: &Game2dText,
    surf_w: i32,
    surf_h: i32,
) -> Option<Rect> {
    union_or_either(
        text_footprint_rect(prev_item, surf_w, surf_h),
        text_footprint_rect(cur_item, surf_w, surf_h),
        surf_w,
        surf_h,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::GAME2D_TEXT_BYTES;

    const SURF: i32 = 320;

    // A 1x1 stamp at cell (0,0): one nibble `0` at heap byte `cells_off`.
    fn single_cell_heap() -> [u8; 4] {
        [0x00, 0, 0, 0]
    }

    fn stamp(cells_off: u32, count: u8) -> Game2dStampDef {
        Game2dStampDef { cells_off, count }
    }

    fn sprite(stamp_id: u8, x: i16, y: i16, visible: bool) -> Game2dSprite {
        Game2dSprite {
            stamp_id,
            x,
            y,
            tile_ref: 0,
            visible,
        }
    }

    fn text(x: i16, y: i16, visible: bool) -> Game2dText {
        Game2dText {
            x,
            y,
            rgb565: 0xFFFF,
            bytes: [b'9'; GAME2D_TEXT_BYTES],
            len: 3,
            visible,
        }
    }

    // Reference re-implementations of the firmware originals (hardcoded 320), to
    // prove the parameterised forms equal the pre-move behaviour on a grid sweep.
    fn ref_clip(x: i32, y: i32, w: i32, h: i32) -> Option<Rect> {
        let x0 = x.clamp(0, 320);
        let y0 = y.clamp(0, 320);
        let x1 = x.saturating_add(w).clamp(0, 320);
        let y1 = y.saturating_add(h).clamp(0, 320);
        (x1 > x0 && y1 > y0).then_some(Rect {
            x: x0,
            y: y0,
            w: x1 - x0,
            h: y1 - y0,
        })
    }

    fn ref_text_band(x: i32, y: i32) -> Option<Rect> {
        ref_clip(x, y, 320 - x.max(0), 17)
    }

    #[test]
    fn push_dirty_accumulates_then_overflows() {
        let mut dirty = [Rect {
            x: 0,
            y: 0,
            w: 0,
            h: 0,
        }; 2];
        let (mut len, mut area, mut overflow) = (0usize, 0u32, false);
        push_dirty(
            &mut dirty,
            &mut len,
            &mut area,
            &mut overflow,
            Rect {
                x: 0,
                y: 0,
                w: 4,
                h: 5,
            },
        );
        push_dirty(
            &mut dirty,
            &mut len,
            &mut area,
            &mut overflow,
            Rect {
                x: 0,
                y: 0,
                w: 2,
                h: 3,
            },
        );
        assert_eq!((len, area, overflow), (2, 26, false));
        // Third rect past the cap flags overflow but still sums area.
        push_dirty(
            &mut dirty,
            &mut len,
            &mut area,
            &mut overflow,
            Rect {
                x: 0,
                y: 0,
                w: 1,
                h: 1,
            },
        );
        assert_eq!((len, overflow), (2, true));
        assert_eq!(area, 27);
    }

    #[test]
    fn command_rect_matches_reference_over_a_grid() {
        for x in [-40, -1, 0, 10, 300, 319, 320, 400] {
            for w in [0, 1, 16, 64, 400] {
                let rect = AppDrawCommand::Rect {
                    x,
                    y: 30,
                    w,
                    h: 20,
                    rgb565: 0,
                };
                assert_eq!(command_rect(rect, SURF, SURF), ref_clip(x, 30, w, 20));
                let px = AppDrawCommand::Pixels {
                    x,
                    y: 30,
                    w,
                    h: 20,
                    off: 0,
                    len: 0,
                };
                assert_eq!(command_rect(px, SURF, SURF), ref_clip(x, 30, w, 20));
                let txt = AppDrawCommand::Text {
                    x,
                    y: 30,
                    rgb565: 0,
                    bytes: [0; 64],
                    len: 0,
                };
                assert_eq!(command_rect(txt, SURF, SURF), ref_text_band(x, 30));
            }
        }
        assert_eq!(command_rect(AppDrawCommand::Empty, SURF, SURF), None);
    }

    #[test]
    fn command_dirty_rect_unions_old_and_new() {
        let a = AppDrawCommand::Rect {
            x: 0,
            y: 0,
            w: 16,
            h: 16,
            rgb565: 0,
        };
        let b = AppDrawCommand::Rect {
            x: 40,
            y: 40,
            w: 16,
            h: 16,
            rgb565: 0,
        };
        assert_eq!(
            command_dirty_rect(a, b, SURF, SURF),
            Some(Rect {
                x: 0,
                y: 0,
                w: 56,
                h: 56
            }),
        );
        // Disappearing command erases its old footprint.
        assert_eq!(
            command_dirty_rect(a, AppDrawCommand::Empty, SURF, SURF),
            Some(Rect {
                x: 0,
                y: 0,
                w: 16,
                h: 16
            }),
        );
        assert_eq!(
            command_dirty_rect(AppDrawCommand::Empty, AppDrawCommand::Empty, SURF, SURF),
            None,
        );
    }

    #[test]
    fn board_band_rect_scales_and_offsets() {
        let band = TileBand {
            col: 1,
            row: 2,
            w: 3,
            h: 4,
        };
        // origin (8,0) + (16,32), size (48,64).
        assert_eq!(
            board_band_rect(band, SURF, SURF),
            Some(Rect {
                x: 24,
                y: 32,
                w: 48,
                h: 64
            }),
        );
        // A band entirely past the bottom edge clips away.
        let off = TileBand {
            col: 0,
            row: 40,
            w: 1,
            h: 1,
        };
        assert_eq!(board_band_rect(off, SURF, SURF), None);
    }

    #[test]
    fn sprite_footprint_handles_visibility_and_clipping() {
        let heap = single_cell_heap();
        let stamps = [stamp(0, 1)];
        // Visible 1x1 sprite at (32, 48): one 16x16 tile.
        assert_eq!(
            sprite_footprint_rect(&sprite(0, 32, 48, true), &stamps, &heap, SURF, SURF),
            Some(Rect {
                x: 32,
                y: 48,
                w: 16,
                h: 16
            }),
        );
        // Hidden, undefined-stamp, and off-screen all yield None.
        assert_eq!(
            sprite_footprint_rect(&sprite(0, 32, 48, false), &stamps, &heap, SURF, SURF),
            None,
        );
        let empty_stamps = [stamp(0, 0)];
        assert_eq!(
            sprite_footprint_rect(&sprite(0, 32, 48, true), &empty_stamps, &heap, SURF, SURF),
            None,
        );
        assert_eq!(
            sprite_footprint_rect(&sprite(0, 400, 48, true), &stamps, &heap, SURF, SURF),
            None,
        );
    }

    #[test]
    fn sprite_dirty_rect_unions_move() {
        let heap = single_cell_heap();
        let stamps = [stamp(0, 1)];
        let moved = sprite_dirty_rect(
            &sprite(0, 0, 0, true),
            &stamps,
            &sprite(0, 32, 0, true),
            &stamps,
            &heap,
            SURF,
            SURF,
        );
        assert_eq!(
            moved,
            Some(Rect {
                x: 0,
                y: 0,
                w: 48,
                h: 16
            })
        );
        // Appearing sprite: only the new footprint.
        let appeared = sprite_dirty_rect(
            &sprite(0, 0, 0, false),
            &stamps,
            &sprite(0, 32, 0, true),
            &stamps,
            &heap,
            SURF,
            SURF,
        );
        assert_eq!(
            appeared,
            Some(Rect {
                x: 32,
                y: 0,
                w: 16,
                h: 16
            })
        );
    }

    #[test]
    fn text_footprint_and_dirty_match_reference() {
        for x in [-8, 0, 100, 319, 320] {
            let item = text(x as i16, 50, true);
            assert_eq!(text_footprint_rect(&item, SURF, SURF), ref_text_band(x, 50),);
        }
        assert_eq!(text_footprint_rect(&text(10, 50, false), SURF, SURF), None);
        // A text value moving down one row dirties both rows.
        let dirty = text_dirty_rect(&text(0, 0, true), &text(0, 17, true), SURF, SURF);
        assert_eq!(
            dirty,
            Some(Rect {
                x: 0,
                y: 0,
                w: 320,
                h: 34
            })
        );
    }

    #[test]
    fn parameterised_surface_clips_tighter_than_320() {
        // On a 64x64 surface a sprite near the edge clips to the smaller bounds.
        let heap = single_cell_heap();
        let stamps = [stamp(0, 1)];
        assert_eq!(
            sprite_footprint_rect(&sprite(0, 56, 56, true), &stamps, &heap, 64, 64),
            Some(Rect {
                x: 56,
                y: 56,
                w: 8,
                h: 8
            }),
        );
    }

    // ---- GFX-0008 aligned immediate diff -----------------------------------

    fn rect_cmd(x: i32, y: i32) -> AppDrawCommand {
        AppDrawCommand::Rect {
            x,
            y,
            w: 16,
            h: 16,
            rgb565: (x + y) as u16 | 1,
        }
    }

    /// Run `collect_immediate_dirty` at the fixed surface with the production cap and
    /// return the collected dirty rects plus the overflow (wide-shift) flag.
    fn collect(prev: &[AppDrawCommand], cur: &[AppDrawCommand]) -> (Vec<Rect>, bool) {
        collect_with_cap(prev, cur, MAX_EDIT_REGION)
    }

    fn collect_with_cap(
        prev: &[AppDrawCommand],
        cur: &[AppDrawCommand],
        cap: usize,
    ) -> (Vec<Rect>, bool) {
        // Working set sized well past the cap so a bounded edit never overflows for
        // capacity reasons — only a wide region trips `overflow`.
        let mut dirty = [Rect {
            x: 0,
            y: 0,
            w: 0,
            h: 0,
        }; 256];
        let (mut len, mut area, mut overflow) = (0usize, 0u32, false);
        collect_immediate_dirty(
            prev,
            cur,
            SURF,
            SURF,
            cap,
            &mut dirty,
            &mut len,
            &mut area,
            &mut overflow,
        );
        (dirty[..len].to_vec(), overflow)
    }

    /// Reference positional diff (the pre-GFX-0008 walk over `0..len.max(len)` with
    /// out-of-range slots read as `Empty`), to prove the equal-length path is
    /// byte-identical and to contrast the misaligned tail on a length shift.
    fn ref_positional(prev: &[AppDrawCommand], cur: &[AppDrawCommand]) -> Vec<Rect> {
        let at = |list: &[AppDrawCommand], i: usize| {
            list.get(i).copied().unwrap_or(AppDrawCommand::Empty)
        };
        let mut rects = Vec::new();
        for i in 0..prev.len().max(cur.len()) {
            let (old, new) = (at(prev, i), at(cur, i));
            if old == new
                || is_full_screen_base(old, SURF, SURF)
                || is_full_screen_base(new, SURF, SURF)
            {
                continue;
            }
            if let Some(rect) = command_dirty_rect(old, new, SURF, SURF) {
                rects.push(rect);
            }
        }
        rects
    }

    #[test]
    fn equal_length_identical_lists_are_clean() {
        // No-op frame: equal length, identical content ⇒ no dirty, no overflow
        // (the alignment must not invent damage).
        let list = [rect_cmd(0, 0), rect_cmd(40, 40), rect_cmd(80, 80)];
        assert_eq!(collect(&list, &list), (Vec::new(), false));
    }

    #[test]
    fn equal_length_matches_positional_walk() {
        // Equal-length lists never misalign, so the aligned diff is byte-identical to
        // the pre-GFX-0008 positional walk — including a changed middle slot.
        let prev = [rect_cmd(0, 0), rect_cmd(40, 40), rect_cmd(80, 80)];
        let cur = [rect_cmd(0, 0), rect_cmd(50, 40), rect_cmd(80, 80)];
        let (rects, overflow) = collect(&prev, &cur);
        assert!(!overflow);
        assert_eq!(rects, ref_positional(&prev, &cur));
        // And it is exactly that one slot's old∪new footprint.
        assert_eq!(
            rects,
            vec![Rect {
                x: 40,
                y: 40,
                w: 26,
                h: 16
            }]
        );
    }

    #[test]
    fn single_removal_at_head_stays_bounded() {
        // [A,B,C,…] (29) → [B,C,…] (28): the positional walk would misalign all 28
        // surviving slots; the aligned diff dirties only A's footprint.
        let prev: Vec<_> = (0..29).map(|i| rect_cmd((i % 20) * 16, i)).collect();
        let cur: Vec<_> = prev[1..].to_vec();
        let (rects, overflow) = collect(&prev, &cur);
        assert!(!overflow);
        assert_eq!(
            rects,
            vec![Rect {
                x: 0,
                y: 0,
                w: 16,
                h: 16
            }]
        );
        // The misaligned positional walk would have produced far more rects.
        assert!(ref_positional(&prev, &cur).len() > rects.len());
    }

    #[test]
    fn single_insertion_mid_list_stays_bounded() {
        // Insert one command in the middle of a 42-long list (42 → 43). Only the
        // inserted command's footprint is dirtied; the shifted tail is byte-identical.
        let prev: Vec<_> = (0..42).map(|i| rect_cmd((i % 18) * 16, i)).collect();
        let inserted = AppDrawCommand::Rect {
            x: 100,
            y: 200,
            w: 16,
            h: 16,
            rgb565: 0xBEEF,
        };
        let mut cur = prev.clone();
        cur.insert(21, inserted);
        let (rects, overflow) = collect(&prev, &cur);
        assert!(!overflow);
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
    fn wide_edit_region_overflows_for_full_repaint() {
        // A length shift whose changed span exceeds the cap is a wide restructure: it
        // flags overflow (the rect-count escalation the policy turns into a
        // CommandCountShift full repaint) without diffing the span.
        let prev: Vec<_> = (0..40).map(|i| rect_cmd((i % 16) * 16, i)).collect();
        // Replace the whole middle and shift length: short common prefix/suffix, so
        // the region is the bulk of the list, well over MAX_EDIT_REGION.
        let mut cur: Vec<_> = (0..39)
            .map(|i| rect_cmd((i % 16) * 16 + 1, i + 1))
            .collect();
        cur[0] = prev[0]; // tiny shared prefix
        let (rects, overflow) = collect(&prev, &cur);
        assert!(overflow);
        // The wide span is not diffed — no rects are pushed when it bails.
        assert!(rects.is_empty());
    }

    #[test]
    fn edit_region_at_cap_stays_bounded() {
        // A region of exactly `cap` slots is still bounded (the bail is strictly
        // greater than the cap), so it diffs rather than overflowing. Shared prefix
        // (slot 0) and suffix (the trailing `z`) leave a 3-wide edit region between a
        // 5-long and a 4-long list.
        let cap = 3;
        let z = rect_cmd(200, 0);
        let prev = [
            rect_cmd(0, 0),
            rect_cmd(16, 0),
            rect_cmd(32, 0),
            rect_cmd(48, 0),
            z,
        ];
        let cur = [rect_cmd(0, 0), rect_cmd(17, 0), rect_cmd(33, 0), z];
        let (rects, overflow) = collect_with_cap(&prev, &cur, cap);
        assert!(!overflow);
        assert!(!rects.is_empty());
    }

    #[test]
    fn full_screen_base_in_edit_region_is_skipped() {
        // A full-screen base appearing in the shifted region never contributes a dirty
        // rect (the clear-to-base owns it); only the real command moves are collected.
        let base = AppDrawCommand::Rect {
            x: 0,
            y: 0,
            w: 320,
            h: 320,
            rgb565: 0x0001,
        };
        let prev = [rect_cmd(10, 10)];
        let cur = [base, rect_cmd(10, 10)];
        let (rects, overflow) = collect(&prev, &cur);
        assert!(!overflow);
        assert!(rects.is_empty());
    }

    // ---- GFX-0011 Stage 0 edit-region diagnostics --------------------------

    #[test]
    fn edit_region_shape_bounded_single_removal() {
        // [A,B,C,…] (29) → [B,C,…] (28): the removed head is the whole edit region, so
        // the shape reports a large common suffix, region 1, and does not bail.
        let prev: Vec<_> = (0..29).map(|i| rect_cmd((i % 20) * 16, i)).collect();
        let cur: Vec<_> = prev[1..].to_vec();
        let shape = EditRegionShape::of(&prev, &cur, MAX_EDIT_REGION);
        assert!(shape.length_shifted);
        assert!(!shape.bailed);
        assert_eq!(shape.prefix_len, 0);
        assert_eq!(shape.suffix_len, 28);
        assert_eq!((shape.edit_region_prev, shape.edit_region_cur), (1, 0));
        assert_eq!(shape.region, 1);
    }

    #[test]
    fn edit_region_shape_wide_shift_bails() {
        // A whole-list shift (near-zero common prefix/suffix, count changed by one): the
        // region spans almost the entire list, so the live path bails (`dirty_skipped`).
        let prev: Vec<_> = (0..36).map(|i| rect_cmd((i % 18) * 16, i)).collect();
        // Shift every element's content and drop one: no stable prefix/suffix.
        let cur: Vec<_> = (0..35)
            .map(|i| rect_cmd((i % 18) * 16 + 1, i + 1))
            .collect();
        let shape = EditRegionShape::of(&prev, &cur, MAX_EDIT_REGION);
        assert!(shape.length_shifted);
        assert!(
            shape.bailed,
            "a wide count shift must bail (the fallback under study)"
        );
        assert_eq!(shape.prefix_len, 0);
        assert_eq!(shape.suffix_len, 0);
        assert!(shape.region > MAX_EDIT_REGION);
    }

    #[test]
    fn edit_region_shape_matches_live_bail() {
        // The shape's `bailed` bit is exactly the live path's overflow escalation: whenever
        // `collect_immediate_dirty` sets overflow with no rect collected, the shape bailed.
        let prev: Vec<_> = (0..40).map(|i| rect_cmd((i % 16) * 16, i)).collect();
        let mut cur: Vec<_> = (0..39)
            .map(|i| rect_cmd((i % 16) * 16 + 1, i + 1))
            .collect();
        cur[0] = prev[0]; // tiny shared prefix
        let (rects, overflow) = collect(&prev, &cur);
        let shape = EditRegionShape::of(&prev, &cur, MAX_EDIT_REGION);
        assert_eq!(overflow, shape.bailed);
        assert!(shape.bailed);
        assert!(
            rects.is_empty(),
            "the bail skips the diff — hence rects_pre=0"
        );
    }

    /// The observe-only probe collects the *wide* edit region the live path skipped and
    /// coalesces it: a smoothly-shifting contiguous list collapses to a handful of bands
    /// and would stay incremental — the Stage-1 rescue candidate.
    #[test]
    fn probe_collects_wide_region_and_coalesces_incremental() {
        // 30 contiguous tiles laid out 15-per-row across two rows (on-screen); the current
        // frame shifts them all down and drops one (a count shift), so every slot differs
        // (no common prefix or suffix) and the live path bails. The probe collects ~30
        // overlapping vertical strips that coalesce into one band.
        let prev: Vec<_> = (0..30)
            .map(|i| rect_cmd((i % 15) * 16, (i / 15) * 16))
            .collect();
        let cur: Vec<_> = (0..29)
            .map(|i| rect_cmd((i % 15) * 16, (i / 15) * 16 + 32))
            .collect();
        // The live path must have bailed (nothing collected).
        let (live_rects, live_overflow) = collect(&prev, &cur);
        assert!(live_overflow);
        assert!(live_rects.is_empty());
        // The probe collects the region and coalesces it under threshold.
        let mut scratch = [Rect {
            x: 0,
            y: 0,
            w: 0,
            h: 0,
        }; 84];
        let probe = probe_command_shift_coalesce(
            FullRepaintPolicy::default(),
            &prev,
            &cur,
            SURF,
            SURF,
            &mut scratch,
            (16 * 16 * 4) as u32,
        );
        assert!(!probe.probe_truncated);
        assert!(
            probe.rects_pre > MAX_EDIT_REGION as u16,
            "the wide region is collected"
        );
        assert!(probe.rects_coalesced <= crate::FULL_REPAINT_RECTS as u16);
        assert!(probe.would_incremental, "a contiguous shift is coalescible");
    }

    /// An irreducible scatter over the edit region stays over threshold after coalescing:
    /// the probe reports `would_incremental=false`, so it does not mislabel a genuinely
    /// fragmented frame as rescuable.
    #[test]
    fn probe_scattered_region_not_incremental() {
        // 30 far-apart single tiles, shifted (count change). No two merge at waste 0.
        let prev: Vec<_> = (0..30)
            .map(|i| rect_cmd((i % 5) * 48, (i / 5) * 48))
            .collect();
        let cur: Vec<_> = (0..29)
            .map(|i| rect_cmd((i % 5) * 48 + 1, (i / 5) * 48 + 1))
            .collect();
        let mut scratch = [Rect {
            x: 0,
            y: 0,
            w: 0,
            h: 0,
        }; 84];
        let probe = probe_command_shift_coalesce(
            FullRepaintPolicy::default(),
            &prev,
            &cur,
            SURF,
            SURF,
            &mut scratch,
            0,
        );
        assert!(!probe.probe_truncated);
        assert!(probe.rects_coalesced > crate::FULL_REPAINT_RECTS as u16);
        assert!(!probe.would_incremental);
    }

    /// A region larger than the probe buffer overflows: `probe_truncated=true` forces
    /// `would_incremental=false` even if the captured prefix coalesces small.
    #[test]
    fn probe_truncates_past_buffer_capacity() {
        let prev: Vec<_> = (0..40)
            .map(|i| rect_cmd((i % 20) * 16, (i / 20) * 16))
            .collect();
        let cur: Vec<_> = (0..39)
            .map(|i| rect_cmd((i % 20) * 16 + 3, (i / 20) * 16 + 3))
            .collect();
        // A buffer smaller than the region forces truncation.
        let mut scratch = [Rect {
            x: 0,
            y: 0,
            w: 0,
            h: 0,
        }; 8];
        let probe = probe_command_shift_coalesce(
            FullRepaintPolicy::default(),
            &prev,
            &cur,
            SURF,
            SURF,
            &mut scratch,
            (16 * 16 * 4) as u32,
        );
        assert!(probe.probe_truncated);
        assert!(!probe.would_incremental);
    }

    /// The probe is observe-only: running it does not depend on, and cannot change, the
    /// live `collect_immediate_dirty` decision — the same frame still bails identically.
    #[test]
    fn probe_does_not_change_live_decision() {
        let prev: Vec<_> = (0..30)
            .map(|i| rect_cmd((i % 15) * 16, (i / 15) * 16))
            .collect();
        let cur: Vec<_> = (0..29)
            .map(|i| rect_cmd((i % 15) * 16, (i / 15) * 16 + 32))
            .collect();
        let before = collect(&prev, &cur);
        let mut scratch = [Rect {
            x: 0,
            y: 0,
            w: 0,
            h: 0,
        }; 84];
        let _ = probe_command_shift_coalesce(
            FullRepaintPolicy::default(),
            &prev,
            &cur,
            SURF,
            SURF,
            &mut scratch,
            (16 * 16 * 4) as u32,
        );
        let after = collect(&prev, &cur);
        assert_eq!(before, after);
    }

    // ---- GFX-0011 Stage 1 collect-wide-region-then-decide ------------------

    use crate::{
        coalesce_then_decide, decision_snapshot, CoalesceDecision, DeltaDecision,
        FullRepaintReason, FULL_REPAINT_AREA, FULL_REPAINT_RECTS,
    };

    /// The firmware's structural probe cap (`DIRTY_RECT_PROBE_CAP`) and the pre-coalesce
    /// decision cap (`DIRTY_RECT_CAP` = `FULL_REPAINT_RECTS + 1`), reproduced here to drive the
    /// Stage-1 present sequence exactly as `present_app_delta` does.
    const STAGE1_PROBE_CAP: usize = 84;
    const STAGE1_DECISION_CAP: usize = MAX_EDIT_REGION + 1;

    /// Collect the immediate diff with the Stage-1 *expanded* cap (as `present_app_delta` now
    /// does), then run the GFX-0010 coalesce-before-decide path over it — the whole live
    /// sequence for a command-only frame (no board/sprite/text layers push here).
    fn collect_wide_then_decide(
        prev: &[AppDrawCommand],
        cur: &[AppDrawCommand],
        max_waste: u32,
    ) -> CoalesceDecision {
        let mut dirty = [Rect {
            x: 0,
            y: 0,
            w: 0,
            h: 0,
        }; STAGE1_PROBE_CAP];
        let (mut probe_len, mut area, mut overflow) = (0usize, 0u32, false);
        collect_immediate_dirty(
            prev,
            cur,
            SURF,
            SURF,
            STAGE1_PROBE_CAP,
            &mut dirty,
            &mut probe_len,
            &mut area,
            &mut overflow,
        );
        let (decision_len, decision_overflow) =
            decision_snapshot(probe_len, overflow, STAGE1_DECISION_CAP);
        coalesce_then_decide(
            FullRepaintPolicy::default(),
            &mut dirty,
            probe_len,
            area,
            decision_len,
            decision_overflow,
            false,    // board_overflow (no board layer in this command-only fixture)
            overflow, // probe_truncated — a wide region past the cap bails and sets this
            prev.len() != cur.len(),
            max_waste,
        )
    }

    /// The headline Stage-1 rescue: a wide count-shift edit region that used to bail
    /// (`dirty_skipped`, forced full repaint) is now collected and coalesces under threshold,
    /// so the coalesce-before-decide path keeps it incremental.
    #[test]
    fn stage1_wide_count_shift_becomes_incremental_after_coalescing() {
        // 30 contiguous tiles laid out 15-per-row, shifted down and one dropped (a count
        // shift): every slot differs (no common prefix/suffix), so the region is well over
        // MAX_EDIT_REGION but the union rects are contiguous and coalesce to a few bands.
        let prev: Vec<_> = (0..30)
            .map(|i| rect_cmd((i % 15) * 16, (i / 15) * 16))
            .collect();
        let cur: Vec<_> = (0..29)
            .map(|i| rect_cmd((i % 15) * 16, (i / 15) * 16 + 32))
            .collect();
        // Pre-Stage-1 (the live MAX_EDIT_REGION cap) bailed: nothing collected, forced repaint.
        let (live_rects, live_overflow) = collect(&prev, &cur);
        assert!(live_overflow);
        assert!(live_rects.is_empty());
        // Stage 1 collects the wide region; the coalesce-before-decide path rescues it.
        let out = collect_wide_then_decide(&prev, &cur, (16 * 16 * 4) as u32);
        assert_eq!(out.decision, DeltaDecision::Incremental);
        assert!(out.coalesced_len <= FULL_REPAINT_RECTS as usize);
        assert!(out.pressure.converted_to_incremental);
        // The pre-coalesce 25-cap order would have escalated, attributed to the count shift.
        assert_eq!(
            out.pressure.old_reason,
            Some(FullRepaintReason::CommandCountShift)
        );
        assert_eq!(out.pressure.new_reason, None);
        assert!(
            out.pressure.rects_pre > MAX_EDIT_REGION as u16,
            "the wide region is collected"
        );
        assert!(!out.pressure.truncated);
    }

    /// A wide count shift whose collected + coalesced damage is genuinely screen-wide stays a
    /// full repaint on its own merit (area), attributed CommandCountShift (the count changed).
    #[test]
    fn stage1_wide_count_shift_area_exceeded_still_full_repaints() {
        // Full-width bands stacked to cover the surface, shifted by a non-tile offset (so no
        // slot coincides — a genuine wide shift, not a head removal) and one dropped.
        let pband = |i: i32| AppDrawCommand::Rect {
            x: 0,
            y: i * 16,
            w: 320,
            h: 16,
            rgb565: 0x1234,
        };
        let cband = |i: i32| AppDrawCommand::Rect {
            x: 0,
            y: i * 16 + 8,
            w: 320,
            h: 16,
            rgb565: 0x1234,
        };
        let prev: Vec<_> = (0..30).map(pband).collect();
        let cur: Vec<_> = (0..29).map(cband).collect();
        let (_, live_overflow) = collect(&prev, &cur);
        assert!(live_overflow, "pre-Stage-1 bailed on the wide region");
        let out = collect_wide_then_decide(&prev, &cur, (16 * 16 * 4) as u32);
        assert_eq!(
            out.decision,
            DeltaDecision::FullRepaint(FullRepaintReason::CommandCountShift)
        );
        assert!(out.pressure.area_coalesced >= FULL_REPAINT_AREA);
        assert!(!out.pressure.converted_to_incremental);
        assert!(!out.pressure.truncated);
    }

    /// A region wider than even the expanded cap bails (case #4): the truncation fail-safe
    /// forces a full repaint regardless of how the captured prefix would coalesce.
    #[test]
    fn stage1_wide_count_shift_truncated_still_full_repaints() {
        let prev: Vec<_> = (0..STAGE1_PROBE_CAP + 20)
            .map(|i| rect_cmd((i as i32 % 18) * 16, (i as i32 / 18) * 16))
            .collect();
        let cur: Vec<_> = (0..STAGE1_PROBE_CAP + 19)
            .map(|i| rect_cmd((i as i32 % 18) * 16 + 1, (i as i32 / 18) * 16 + 1))
            .collect();
        let out = collect_wide_then_decide(&prev, &cur, (16 * 16 * 4) as u32);
        assert!(matches!(
            out.decision,
            DeltaDecision::FullRepaint(FullRepaintReason::CommandCountShift)
        ));
        assert!(out.pressure.truncated);
        assert!(!out.pressure.converted_to_incremental);
    }

    /// A bounded count shift (region ≤ MAX_EDIT_REGION) collects byte-identically under the old
    /// and the expanded cap, and stays incremental — Stage 1 changes only the wide-region path.
    #[test]
    fn stage1_bounded_count_shift_is_identical() {
        // Single head removal: [A,B,C,…] (29) → [B,C,…] (28), region 1.
        let prev: Vec<_> = (0..29).map(|i| rect_cmd((i % 20) * 16, i)).collect();
        let cur: Vec<_> = prev[1..].to_vec();
        let bounded = collect_with_cap(&prev, &cur, MAX_EDIT_REGION);
        let expanded = collect_with_cap(&prev, &cur, STAGE1_PROBE_CAP);
        assert_eq!(
            bounded, expanded,
            "bounded region collects identically under either cap"
        );
        assert!(!bounded.1, "a bounded region overflows neither cap");
        let out = collect_wide_then_decide(&prev, &cur, (16 * 16 * 4) as u32);
        assert_eq!(out.decision, DeltaDecision::Incremental);
        assert!(!out.pressure.converted_to_incremental);
    }
}
