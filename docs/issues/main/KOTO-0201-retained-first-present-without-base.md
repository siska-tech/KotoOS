# KOTO-0201: Retained first present without a full-screen base

- Status: done
- Type: bug
- Priority: P1
- Requirements: FR-RT-5, NFR-RT-2
- Related: KOTO-0128, KOTO-0135, KOTO-0143, KOTO-0178, KOTO-0199, KOTO-0200, GFX-0007

## Goal

Fix the device host's initial-present path so a scene without a full-screen base
composites every retained layer before it becomes the previous-frame baseline.
Apps must not need a dummy background to make retained content visible, and the
fix must preserve legacy partial immediate-mode rendering.

## Acceptance criteria

- [x] Add a regression fixture that models the device failure: the first
  frame configures a retained 20x20 board, populates an initial bounded slice,
  and presents without a full-screen base. The fixture proves that the initial
  damage includes those board cells before the frame can become the baseline.
- [x] The first present without a full-screen base composites retained static,
  board, sprite, and text layers in the fixed scene order, with immediate
  commands remaining the top overlay when present.
- [x] The host updates `previous_draw` only after the pixels represented by the
  retained scene have actually been transferred to display GRAM.
- [x] Initial damage is derived from an explicit empty scene (or an equivalent
  host-owned baseline) and the current scene. No app-authored full-screen base
  or dummy static background is required.
- [x] Representative regression coverage proves first-frame output for the
  retained board and the other retained layer classes rather than covering
  only the tilemap symptom.
- [x] Existing no-base immediate-only apps, including `sample_dirty_rects`,
  retain their partial-update behavior without flicker, panic, or an unintended
  full-screen clear or transfer.
- [x] KotoSim and the device host use the same fixed-order compositor for the
  visible result of the first
  no-base retained frame.
- [x] Remove the temporary full-screen static-background workaround from both
  KOTO-0200 retained tilemap samples and update their integration tests to
  assert that no `GAME2D_STATIC_BEGIN` / `GAME2D_STATIC_END` sequence is needed.
- [x] On device, both KOTO-0200 samples display their complete upper rows, and
  the dirty-rect sample still renders and exits normally.
- [x] `cargo test -p koto-gfx`, `cargo test -p koto-sim`, and
  `cargo check -p koto-pico --target thumbv6m-none-eabi` pass.
- [x] `python harness/build_apps.py --check` and
  `python harness/check_project.py` pass.

## Design notes

- In this issue, "no base" means that the command list has no full-screen base;
  it does not mean that the logical scene is empty.
- `game2d_present` remains a logical present request. Frame orchestration owns
  the initial baseline and must not mark retained state as presented when the
  selected transfer path omitted those layers.
- Prefer bounded initial-scene damage through the shared retained compositor.
  A full-screen repaint should occur only when required by the existing damage
  policy, not merely because the app omitted a base command.
- The direct no-base command replay predates retained layers and still serves
  partial immediate-mode compatibility. Integrate retained first-present
  behavior without silently broadening those transfers.
- GFX-0007 tracks eventual legacy immediate-path deprecation. This issue is the
  concrete correctness fix and does not depend on that deprecation.

## Implementation notes

- `koto-gfx::collect_initial_scene_dirty` derives the visible footprints of
  static commands, populated board cells, visible sprites/text, and immediate
  overlays from an empty surface. The device coalesces and presents that set
  through the normal fixed-order compositor over black.
- `has_retained_scene_content` gates the new route. A no-base app with only
  immediate commands stays on the legacy direct transfer path.
- Automated validation passed on 2026-07-14. Hardware validation of all samples
  was confirmed on 2026-07-14, including both retained tilemap samples and
  `sample_dirty_rects`.
