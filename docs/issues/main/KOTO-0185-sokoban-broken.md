# KOTO-0185: Sokoban does not work correctly

- Status: DONE — device-confirmed 2026-07-12 (user: boots fine, clears all stages);
  board texture restored (seams + two-tone) after the first cut read too flat
- Type: bug
- Priority: P2
- Related: KOTO-0094-era Game2D work (sokoban is the package-local tile-sheet
  app), GFX-0013 (static-rebuild diff — sokoban captures its board into the
  retained static layer, the path that changed most recently).

## Symptom

倉庫番 (`apps/sokoban`) is reported as not working correctly (2026-07-11).
The failing behavior is not yet pinned down: launch failure vs. wrong
rendering vs. broken input/rules, and sim vs. device, are all unknown.

First step is a capture: run in KotoSim (`apps/sokoban/scenarios/smoke.txt` /
`showcase.txt`) and on device, and record what "incorrect" means here.

## Context

- The sibling rewrite **KotoSoko was deleted from the repo on 2026-07-11**
  (user decision, same triage that filed this issue) — sokoban is now the only
  Sokoban implementation, so it is worth fixing rather than abandoning.
- The `sokoban_play_uses_retained_game2d_chrome_and_text` fixture expectations in
  `src/koto-sim/tests/fixture_runner.rs` still pass at the harness level, so
  the breakage is likely visual/interactive or device-side — exactly the
  class the fixtures don't see.

## Root cause (2026-07-12) — immediate draw-command overflow on device

Confirmed **device-only** by the user; the sim was always clean. Full sim playthrough
(rules, push, undo, clear celebration, stage advance, all three stages solvable and
well-formed) showed no defect. The break is the immediate command budget:

- Sokoban redrew its entire 28×28 board as immediate `draw_rect` calls **every
  frame** — measured **~340 draw commands/frame** (1353 over 4 frames via the fixture
  harness).
- The device's immediate list is a fixed `commands[MAX_APP_DRAW_COMMANDS]` buffer,
  cap **96** (`koto_gfx::APP_DRAW_BUDGET.total_commands()`). `DeviceRuntimeHost::push`
  returns `NO_MEMORY` past the cap and **silently drops the tail**
  (`src/koto-pico/src/firmware/app_host.rs`). So ~72% of every frame — crates, goals,
  and the porter included — never reached the panel on hardware.
- The sim's `draw_rect` pushes into an **unbounded `Vec`**
  (`src/koto-sim/src/runtime/host.rs`), so it rendered all ~340 and looked perfect;
  the fixtures only asserted host-call *presence*, never that the immediate list fit
  the cap.
- Latent regression: this worked when the immediate cap was 384 and broke when
  KOTO-0135 cut it to 96 (`config.rs` still warns "Do not raise this back to 384").

## Fix (2026-07-12)

Split the board across the retained layers (`apps/sokoban/src/main.koto`):

- The **non-moving art** — floor (flat fill + 1px tile seams), walls (one rect each,
  two-toned by cell parity for a block texture), and empty-goal rings — is captured
  into the **retained static layer, rebuilt once per stage**. Fidelity traded vs. the
  original: the per-cell floor checkerboard, 4-rect 3D wall shading, and goal pulse
  are gone (all blew the caps), but seams + two-tone keep it from reading as flat
  single-colour. Worst stage ≈ 74 of the 80 static commands.
- Only the **crates and porter** stay immediate: **~30 commands/frame** (was ~340),
  well under the 96 cap.
- Device-confirmed by the user: boots and clears all stages. Aesthetic note: the
  retained tile-layer (16×16) alternative was considered but rejected for Sokoban —
  its fixed 10×20 grid at origin (8,0) forces a 160×128 board that collides with the
  top chrome bar and is half the current 28px board's size.

## Regression guard

`src/koto-sim/tests/fixture_runner.rs` now segments a **per-frame immediate command
peak** (by VM frame, matching the device's per-frame `clear_frame`) and a **per-rebuild
static-layer peak**, and the shared runner asserts both stay ≤ their device caps
(96 / `GAME2D_STATIC_CMD_CAP` 80) for **every** fixture — the invariant the old
host-call-count checks missed. Live immediate peaks: sokoban 30, kotorun 92,
kotosnake 34, koto_blocks 39; sokoban static rebuild 70/80. The Sokoban test pins the
immediate cap explicitly and no longer mis-asserts on `draw_pixels` (it draws with
rects).

## Acceptance Criteria

- [x] Symptom reproduced and written down — root-caused above (sim clean; device
      drops ~72% of each frame's draw commands).
- [x] Root-caused and fixed (static-board split; immediate peak 340 → 30).
- [x] Sim fixture extended — universal per-frame immediate-cap guard + explicit
      Sokoban assertion.
- [x] Device smoke of a full level clear — user-confirmed: boots fine and clears all
      stages. First cut read too flat (single-colour walls/floor); texture restored.
