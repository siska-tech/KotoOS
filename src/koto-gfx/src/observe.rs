//! Observe-only immediate-overlay budget metering (GFX-0006B, observe mode).
//!
//! [`budget`](crate::budget) is pure policy: it can decide what an immediate draw
//! list *would* admit, degrade, or reject, but nothing consults it on a draw path
//! yet. This module is the first thing that runs the model against a real frame's
//! immediate commands — and it does so **without changing what is drawn**. The
//! firmware builds its per-frame immediate list ([`AppDrawCommand`]s) exactly as
//! before, then hands the finished list here to record *what
//! [`APP_DRAW_BUDGET`](crate::APP_DRAW_BUDGET) would have decided*. No command is
//! dropped, degraded, reordered, or modified; the budget is run in dry-run only.
//!
//! Classification is deliberately **app-agnostic**. The observer does not know any
//! app's palette or layout — it keys a command into a generic [`DrawClass`] purely
//! from the primitive kind and its on-screen size ([`classify_command`]). That is
//! coarse on purpose: the point of observe mode is to measure pressure against the
//! shared cap with a single generic policy, not to profile one game. (The richer,
//! palette-aware KotoSnake classifier in the `koto-sim` fixture harness is a test
//! fixture; it is not how the firmware observes.)

use crate::budget::{BudgetDecision, BudgetStats, DrawBudget, DrawClass, DRAW_CLASS_COUNT};
use crate::layer::AppDrawCommand;

/// A draw no larger than this on *both* sides is treated as a particle/spark — a
/// small reactive effect (food-pickup burst, hit spark) rather than an actor.
const PARTICLE_MAX_PX: i32 = 6;

/// A rect spanning at least this many pixels on either axis (half the 320px
/// surface) is treated as backdrop/playfield/banner chrome — generic
/// [`DrawClass::CoreGameplay`] — rather than a per-actor draw.
const CORE_MIN_SPAN: i32 = 160;

/// Classify one immediate draw command into a generic [`DrawClass`], app-agnostically.
///
/// This reads only the primitive kind and geometry — never any app palette or
/// layout — so the same rule applies to every app:
///
/// - [`AppDrawCommand::Empty`] → `None` (an unused slot, metered as nothing).
/// - [`AppDrawCommand::Text`] → [`DrawClass::CriticalUi`]: text conveys state
///   (score, lives, prompts) and is generically important UI.
/// - [`AppDrawCommand::Pixels`] → [`DrawClass::Actor`]: a heap pixel blit is an
///   image/sprite, i.e. a gameplay actor.
/// - [`AppDrawCommand::Rect`], by size:
///   - `<= PARTICLE_MAX_PX` on both sides → [`DrawClass::Particles`] (a spark).
///   - `>= CORE_MIN_SPAN` on either axis → [`DrawClass::CoreGameplay`] (a backdrop,
///     playfield, or full-width banner).
///   - otherwise → [`DrawClass::Actor`] (a cell-sized gameplay rect).
///
/// The classifier never returns [`DrawClass::Decoration`] or [`DrawClass::Debug`]:
/// telling those apart from actors needs app semantics the observer deliberately
/// does not have. Observe mode would rather under-credit decoration as an actor
/// than smuggle app-specific knowledge into a generic path.
pub fn classify_command(command: &AppDrawCommand) -> Option<DrawClass> {
    match command {
        AppDrawCommand::Empty => None,
        AppDrawCommand::Text { .. } => Some(DrawClass::CriticalUi),
        AppDrawCommand::Pixels { .. } => Some(DrawClass::Actor),
        AppDrawCommand::Rect { w, h, .. } => {
            let (w, h) = (*w, *h);
            if w <= PARTICLE_MAX_PX && h <= PARTICLE_MAX_PX {
                Some(DrawClass::Particles)
            } else if w >= CORE_MIN_SPAN || h >= CORE_MIN_SPAN {
                Some(DrawClass::CoreGameplay)
            } else {
                Some(DrawClass::Actor)
            }
        }
    }
}

/// What [`APP_DRAW_BUDGET`](crate::APP_DRAW_BUDGET) *would* have decided for one
/// frame's immediate command list — recorded without gating any draw.
///
/// Built by [`observe`](BudgetObservation::observe): every non-[`Empty`] command is
/// classified, consecutive same-class commands are grouped into one logical request
/// (so the model can *degrade* an effect — admit fewer of it — rather than only
/// admit/reject single commands), and each request is dry-run through
/// [`DrawBudget::request`] against a throwaway [`BudgetStats`]. The frame's real
/// command list is untouched.
#[derive(Clone, Copy, Debug)]
pub struct BudgetObservation {
    /// Total observed immediate commands (excludes `Empty` slots).
    total: u16,
    /// Commands classified into each [`DrawClass`], indexed by [`DrawClass::index`].
    /// The sum equals [`total`](Self::total).
    requested: [u16; DRAW_CLASS_COUNT],
    /// The dry-run admission ledger. `stats.total_used()` can never exceed the cap.
    stats: BudgetStats,
    /// The first class the budget would have degraded or rejected this frame, in
    /// emission order — the first place pressure shows up. `None` if every command
    /// would have been admitted in full.
    first_pressure: Option<DrawClass>,
}

impl BudgetObservation {
    /// Meter `commands` against `budget` in dry-run mode and summarise the verdict.
    /// `commands` is the frame's immediate list in emission order (e.g.
    /// `host.draw.commands[..host.draw.len]`).
    pub fn observe<'a, I>(budget: &DrawBudget, commands: I) -> Self
    where
        I: IntoIterator<Item = &'a AppDrawCommand>,
    {
        let mut stats = BudgetStats::new();
        let mut requested = [0u16; DRAW_CLASS_COUNT];
        let mut total = 0u16;
        let mut first_pressure: Option<DrawClass> = None;
        let mut run: Option<(DrawClass, u16)> = None;

        for command in commands {
            let Some(class) = classify_command(command) else {
                continue;
            };
            total = total.saturating_add(1);
            match run {
                Some((c, ref mut cost)) if c == class => {
                    *cost = cost.saturating_add(1);
                }
                _ => {
                    if let Some((c, cost)) = run.take() {
                        Self::meter(
                            budget,
                            &mut stats,
                            &mut requested,
                            &mut first_pressure,
                            c,
                            cost,
                        );
                    }
                    run = Some((class, 1));
                }
            }
        }
        if let Some((c, cost)) = run {
            Self::meter(
                budget,
                &mut stats,
                &mut requested,
                &mut first_pressure,
                c,
                cost,
            );
        }

        Self {
            total,
            requested,
            stats,
            first_pressure,
        }
    }

    /// Dry-run one grouped request and fold its outcome into the running summary.
    fn meter(
        budget: &DrawBudget,
        stats: &mut BudgetStats,
        requested: &mut [u16; DRAW_CLASS_COUNT],
        first_pressure: &mut Option<DrawClass>,
        class: DrawClass,
        cost: u16,
    ) {
        requested[class.index()] = requested[class.index()].saturating_add(cost);
        let decision = budget.request(stats, class, cost);
        if first_pressure.is_none() && !matches!(decision, BudgetDecision::Admit) {
            *first_pressure = Some(class);
        }
    }

    /// Total observed immediate commands this frame.
    pub fn total(&self) -> u16 {
        self.total
    }

    /// Commands classified into `class`.
    pub fn requested(&self, class: DrawClass) -> u16 {
        self.requested[class.index()]
    }

    /// The dry-run admission ledger, for callers that want the full per-class detail.
    pub fn stats(&self) -> &BudgetStats {
        &self.stats
    }

    /// Commands the budget would have admitted (full + degraded), across all classes.
    /// Never exceeds the budget cap.
    pub fn would_admit(&self) -> u16 {
        self.stats.total_used()
    }

    /// Requests the budget would have *partially* admitted (degraded), across all
    /// classes. Counts requests, not commands — one thinned effect is one degrade.
    pub fn would_degrade(&self) -> u16 {
        let mut sum = 0u16;
        for class in DrawClass::ALL {
            sum = sum.saturating_add(self.stats.degraded(class));
        }
        sum
    }

    /// Commands the budget would have rejected (dropped), across all classes.
    pub fn would_reject(&self) -> u16 {
        let mut sum = 0u16;
        for class in DrawClass::ALL {
            sum = sum.saturating_add(self.stats.rejected(class));
        }
        sum
    }

    /// The first class the budget would have degraded or rejected, or `None` if
    /// every command would have been admitted in full.
    pub fn first_pressure(&self) -> Option<DrawClass> {
        self.first_pressure
    }

    /// `true` if any command this frame would have been degraded or rejected — the
    /// signal a low-volume diagnostic can gate on.
    pub fn has_pressure(&self) -> bool {
        self.first_pressure.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::APP_DRAW_BUDGET;

    fn rect(w: i32, h: i32) -> AppDrawCommand {
        AppDrawCommand::Rect {
            x: 0,
            y: 0,
            w,
            h,
            rgb565: 0,
        }
    }

    fn text() -> AppDrawCommand {
        AppDrawCommand::Text {
            x: 0,
            y: 0,
            rgb565: 0,
            bytes: [0u8; crate::MAX_APP_TEXT_BYTES],
            len: 1,
        }
    }

    fn pixels(w: i32, h: i32) -> AppDrawCommand {
        AppDrawCommand::Pixels {
            x: 0,
            y: 0,
            w,
            h,
            off: 0,
            len: (w * h * 2) as u32,
        }
    }

    #[test]
    fn classification_is_geometry_only_and_app_agnostic() {
        assert_eq!(classify_command(&AppDrawCommand::Empty), None);
        assert_eq!(classify_command(&text()), Some(DrawClass::CriticalUi));
        assert_eq!(classify_command(&pixels(16, 16)), Some(DrawClass::Actor));
        // Tiny square => particle; cell-sized => actor; wide/tall => core backdrop.
        assert_eq!(classify_command(&rect(4, 4)), Some(DrawClass::Particles));
        assert_eq!(classify_command(&rect(14, 14)), Some(DrawClass::Actor));
        assert_eq!(
            classify_command(&rect(240, 2)),
            Some(DrawClass::CoreGameplay)
        );
        assert_eq!(
            classify_command(&rect(2, 200)),
            Some(DrawClass::CoreGameplay)
        );
    }

    #[test]
    fn empty_slots_are_not_counted() {
        let commands = [AppDrawCommand::Empty, rect(14, 14), AppDrawCommand::Empty];
        let obs = BudgetObservation::observe(&APP_DRAW_BUDGET, commands.iter());
        assert_eq!(obs.total(), 1);
        assert_eq!(obs.requested(DrawClass::Actor), 1);
    }

    #[test]
    fn light_frame_is_fully_admitted_with_no_pressure() {
        // A handful of actors, some text, a backdrop — well under the cap.
        let mut commands = vec![rect(240, 4)]; // CoreGameplay backdrop
        commands.extend(core::iter::repeat_n(rect(14, 14), 8)); // Actors
        commands.extend(core::iter::repeat_n(text(), 3)); // CriticalUi
        let obs = BudgetObservation::observe(&APP_DRAW_BUDGET, commands.iter());

        assert_eq!(obs.total(), 12);
        assert_eq!(obs.would_admit(), 12);
        assert_eq!(obs.would_degrade(), 0);
        assert_eq!(obs.would_reject(), 0);
        assert_eq!(obs.first_pressure(), None);
        assert!(!obs.has_pressure());
    }

    #[test]
    fn per_class_requested_sums_to_total() {
        let mut commands = vec![rect(240, 4), rect(14, 14), rect(4, 4)];
        commands.push(text());
        commands.push(pixels(8, 8));
        let obs = BudgetObservation::observe(&APP_DRAW_BUDGET, commands.iter());
        let sum: u16 = DrawClass::ALL.iter().map(|&c| obs.requested(c)).sum();
        assert_eq!(sum, obs.total());
    }

    #[test]
    fn overload_shows_pressure_but_never_admits_past_cap() {
        // A flood of tiny particle squares: they share a floored pool, so the budget
        // would degrade/reject the tail well before the hard cap.
        let commands: Vec<AppDrawCommand> = core::iter::repeat_n(rect(4, 4), 200).collect();
        let obs = BudgetObservation::observe(&APP_DRAW_BUDGET, commands.iter());

        assert_eq!(obs.total(), 200);
        assert!(obs.has_pressure());
        assert_eq!(obs.first_pressure(), Some(DrawClass::Particles));
        assert!(obs.would_reject() > 0);
        // The dry run never admits past the cap, by construction.
        assert!(obs.would_admit() <= APP_DRAW_BUDGET.total_commands() as u16);
        // Nothing was lost in the accounting: admitted + rejected == requested.
        assert_eq!(obs.would_admit() + obs.would_reject(), obs.total());
    }

    #[test]
    fn observation_is_deterministic() {
        let commands = [rect(240, 4), rect(14, 14), rect(4, 4), text()];
        let a = BudgetObservation::observe(&APP_DRAW_BUDGET, commands.iter());
        let b = BudgetObservation::observe(&APP_DRAW_BUDGET, commands.iter());
        assert_eq!(a.total(), b.total());
        assert_eq!(a.would_admit(), b.would_admit());
        assert_eq!(a.would_reject(), b.would_reject());
        assert_eq!(a.first_pressure(), b.first_pressure());
    }
}
