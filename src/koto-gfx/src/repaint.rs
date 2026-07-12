//! Full-repaint policy for the retained delta present path.
//!
//! When an incremental frame's dirty diff goes wide — the immediate command
//! count shifts and the positional diff misaligns, the changed area approaches
//! the whole surface, or the rectangle count fragments past what stays cheap —
//! a single clean whole-surface recompose is cheaper in both raster and transfer
//! than a flurry of overlapping near-full-screen partial transfers. This module
//! owns the thresholds and the attribution logic for that decision.
//!
//! [`FullRepaintReason`] moved here verbatim from the Pico firmware's `diag`
//! module (which re-exports it unchanged). The thresholds
//! ([`FULL_REPAINT_AREA`], [`FULL_REPAINT_RECTS`]) and the
//! [`FullRepaintPolicy::decide`] branch were inline in `app_render`'s
//! `present_app_delta`; they are lifted here 1:1 so the decision is identical,
//! just testable in isolation.
//!
//! [`coalesce_then_decide`] is the GFX-0010 Stage-1B *behaviour-changing* reorder:
//! it batch-coalesces the full expanded raw dirty set (collected into the structural
//! probe buffer) and then runs [`FullRepaintPolicy::decide`] on the *post-coalesce*
//! count and re-summed area, so a fragmented-but-coalescible frame stays incremental
//! at its true pass count instead of escalating on the raw rect count. It returns the
//! decision the present path acts on, the surviving coalesced rect count, and a
//! [`CoalescePressure`] record that contrasts the new decision with the pre-coalesce
//! order (so the `phase=171` diagnostic can report which frames were *converted* from a
//! full repaint back to incremental). A truncated raw set (probe overflow or dropped
//! board bands) fails safe to a full repaint regardless of the coalesced count.

use crate::dirty::coalesce_rects;
use crate::Rect;

/// Why an app frame fell back to a whole-surface full repaint (KOTO-0143). Every
/// `full=1` frame carries exactly one reason so a regression is attributable
/// rather than mysterious. The present path picks the reason by a fixed priority
/// (see [`FullRepaintPolicy::decide`] and `present_app_delta`): base change, then
/// static rebuild (both early returns in the present path), then — when the dirty
/// diff escalates — a command-count shift (the root cause that misaligns the
/// positional diff) outranks the area and rect thresholds it inflates, and area
/// outranks rect-count.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FullRepaintReason {
    /// The full-screen base color appeared, disappeared, or recolored.
    BaseChange,
    /// The retained static/background layer was rebuilt (e.g. title -> gameplay).
    StaticRebuild,
    /// The summed dirty area crossed the full-repaint area threshold.
    AreaExceeded,
    /// The dirty rectangle count crossed the full-repaint rect threshold.
    RectsExceeded,
    /// The immediate command-list length changed, misaligning the positional diff.
    CommandCountShift,
}

impl FullRepaintReason {
    pub fn as_str(self) -> &'static str {
        match self {
            FullRepaintReason::BaseChange => "BaseChange",
            FullRepaintReason::StaticRebuild => "StaticRebuild",
            FullRepaintReason::AreaExceeded => "AreaExceeded",
            FullRepaintReason::RectsExceeded => "RectsExceeded",
            FullRepaintReason::CommandCountShift => "CommandCountShift",
        }
    }
}

// Delta full-repaint thresholds (KOTO-0131). The positional command diff compares
// command[i] old-vs-new; when an app's command count shifts (KotoBlocks growing
// its board, or a line clear collapsing it), every later index is misaligned and
// the per-command union rects balloon. Past these bounds a single clean full
// compose is cheaper in *both* raster (one pass over the list, not one per rect)
// and transfer (one set of windows, no double-painted overlaps), so we take it.
//   - area: ~3/4 of the 320x320 surface. Beyond this the partial transfers
//     already approach a whole-screen blit, minus the per-rect overhead.
//   - rects: each changed rectangle re-rasters the entire command list clipped to
//     it; past ~24 rectangles that O(rects x commands) raster dominates.
/// Default summed-dirty-area threshold past which a delta escalates to a full
/// repaint (~3/4 of the 320x320 surface).
pub const FULL_REPAINT_AREA: u32 = 320 * 320 * 3 / 4;
/// Default dirty-rectangle count past which a delta escalates to a full repaint.
pub const FULL_REPAINT_RECTS: u32 = 24;

/// The collected per-frame dirty-diff summary the policy decides on. Mirrors the
/// quantities `present_app_delta` accumulates while walking the command, board,
/// sprite, and text layers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeltaInputs {
    /// Dirty rectangles collected into the working set.
    pub dirty_rects: usize,
    /// Summed area of those rectangles (overlaps double-counted).
    pub dirty_area: u32,
    /// The board's dirty cells fragmented past the band buffer (cannot drop a
    /// band, so treated as a rect-count escalation).
    pub board_overflow: bool,
    /// The working-set rect cap overflowed (more rects than stay incremental).
    pub rect_overflow: bool,
    /// The immediate command-list length changed this frame (the root cause that
    /// misaligns the positional command diff).
    pub command_count_changed: bool,
}

/// What the present path should do with a frame's collected dirty diff.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeltaDecision {
    /// Nothing changed; the retained frame can be left on the panel.
    Skip,
    /// Transfer the collected dirty rectangles incrementally.
    Incremental,
    /// Recompose the whole surface; carries the attributed reason.
    FullRepaint(FullRepaintReason),
}

/// The full-repaint escalation thresholds. `Default` is the live firmware policy
/// ([`FULL_REPAINT_AREA`] / [`FULL_REPAINT_RECTS`]); the fields are exposed so a
/// different surface size could be configured later without touching the logic.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FullRepaintPolicy {
    pub area_threshold: u32,
    pub rect_threshold: u32,
}

impl Default for FullRepaintPolicy {
    fn default() -> Self {
        Self {
            area_threshold: FULL_REPAINT_AREA,
            rect_threshold: FULL_REPAINT_RECTS,
        }
    }
}

impl FullRepaintPolicy {
    pub const fn new(area_threshold: u32, rect_threshold: u32) -> Self {
        Self {
            area_threshold,
            rect_threshold,
        }
    }

    /// Decide what to do with a frame's collected dirty diff. This is the exact
    /// branch `present_app_delta` ran inline:
    ///
    /// - A frame with no dirty rects and no overflow is [`DeltaDecision::Skip`].
    /// - Any overflow, or a summed area / rect count past the thresholds, escalates
    ///   to [`DeltaDecision::FullRepaint`]. The reason is attributed by fixed
    ///   priority: a command-count shift (root cause) outranks area, which outranks
    ///   rect count; overflows are rect-count symptoms and fall through to whichever
    ///   of those applies.
    /// - Otherwise the frame stays [`DeltaDecision::Incremental`].
    ///
    /// (`BaseChange`/`StaticRebuild` are handled by earlier early-returns in the
    /// present path, before any diff is collected, so they are never produced here.)
    pub fn decide(&self, inputs: DeltaInputs) -> DeltaDecision {
        if inputs.dirty_rects == 0 && !inputs.board_overflow && !inputs.rect_overflow {
            return DeltaDecision::Skip;
        }
        if inputs.board_overflow
            || inputs.rect_overflow
            || inputs.dirty_area >= self.area_threshold
            || inputs.dirty_rects > self.rect_threshold as usize
        {
            let reason = if inputs.command_count_changed {
                FullRepaintReason::CommandCountShift
            } else if inputs.dirty_area >= self.area_threshold {
                FullRepaintReason::AreaExceeded
            } else {
                FullRepaintReason::RectsExceeded
            };
            return DeltaDecision::FullRepaint(reason);
        }
        DeltaDecision::Incremental
    }
}

/// The contrast between the pre-coalesce decision order and the GFX-0010 Stage-1B
/// coalesce-before-decide order, recorded for the throttled `phase=171` diagnostic.
/// Stage 1B now coalesces on the *real* present path (it no longer dry-runs a scratch
/// copy), so these are the actual quantities the decision consumed — not a hypothetical.
///
/// The `decision_*` fields are the live 25-cap snapshot the *pre-coalesce* order would
/// have decided on (so a log reader can see what would have escalated); `rects_pre` /
/// `rects_coalesced` / `area_*` describe the expanded raw set and the post-coalesce set
/// the new order actually decided on; `old_reason` / `new_reason` / `converted_to_incremental`
/// say whether the reorder rescued the frame.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CoalescePressure {
    /// Rect count in the *decision* snapshot — the 25-cap count the pre-coalesce order
    /// would have decided on (`min(raw, DIRTY_RECT_CAP)`).
    pub decision_rects: u16,
    /// The decision snapshot was truncated — the 25-cap was exceeded or a layer
    /// overflowed upstream. This is the signal that drove the pre-coalesce `RectsExceeded`
    /// escalation that Stage 1B may now avoid.
    pub decision_truncated: bool,
    /// Collected dirty rects before coalescing, from the *expanded probe* collection
    /// (`DIRTY_RECT_PROBE_CAP`) — the raw fragmentation, no longer clipped at 25.
    pub rects_pre: u16,
    /// Rects surviving [`crate::coalesce_rects`] over the expanded set — the passes the
    /// frame actually costs (when incremental) or would have cost (when still escalated).
    pub rects_coalesced: u16,
    /// Summed area of the pre-coalesce probe rects (overlaps double-counted).
    pub area_pre: u32,
    /// Re-summed area of the coalesced survivors (grown to their bounding boxes) — the
    /// area the Stage-1B decision was actually taken on.
    pub area_coalesced: u32,
    /// Bounding box over all pre-coalesce probe rects — a value far above `area_pre`
    /// reads as "scattered", the signature coalescing targets.
    pub bbox_area: u32,
    /// What the *pre-coalesce* order would have decided (`None` if it stayed incremental
    /// or skipped) — the escalation Stage 1B is contrasted against.
    pub old_reason: Option<FullRepaintReason>,
    /// What the *post-coalesce* Stage-1B order decided (`None` if incremental or skipped)
    /// — the reason the present path actually acts on when it still escalates.
    pub new_reason: Option<FullRepaintReason>,
    /// True when the pre-coalesce order would have full-repainted (for any reason) but
    /// the coalesced set stays incremental: the frame Stage 1B rescued.
    pub converted_to_incremental: bool,
    /// The *probe* collection overflowed its (larger) cap, or a layer overflowed
    /// upstream (board band buffer / wide command restructure): the raw set is truncated,
    /// so Stage 1B fails safe to a full repaint (a truncated set cannot be trusted to
    /// have captured every change). With the expanded cap this is far rarer than the
    /// 25-cap decision truncation.
    pub truncated: bool,
}

/// The result of the GFX-0010 Stage-1B coalesce-before-decide reorder: the decision the
/// present path acts on, the surviving coalesced rect count (packed into the front of
/// the buffer the caller passed), and the [`CoalescePressure`] diagnostic record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CoalesceDecision {
    /// The decision the present path acts on (taken on the *post-coalesce* set).
    pub decision: DeltaDecision,
    /// Surviving coalesced rect count, packed into `rects[..coalesced_len]`.
    pub coalesced_len: usize,
    /// Diagnostic contrast against the pre-coalesce order, for `phase=171`.
    pub pressure: CoalescePressure,
}

/// Batch-coalesce the full expanded raw dirty set, then decide on the post-coalesce
/// count and re-summed area (GFX-0010 Stage 1B, behaviour-changing).
///
/// `rects[..probe_len]` is the collected *expanded probe* dirty set; this coalesces it
/// in place (so on return `rects[..result.coalesced_len]` are the survivors the caller
/// transfers). The decision the caller acts on is `result.decision`, taken on the
/// post-coalesce count and `area_coalesced`, so a fragmented-but-coalescible frame stays
/// [`DeltaDecision::Incremental`] instead of escalating on the raw rect count.
///
/// - `decision_rects` / `decision_truncated` are the live 25-cap snapshot
///   (`decision_snapshot`) the *pre-coalesce* order would have decided on; they are used
///   only to compute `old_reason` / `converted_to_incremental` for the diagnostic and do
///   not drive the returned decision.
/// - `area_pre` is the pre-coalesce summed area; `command_count_changed` is the immediate
///   command-list length-change flag (its only effect is attribution — it reorders a
///   still-escalating frame's reason to `CommandCountShift`, exactly as before).
/// - `probe_truncated` (probe-buffer overflow **or** an upstream layer overflow such as
///   `board_overflow`) is the truncation fail-safe: a truncated raw set cannot be trusted
///   to hold every change, so it forces a full repaint regardless of the coalesced count
///   — preserving the pre-coalesce `CommandCountShift`/overflow safety unless the set is
///   fully derived and non-truncated.
#[allow(clippy::too_many_arguments)]
pub fn coalesce_then_decide(
    policy: FullRepaintPolicy,
    rects: &mut [Rect],
    probe_len: usize,
    area_pre: u32,
    decision_rects: usize,
    decision_truncated: bool,
    board_overflow: bool,
    probe_truncated: bool,
    command_count_changed: bool,
    max_waste: u32,
) -> CoalesceDecision {
    let len = probe_len.min(rects.len());
    let rects_pre = len as u16;
    // What the pre-coalesce order would have decided on the 25-cap snapshot — recorded
    // only so the diagnostic can show the contrast (it no longer drives the present).
    let old_decision = policy.decide(DeltaInputs {
        dirty_rects: decision_rects,
        dirty_area: area_pre,
        board_overflow,
        rect_overflow: decision_truncated,
        command_count_changed,
    });
    // Bounding box over the *pre*-coalesce set, before `coalesce_rects` mutates it.
    let bbox_area = match rects[..len].split_first() {
        None => 0,
        Some((first, rest)) => rest.iter().fold(*first, |acc, r| acc.bbox(*r)).area(),
    };
    // Coalesce the full raw set, then decide on the post-coalesce count and area.
    let coalesced_len = coalesce_rects(rects, len, max_waste);
    let area_coalesced = rects[..coalesced_len].iter().map(|r| r.area()).sum();
    // Truncation fail-safe: an incomplete raw set forces escalation via `rect_overflow`,
    // so a probe/board overflow still full-repaints (and `command_count_changed` still
    // attributes it `CommandCountShift`), exactly as the pre-coalesce order did.
    let new_decision = policy.decide(DeltaInputs {
        dirty_rects: coalesced_len,
        dirty_area: area_coalesced,
        board_overflow,
        rect_overflow: probe_truncated,
        command_count_changed,
    });
    let reason_of = |d: DeltaDecision| match d {
        DeltaDecision::FullRepaint(r) => Some(r),
        _ => None,
    };
    // A rescue: the pre-coalesce order would have full-repainted (for *any* reason —
    // a count shift that also over-fragments is attributed `CommandCountShift`, yet is
    // just as much a coalescible rescue once the bounded edit region collapses), but the
    // coalesced set stays incremental.
    let converted_to_incremental = matches!(new_decision, DeltaDecision::Incremental)
        && matches!(old_decision, DeltaDecision::FullRepaint(_));
    CoalesceDecision {
        decision: new_decision,
        coalesced_len,
        pressure: CoalescePressure {
            decision_rects: decision_rects as u16,
            decision_truncated,
            rects_pre,
            rects_coalesced: coalesced_len as u16,
            area_pre,
            area_coalesced,
            bbox_area,
            old_reason: reason_of(old_decision),
            new_reason: reason_of(new_decision),
            converted_to_incremental,
            truncated: probe_truncated,
        },
    }
}

/// Derive the byte-identical pre-Stage-1A decision snapshot from an expanded-probe
/// collection (GFX-0010 Stage 1A).
///
/// Stage 1A collects the dirty set into a buffer larger than `decision_cap` so the
/// observe-only coalesce probe sees the full fragmentation, but the policy must still
/// decide on exactly what it saw before the buffer was widened: the count capped at
/// `decision_cap`, and an overflow flag set whenever the raw count crossed that cap or a
/// layer overflowed upstream (folded into `probe_overflow`). Returns
/// `(decision_rects, decision_rect_overflow)` for [`DeltaInputs`]. The summed dirty area
/// is *not* derived here — `push_dirty` accumulates area independent of the buffer cap,
/// so the area the policy decides on is identical whatever the collection cap.
///
/// Equivalence to the pre-Stage-1A 25-cap collection (where `rect_overflow` was
/// `region_overflow || raw_count > cap`): `probe_overflow` already folds in
/// `region_overflow`, and `raw_count > cap` ⟺ `probe_len > cap` (a probe that itself
/// overflowed has `probe_len == probe_cap > cap`), so the result matches for every raw
/// count.
pub fn decision_snapshot(
    probe_len: usize,
    probe_overflow: bool,
    decision_cap: usize,
) -> (usize, bool) {
    (
        probe_len.min(decision_cap),
        probe_overflow || probe_len > decision_cap,
    )
}

/// One-shot "force a full repaint on the present *after* a retained-static-layer rebuild"
/// latch (BUG-GFX-0012).
///
/// The incremental present path (GFX-0010 coalesce-before-decide, GFX-0011 wide-region
/// collection) is only sound when the retained GRAM already equals what a full repaint would
/// produce *outside* the dirty rects. That invariant does **not** hold on the frame that first
/// renders a freshly (re)built static layer: the rebuild frame composites the new static layer,
/// but its own immediate commands can overdraw the just-composited chrome before it is seen
/// (e.g. a title screen still painting a full-screen fill over the newly built HUD on the frame
/// it crosses into gameplay). The *next* frame is then legitimately kept incremental by the
/// coalesce rescue — yet it is the frame that must reveal the retained chrome, whose region is
/// not in the incremental dirty set, so the chrome never reaches GRAM. Before GFX-0010/0011 that
/// frame happened to full-repaint on its wide command change, masking the gap.
///
/// The fix keeps the coalesce rescue for steady frames but forces a full repaint on the first
/// present following a rebuild. `static_rebuilt` is whether *this* frame rebuilt the static
/// layer; `latched` is the pending bit carried from the previous present. Returns
/// `(force_full_this_present, latch_for_next_present)`. Pure — the caller owns the latch bit
/// across frames. The rebuild frame itself is already a full repaint via the existing
/// `StaticRebuild` path, so this only forces the one following present; a steady frame (no
/// recent rebuild) is untouched, preserving the GFX-0010/0011 incremental behaviour.
pub fn force_full_repaint_after_static_rebuild(
    static_rebuilt: bool,
    latched: bool,
) -> (bool, bool) {
    (latched, static_rebuilt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::push_dirty;

    fn inputs() -> DeltaInputs {
        DeltaInputs {
            dirty_rects: 0,
            dirty_area: 0,
            board_overflow: false,
            rect_overflow: false,
            command_count_changed: false,
        }
    }

    #[test]
    fn clean_frame_skips() {
        assert_eq!(
            FullRepaintPolicy::default().decide(inputs()),
            DeltaDecision::Skip
        );
    }

    #[test]
    fn small_change_stays_incremental() {
        let i = DeltaInputs {
            dirty_rects: 3,
            dirty_area: 4 * 256,
            ..inputs()
        };
        assert_eq!(
            FullRepaintPolicy::default().decide(i),
            DeltaDecision::Incremental
        );
    }

    #[test]
    fn area_over_threshold_escalates_as_area_exceeded() {
        let i = DeltaInputs {
            dirty_rects: 2,
            dirty_area: FULL_REPAINT_AREA,
            ..inputs()
        };
        assert_eq!(
            FullRepaintPolicy::default().decide(i),
            DeltaDecision::FullRepaint(FullRepaintReason::AreaExceeded)
        );
    }

    #[test]
    fn rect_count_over_threshold_escalates_as_rects_exceeded() {
        let i = DeltaInputs {
            dirty_rects: FULL_REPAINT_RECTS as usize + 1,
            dirty_area: 1,
            ..inputs()
        };
        assert_eq!(
            FullRepaintPolicy::default().decide(i),
            DeltaDecision::FullRepaint(FullRepaintReason::RectsExceeded)
        );
    }

    #[test]
    fn rect_count_at_threshold_stays_incremental() {
        // The escalation is strictly greater-than, so exactly the threshold count
        // is still incremental — matching `dirty_len > FULL_REPAINT_RECTS`.
        let i = DeltaInputs {
            dirty_rects: FULL_REPAINT_RECTS as usize,
            dirty_area: 1,
            ..inputs()
        };
        assert_eq!(
            FullRepaintPolicy::default().decide(i),
            DeltaDecision::Incremental
        );
    }

    #[test]
    fn command_count_shift_outranks_area_and_rects() {
        // Even when the area and rect thresholds are also tripped, a command-count
        // shift is the attributed root cause.
        let i = DeltaInputs {
            dirty_rects: FULL_REPAINT_RECTS as usize + 5,
            dirty_area: FULL_REPAINT_AREA + 1000,
            command_count_changed: true,
            ..inputs()
        };
        assert_eq!(
            FullRepaintPolicy::default().decide(i),
            DeltaDecision::FullRepaint(FullRepaintReason::CommandCountShift)
        );
    }

    #[test]
    fn area_outranks_rect_count_when_both_tripped() {
        let i = DeltaInputs {
            dirty_rects: FULL_REPAINT_RECTS as usize + 5,
            dirty_area: FULL_REPAINT_AREA,
            ..inputs()
        };
        assert_eq!(
            FullRepaintPolicy::default().decide(i),
            DeltaDecision::FullRepaint(FullRepaintReason::AreaExceeded)
        );
    }

    #[test]
    fn board_overflow_with_no_rects_escalates_not_skips() {
        // A board that fragmented past its band buffer must repaint even though no
        // rect was collected — overflow is a rect-count symptom (RectsExceeded).
        let i = DeltaInputs {
            board_overflow: true,
            ..inputs()
        };
        assert_eq!(
            FullRepaintPolicy::default().decide(i),
            DeltaDecision::FullRepaint(FullRepaintReason::RectsExceeded)
        );
    }

    #[test]
    fn rect_overflow_escalates() {
        let i = DeltaInputs {
            dirty_rects: FULL_REPAINT_RECTS as usize + 1,
            rect_overflow: true,
            ..inputs()
        };
        assert_eq!(
            FullRepaintPolicy::default().decide(i),
            DeltaDecision::FullRepaint(FullRepaintReason::RectsExceeded)
        );
    }

    // ---- GFX-0010 Stage-1B coalesce-before-decide -------------------------

    fn cell(x: i32, y: i32) -> Rect {
        Rect { x, y, w: 16, h: 16 }
    }

    /// Run the Stage-1B reorder the way the firmware does: derive the 25-cap snapshot
    /// from the raw probe count, then coalesce-then-decide on the full set.
    fn decide_1b(
        rects: &mut [Rect],
        probe_len: usize,
        area_pre: u32,
        probe_overflow: bool,
        board_overflow: bool,
        command_count_changed: bool,
        max_waste: u32,
    ) -> CoalesceDecision {
        const DECISION_CAP: usize = FULL_REPAINT_RECTS as usize + 1; // firmware DIRTY_RECT_CAP
        let (decision_rects, decision_truncated) =
            decision_snapshot(probe_len, probe_overflow, DECISION_CAP);
        coalesce_then_decide(
            FullRepaintPolicy::default(),
            rects,
            probe_len,
            area_pre,
            decision_rects,
            decision_truncated,
            board_overflow,
            probe_overflow || board_overflow,
            command_count_changed,
            max_waste,
        )
    }

    /// The headline Stage-1B win: a >25 raw-rect frame whose rects coalesce under the
    /// threshold now stays incremental instead of escalating on the raw count — and the
    /// diagnostic records the conversion (`old_reason=RectsExceeded`, no `new_reason`).
    #[test]
    fn coalescible_overcount_becomes_incremental() {
        // 40 edge-adjacent cells across two contiguous rows: raw count 40 (> 25, so the
        // pre-coalesce order escalates `RectsExceeded`), but they coalesce to two bands.
        let mut rects: Vec<Rect> = (0..20)
            .map(|i| cell(i * 16, 0))
            .chain((0..20).map(|i| cell(i * 16, 16)))
            .collect();
        let area_pre = rects.iter().map(|r| r.area()).sum();
        let out = decide_1b(&mut rects, 40, area_pre, false, false, false, 1024);
        assert_eq!(out.decision, DeltaDecision::Incremental);
        assert!(out.coalesced_len <= FULL_REPAINT_RECTS as usize);
        assert!(out.pressure.converted_to_incremental);
        assert_eq!(
            out.pressure.old_reason,
            Some(FullRepaintReason::RectsExceeded)
        );
        assert_eq!(out.pressure.new_reason, None);
        assert_eq!(out.pressure.rects_pre, 40);
        assert!(out.pressure.decision_truncated); // the pre-coalesce 25-cap was truncated…
        assert!(!out.pressure.truncated); // …but the expanded raw set was complete
                                          // The survivors packed into the front coalesce exactly as a direct call.
        let mut direct: Vec<Rect> = (0..20)
            .map(|i| cell(i * 16, 0))
            .chain((0..20).map(|i| cell(i * 16, 16)))
            .collect();
        assert_eq!(
            coalesce_rects(&mut direct, 40, 1024) as u16,
            out.pressure.rects_coalesced
        );
    }

    /// An irreducible scatter (no two rects merge within the budget) stays a full
    /// repaint after coalescing — Stage 1B must not rescue what cannot coalesce.
    #[test]
    fn scattered_overcount_still_full_repaints() {
        // 30 single cells with a clean gap between each (step 20 > the 16-wide cell): at
        // the tightest budget none merge, so the post-coalesce count stays 30 (> 24).
        let mut rects: Vec<Rect> = (0..30).map(|i| cell((i % 5) * 40, (i / 5) * 40)).collect();
        let area_pre = rects.iter().map(|r| r.area()).sum();
        let out = decide_1b(&mut rects, 30, area_pre, false, false, false, 0);
        assert_eq!(
            out.decision,
            DeltaDecision::FullRepaint(FullRepaintReason::RectsExceeded)
        );
        assert!(out.coalesced_len > FULL_REPAINT_RECTS as usize);
        assert!(!out.pressure.converted_to_incremental);
        assert_eq!(
            out.pressure.new_reason,
            Some(FullRepaintReason::RectsExceeded)
        );
    }

    /// A truncated raw collection (probe-buffer or upstream overflow) fails safe to a
    /// full repaint even when the captured prefix coalesces small — an incomplete set
    /// cannot be trusted to have captured every change.
    #[test]
    fn truncated_collection_still_full_repaints() {
        // The captured cells coalesce to one band, but the collection overflowed, so the
        // fail-safe escalates regardless.
        let mut rects: Vec<Rect> = (0..25).map(|i| cell(i * 16, 0)).collect();
        let area_pre = rects.iter().map(|r| r.area()).sum();
        let out = decide_1b(&mut rects, 25, area_pre, true, false, false, 1024);
        assert!(matches!(out.decision, DeltaDecision::FullRepaint(_)));
        assert!(out.pressure.truncated);
        assert!(!out.pressure.converted_to_incremental);
        // Even though the captured rects collapsed under the threshold.
        assert!(out.pressure.rects_coalesced <= FULL_REPAINT_RECTS as u16);
    }

    /// A board fragmenting past its band buffer (`board_overflow`) is an upstream
    /// truncation: it escalates regardless of how the collected rects coalesce.
    #[test]
    fn board_overflow_still_full_repaints() {
        let mut rects: Vec<Rect> = (0..4).map(|i| cell(i * 16, 0)).collect();
        let area_pre = rects.iter().map(|r| r.area()).sum();
        let out = decide_1b(&mut rects, 4, area_pre, false, true, false, 1024);
        assert!(matches!(out.decision, DeltaDecision::FullRepaint(_)));
        assert!(out.pressure.truncated);
    }

    /// The CommandCountShift fail-safe is preserved: when the raw set is truncated by a
    /// wide command restructure (`command_count_changed` + overflow), the still-escalated
    /// frame is attributed `CommandCountShift`, not `RectsExceeded`.
    #[test]
    fn command_count_shift_attribution_preserved_on_truncation() {
        let mut rects: Vec<Rect> = (0..25).map(|i| cell(i * 16, 0)).collect();
        let area_pre = rects.iter().map(|r| r.area()).sum();
        // probe_overflow=true models the wide-edit-region bail; command_count_changed=true.
        let out = decide_1b(&mut rects, 25, area_pre, true, false, true, 1024);
        assert_eq!(
            out.decision,
            DeltaDecision::FullRepaint(FullRepaintReason::CommandCountShift)
        );
        assert!(!out.pressure.converted_to_incremental);
    }

    /// A command-count change whose bounded edit region coalesces small stays
    /// incremental — the reorder only escalates a count shift when the set is truncated
    /// (the fail-safe), not when it is fully derived.
    #[test]
    fn command_count_shift_coalescible_stays_incremental() {
        // Two contiguous rows of 40 cells each -> coalesces to two bands.
        let mut rects: Vec<Rect> = (0..40)
            .map(|i| cell(i * 16, 0))
            .chain((0..40).map(|i| cell(i * 16, 16)))
            .collect();
        let area_pre = rects.iter().map(|r| r.area()).sum();
        let probe_len = rects.len();
        // Not truncated, but the command count changed this frame.
        let out = decide_1b(&mut rects, probe_len, area_pre, false, false, true, 1024);
        assert_eq!(out.decision, DeltaDecision::Incremental);
        assert!(out.pressure.converted_to_incremental);
    }

    /// `bbox_area` is the bounding box over the *pre*-coalesce raw set (computed before
    /// the in-place merge), so a scattered set's bbox far exceeds its summed `area_pre`.
    #[test]
    fn bbox_area_is_over_the_precoalesce_set() {
        let mut rects = [cell(0, 0), cell(300, 300)];
        let out = decide_1b(&mut rects, 2, 16 * 16 * 2, false, false, false, 1024);
        // bbox spans (0,0)..(316,316).
        assert_eq!(out.pressure.bbox_area, 316 * 316);
        assert_eq!(out.pressure.area_pre, 16 * 16 * 2);
    }

    /// Golden identity for a frame that needed no rescue: a small in-bounds set decides
    /// `Incremental` (not converted), with the survivors coalesced exactly as today's
    /// post-decision coalesce would produce. The pre-coalesce order and the reorder agree.
    #[test]
    fn small_incremental_frame_is_unchanged() {
        let mut rects = [cell(0, 0), cell(40, 0), cell(0, 40)];
        let area_pre = rects.iter().map(|r| r.area()).sum();
        let out = decide_1b(&mut rects, 3, area_pre, false, false, false, 1024);
        assert_eq!(out.decision, DeltaDecision::Incremental);
        assert!(!out.pressure.converted_to_incremental);
        assert_eq!(out.pressure.old_reason, None);
        // Survivors match a direct coalesce of the same set (no behaviour drift).
        let mut direct = [cell(0, 0), cell(40, 0), cell(0, 40)];
        let n = coalesce_rects(&mut direct, 3, 1024);
        assert_eq!(out.coalesced_len, n);
        assert_eq!(&rects[..out.coalesced_len], &direct[..n]);
    }

    /// A clean frame (no raw rects, no overflow) skips, exactly as the pre-coalesce
    /// order did — coalesce-before-decide does not invent work.
    #[test]
    fn empty_frame_skips() {
        let mut rects: [Rect; 0] = [];
        let out = decide_1b(&mut rects, 0, 0, false, false, false, 1024);
        assert_eq!(out.decision, DeltaDecision::Skip);
        assert_eq!(out.coalesced_len, 0);
        assert!(!out.pressure.converted_to_incremental);
    }

    // ---- GFX-0010 Stage-1A decision-snapshot invariant --------------------

    /// Simulate the firmware collection into both the old 25-cap buffer and the
    /// expanded probe buffer over the *same* push sequence, and assert the policy
    /// reaches the identical decision from the Stage-1A snapshot — for every raw count
    /// from empty through past the probe cap. This is the host-side proof that widening
    /// the probe cannot change what `present_app_delta` decides (no attribution drift).
    #[test]
    fn expanded_probe_never_changes_the_decision() {
        const DECISION_CAP: usize = FULL_REPAINT_RECTS as usize + 1; // firmware DIRTY_RECT_CAP
        const PROBE_CAP: usize = 84; // firmware DIRTY_RECT_PROBE_CAP (structural sum)
        let policy = FullRepaintPolicy::default();

        for raw in 0..=PROBE_CAP + 5 {
            // The same rect sequence pushed into each buffer (small cells; any shape).
            let push = |buf: &mut [Rect]| {
                let (mut len, mut area, mut overflow) = (0usize, 0u32, false);
                for i in 0..raw {
                    push_dirty(
                        buf,
                        &mut len,
                        &mut area,
                        &mut overflow,
                        cell((i as i32 % 20) * 16, (i as i32 / 20) * 16),
                    );
                }
                (len, area, overflow)
            };

            // Pre-Stage-1A: collect into the 25-cap buffer; decide directly on it.
            let mut old_buf = [cell(0, 0); DECISION_CAP];
            let (old_len, old_area, old_overflow) = push(&mut old_buf);
            let old_decision = policy.decide(DeltaInputs {
                dirty_rects: old_len,
                dirty_area: old_area,
                board_overflow: false,
                rect_overflow: old_overflow,
                command_count_changed: false,
            });

            // Stage-1A: collect into the probe buffer; derive the decision snapshot.
            let mut probe_buf = [cell(0, 0); PROBE_CAP];
            let (probe_len, probe_area, probe_overflow) = push(&mut probe_buf);
            let (decision_len, decision_overflow) =
                decision_snapshot(probe_len, probe_overflow, DECISION_CAP);
            // Area is independent of the buffer cap (push_dirty always sums it).
            assert_eq!(probe_area, old_area, "area diverged at raw={raw}");
            let new_decision = policy.decide(DeltaInputs {
                dirty_rects: decision_len,
                dirty_area: probe_area,
                board_overflow: false,
                rect_overflow: decision_overflow,
                command_count_changed: false,
            });

            assert_eq!(old_decision, new_decision, "decision diverged at raw={raw}");
            // And the snapshot reproduces the old buffer's exact (len, overflow).
            assert_eq!(decision_len, old_len, "decision_len diverged at raw={raw}");
            assert_eq!(
                decision_overflow, old_overflow,
                "overflow diverged at raw={raw}"
            );
        }
    }

    // ---- BUG-GFX-0012 static-rebuild reveal latch -------------------------

    /// The reveal frame after a static rebuild is forced full, and the latch is a strict
    /// one-shot: it fires on exactly the present following the rebuild, then clears — so a
    /// steady frame keeps the GFX-0010/0011 incremental behaviour.
    #[test]
    fn static_rebuild_forces_exactly_the_following_present_full() {
        let mut latched = false;
        // A steady frame before any rebuild: not forced, no latch.
        let (force, next) = force_full_repaint_after_static_rebuild(false, latched);
        assert!(!force, "no recent rebuild -> incremental stays available");
        latched = next;
        assert!(!latched);
        // The rebuild frame itself is handled by the existing StaticRebuild path (not forced
        // here), but it latches the *next* present.
        let (force, next) = force_full_repaint_after_static_rebuild(true, latched);
        assert!(
            !force,
            "the rebuild frame's fullness is the existing StaticRebuild path"
        );
        latched = next;
        assert!(latched, "the following present is latched");
        // The reveal frame (no rebuild this frame) is forced full and consumes the latch.
        let (force, next) = force_full_repaint_after_static_rebuild(false, latched);
        assert!(
            force,
            "the present after a rebuild must be full (reveal the chrome)"
        );
        latched = next;
        assert!(!latched, "the one-shot latch clears");
        // The next steady frame is incremental again (perf preserved).
        let (force, next) = force_full_repaint_after_static_rebuild(false, latched);
        assert!(!force);
        assert!(!next);
    }

    /// Back-to-back rebuilds keep latching: each rebuild forces the following present and
    /// re-arms, so a run of rebuilds never leaves a stale incremental reveal.
    #[test]
    fn consecutive_static_rebuilds_keep_latching() {
        // rebuild, then another rebuild on the very next present.
        let (force_a, latch_a) = force_full_repaint_after_static_rebuild(true, false);
        assert!(!force_a);
        assert!(latch_a);
        let (force_b, latch_b) = force_full_repaint_after_static_rebuild(true, latch_a);
        assert!(
            force_b,
            "the second rebuild present is both a reveal (latched) and re-arms"
        );
        assert!(latch_b, "and it latches the present after it");
        let (force_c, latch_c) = force_full_repaint_after_static_rebuild(false, latch_b);
        assert!(force_c);
        assert!(!latch_c);
    }

    /// The probe buffer holds the full structural maximum without truncating: pushing
    /// exactly `DIRTY_RECT_PROBE_CAP` rects fits (no overflow), one more overflows. This
    /// is the property that makes `probe_truncated=0` reachable on a realistic frame —
    /// the Stage-1A win over the 25-cap.
    #[test]
    fn probe_buffer_holds_full_structural_max() {
        const PROBE_CAP: usize = 84; // firmware DIRTY_RECT_PROBE_CAP

        let mut buf = [cell(0, 0); PROBE_CAP];
        let (mut len, mut area, mut overflow) = (0usize, 0u32, false);
        for i in 0..PROBE_CAP {
            push_dirty(
                &mut buf,
                &mut len,
                &mut area,
                &mut overflow,
                cell(i as i32, 0),
            );
        }
        assert_eq!(len, PROBE_CAP);
        assert!(!overflow, "structural max must fit the probe buffer");

        // One past the structural max overflows — honestly reported as truncated.
        push_dirty(&mut buf, &mut len, &mut area, &mut overflow, cell(0, 0));
        assert_eq!(len, PROBE_CAP);
        assert!(overflow);
    }
}
