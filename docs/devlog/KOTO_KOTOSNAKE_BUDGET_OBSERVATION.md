# KotoSnake immediate-overlay budget observation

This is the first wiring of the KotoGFX immediate-overlay budget model
(`koto_gfx::{DrawBudget, DrawClass, BudgetStats, APP_DRAW_BUDGET}`, see
[kotogfx-architecture.md](../architecture/kotogfx-architecture.md) §*budgeted immediate overlay model*)
into anything that runs. It is **observation only**: KotoSnake's immediate draws are
classified and metered against the budget to record *what the model would have decided*,
but nothing is gated — every command is still drawn, so no rendered output changes.

It exists to answer the question the model was built for before we let it act: under a
long snake and food-pickup bursts, **which draw classes actually exceed their budget,
and by how much?**

## How it observes (no app change)

KotoSnake's bytecode, opcodes, hostcalls, and ABI are untouched. The fixture runner
(`src/koto-sim/tests/fixture_runner.rs`) records every immediate `draw_*` the app already
emits (with its geometry + RGB565 colour, and whether it fell inside a
`game2d_static_begin/_end` capture), then:

1. **Classifies** each immediate, non-static draw into a `DrawClass` host-side
   (`classify_kotosnake`) from KotoSnake's existing palette + geometry signatures
   (`apps/kotosnake/src/main.koto`):
   - **Actor** — the snake body (14×14 near-head cells, coalesced tail runs), the head
     chrome/eyes, and the apple body/halo/shine/sparkle.
   - **Particles** — the food-pickup spark pool (small 2..6 px rainbow squares).
   - **Decoration** — the colour-cycling header logo, the eat-flash frame, the "+10"
     popup, the SPEED UP / music banners, and the game-over panel.
   - **CoreGameplay / CriticalUi / Debug** — *none*: KotoSnake's board/grid is on the
     retained Game2D static layer and its score/length/best HUD is retained text, so
     they never reach the immediate path; it emits no debug overlay.
2. **Meters** the frame: consecutive same-class draws are grouped into one logical
   request (so the model can *degrade* an effect — fewer sparks, a shorter tail —
   instead of only admit/reject single commands) and passed to `DrawBudget::request`
   against `APP_DRAW_BUDGET` (the live cap of 96, with its illustrative reservations).

The two tests are `kotosnake_immediate_overlay_budget_observation` (real greedy play)
and `kotosnake_worst_case_long_snake_budget_pressure` (the structural worst case).

## Observed pressure

### Real greedy play — 6000 frames, max length 19

```
class         rsv  requested   admitted   degraded   rejected
CoreGameplay   16          0          0          0          0
Actor           8     207144     207144          0          0
CriticalUi     12          0          0          0          0
Particles       0       4219       4219          0          0
Decoration      0      38583      37457         50       1126
Debug           0          0          0          0          0
```

(`peak_app_draw = 69`, `max_total_used = 60`, both well under the 96 cap.)

- **Actor is always admitted in full** — the body and apple are protected, as intended.
- **Decoration is the pressure real play reaches.** Once the snake body and a particle
  burst have spent the shared pool down past the decoration floor, the *late* overlays
  (eat-flash / popup / banners, emitted after the body) are rejected — 1126 commands
  rejected and 50 requests degraded over the run. The header logo, emitted *first* each
  frame while the pool is still full, is always admitted; emission order matters.
- **Particles still fit** at the lengths the greedy seeker reaches (it traps itself
  around length 19, where the Actor body has not yet claimed enough of the pool to
  squeeze the sparks).

### Structural worst case — one maximally coiled long snake mid-bite

The documented body bound (head 8 + (RICH_N−1)·2 + TAIL_BUDGET = 46 rects, see
[KOTO_KOTOSNAKE_LONG_SNAKE_BUDGET.md](KOTO_KOTOSNAKE_LONG_SNAKE_BUDGET.md)) + the apple
(4) + a full 24-spark burst + eat-flash/popup overlays — 81 immediate commands:

```
class         requested   admitted   degraded   rejected
Actor                50         50          0          0
Particles            24          9          1         15
Decoration            7          1          0          6
```

- **Actor (50) admitted in full** — the body wins its room regardless of emission order.
- **Particles degraded (9 of 24)** — the burst is *thinned*, not dropped wholesale:
  exactly "shed individual particles under pressure".
- **Decoration rejected (6 of 7)** — trailing overlays yield to the gameplay actors.
- **`total_used = 60 < 96`** — all of this shedding happens strictly *below* the hard
  cap, i.e. before the tail-drop + full repaint the cap alone (first-come-first-served)
  would cause.

## The finding: stranded reservations

The default `APP_DRAW_BUDGET` reserves **16 commands for CoreGameplay and 12 for
CriticalUi** — sensible for a KotoBlocks-class app that blits its board and HUD
immediately. KotoSnake uses **neither** on the immediate path (both are retained), so
those **28 reserved commands sit stranded**, shrinking the shared pool the snake body,
particles, and overlays actually compete for. That stranded headroom is the main reason
Particles/Decoration feel pressure here well before the 96 cap.

This is the per-app tuning question the model was designed to expose — and exactly why
the reservations live as data in `koto_gfx::APP_DRAW_BUDGET` rather than as a bare
constant. **No re-tuning is done in this step**: the budget is observed, not enforced,
and the reservation layout is unchanged. The next step (separating the budgeted
immediate path from the retained layers, then enforcing per-app budgets) can use these
numbers to right-size the reservations for KotoSnake-shaped apps.

## Invariants the observation also locks in

- The model never admits past the cap (`total_used ≤ 96`) on any frame — by construction.
- The classifier accounts for *every* immediate command (per-class requested sums to the
  frame's `app_draw`), so the observation is exhaustive and non-destructive.
- A class is never admitted more than it requested.
