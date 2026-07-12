# GFX-0006: KotoGame2D API layer + budgeted-immediate enforcement

- Status: in-progress — **GFX-0006A (API layer) done; GFX-0006B (budget
  *observe* mode) done; GFX-0006C (budget *enforcement*) todo**
- Type: feature
- Priority: P2
- Requirements: FR-PM-1, NFR-DRAW-1, NFR-PERF-1

> **Split into three steps.** This issue bundles a non-behaviour-changing refactor
> (lift the app-facing Game2D semantics into a `koto-game2d` layer) with the one
> behavioural step in the migration (route the immediate list through `DrawBudget`).
> They ship separately so the budget change lands alone, behind golden-frame
> sign-off. The behavioural step is itself split so measurement lands before
> policy: **GFX-0006A** is the API-layer half (**done**); **GFX-0006B** is the
> observe-only budget path that meters the immediate list without gating anything
> (**done**); **GFX-0006C** is the actual budget *enforcement* (**still todo**).
> See the checklist below.

Source of truth: [KOTOGFX_RENDER_MIGRATION_PLAN.md](../../architecture/KOTOGFX_RENDER_MIGRATION_PLAN.md)
§4 Stage 6 + §3, [kotogfx-architecture.md](../../architecture/kotogfx-architecture.md)
§budgeted immediate overlay model,
[KOTO_KOTOSNAKE_BUDGET_OBSERVATION.md](../../devlog/KOTO_KOTOSNAKE_BUDGET_OBSERVATION.md).

Depends on: [GFX-0002](GFX-0002-retained-layer-data-model-into-koto-gfx.md) …
[GFX-0005](GFX-0005-display-service-extraction.md).

## Goal

Lift the app-facing semantics (`tile_set` / `sprite_set` / `text_set` / `present`)
into a `koto-game2d` layer over the `koto-gfx` retained model, then make the
budget model **act**: route the immediate command list through `DrawBudget` at
admission time. This is the first stage that changes the immediate path's
behaviour — separately gated.

## Acceptance Criteria

### GFX-0006A — KotoGame2D API layer (done, no behaviour change)

- [x] `koto-game2d` exposes the app-agnostic retained API over `koto-gfx`; firmware
      hostcall IDs become a thin dispatch shim (IDs unchanged). The new crate
      ([`koto-game2d`](../../../src/koto-game2d/src/lib.rs)) owns the app-facing
      Game2D semantics — `set_tile` / `clear_layer`, `stamp_define`, `sprite_set` /
      `sprite_hide` / `sprite_clear_all`, `text_set` / `text_hide` /
      `text_clear_all`, and the `present()` ack — as pure operations over a borrowed
      [`Game2dScene`] view of the koto-gfx retained POD model. The firmware
      ([`app_host.rs`](../../../src/koto-pico/src/firmware/app_host.rs)) keeps the
      `VmHost` dispatch; each `game2d_*` method now borrows its layers via
      `game2d_scene()` and maps the `Game2dError` back with `map_game2d_result`, so
      the validation order and `HostErrorCode`s are byte-identical.
- [x] No new per-game hostcalls are added (the API-bloat rule): the layer is
      app-agnostic; no opcode, bytecode ABI, hostcall ID, or app source changed.
- [x] **Budget is *not* enforced yet** (deferred to GFX-0006B); the immediate path,
      static-capture routing (`capturing_static`), and immediate-draw compatibility
      paths are untouched.
- [x] `cargo test -p koto-game2d` (7) / `-p koto-gfx` (80) / `-p koto-core` (132) /
      `-p koto-sim` (13 golden frames) pass; `thumbv6m` firmware build green;
      `build_apps.py --check` OK with no bytecode rebuild; firmware-lib clippy adds
      no new finding (the pre-existing `probe_keyboard` bin lint is untouched).
- [ ] **Hardware-only, unverified on the authoring host (no device):**
      KotoBlocks / KotoSnake `phase=160/164` smoke + the golden-frame parity remain
      unchanged on a device. Byte-equivalent by construction (the semantics were
      lifted verbatim; only the call site wraps a `Game2dScene`).

### GFX-0006B — budgeted-immediate *observe* mode (done, no behaviour change)

- [x] The firmware dry-runs each frame's finished immediate command list through
      `koto_gfx::APP_DRAW_BUDGET` to record what it *would* admit/degrade/reject,
      **without gating, dropping, degrading, reordering, or modifying any command**.
      The metering lives in koto-gfx as [`BudgetObservation`](../../../src/koto-gfx/src/observe.rs)
      over a generic, app-agnostic [`classify_command`]: each immediate command is
      classified into a [`DrawClass`] purely from primitive kind + geometry (text →
      `CriticalUi`, heap pixel blit → `Actor`, tiny rect → `Particles`, wide/tall
      rect → `CoreGameplay`, else `Actor`) — no app palette, no per-game branch, no
      KotoSnake-named constants. (The richer palette-aware KotoSnake classifier
      stays a `koto-sim` test fixture; it is *not* how the firmware observes.)
- [x] `APP_DRAW` capacity, hostcall IDs, the bytecode ABI, and every app source are
      unchanged; no bytecode rebuild. The immediate path, static-capture routing,
      and rendering are byte-identical — the observation reads the finished list.
- [x] Compact, low-volume diagnostics: the firmware emits one `phase=168
      app-budget-obs … mode=observe` line per app on the throttled `phase=160`
      cadence, plus a one-shot the first frame pressure appears (latched so
      sustained pressure rides the periodic sample). It carries total observed
      commands, per-class usage, `would_admit` / `would_degrade` / `would_reject`,
      and the first degraded/rejected class.
- [x] `cargo test -p koto-gfx` (86, +6 observe tests) / `-p koto-game2d` (7) /
      `-p koto-core` (132) / `-p koto-sim` (13 golden frames) pass; `thumbv6m`
      firmware build green; `build_apps.py --check` OK with no bytecode rebuild;
      firmware-lib clippy adds no new finding.
- [ ] **Hardware-only, unverified on the authoring host (no device):** `phase=168`
      shows budget observation data for KotoBlocks / KotoSnake and golden-frame
      parity is unchanged on a device (no draw is gated, so visuals are identical by
      construction).

### GFX-0006C — budgeted-immediate enforcement (todo, behavioural)

- [ ] The immediate list is admitted through `koto_gfx::DrawBudget`, with
      reservations right-sized from the observation (the stranded
      CoreGameplay/CriticalUi reservations addressed) — the first step that actually
      drops/degrades a command.
- [ ] KotoBlocks and Sokoban immediate lists are shown to stay **fully admitted**
      (no behaviour change); only classes the observation already shows degrading/
      rejecting below cap are affected.
- [ ] Golden frames prove visual identity at the chosen budget for all three apps;
      hardware `phase=160` `peak`/`ovf` parity where a device is available.

## Notes

Budget enforcement is the one behavioural-policy step in the migration; ship it
behind golden-frame sign-off and right-sized reservations, not the stranded
defaults. GFX-0006B lands the measurement first (observe mode) so GFX-0006C's
reservations are set from real firmware numbers, not guesses — **enforcement
remains disabled until GFX-0006C** (see
[KOTO_BUDGET_OBSERVE_MODE.md](../../devlog/KOTO_BUDGET_OBSERVE_MODE.md)). The retained
layers and the budgeted immediate path are fully separated after GFX-0006C.
