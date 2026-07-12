# KOTO-0175: KotoRun's recurring CommandCountShift full repaints (fps-8 hitches)

- Status: **DONE 2026-07-11** — reopened same day (the 2026-07-09 won't-fix
  conflated two costs, see "Why reopened"), lever 1 landed and
  device-confirmed: 540-frame capture with zero `CommandCountShift` repaints,
  per-state counts 92/93/17 on the wire, fps-8 hitches gone. The scroll's
  ~40% dirty floor (fps 13–20 active) remains the inherent, accepted cost.
  Previously: CLOSED won't-fix 2026-07-09 — Stage 0
  attributed the hitches to a full-screen parallax scroll, which has no clean
  fix on this renderer (see the conclusion below). KotoRun is playable (30–60
  fps steady); the fps-8 hitches were called the inherent price. Spun out of
  the KOTO-0174 present-path A/B.
- Type: performance investigation (dirty policy / app render pattern)
- Priority: P2
- Requirements: NFR-PERF-1

Source of truth:
[apps/kotorun/src/main.koto](../../../apps/kotorun/src/main.koto) (the per-frame
immediate draw pattern: parallax scenery + a variable count of particles /
obstacles / coins), [koto-gfx repaint decision](../../../src/koto-gfx/src/repaint.rs)
(`coalesce_then_decide`, `FullRepaintReason::CommandCountShift`,
`FULL_REPAINT_AREA`), [app_render.rs `present_app_delta`](../../../src/koto-pico/src/firmware/app_render.rs#L308)
(the positional two-list diff + GFX-0011 collect-then-decide).

Relates to: [GFX-0011](../kotogfx/GFX-0011-commandcountshift-fallback-diagnosis.md)
(built the collect-then-decide path that already rescued KotoSnake's count
shifts — KotoRun is the case it *correctly* escalates), KOTO-0135 / KOTO-0141
(the retained-layer / stable-slot fixes that removed KotoBlocks' equivalent
churn), [KOTO-0174](KOTO-0174-present-path-cost-reduction.md) (H-P made raster
cheap; these full repaints are now transfer-bound and the worst KotoRun frames).

## Observed (device, 2026-07-08, KotoRun, `DIAG_PROFILE = Gfx`)

```
frame=120 ... raster_us=25187 transfer_us=76944 dirty_px=102400 rects=20 full=1 full_reason=CommandCountShift ... fps=8 lat_ms=114
frame=180 ... raster_us=24983 transfer_us=77018 dirty_px=102400 rects=20 full=1 full_reason=CommandCountShift ... fps=8 lat_ms=115
```

- Recurring **CommandCountShift full repaints** (~every ~60 frames): the whole
  320×320 surface (`dirty_px=102400`) recomposited + transferred, ~100 ms, **fps
  8** — a visible hitch. Between them, incremental frames run `dirty_px≈32k–48k`
  at fps 12–19 (KotoRun is a full-scene scroller, so even incremental frames are
  ~40% dirty).
- transfer (~77 ms) dominates each hitch: a full repaint is a full-surface
  transfer at the ~0.80 µs/px SPI rate (KOTO-0174).

## Attribution — why KotoRun trips it

KotoRun draws its scene as a **variable-length immediate command list** every
frame:

- Parallax scenery (clouds, mountains, ground segments, obstacles, coins) drawn
  with immediate `draw_rect` at scroll-shifted positions (`cam`/`off` advance
  every frame).
- **Particles are conditionally drawn:** `draw_particles` iterates 12 slots and
  emits a rect only `if actor_state(parts, i) != 0`
  ([main.koto:86](../../../apps/kotorun/src/main.koto#L86)) — so the immediate rect
  count swings with the active-particle count (0–12), and obstacle/coin rects
  swing as segments scroll on/off. The device `rect=` field bears this out:
  41 → 42 → 52 → 60 → 64 → 65 → 58 … frame to frame.

The device present path diffs the current vs previous immediate list
**positionally** (`command[i]` old-vs-new, KOTO-0128). When the count changes,
every command after the insertion/removal point misaligns; because the scene has
*also scrolled*, few post-shift commands match by content, so the edit region
spans most of the list. GFX-0011's collect-then-decide unions those edit-region
rects and coalesces them, but for a full-width scrolling scene the coalesced
area exceeds `FULL_REPAINT_AREA = 76800`, so it **correctly** escalates to a full
repaint (this is GFX-0011's designed case #3, not a bug).

So the escalation is *not* a false positive in the GFX-0011 sense — it is a real
consequence of an **unstable immediate command count** feeding a positional diff
in a scrolling scene. The cost (102400 px vs the ~40k a stable-count incremental
frame would dirty) is what hurts.

## Levers

1. **App-side: stabilize KotoRun's immediate command count/order (recommended,
   low risk).** Emit a *fixed* number of commands in a *fixed* order every frame
   — draw all 12 particle slots always (an inactive slot as a degenerate/
   off-screen rect), and pad obstacle/coin draws to fixed per-frame slots. With
   `command[i]` always the same logical element, the positional diff never
   count-shifts; each scroll frame stays incremental at its true ~40% dirty
   (fps ~12–19) and the fps-8 hitches disappear. This is the KOTO-0135 / KOTO-0141
   stable-slot lesson applied to KotoRun; cost is a handful of extra clipped/no-op
   draw commands per frame (cheap) and no engine change. **Only touches
   `apps/kotorun`.**
2. **App-side: move particles to the retained Game2D sprite layer (KOTO-0140).**
   Sprites are diffed by stable `inst_id`, so a moving/despawning particle yields
   a small stable band, never a count shift. Cleaner than stable immediate slots
   but a larger KotoRun rewrite (particle system → sprite instances + stamps).
3. **Engine-side: content/id-keyed immediate diff.** Match immediate commands by
   content rather than position so a count shift never misaligns the tail.
   Benefits every app, but a substantial, risky change to the KOTO-0128 diff and
   the GFX-0011 policy — deferred unless multiple apps need it.

## Conclusion — won't-fix (2026-07-09) — SUPERSEDED for lever 1 (see "Why reopened")

None of the three levers is worth taking, and the honest reason is deeper than
"the count shifts":

- **The escalation is driven by the scroll, not just the particle count.**
  Because the whole scene scrolls, the immediate commands are at *different
  positions* every frame, so GFX-0011's prefix/suffix match fails at command 0
  and the edit region is the whole list — any count change then unions to a
  near-full-screen area and (correctly) full-repaints. So **lever 3 (content/id
  -keyed diff) would not help either**: content *is* position for a scroller, so
  nothing matches frame-to-frame regardless of the diff key.
- **Lever 1 (pad the immediate list to a fixed count) is a hack** — it exploits
  the positional-diff internals from app code; it does not generalize and is
  exactly the kind of per-app artisanal tuning to avoid.
- **Lever 2 (particles → retained sprite layer) is idiomatic but insufficient.**
  It removes the *particle* count shift, but KotoRun's dominant dirty area is the
  parallax *scenery* scroll (procedural terrain/clouds/mountains drawn as
  immediate rects), which cannot practically become sprites and dirties ~40% of
  the surface every frame regardless.

**Root truth:** a full-screen parallax scroller is a poor fit for a dirty-rect +
SPI-transfer renderer with no framebuffer. There is no clean engine lever that
makes it cheap — even a perfect diff leaves ~40% dirty every frame, transfer-
bound at ~15–20 fps, and the count-shift frames escalate that to ~full. KotoRun
is nonetheless **playable** (device: 30–60 fps steady, 55–86 fps on small-dirty
frames after KOTO-0174 H-P); the recurring fps-8 full-repaint is the inherent
price, not a bug to engineer around for one app.

**Decision:** close won't-fix. Do not add app-side count-padding (a hack) or a
speculative engine diff rewrite (won't help scrollers) for a single app. If a
*class* of scrolling apps ever appears, revisit at the architecture level (a
framebuffer / a dedicated scroll render path), not the dirty-rect policy.

## Why reopened (2026-07-11)

The won't-fix reasoning conflated two separate costs, and its own Stage-0 data
separates them:

- **The scroll's ~40% dirty per frame is inherent** (fps 12–19 floor on active
  frames, transfer-bound). True, and still not worth an architecture change for
  one app. That part of the conclusion stands.
- **The count-shift escalation is not.** The incremental frames *between* the
  hitches (`dirty_px≈32k–48k`, under `FULL_REPAINT_AREA = 76800`) prove that a
  count-stable scrolled frame diffs incrementally just fine — the fps-8 full
  repaints fire only when the count changes. So the most visible artifact (a
  ~100 ms hitch every ~60 frames) was avoidable after all.
- The "lever 1 is a hack" judgement contradicted the project's own precedent:
  KOTO-0135 / KOTO-0141 stabilized KotoBlocks' slots the same way and the file
  cites them as "the stable-slot lesson". The positional diff is the engine's
  designed contract; **keeping the immediate command count/order fixed is the
  idiom that contract implies**, not an exploit of its internals.

## Lever 1 record — fixed-slot immediate list (landed 2026-07-11)

[apps/kotorun/src/main.koto](../../../apps/kotorun/src/main.koto): every
conditional immediate draw now keeps its slot, padding with a byte-stable 1×1
off-screen rect (`pad_cmds`; `draw_rect(-8,-8,1,1,0)`) when the real thing is
absent. Pads are recorded (the count holds — device `draw_rect` rejects only
`w<=0`/`h<=0`) but clip to `None` in `command_rect`, so they paint nothing and
contribute zero dirty. Per-state counts are fixed: **PLAY 92 / OVER 93 /
TITLE 17** of the 96-command budget; the only remaining count shifts are the
three state transitions (one full repaint each at start/death/restart — scene
changes anyway). Design points:

- **Hazard slots are keyed by segment residue, not window position.**
  `hazard()` keys off `seg % 12` and the 7-segment window holds at most one
  segment per residue, so the fixed order — gap(v11), gap(v4), spike(v5),
  spike(v1), drone(v6), drone(v7), drone(v8) — gives each hazard a slot that
  tracks it continuously while it scrolls (small dirty unions), instead of
  hazards hopping between slots (cross-screen unions). `seg_for(first, r)`
  resolves the window segment for a residue arithmetically.
- **Particles: always 12 commands** (one per pool slot, dead slots pad);
  coin 3, dive streaks 3 (reserved in PLAY only — dive/bonus/chain are forced
  to 0 at death so S_OVER has its own fixed count without those pads), legs
  padded to 3, SMASH! toast 1, chain badge 3, game-over retry blink 1.
- **The compiler inlines every call**, so the slot bodies are written as
  residue *loops* (one inlined body each), not per-slot helpers — the first
  cut with helpers grew the bytecode 37.4 → 60.5 KB and pushed steady play to
  a 3-tile code span (KOTO-0173's two-tile window is a hard budget; a 3-tile
  monotone walk on the 2-slot device window re-refills every frame).
- **The title screen's dead `draw_world` was removed**: it painted the whole
  scrolling world and then hid it entirely under an opaque full-screen panel —
  zero visible pixels for a second inlined copy of draw_world. Dropping it
  paid for the padding code almost exactly (final 37,824 B vs 37,410 B before)
  and put steady play back inside two tiles.

Verification (host, 2026-07-11):

- New guard [`kotorun_immediate_command_count_is_state_stable`](../../../src/koto-sim/tests/fixture_runner.rs)
  replays title → unassisted run → spike death → game over (183 frames) and
  asserts the per-frame immediate counts form **exactly three plateaus**:
  `[(17, 3), (92, 108), (93, 72)]` — hazards scrolling in/out, the coin,
  the death burst and its 12-particle decay all cross with zero count shifts.
- All three scenarios (`smoke`/`play`/`chain`) render **pixel-identical**
  frames vs the pre-change bytecode (the pads are invisible; hazard slot
  order is pixel-equivalent because hazards are segment-disjoint).
- `kotorun_code_window_tile_profile` back to the 2-tile monotone walk;
  `kotorun_steady_flat_run_keeps_immediate_rects_byte_stable` unchanged
  (pads are byte-stable, churn bound holds); full koto-sim suite green.
- Local-slot budget 43/45 (`--slot-map`).

Expected device effect: the recurring `CommandCountShift` full repaints
(~100 ms, fps 8, every ~60 frames) disappear; active-scroll frames keep their
existing incremental cost (fps 12–19, transfer-bound — that part is the real
inherent price, still owned by the KOTO-0174 conclusion). VM cost of the pads
is ~25–35 extra `draw_rect` hostcalls/frame (expected well under 0.5 ms of
`vm_us`); the `phase=160` A/B should confirm `full=1` stops appearing outside
state transitions.

## Acceptance criteria

- [x] Stage 0: the CommandCountShift full repaints attributed to KotoRun's
      variable immediate command count in a scrolling scene (device `rect=`
      variance + the `draw_particles` conditional-emit source), and confirmed as
      GFX-0011's correct escalation rather than a policy bug.
- [x] Numbers-backed decision recorded: ~~won't-fix~~ — superseded 2026-07-11:
      the scroll drives the *incremental* cost floor (that part stands), but
      the fps-8 escalations were count-driven and avoidable (see "Why
      reopened").
- [x] Lever 1 landed: fixed-slot immediate list, per-state counts 92/93/17,
      pixel-identical scenarios, three-plateau guard test, 2-tile code span
      preserved (2026-07-11).
- [x] Device A/B (`phase=160`, KotoRun, 2026-07-11, 540 frames): **no
      `CommandCountShift` at all** — the only `full=1` is frame 1's boot-time
      `StaticRebuild`. `peak=92` on play frames (`rect=88 text=4`), `93` on
      game-over, title `rect=12 text=5` = 17 — exactly the designed per-state
      counts. Active-scroll frames fps 13–20 (dirty 30–48k px, transfer-bound
      as expected — the KOTO-0174 floor), small-dirty frames fps 47–63, title
      fps 310. `vm_us` 6.7–11.3 ms across the run, same band as the ~8.1 ms
      pre-change steady figure (pad hostcall cost in noise; `hostcalls=94`).
      **The fps-8 hitches are gone. Issue closed — lever 1 shipped.**
