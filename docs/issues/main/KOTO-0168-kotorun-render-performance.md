# KOTO-0168: KotoRun steady-play render performance

- Status: **Stages 1-3 implemented** (sim-validated; device validation is the open gate).
  **Stage 1** — the parallax base bands and flat ground bands moved into the retained
  static layer (8 static commands, built once); gaps paint an ink overlay plus the
  approach-edge lip. Four pre-existing draw populations turned out to be *provably
  never visible* and are dropped rather than restructured: the `i = -1` tower tile of
  each parallax layer (off-screen for every offset), the far-tower spans below y=204
  (the mid band always overpainted them — the towers now end there, shrinking their
  move footprint ~40%), the per-segment dirt flecks, and the gap right-edge lip (both
  always overpainted by the merged-run flush that followed them). Pixel parity was
  verified against the pre-change bytecode by byte-identical `--image` dumps on a
  steady flat-run frame, a gap/game-over frame, and the title screen.
  **Stage 2** — background motion quantized at unchanged average speed (far towers
  4 px per 24 cam, mid towers + window lights 4 px per 12 cam, clouds 3 px per 27/39
  cam; step sizes divide the wrap moduli so tiling stays seamless); window-light
  flicker slowed to an 8-frame cadence; the HUD score text refreshes every 8 frames
  in play (exact outside play, `cam < 40` covers run start), with the shared format
  buffer widened to 24 bytes so score/coins/best/chain each own their bytes — no new
  locals (`local_peak` stays 46/48).
  **Stage 3** — particle pool 18 → 12 (`actor_array_new`, spawn/draw loops), so smash
  bursts degrade by spawning fewer motes instead of overflowing the 96-slot APP_DRAW
  budget and tail-dropping.
  Sim results: steady-script `draw_rects_peak` 79 → 62, `local_peak` 46 (unchanged),
  `heap_peak` 486 → 430; `build_apps.py --check` clean; all 19 `fixture_runner` tests
  green including the new
  `kotorun_steady_flat_run_keeps_immediate_rects_byte_stable` (measured: 67 on-screen
  changed slots over 11 flat-run pairs ≈ 6.1/frame vs ~30+/frame pre-change, 7 quiet
  pairs, constant rect count, APP_DRAW peak < 96 over 100 frames incl. spike + 3
  drones + coin) and the pre-existing `kotorun_code_window_tile_profile`.
- Type: app-side performance (apps/kotorun) — no engine/policy change
- Priority: P2
- Requirements: NFR-PERF-1, NFR-DRAW-1

Source of truth:
[apps/kotorun/src/main.koto](../../../apps/kotorun/src/main.koto),
[derive.rs `collect_immediate_dirty`](../../../src/koto-gfx/src/derive.rs)
(the positional diff whose stability rules this plan writes to),
[repaint.rs `FullRepaintPolicy`](../../../src/koto-gfx/src/repaint.rs)
(`FULL_REPAINT_AREA = 76800`, `FULL_REPAINT_RECTS = 24`).

Relates to: [GFX-0008](../kotogfx/GFX-0008-commandcountshift-policy-refinement.md) /
[GFX-0011](../kotogfx/GFX-0011-commandcountshift-fallback-diagnosis.md) (the aligned
immediate diff + wide-count-shift collection this app now plays to),
[GFX-0013](../kotogfx/GFX-0013-diffed-static-rebuild-damage.md) (static-layer rebuild diff),
[KOTO-0159](KOTO-0159-kotoblocks-dirty-rect-coalescing.md) (dirty-rect coalescing).

## Observed problem (device, 2026-07-05 session)

KotoRun steady play runs at **fps=4–8, lat_ms 114–222**:

```
phase=160 ... frame=60  vm_us=18175 raster_us=59687  transfer_us=34190 dirty_px=42230  rects=17 full=0 fps=8
phase=160 ... frame=120 vm_us=20446 raster_us=106431 transfer_us=77788 dirty_px=102400 rects=20 full=1 full_reason=CommandCountShift fps=4
phase=162 app-draw-overflow frame=91 used=96/96 peak=96 note=tail-dropped   (ovf climbs to 14)
```

Three distinct app-side defects:

1. **False dirty on pixel-identical geometry (~24k px/frame).** The merged ground
   run is emitted as `draw_rect(run*SEG - cam, 246, (s-run)*SEG+1, 74, C_DIRT)`
   (+ two grass strips): `x`/`w` change every frame while the *painted pixels* are a
   constant full-width band. The immediate diff compares command geometry, not
   pixels, so it unions the old∪new clipped footprints — the whole 320×74 band —
   every frame. Same mechanism, smaller scale, for the parallax base bands' towers.
2. **Parallax towers dirty the whole mid-screen band every frame (~38k px).** The
   far (`cam/6`) and mid (`cam/3`) silhouette rects move 1–2 px nearly every frame,
   so their y=127..246 strip re-rasterizes and re-transfers continuously. The mid
   layer's 6 window lights flicker on `tick & 1` — 6 command slots churn *every*
   frame for a 1-px decoration.
3. **Count-shift frames escalate to full repaints, and the list overflows.**
   Particle expiry / hazard scroll-in changes the immediate command count; because
   the commands *before* the shift point (clouds, towers, ground, score text) also
   differ every frame, the common prefix is near zero, the edit region is wide, and
   the post-coalesce damage exceeds `FULL_REPAINT_AREA` → periodic
   `full=1 CommandCountShift` (dirty_px=102400, fps=4). Worst-case frames also
   exceed the 96-slot immediate budget (`used=96/96 tail-dropped`, `ovf=14`),
   dropping the (deliberately last-drawn) particles.

The engine-side rescues are already in place (GFX-0008/0010/0011 collect and
coalesce wide count shifts; GFX-0013 diffs static rebuilds). What is missing is an
app that *plays to* the differ: byte-stable commands for everything that does not
visibly move, so damage localizes.

## Plan (all stages app-side, `apps/kotorun/src/main.koto` only)

### Stage 1 — constant base + overlay ground; never-visible tower slots dropped

- Move the constant background bands into the retained static layer (built once):
  far band (0,174,320,72), mid band (0,204,320,42), ground dirt (246..320), grass
  strip + highlight. The static abyss rect is retired — gaps paint their own ink.
- Gaps become **overlays**: per visible gap segment, one `C_INK` rect
  (x+1, 246, SEG-1, 74 — matching the old runs' 1-px overhang for pixel parity)
  plus the two stone lips. A flat course now costs **zero** ground commands and
  zero ground dirty; a visible gap costs 3 commands and ~7.5k px/frame while it
  scrolls past.
- Drop the never-visible tower loop iterations (far `i = -1`, mid `i = -1` are
  provably off-screen for all offsets): −5 immediate slots, pixel-identical.
- Expected: pixel-identical output (verified by `--image` byte-compare on the
  smoke script), steady dirty_px ≈ 42k → ≈ 18k.

### Stage 2 — quantized motion + content stability

- Parallax moves in coarse steps at the same average speed, so tower/cloud slots
  are byte-identical between steps: far `off = (cam/24)*4 % 120` (4-px step ≈
  every 4–6 frames), mid `off = (cam/12)*4 % 84`, clouds 3-px steps
  (`(cam/27)*3`, `(cam/39)*3`). Wrap moduli stay divisible by the step.
- Window lights flicker on `(tick>>3) & 1` instead of `tick & 1` (6 slots stop
  churning every frame).
- HUD score renders a **cached** value refreshed every 8 frames (own `buf`, so the
  coin display no longer shares the format buffer and its slot goes byte-stable
  until the count actually changes). The authoritative `score` is untouched —
  game-over shows the exact value.
- Expected: on frames where no layer steps, the whole pre-particle prefix is
  byte-identical → a particle-count shift localizes to the particle block
  (region ≤ 18 < `MAX_EDIT_REGION`), and even move-frame count shifts stay under
  `FULL_REPAINT_AREA` after coalescing → `CommandCountShift` full repaints
  disappear from steady play. Steady dirty on quiet frames ≈ 7k.

### Stage 3 — command-count headroom (overflow fix)

- Particle pool 18 → 12 (matches the budget observer's particle class; bursts
  degrade by spawning fewer, never by tail-dropping unrelated commands).
- Expected: worst-case immediate count ≤ ~90 < 96; `phase=162 app-draw-overflow`
  gone; budget observe stops reporting `would_reject`.

### Stage 4 — recorded, not implemented (follow-ups if the device still wants more)

- Migrate the 4 HUD texts to the retained text layer (KOTO-0141) — takes them out
  of the immediate prefix entirely.
- Coarser parallax steps (6 px) or shorter far towers if move-frame raster is
  still the pacing item.
- Audio: the session log shows a constant `drops=512 buffer_level=0` in
  `phase=173` from frame 1 — pre-existing, out of scope here.
- The quiet-frame fps ceiling after this issue is `vm_us` (17-24 ms vs raster
  11-13 ms on the post-landing device session) — split out into
  [KOTO-0169](KOTO-0169-vm-frame-cost-attribution.md). Sprite/stamp migration of
  the actors was evaluated and rejected: sprites composite *below* the immediate
  list (static→board→sprites→text→immediate), and KotoRun's scrolling towers /
  gap ink must stay immediate, so a sprite runner would render behind them; a
  sprite also dirties the same old∪new union a moved rect does, so it would not
  reduce dirty area anyway.

## Validation

- `python harness/build_apps.py` (kotorun only affected) + `--check` clean.
- `cargo run -p koto-sim -- --app dev.koto.games.kotorun --app-script ... --image`:
  Stage 1 byte-identical vs baseline; Stage 2/3 visually reviewed (title + play).
- `cargo run -p koto-sim -- ... --inspect --budget`: per-frame command counts ≤ 90.
- Fixture test (`koto-sim/tests/fixture_runner.rs`): steady flat-run frames keep
  the immediate list positionally byte-stable (≥ 60% identical slots) and under
  the command cap — guards the stability contract this plan is built on.
- Existing `kotorun_code_window_tile_profile` stays green (≤ 2 refills/tiles).
- Device validation (user): `phase=160` steady `dirty_px` down ~4×, no
  `CommandCountShift` full repaints in steady play, `ovf=0`, fps ≥ 2× baseline.

## Acceptance criteria

- [x] Stage 1: ground/bands static + gap overlays; sim screenshots byte-identical
      to the pre-change bytecode (steady flat-run, gap/game-over, title).
- [x] Stage 2: quantized parallax/clouds, slow lights, cached HUD score (no new
      locals; `local_peak` 46/48 unchanged).
- [x] Stage 3: particle pool 12; sim steady-script `draw_rects_peak` 62, fixture
      APP_DRAW peak < 96 over a 100-frame course with spike + 3 drones + coin.
- [x] Fixture stability test added and green
      (`kotorun_steady_flat_run_keeps_immediate_rects_byte_stable`); tile-profile
      test still green.
- [x] `build_apps.py --check` clean; visual review of title/play/game-over
      screenshots (score text lags ≤ 7 frames in play by design; exact in menus).
- [ ] Device: steady fps ≥ 2× baseline, `ovf=0`, no steady-play
      `full_reason=CommandCountShift`, `phase=162 app-draw-overflow` gone.
