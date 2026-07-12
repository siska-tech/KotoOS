//! Budgeted immediate-overlay admission model (KotoGFX next step).
//!
//! The long-term KotoGFX plan (see `docs/architecture/kotogfx-architecture.md`) keeps the
//! main pipeline retained (Tile/Sprite/Text layers) and pushes *immediate*
//! drawing — flashes, popups, food-pickup particles, the flowing rainbow snake
//! body, transient overlays — onto a separate, explicitly budgeted path. The
//! firmware already has a hard ceiling on immediate draw commands per frame
//! (`MAX_APP_DRAW_COMMANDS`): once it is hit, every further push returns
//! `NO_MEMORY` and the app *silently drops its tail commands*, which then trips
//! the delta path into a whole-surface full repaint. That ceiling is purely
//! first-come-first-served, so whichever commands an app happens to emit last —
//! often the head/apple/score it most needs on screen — are the ones dropped.
//!
//! This module turns that single scattered constant into a small, pure policy
//! layer. Immediate draws are tagged with a [`DrawClass`]; each class carries an
//! [`OverlayPriority`] and may be granted a guaranteed [`DrawBudget`]
//! reservation. Admission ([`DrawBudget::request`]) draws first from the class's
//! own reservation, then from a shared pool that protects headroom for more
//! important classes — so critical gameplay is admitted first and decorative
//! effects are rejected or degraded *before* the hard cap is reached (and thus
//! before any tail-drop / full-repaint). The accounting ([`BudgetStats`]) can
//! never admit past the configured cap.
//!
//! This is pure `no_std` data: it does not draw anything, does not allocate, and
//! is **not yet wired into any app's visuals**. Only the cap *value* is shared
//! with the firmware (see [`APP_DRAW_BUDGET`]); behaviour is unchanged.

/// Number of [`DrawClass`] variants (array sizing for the per-class tables).
pub const DRAW_CLASS_COUNT: usize = 6;

/// What an immediate draw *is*, so the budget can protect the draws that matter.
///
/// The classes are ordered from most to least essential. Each maps to a fixed
/// [`OverlayPriority`] ([`DrawClass::priority`]); the budget reserves command
/// headroom for the important ones and lets the rest compete for a shared pool.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DrawClass {
    /// Core gameplay surface drawn immediately (not via a retained layer): the
    /// playfield, walls, board chrome an app blits each frame. Must always land.
    CoreGameplay,
    /// The actors you must always see: snake head/body anchor, apple, the
    /// player. Small, fixed command count; never sacrificed to effects.
    Actor,
    /// Text and critical UI: score, lives, prompts, popups conveying state.
    CriticalUi,
    /// Gameplay-reactive particles: food-pickup bursts, hit sparks. Nice to keep
    /// whole, but may shed individual particles under pressure.
    Particles,
    /// Decorative effects: the flowing rainbow body shimmer, flashes, transient
    /// ambiance. First to be degraded; never allowed to starve the above.
    Decoration,
    /// Debug overlays: counters, dirty-rect outlines, profiling text. Lowest
    /// priority; only drawn from whatever headroom is left.
    Debug,
}

impl DrawClass {
    /// All classes in priority order (most essential first). Useful for tests and
    /// for iterating the per-class tables deterministically.
    pub const ALL: [DrawClass; DRAW_CLASS_COUNT] = [
        DrawClass::CoreGameplay,
        DrawClass::Actor,
        DrawClass::CriticalUi,
        DrawClass::Particles,
        DrawClass::Decoration,
        DrawClass::Debug,
    ];

    /// Index into the per-class budget/stat tables.
    pub const fn index(self) -> usize {
        self as usize
    }

    /// The fixed priority tier this class is admitted at.
    pub const fn priority(self) -> OverlayPriority {
        match self {
            DrawClass::CoreGameplay => OverlayPriority::Critical,
            DrawClass::Actor => OverlayPriority::Critical,
            DrawClass::CriticalUi => OverlayPriority::High,
            DrawClass::Particles => OverlayPriority::Normal,
            DrawClass::Decoration => OverlayPriority::Low,
            DrawClass::Debug => OverlayPriority::Optional,
        }
    }

    /// Stable name for diagnostics / UART triage lines.
    pub const fn as_str(self) -> &'static str {
        match self {
            DrawClass::CoreGameplay => "CoreGameplay",
            DrawClass::Actor => "Actor",
            DrawClass::CriticalUi => "CriticalUi",
            DrawClass::Particles => "Particles",
            DrawClass::Decoration => "Decoration",
            DrawClass::Debug => "Debug",
        }
    }
}

/// How important a draw is, used to order admission and to size the shared-pool
/// headroom protected for more important classes.
///
/// Declared most-important first, so the derived `Ord` makes
/// `Critical < … < Optional` — a *smaller* value is *more* important. Use
/// [`OverlayPriority::is_at_least`] to read that intent without juggling the
/// reversed comparison.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum OverlayPriority {
    /// Must be drawn whenever it fits the hard cap (gameplay, actors).
    Critical,
    /// Strongly preferred (score / critical UI).
    High,
    /// Kept when there is room (reactive particles).
    Normal,
    /// Degraded early (decorative effects).
    Low,
    /// Only uses leftover headroom (debug overlays).
    Optional,
}

impl OverlayPriority {
    /// `true` if `self` is at least as important as `other` (i.e. sorts no later).
    pub const fn is_at_least(self, other: OverlayPriority) -> bool {
        (self as u8) <= (other as u8)
    }
}

/// Immutable per-frame immediate-overlay budget: a hard command cap plus, for
/// each [`DrawClass`], a guaranteed reservation and a shared-pool protection
/// floor. This is the data that was previously just the scalar
/// `MAX_APP_DRAW_COMMANDS`.
///
/// Invariant (checked by [`DrawBudget::is_valid`]): the reservations never sum
/// past `total`, so the shared pool is non-negative and the hard cap is always
/// honourable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DrawBudget {
    /// Hard ceiling on admitted immediate commands per frame. Never exceeded.
    total: u16,
    /// Per-class guaranteed commands. A class always gets at least this many,
    /// regardless of how much the shared pool has already been spent — this is
    /// what stops a flood of decorative draws from starving the head/apple/score.
    reserved: [u16; DRAW_CLASS_COUNT],
    /// Per-class shared-pool floor: when a class draws from the shared pool it
    /// may not take the pool below this many free commands, leaving that headroom
    /// for (typically more important) classes that emit later in the frame. `0`
    /// for Critical/High classes (they may consume the whole pool); larger for
    /// decorative/debug classes so they yield first.
    shared_floor: [u16; DRAW_CLASS_COUNT],
}

impl DrawBudget {
    /// Construct an explicit budget. `reserved` and `shared_floor` are indexed by
    /// [`DrawClass::index`]. `const` so the firmware can derive its compile-time
    /// command-array sizing from a single budget value.
    pub const fn new(
        total: u16,
        reserved: [u16; DRAW_CLASS_COUNT],
        shared_floor: [u16; DRAW_CLASS_COUNT],
    ) -> Self {
        Self {
            total,
            reserved,
            shared_floor,
        }
    }

    /// The hard command cap as a `usize`, for sizing fixed command arrays.
    pub const fn total_commands(&self) -> usize {
        self.total as usize
    }

    /// Sum of all per-class reservations.
    pub fn reserved_total(&self) -> u16 {
        let mut sum = 0u16;
        let mut i = 0;
        while i < DRAW_CLASS_COUNT {
            sum = sum.saturating_add(self.reserved[i]);
            i += 1;
        }
        sum
    }

    /// Commands not reserved to any class: the shared pool every class competes
    /// for after exhausting its own reservation.
    pub fn shared_pool(&self) -> u16 {
        self.total.saturating_sub(self.reserved_total())
    }

    /// Guaranteed reservation for one class.
    pub const fn reserved(&self, class: DrawClass) -> u16 {
        self.reserved[class.index()]
    }

    /// `true` if the reservations fit inside the cap (so the pool is well-defined).
    pub fn is_valid(&self) -> bool {
        self.reserved_total() <= self.total
    }

    /// Decide how much of a `cost`-command immediate draw of `class` to admit,
    /// recording the outcome in `stats`.
    ///
    /// A request is served first from the class's own reservation, then from the
    /// shared pool down to the class's protection floor:
    ///
    /// - All of it fits → [`BudgetDecision::Admit`].
    /// - Some of it fits → [`BudgetDecision::Degrade`] with the admitted count
    ///   (the caller should draw that many and drop / thin the rest).
    /// - None of it fits → [`BudgetDecision::Reject`].
    ///
    /// Because admission never draws from another class's reservation nor past
    /// the shared pool, `stats.total_used()` can never exceed [`Self::total`] —
    /// the cap is honoured by construction, with no tail-drop and no resulting
    /// full repaint. A zero-cost request is trivially admitted and records
    /// nothing.
    pub fn request(&self, stats: &mut BudgetStats, class: DrawClass, cost: u16) -> BudgetDecision {
        if cost == 0 {
            return BudgetDecision::Admit;
        }
        let c = class.index();

        // 1. The class's own guaranteed reservation, always available to it.
        let res_remaining = self.reserved[c].saturating_sub(stats.res_used[c]);
        let from_res = if cost < res_remaining {
            cost
        } else {
            res_remaining
        };

        // 2. The shared pool, but only down to the floor this class must leave
        //    free for more important late-emitting classes.
        let mut from_shared = 0u16;
        let need = cost - from_res;
        if need > 0 {
            let shared_free = self.shared_pool().saturating_sub(stats.shared_used);
            let usable = shared_free.saturating_sub(self.shared_floor[c]);
            from_shared = if need < usable { need } else { usable };
        }

        // Commit the accounting.
        stats.res_used[c] = stats.res_used[c].saturating_add(from_res);
        stats.shared_used = stats.shared_used.saturating_add(from_shared);
        let admitted = from_res + from_shared;
        stats.admitted[c] = stats.admitted[c].saturating_add(admitted);

        if admitted == cost {
            BudgetDecision::Admit
        } else if admitted == 0 {
            stats.rejected[c] = stats.rejected[c].saturating_add(cost);
            BudgetDecision::Reject
        } else {
            stats.degraded[c] = stats.degraded[c].saturating_add(1);
            stats.rejected[c] = stats.rejected[c].saturating_add(cost - admitted);
            BudgetDecision::Degrade { admitted }
        }
    }
}

impl Default for DrawBudget {
    /// The live firmware immediate-overlay budget (see [`APP_DRAW_BUDGET`]).
    fn default() -> Self {
        APP_DRAW_BUDGET
    }
}

/// The default immediate-overlay budget, sized to the firmware's existing
/// per-frame immediate command cap.
///
/// `total` is the current `MAX_APP_DRAW_COMMANDS` value; the firmware derives its
/// command-array sizing from [`DrawBudget::total_commands`] so the cap lives here
/// as data rather than as a bare scattered constant. The reservations and floors
/// are an illustrative starting layout for the KotoSnake-class apps the model
/// targets — they are **not yet consulted by any draw path**, so changing them
/// does not change current visuals.
///
/// Layout (total = 96): 36 reserved (CoreGameplay 16, Actor 8, CriticalUi 12),
/// leaving a 60-command shared pool. Particles may draw the pool down to 8 free,
/// Decoration to 24, Debug to 40 — so decorative and debug draws yield headroom
/// to reactive particles and to late critical draws before the cap is reached.
pub const APP_DRAW_BUDGET: DrawBudget = DrawBudget::new(
    96,
    // CoreGameplay, Actor, CriticalUi, Particles, Decoration, Debug
    [16, 8, 12, 0, 0, 0],
    [0, 0, 0, 8, 24, 40],
);

/// What [`DrawBudget::request`] decided for one immediate draw.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BudgetDecision {
    /// Every requested command fits; draw the whole thing.
    Admit,
    /// Only `admitted` (`> 0`, `< requested`) commands fit; draw that many and
    /// thin / drop the remainder (e.g. fewer particles, shorter rainbow tail).
    Degrade { admitted: u16 },
    /// No commands fit for this class; skip the draw entirely.
    Reject,
}

impl BudgetDecision {
    /// Number of commands the caller may actually draw, given what it requested.
    pub const fn admitted_commands(self, requested: u16) -> u16 {
        match self {
            BudgetDecision::Admit => requested,
            BudgetDecision::Degrade { admitted } => admitted,
            BudgetDecision::Reject => 0,
        }
    }

    /// `true` if any commands were admitted (full or degraded).
    pub const fn is_drawn(self) -> bool {
        !matches!(self, BudgetDecision::Reject)
    }
}

/// Running per-frame admission accounting for a [`DrawBudget`]. Heap-free and
/// `Copy`; [`reset`](BudgetStats::reset) it (or assign `Default`) at frame start.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub struct BudgetStats {
    /// Per-class commands taken from each class's own reservation.
    res_used: [u16; DRAW_CLASS_COUNT],
    /// Commands taken from the shared pool (across all classes).
    shared_used: u16,
    /// Per-class commands admitted (reservation + shared).
    admitted: [u16; DRAW_CLASS_COUNT],
    /// Per-class commands that did not fit (the dropped remainder).
    rejected: [u16; DRAW_CLASS_COUNT],
    /// Per-class count of requests that were partially admitted (degraded).
    degraded: [u16; DRAW_CLASS_COUNT],
}

impl BudgetStats {
    /// A zeroed ledger.
    pub const fn new() -> Self {
        Self {
            res_used: [0; DRAW_CLASS_COUNT],
            shared_used: 0,
            admitted: [0; DRAW_CLASS_COUNT],
            rejected: [0; DRAW_CLASS_COUNT],
            degraded: [0; DRAW_CLASS_COUNT],
        }
    }

    /// Clear the ledger for a new frame.
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Total commands admitted across all classes (reservation + shared). Never
    /// exceeds the budget's `total`.
    pub fn total_used(&self) -> u16 {
        let mut sum = self.shared_used;
        let mut i = 0;
        while i < DRAW_CLASS_COUNT {
            sum = sum.saturating_add(self.res_used[i]);
            i += 1;
        }
        sum
    }

    /// Commands admitted for one class.
    pub fn admitted(&self, class: DrawClass) -> u16 {
        self.admitted[class.index()]
    }

    /// Commands rejected (dropped) for one class.
    pub fn rejected(&self, class: DrawClass) -> u16 {
        self.rejected[class.index()]
    }

    /// Count of degraded (partially admitted) requests for one class.
    pub fn degraded(&self, class: DrawClass) -> u16 {
        self.degraded[class.index()]
    }

    /// Commands drawn from the shared pool across all classes.
    pub fn shared_used(&self) -> u16 {
        self.shared_used
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small, easy-to-reason-about budget: cap 10, one reserved class, the rest
    /// fight over a 6-command shared pool with decoration yielding 4 of it.
    fn budget() -> DrawBudget {
        DrawBudget::new(
            10,
            // CoreGameplay, Actor, CriticalUi, Particles, Decoration, Debug
            [2, 2, 0, 0, 0, 0],
            [0, 0, 0, 0, 4, 4],
        )
    }

    #[test]
    fn default_budget_is_valid_and_matches_cap() {
        assert!(APP_DRAW_BUDGET.is_valid());
        assert_eq!(APP_DRAW_BUDGET.total_commands(), 96);
        assert_eq!(APP_DRAW_BUDGET.shared_pool(), 96 - (16 + 8 + 12));
    }

    #[test]
    fn priority_ordering_is_most_important_first() {
        assert!(OverlayPriority::Critical < OverlayPriority::Optional);
        assert!(OverlayPriority::Critical.is_at_least(OverlayPriority::High));
        assert!(!OverlayPriority::Low.is_at_least(OverlayPriority::Critical));
        assert_eq!(
            DrawClass::CoreGameplay.priority(),
            OverlayPriority::Critical
        );
        assert_eq!(DrawClass::Debug.priority(), OverlayPriority::Optional);
    }

    #[test]
    fn full_request_within_reservation_is_admitted() {
        let b = budget();
        let mut s = BudgetStats::new();
        assert_eq!(
            b.request(&mut s, DrawClass::Actor, 2),
            BudgetDecision::Admit
        );
        assert_eq!(s.admitted(DrawClass::Actor), 2);
        assert_eq!(s.shared_used(), 0); // came entirely from the reservation
    }

    #[test]
    fn critical_reservation_survives_a_decoration_flood() {
        // A decorative effect tries to take the entire surface first; the actor's
        // reservation must still be admitted afterward — critical drawing is
        // admitted first in the sense that its room is guaranteed regardless of
        // emission order.
        let b = budget();
        let mut s = BudgetStats::new();
        // Decoration can only reach the pool down to its floor (4), so it gets at
        // most shared_pool(6) - 4 = 2, even though it asked for the whole surface.
        let d = b.request(&mut s, DrawClass::Decoration, 100);
        assert_eq!(d, BudgetDecision::Degrade { admitted: 2 });
        // The actor's full reservation is still entirely available.
        assert_eq!(
            b.request(&mut s, DrawClass::Actor, 2),
            BudgetDecision::Admit
        );
        assert_eq!(s.admitted(DrawClass::Actor), 2);
    }

    #[test]
    fn low_priority_is_degraded_then_rejected_before_the_cap() {
        let b = budget();
        let mut s = BudgetStats::new();
        // Decoration is held to the pool floor: asks for 5, gets 2 (degraded).
        assert_eq!(
            b.request(&mut s, DrawClass::Decoration, 5),
            BudgetDecision::Degrade { admitted: 2 }
        );
        // A second decorative request gets nothing — its floor is already reached.
        assert_eq!(
            b.request(&mut s, DrawClass::Decoration, 5),
            BudgetDecision::Reject
        );
        // Crucially this happened well below the hard cap: only 2 of 10 used.
        assert!(s.total_used() < b.total_commands() as u16);
    }

    #[test]
    fn higher_priority_keeps_pool_a_low_class_could_not_reach() {
        // Particles (floor 0) may consume the shared pool that Decoration (floor 4)
        // was forced to leave — the floor protects headroom for the better class.
        let b = budget();
        let mut s = BudgetStats::new();
        let _ = b.request(&mut s, DrawClass::Decoration, 100); // takes 2
                                                               // Particles can still take the remaining 4 of the 6-command pool.
        assert_eq!(
            b.request(&mut s, DrawClass::Particles, 4),
            BudgetDecision::Admit
        );
        assert_eq!(s.shared_used(), 6);
    }

    #[test]
    fn accounting_never_exceeds_the_cap_under_overload() {
        // Hammer every class with far more than fits, in priority order and again
        // in reverse, and confirm the total admitted never passes the cap.
        let b = budget();
        let mut s = BudgetStats::new();
        for class in DrawClass::ALL {
            let _ = b.request(&mut s, class, 50);
        }
        for class in DrawClass::ALL.into_iter().rev() {
            let _ = b.request(&mut s, class, 50);
        }
        assert!(s.total_used() <= b.total_commands() as u16);
        // The two reserved criticals each got at least their guaranteed
        // reservation regardless of the overload (CoreGameplay, asked first, also
        // spilled into the shared pool; Actor still got its reserved 2).
        assert!(s.admitted(DrawClass::CoreGameplay) >= b.reserved(DrawClass::CoreGameplay));
        assert!(s.admitted(DrawClass::Actor) >= b.reserved(DrawClass::Actor));
    }

    #[test]
    fn decisions_are_deterministic() {
        // The same sequence of requests against a fresh ledger yields identical
        // decisions and identical accounting every time (no hidden state).
        fn run() -> (BudgetStats, [BudgetDecision; 4]) {
            let b = budget();
            let mut s = BudgetStats::new();
            let d = [
                b.request(&mut s, DrawClass::CoreGameplay, 2),
                b.request(&mut s, DrawClass::Decoration, 5),
                b.request(&mut s, DrawClass::Particles, 5),
                b.request(&mut s, DrawClass::Debug, 5),
            ];
            (s, d)
        }
        assert_eq!(run(), run());
    }

    #[test]
    fn zero_cost_request_is_admitted_and_records_nothing() {
        let b = budget();
        let mut s = BudgetStats::new();
        assert_eq!(
            b.request(&mut s, DrawClass::Debug, 0),
            BudgetDecision::Admit
        );
        assert_eq!(s.total_used(), 0);
    }

    #[test]
    fn admitted_commands_helper_reports_drawable_count() {
        assert_eq!(BudgetDecision::Admit.admitted_commands(7), 7);
        assert_eq!(
            BudgetDecision::Degrade { admitted: 3 }.admitted_commands(7),
            3
        );
        assert_eq!(BudgetDecision::Reject.admitted_commands(7), 0);
        assert!(BudgetDecision::Degrade { admitted: 1 }.is_drawn());
        assert!(!BudgetDecision::Reject.is_drawn());
    }

    #[test]
    fn reservation_then_spills_into_shared_pool_as_degrade_accounting() {
        // CoreGameplay reserves 2 but asks for 5: 2 from its reservation, then 3
        // from the shared pool (floor 0) — all admitted, none rejected.
        let b = budget();
        let mut s = BudgetStats::new();
        assert_eq!(
            b.request(&mut s, DrawClass::CoreGameplay, 5),
            BudgetDecision::Admit
        );
        assert_eq!(s.admitted(DrawClass::CoreGameplay), 5);
        assert_eq!(s.shared_used(), 3);
        assert_eq!(s.rejected(DrawClass::CoreGameplay), 0);
    }
}
