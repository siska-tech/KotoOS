# KotoGFX rendering realignment — staged migration plan

> **Status: architecture / migration report.** No rendering behaviour, VM
> semantics, opcode, hostcall ID, bytecode ABI, PSRAM/LCD/CodeWindow/audio
> behaviour, or app (KotoBlocks/KotoSnake/Sokoban) behaviour is changed by this
> document. It maps where rendering responsibilities live *today*, where the
> KotoGFX architecture and the KotoOS resource-ownership model say they *should*
> live, and a sequence of small, individually behaviour-preserving commits to
> close the gap. It does **not** authorise a rewrite.

## 0. Source documents

This plan follows:

- [`design/KOTOOS_RESOURCE_OWNERSHIP.md`](../design/KOTOOS_RESOURCE_OWNERSHIP.md) —
  the resource-ownership model. The clauses that bind this migration:
  - §1: physical resource vs logical object — "display present requestはSPIやDMA
    の予約ではなく、display serviceへの要求である."
  - Resource table: *SPI LCD bus* owner = "koto-gfx display service + HAL SPI
    backend"; *display framebuffer/surface* owner = "koto-gfx", normal-app access
    = "logical app surfaceを所有。物理LCDは不可".
  - §3 CPU0/CPU1 policy: CPU1 is a **single fixed owner**; "Display-owned CPU1"
    (SPI flush + dirty-rect conversion + surface composition) is a named
    candidate. Services must not race for CPU1.
  - §4 Display ownership — the canonical present flow: app draws to surface →
    dirty rect → **present request** → display service merges damage and
    composites system overlay/status bar → flush task drives SPI/DMA/PIO. "app
    はpresentを『可視LCDを所有した証拠』ではなく『表示要求』として扱う."
  - §9 hostcall boundary: normal-app display hostcalls = surface create, dirty
    rect, present, logical size query; overlay/status-bar/foreground-takeover/
    capture are **system-app only**.
- [`research/piece-analysis/02b_resource_reservation_model.md`](../research/piece-analysis/02b_resource_reservation_model.md)
  — the P/ECE-derived rationale: LCD physical transfer must be a display-flush-task
  internal resource (handing it to apps corrupts DMA/serial/LCD ports at once);
  apps get *buffer + flush request*, never the bus.
- [`kotogfx-architecture.md`](kotogfx-architecture.md) — the canonical KotoGFX
  design: retained Tile/Sprite/Text pipeline, the `koto-display` flush layer, the
  immediate-API separation, and the budgeted immediate-overlay model.
- [`KOTOGFX_CRATE_BOUNDARY.md`](KOTOGFX_CRATE_BOUNDARY.md) — the v0 extraction
  rules and the dependency ordering `koto-gfx ← koto-core ← koto-pico/koto-sim`.
- [`GAME2D_RETAINED_RENDER_ARCHITECTURE.md`](GAME2D_RETAINED_RENDER_ARCHITECTURE.md)
  — the retained layer model and the "immediate = debug/overlay/fallback only" rule.
- [`ARCHITECTURE.md`](ARCHITECTURE.md) — `ARCHITECTURE.md:77` "The HAL owns the
  physical LCD transfer strategy. Core code describes what changed; the backend
  decides how to push pixels."

## 1. Current rendering responsibility map

The live path is, per frame: VM step mutates host state → frame loop diffs the
host double-buffer → `present_app_delta` collects dirty rects from five sources,
runs `FullRepaintPolicy`, coalesces, and for each surviving rect composites the
whole layer stack into a strip and flushes it to the LCD.

| # | Responsibility | Concrete symbols | File |
| :-- | :-- | :-- | :-- |
| R1 | Rect type, dirty-rect/tile coalescing | `Rect`, `coalesce_rects`, `coalesce_dirty_tiles`, `TileBand` | `koto-gfx` (`rect.rs`, `dirty.rs`) |
| R2 | Full-repaint policy | `FullRepaintPolicy::decide`, `DeltaInputs`, `DeltaDecision`, `FullRepaintReason`, `FULL_REPAINT_AREA/RECTS` | `koto-gfx` (`repaint.rs`) |
| R3 | Dirty-rect fragmentation diagnostics model | `DirtyRectGeometry`, `DIRTY_SAMPLE_QUADS` | `koto-gfx` (`stats.rs`) |
| R4 | Budgeted-immediate admission model (observation only) | `DrawBudget`, `DrawClass`, `OverlayPriority`, `BudgetDecision`, `BudgetStats`, `APP_DRAW_BUDGET` | `koto-gfx` (`budget.rs`) |
| R5 | Retained state store + immediate command buffer | `DeviceRuntimeHost` (`board`, `sprites`, `stamps`, `text_items`, `commands`, `static_layer`), `AppStaticLayer`, `Game2dSprite`, `Game2dText`, `Game2dStampDef`, `AppDrawCommand` | `firmware/app_host.rs` |
| R6 | VM hostcall boundary (Game2D + immediate draw) | `game2d_set_tile`, `game2d_sprite_set/hide/clear_all`, `game2d_stamp_define`, `game2d_text_set/hide/clear_all`, `game2d_static_begin/end`, `game2d_present`, `draw_rect/pixels/text`, `push_draw` | `firmware/app_host.rs` |
| R7 | Layer compositor (CPU raster into strip) | `paint_app_commands`, `paint_board_layer`, `paint_sprite_layer`, `paint_text_layer`, `paint_command_list`, `stamp_cell` | `firmware/app_render.rs` |
| R8 | Dirty derivation (footprints/diff) | `command_dirty_rect`, `app_command_rect`, `sprite_dirty_rect`, `sprite_footprint_rect`, `text_dirty_rect`, `text_footprint_rect`, `board_band_rect`, `push_dirty` | `firmware/app_render.rs` |
| R9 | App-surface clipping / geometry (hardcoded 320×320) | `clip_app_rect`, `union_rect`, `is_full_screen_base`, `full_screen_base_color/_in`, the literal `320` in the full-repaint loop and text band | `firmware/app_render.rs` |
| R10 | Present orchestration (collect → decide → coalesce → compose → flush) | `present_app_delta`, `present_app_commands`, `present_rect_banded` | `firmware/app_render.rs` |
| R11 | Frame loop / present trigger / double-buffer diff trigger | the `present_app_*` call site, `*host.draw != *previous_draw` gate | `firmware/app_runtime.rs` |
| R12 | Per-frame timing metrics (hardware-coupled) | `PaintMetrics` (`embassy_time::Instant`), `record_raster/transfer/dirty_geometry`, `mark_full_repaint` | `firmware/diag.rs` |
| R13 | `phase=160` app-frame, `phase=164` dirty-rects, `phase=161` command sample UART diagnostics | `log_app_frame_metrics`, `log_dirty_rect_geometry`, `log_command_sample` | `firmware/diag.rs`, `firmware/app_render.rs` |
| R14 | Rasterizer surface + font + colour | `Canvas`, `BitmapFont`, `Rgb565` | `koto-core` |
| R15 | Display backend (SPI/DMA/LCD window flush) | `PicoCalcLcd::{fill, fill_rect, write_rgb565_rect, init}`, `rgb565_to_rgb888`, `Rgb888` | `koto-pico/src/lcd.rs`, `app_render.rs` |
| R16 | Hardware bring-up / panic / fallback immediate | `present_pixel_diagnostic` (KOTO-0129 tile), the immediate command list as top z-layer | `firmware/app_render.rs` |

### What is already correctly placed

- **R1–R4** already live in `koto-gfx` (the v0 extraction). Correct.
- **R12–R13** are intentionally hardware-coupled (they read `embassy_time` and
  write UART) and correctly stay in firmware `diag`.
- **R15** is correctly the HAL/display backend: `PicoCalcLcd` owns the SPI/DMA
  window transfer. Per `ARCHITECTURE.md:77` this is exactly where it belongs.
- **Raw-hardware ownership at the VM boundary is already satisfied.** Apps reach
  the panel *only* through hostcalls (R6) that mutate host state (R5); no app
  touches `PicoCalcLcd`, SPI, or DMA. The resource-ownership rule "通常アプリは
  raw hardwareへ直接アクセスしない … 物理LCDは不可" already holds — this migration
  does **not** need to claw hardware back from apps; it needs to relocate the
  *host-side* compositor and present policy into the right crates and interpose a
  real display service.

### What is misaligned (the actual gap)

- **R7, R8, R9, R10 are the KotoGFX core compositor + dirty tracker + present
  policy, but they live in firmware `app_render.rs`.** Per
  `kotogfx-architecture.md` §*gfx core層 (koto-gfx)* the compositor, dirty
  tracker, and layer state are the heart of `koto-gfx`; today they are firmware
  functions hardcoded to a 320×320 panel.
- **R5 commingles two concerns in `DeviceRuntimeHost`:** the *retained layer data
  model* (which is hardware-independent POD and belongs in `koto-gfx`) and the
  *VM hostcall landing pad* (which belongs at the VM hostcall boundary). They are
  one struct today.
- **R10 (present orchestration) is not a display service.** It directly owns both
  the koto-gfx-class compose decision *and* the `lcd.write_rgb565_rect` flush in
  one function. Per `KOTOOS_RESOURCE_OWNERSHIP.md` §4 the target wants a distinct
  **display service** that (a) receives a *present request*, (b) merges damage,
  (c) composites the system overlay / status bar, then (d) hands a flush task to
  the HAL SPI backend. Today there is **no damage-merge-and-overlay seam at all**:
  the path goes app-state → compose → LCD with no place to insert a status bar or
  a system-menu takeover, and `present` is effectively "paint to the live panel"
  rather than a request the service may reorder, freeze, or overlay.
- **There is no explicit "app surface" object.** The resource model makes the app
  surface the koto-gfx-owned logical drawing target; today it is implicit in
  `DeviceRuntimeHost`'s scattered fields. An explicit surface is what later lets a
  launcher/system-menu *takeover* freeze/hide/replace an app's surface (§4) — out
  of scope to build now, but the relocation should not foreclose it.
- **R14 (`Canvas`) lives in `koto-core`, below the compositor that uses it.** The
  rasterizer surface is a graphics-core concern; its current home blocks lifting
  R7 into `koto-gfx` without a dependency inversion.

## 2. Responsibility classification (target layers)

Each current responsibility, assigned to the target layer named by
`kotogfx-architecture.md`:

| Target layer | Owns | From |
| :-- | :-- | :-- |
| **KotoGFX core** (`koto-gfx`) | Rect + coalescing (R1); full-repaint policy (R2); dirty-geometry model (R3); budget model (R4); **retained layer data model** (the POD half of R5); **layer compositor** (R7); **dirty derivation** (R8); **app-surface geometry** parameterised by surface dims (R9); **rasterizer surface** (R14) | already there (R1–4); move R7/R8/R9/R14 + POD-of-R5 here |
| **KotoGame2D API layer** (`koto-game2d`) | The app-facing semantic API over the retained layers — `tile_set`, `sprite_set`, `text_set`, `present` request — app-agnostic, no per-game calls | new thin layer wrapping R6's semantics |
| **Display service** | The §4 present flow: receive a **present request**, merge damage, composite system overlay / status bar, run `FullRepaintPolicy`, coalesce, drive compose (koto-gfx) → flush (HAL); serialize bus use; own the present trigger. Owns no app state — only the surface registry + damage. (Overlay/status-bar/takeover are system-app-only per §9; not built now, but the seam is created here.) | R10 + the present half of R11 |
| **Firmware diagnostics** | `PaintMetrics` timing (R12); `phase=160/161/164` UART (R13); `present_pixel_diagnostic` (R16) | stays in firmware |
| **HAL / display backend** (`koto-display` / `lcd.rs`) | SPI/DMA/LCD window flush; colour conversion (R15) | stays; R15 is correct |
| **VM hostcall boundary** | Argument decode + dispatch of Game2D/immediate hostcalls into KotoGame2D state (the dispatch half of R6); the hostcall landing fields' lifetime | stays in firmware `app_host`, but delegates state layout to koto-gfx |

The **frame loop** (R11) stays in firmware `app_runtime`: it owns input, VM
stepping, and pacing, and *calls* the display service.

## 3. Which immediate APIs stay, and as what

`kotogfx-architecture.md` §*immediate APIとの関係* and §*budgeted immediate
overlay model* require three immediate roles to survive — and only these three:

1. **Panic / debug / fallback immediate.** Keep. This is `present_pixel_diagnostic`
   (hardware bring-up tile) and the immediate command list in its role as the
   **top z-layer for debug overlays, transitions, and one-off custom pixels**
   (`paint_command_list` over `host.commands`, painted last in
   `paint_app_commands`). This is the BSOD/debug-overlay path the architecture
   explicitly preserves. It must never re-enter the main retained pipeline.
2. **Budgeted immediate overlay.** Keep, and make it the *managed* immediate
   path. KotoSnake's rainbow body, food sparks, eat-flash, popups, and banners
   are genuinely immediate effects (per the budget observation,
   [KOTO_KOTOSNAKE_BUDGET_OBSERVATION.md](../devlog/KOTO_KOTOSNAKE_BUDGET_OBSERVATION.md)).
   These should route through `koto_gfx::DrawBudget` (R4) — today observation-only
   — so important effects are reserved a seat and decorative ones degrade/reject
   *before* the `MAX_APP_DRAW_COMMANDS` tail-drop + full repaint.
3. **Legacy compatibility path.** Keep, deprecated. Apps that still emit a
   full-screen base `Rect` every frame and per-frame `draw_text`/`draw_rect`
   (detected by `full_screen_base_in`, handled by `present_app_commands`) must
   keep working byte-for-byte — **no bytecode rebuild is in scope.** This path is
   marked deprecated in docs and removed only after all shipped apps migrate to
   retained layers, which is out of this plan's scope.

No new per-game immediate hostcalls are added (the §*API肥大化* risk). The budget
model already shares only the *cap value* (`APP_DRAW_BUDGET`), not per-app policy.

## 4. Staged migration plan (small, behaviour-preserving commits)

Methodology, identical to the proven v0 extraction
(`KOTOGFX_CRATE_BOUNDARY.md`): **lift verbatim, re-export from the old path, change
no pixels and no timing.** Each stage is one reviewable commit (or a tight
cluster), builds for `thumbv6m-none-eabi`, and is gated by the test set in §6.
Each stage is independently revertable and ships value on its own.

### Stage 1 — App-surface geometry into koto-gfx *(first safe code-moving step — see §5)*

Move the pure surface math (R9: `clip_app_rect`, `union_rect`) into `koto-gfx`
as surface-dimension-parameterised `Rect` helpers; firmware keeps identically
-signed private wrappers that pass `320, 320`. Pure integer geometry, no Canvas,
no host state, no timing, no transfer. Zero call-site churn, zero pixel change.

### Stage 2 — Retained layer data model into koto-gfx (split R5)

Move the POD layer types — `AppDrawCommand`, `Game2dSprite`, `Game2dText`,
`Game2dStampDef`, the board cell array shape, `AppStaticLayer` — into `koto-gfx`
as plain `no_std` structs; `app_host.rs` re-exports them and keeps the
hostcall methods. This splits "state layout" (koto-gfx) from "VM hostcall landing
pad" (firmware) **without changing the hostcall IDs, the dispatch, or the field
bytes**. `DeviceRuntimeHost` keeps the *instances* (its diff double-buffer is
firmware-owned for now, per the SRAM-doubling note in
`GAME2D_RETAINED_RENDER_ARCHITECTURE.md` §9).

### Stage 3 — Dirty derivation into koto-gfx (R8)

Move `*_footprint_rect`, `*_dirty_rect`, `board_band_rect`, `push_dirty` onto the
Stage-2 POD types. `present_app_delta`'s collect loops call into koto-gfx instead
of local fns. Pure geometry over the moved types; the FullRepaintPolicy call is
already koto-gfx (R2). No timing/transfer touched.

### Stage 4 — Rasterizer + compositor into koto-gfx (R7, R14)

Resolve the dependency inversion: move `Canvas` (R14) from `koto-core` into
`koto-gfx` (re-exported from `koto-core` so `shell_render` and other consumers are
unchanged), then move `paint_board/sprite/text/command_list` + `paint_app_commands`
(R7) into koto-gfx as the **chunk compositor** operating on a koto-gfx `Canvas` +
the Stage-2 layer model + the app heap slice. Still CPU raster into a caller-owned
strip; the firmware passes the strip in. Golden-frame parity is the gate here —
this is the highest-risk stage and rides the existing `present_rect_banded`
"clear-to-base, paint-clipped" path unchanged.

### Stage 5 — Extract the display service (R10 + present half of R11)

Introduce a display-service seam: a `present(frame_inputs, surface, &mut lcd,
strip, scratch, &mut metrics)` entry that runs collect (R8) → **damage merge** →
**overlay/status-bar composite hook** → decide (R2) → coalesce (R1) → compose
(R7, koto-gfx) → flush (R15, HAL). The firmware frame loop (R11) issues a
**present request** and no longer inlines the compose/flush decision — realising
`KOTOOS_RESOURCE_OWNERSHIP.md` §4 ("present is a request, not proof of owning the
live panel"). The overlay/status-bar hook is a **no-op placeholder** in this stage
(a system-app-only capability per §9, not built here) — its purpose is only to
create the seam where it will live, so present is no longer a straight shot to the
LCD. `PaintMetrics` (R12) and the `phase=160/164` logs (R13) stay in firmware and
are threaded through unchanged.

**CPU0/CPU1 note (no behaviour change in scope).** Per §3 the display service is a
candidate "Display-owned CPU1" single owner, and KotoOS already has a CPU1
render-prep worker (KOTO-0147). This stage keeps whatever the current CPU0/CPU1
split is — it only draws the service boundary so a *future* decision to pin the
display service to `system_service_core` is a localised change, not a rewrite. Do
**not** re-home work across cores as part of this migration.

### Stage 6 — KotoGame2D API layer + budgeted-immediate enforcement (R4, R6)

Lift the app-facing semantics (`tile_set`/`sprite_set`/`text_set`/`present`) into
a `koto-game2d` layer over the koto-gfx retained model, keeping the firmware
hostcall IDs as a thin dispatch shim. Then route the immediate command list
through `DrawBudget` (R4) at admission time, using the observed KotoSnake
reservation numbers to right-size reservations for KotoSnake-shaped apps. This is
the first stage that *acts* on the budget rather than observing — gated by golden
frames proving visual identity at the chosen budget, and explicitly **not** a
behaviour change for KotoBlocks/Sokoban (whose immediate lists stay within
budget).

### Stage 7 — Deprecate legacy per-frame-base path (docs only here)

Document the legacy immediate-base path as deprecated; schedule its removal only
after all shipped apps emit retained layers. No code or bytecode change in scope.

**Dependency order:** 1 → 2 → 3 → 4 → 5 are strictly sequential (each consumes the
prior). 6 depends on 2–5. 7 is docs-only and can land anytime. Stages 1–3 are
low-risk pure-geometry/data moves; Stage 4 is the pixel-parity-critical one;
Stage 5 is structural but mechanical; Stage 6 is the only behavioural-policy step
and is separately gated.

## 5. First safe code-moving step (explicit)

**Stage 1 is the first safe move.** Concretely, the single commit is:

- In `koto-gfx` `rect.rs`, add two pure, surface-parameterised helpers with unit
  tests:
  - `Rect::clip(x: i32, y: i32, w: i32, h: i32, surf_w: i32, surf_h: i32) -> Option<Rect>`
    — the body of `clip_app_rect` with `320` replaced by `surf_w/surf_h`.
  - `Rect::union_clipped(a: Rect, b: Rect, surf_w: i32, surf_h: i32) -> Option<Rect>`
    — the body of `union_rect`, delegating its final clip to `Rect::clip`.
- In `firmware/app_render.rs`, reduce the existing private `clip_app_rect` /
  `union_rect` to one-line delegations passing `320, 320`. **Their signatures are
  unchanged, so none of the ~8 call sites change.**

Why this is safe:
- Pure integer geometry: no `Canvas`, no `DeviceRuntimeHost`, no `embassy_time`,
  no LCD. It cannot change a pixel or a microsecond.
- `clip_app_rect`'s callers already pass already-clipped rects into `union_rect`,
  so the union's final clip is a no-op today and stays one — but it is preserved
  verbatim, so even that assumption is not relied upon.
- `koto-gfx` stays `no_std`, heap-free, dependency-free (only `Rect`), so the
  `thumbv6m-none-eabi` build is unaffected.
- It does not touch R5/R6/R10–R13/R15 at all, so VM semantics, opcodes, hostcall
  IDs, bytecode ABI, PSRAM/LCD/CodeWindow/audio, and the three target apps are
  untouched by construction.

> **Not applied in this commit.** This report ships the plan only. Stage 1 is a
> firmware (`koto-pico`) edit, and behaviour-preservation for the firmware path
> must be confirmed by a `thumbv6m-none-eabi` build + the golden-frame suite,
> which is not available on the authoring host. The diff above is small enough to
> apply and verify as its own PR.

## 6. Tests required at each stage

Baseline gate for **every** stage (nothing below may regress):

- `cargo build -p koto-gfx` and `cargo build -p koto-pico --target thumbv6m-none-eabi --bins`
  (firmware is not in the default clippy gate — lint it explicitly, per the
  clippy-gate note).
- `cargo test -p koto-gfx` (pure unit tests).
- `cargo test -p koto-pico` — the in-file `app_render` invariants
  (`disappeared_immediate_rect_dirties_its_old_footprint`,
  `disappeared_immediate_text_…`, `moved_immediate_command_dirties_union_…`,
  `stable_empty_immediate_list_is_clean`).
- `cargo test -p koto-sim` — the `fixture_runner` golden-frame parity tests
  (KotoBlocks / KotoSnake / Sokoban) and the budget observation tests
  (`kotosnake_immediate_overlay_budget_observation`,
  `kotosnake_worst_case_long_snake_budget_pressure`).

Per-stage additions:

| Stage | Additional tests |
| :-- | :-- |
| 1 | koto-gfx unit tests for `Rect::clip`/`union_clipped`: off-screen, partial-overlap, zero-area, saturating bounds, surf_w/h other than 320; assert firmware wrappers return identical rects to pre-move for a sampled grid. |
| 2 | Struct-size/layout assertions (`size_of`/`align_of`) on the moved POD types equal pre-move; re-export path compiles for every consumer; golden frames unchanged. |
| 3 | Property tests: moved footprint/union fns equal the old ones on randomised sprite/text/board inputs; golden frames + the four app_render invariants unchanged. |
| 4 | **Golden-frame pixel parity is mandatory** (the `ImageChops` method from KOTO-0137/0138; capture via the KotoSim `--image` BMP→PNG workflow). Per-frame compose output must be byte-identical for a full KotoBlocks game, a KotoSnake run, and Sokoban. Confirm `shell_render` (a `Canvas` consumer) still builds and renders. |
| 5 | Assert `phase=160`/`phase=164` field set and values are identical pre/post (diagnostics threaded, not changed); golden frames unchanged; the present-trigger gate (`*host.draw != *previous_draw \|\| static_rebuilt`) fires on the same frames. |
| 6 | Golden frames prove visual identity at the chosen budget for all three apps; budget enforcement changes *only* classes that the observation shows already degrade/reject below the cap; KotoBlocks/Sokoban immediate lists shown to stay fully admitted. Hardware UART `phase=160` `peak`/`ovf` parity where a device is available. |
| 7 | Docs only — no test change. |

Where a device is available, confirm hardware parity by diffing `phase=160`
(`raster_us`, `transfer_us`, `dirty_px`, `rects`, `full`, `full_reason`) and
`phase=164` lines for a fixed scripted session before/after each code stage.

## 7. Risks

- **Dependency inversion at Stage 4 (highest).** `Canvas` lives in `koto-core`,
  below the compositor. Moving it into `koto-gfx` ripples `koto-core` consumers
  (notably `shell_render`). *Mitigation:* re-export `Canvas` from `koto-core` so
  no consumer path changes; gate on the full golden-frame suite.
- **Pixel drift in the compositor move (Stage 4).** The clipped-recomposite must
  reproduce exactly the static→board→sprite→text→immediate z-order and the
  `clear-to-base` semantics. *Mitigation:* verbatim move, ride the existing
  `present_rect_banded` path, mandatory byte-level golden parity.
- **SRAM regression.** Per the firmware-stack-headroom and budget notes, every
  field added to `DeviceRuntimeHost` doubles (current + previous diff buffers),
  and the 8 KiB code window cannot grow. *Mitigation:* Stages 2–3 move *types*,
  not instances, so footprint is unchanged; keep single-instance retained state
  (static layer, stamp defs) out of the doubled host; re-measure free SRAM each
  stage (currently ~84 KiB free).
- **CodeWindow / refill sensitivity.** Moving compositor code between crates can
  shift the firmware code layout and perturb 8 KiB code-window refills (the
  KOTO-0156/0159 work shows this is sensitive). *Mitigation:* watch `refills=` /
  `code_tiles=` on `phase=160` before/after Stage 4–5; treat a refill regression
  as a blocking finding, not a cosmetic one.
- **Diagnostic drift.** `PaintMetrics` and `phase=160/164/161` must keep emitting
  identical fields or the hardware triage corpus is invalidated. *Mitigation:*
  keep R12/R13 in firmware; thread, don't rewrite; field-parity assertion at
  Stage 5.
- **Scope creep into a rewrite.** The temptation at Stage 4–5 is to also build the
  PSRAM-backed surface and the real chunk pipeline. *Mitigation:* those are
  explicitly out of scope (`KOTOGFX_CRATE_BOUNDARY.md` §*Not in v0*); this plan
  only *relocates* the existing CPU compositor, it does not replace it.
- **Behavioural change leaking into Stage 6.** Budget enforcement is the one stage
  that can change pixels. *Mitigation:* enforce only where the observation already
  shows sub-cap degrade/reject; prove KotoBlocks/Sokoban unaffected; ship behind
  golden-frame sign-off and the right-sized reservations, not the stranded
  defaults.
- **Display-service seam scope creep (Stage 5).** The §4 flow names damage merge,
  overlay/status-bar composition, and system-menu takeover. The temptation is to
  build them here. *Mitigation:* Stage 5 creates the *seam* only — the overlay
  hook is a no-op placeholder and a system-app-only capability (§9); building
  overlays/takeover/capture is separate, later work and not in this plan.
- **CPU core re-homing.** §3 floats a "Display-owned CPU1"; this migration must
  not move work between CPU0/CPU1 (that perturbs audio deadlines and the
  KOTO-0147 render-prep worker). *Mitigation:* §8 forbids it; Stage 5 keeps the
  existing split and only localises the boundary.

## 8. What this plan deliberately does **not** do

- No change to VM semantics, opcode values, bytecode ABI, or hostcall IDs.
- No change to PSRAM, LCD, CodeWindow, or audio behaviour.
- No change to KotoBlocks / KotoSnake / Sokoban behaviour; no app bytecode rebuild.
- No new per-game graphics hostcalls; no raw-hardware exposure to apps (already
  none).
- No broad rewrite, no PSRAM-backed surface, no new compositor — only relocation
  of the existing one into the layers the architecture names.
- No CPU0/CPU1 re-homing; no system overlay / status bar / display-takeover /
  capture (those are system-app-only per §9 and are later work).
- No new app-facing "surface" object semantics beyond what the relocation needs;
  the explicit-surface design is enabled, not implemented, here.
