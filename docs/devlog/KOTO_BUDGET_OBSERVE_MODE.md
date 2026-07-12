# Immediate-overlay budget — observe mode (GFX-0006B)

**Enforcement is still disabled.** This is the firmware-side, observe-only wiring of
the KotoGFX immediate-overlay budget model
(`koto_gfx::{DrawBudget, DrawClass, BudgetStats, APP_DRAW_BUDGET}`, see
[kotogfx-architecture.md](../architecture/kotogfx-architecture.md) §*budgeted immediate overlay
model*). The device meters each frame's immediate draws against the budget to record
*what the model would decide* — **but nothing is gated**: every command an app emits
is still drawn, so no rendered output changes. Dropping/degrading/reordering happens
only once GFX-0006C turns enforcement on.

This complements the host-side
[KotoSnake budget observation](KOTO_KOTOSNAKE_BUDGET_OBSERVATION.md): that one runs a
palette-aware classifier in the `koto-sim` fixture harness; this one runs on real
hardware with a single **generic** classifier shared by every app.

## What runs

`src/koto-pico/src/firmware/app_runtime.rs`, after the VM step builds the finished
per-frame immediate command list, calls `koto_gfx::BudgetObservation::observe(...)`
over `host.draw.commands[..host.draw.len]`. The observation is a pure dry run against
a throwaway `BudgetStats` — it never touches the real command list, the retained
layers, `APP_DRAW` capacity, hostcall IDs, the bytecode ABI, or any app source.

### Generic classification (no app profiles)

`koto_gfx::classify_command` keys each immediate command into a `DrawClass` purely
from its primitive kind and on-screen geometry — never an app palette or layout:

| Command                        | DrawClass        |
| ------------------------------ | ---------------- |
| `Text`                         | `CriticalUi`     |
| `Pixels` (heap blit)           | `Actor`          |
| `Rect` ≤ 6px on both sides     | `Particles`      |
| `Rect` ≥ 160px on either axis  | `CoreGameplay`   |
| `Rect` (other)                 | `Actor`          |
| `Empty`                        | *(not counted)*  |

It never emits `Decoration` or `Debug`: separating those from actors needs app
semantics the observer deliberately does not have. Consecutive same-class commands
are grouped into one logical request, so the model can *degrade* an effect (admit
fewer of it) rather than only admit/reject single commands.

## Diagnostics

One low-volume UART line per app, on the throttled `phase=160` cadence plus a
one-shot the first frame any command would be degraded/rejected (latched, so
sustained pressure rides the periodic sample instead of spamming):

```
phase=168 app-budget-obs app=<id> frame=<n> mode=observe total=<commands>
  core=<n> actor=<n> ui=<n> part=<n> decor=<n> debug=<n>
  would_admit=<n> would_degrade=<n> would_reject=<n> first_pressure=<class|none>
```

- `total` — observed immediate commands this frame (excludes empty slots).
- `core`/`actor`/`ui`/`part`/`decor`/`debug` — commands per `DrawClass`; they sum to
  `total`.
- `would_admit` — commands the budget would admit (full + degraded); never exceeds
  the cap.
- `would_degrade` — *requests* the budget would partially admit (thin an effect).
- `would_reject` — commands the budget would drop.
- `first_pressure` — the first class that would degrade/reject, or `none` when the
  whole frame fits.

Because nothing is gated, `would_degrade`/`would_reject` being non-zero is **not** a
visible drop today — it is the measurement GFX-0006C will use to right-size the
`APP_DRAW_BUDGET` reservations before enforcement is switched on.

## CommandCountShift correlation (GFX-0006C pre-work)

Before GFX-0006C enforces the budget, we need to know whether enforcement would
actually fix the remaining heavy frames, or whether those frames are heavy for a
reason the budget cannot touch. The dominant heavy-frame cause for the retained apps
is the **full repaint attributed to `CommandCountShift`** — the immediate
command-list length changed (a board grew, a line cleared), which misaligns the
positional command diff and forces a whole-surface recompose (`koto_gfx::repaint`).

A `CommandCountShift` is *list-length* pressure; the budget meters *over-budget* draw
pressure. They are different problems, and only one of them is something enforcement
addresses. So on every frame whose `phase=160` line shows
`full_reason=CommandCountShift`, the firmware also emits a correlation line (same
generic classification as `phase=168`, caller-gated to shift frames, one-shot on the
first plus the throttled sample):

```
phase=169 app-cmdshift app=<id> frame=<n> reason=CommandCountShift
  prev_cmds=<n> cur_cmds=<n> obs_total=<commands>
  core=<n> actor=<n> ui=<n> part=<n> decor=<n> debug=<n>
  would_admit=<n> would_degrade=<n> would_reject=<n> first_pressure=<class|none>
  rects_pre=<n> rects_post=<n>
```

- `prev_cmds`/`cur_cmds` — the immediate command counts that differed to trip the
  shift (`cur_cmds != prev_cmds` is why this frame full-repainted).
- `obs_total` + per-class + `would_admit`/`would_degrade`/`would_reject`/
  `first_pressure` — the same observe-only budget verdict as `phase=168`, for this frame.
- `rects_pre` — the pre-coalesce dirty-rect count the diff had collected when it
  escalated; `rects_post` is `0` (the full-repaint path does not coalesce). On
  incremental frames this geometry rides the `phase=164` line instead.

### How to read it

| `phase=169` signature                                           | Interpretation | Does enforcement help? |
| --------------------------------------------------------------- | -------------- | ---------------------- |
| `would_degrade=0 would_reject=0 first_pressure=none`, small `rects_pre` | The list fits the budget; the repaint is purely the count shift misaligning the positional diff. | **No.** Needs a separate count-shift policy (e.g. index-stable diffing so a growing/shrinking list does not force a full recompose), not budget gating. |
| `would_degrade>0` or `would_reject>0` (or large `rects_pre`/`obs_total`) | The frame is genuinely over the immediate-draw budget *and* shifting its count. | **Partly.** Enforcement would thin/drop the over-budget tail; but right-size reservations from the per-class numbers here, and note enforcement also changes the count, so the shift policy may still be needed. |

The takeaway GFX-0006C is waiting on: if the heavy `CommandCountShift` frames
consistently show `first_pressure=none`, budget enforcement is the wrong lever for
them and the count-shift repaint needs its own fix; if they show real degrade/reject
pressure, the `phase=169` per-class numbers are what the GFX-0006C reservations should
be sized from.
