# GFX-0013: Diffed damage for mid-session static rebuilds

- Status: implemented (Stages 0–2 landed 2026-07-05: `koto-gfx/src/shadow.rs` +
  firmware wiring; hardware validation pending — see the unchecked acceptance
  items below)
- Type: optimization (renderer policy)
- Priority: P2 — now user-visible: every static rebuild costs a whole-surface
  repaint (~180 ms measured), and the retained-render app migrations made
  rebuilds a **recurring per-action event**, not the one-shot transition
  GFX-0009 assumed.
- Requirements: NFR-PERF-1, NFR-DRAW-1
- Relates to: [GFX-0009](GFX-0009-staticrebuild-cost-investigation.md) (cost
  classification this issue supersedes in part),
  [BUG-GFX-0012](BUG-GFX-0012-kotosnake-initial-ui-invisible.md) (the
  correctness bug the current force-full latch fixes — its guarantee is the
  hard invariant here), [GFX-0008](GFX-0008-commandcountshift-policy-refinement.md)
  (the prefix/suffix list-alignment technique this issue reuses),
  [KOTO-0136](../main/KOTO-0136-game2d-static-layer.md) (single-instance static
  layer, no double buffer).

Source of truth:
[repaint.rs](../../../src/koto-gfx/src/repaint.rs)
(`force_full_repaint_after_static_rebuild`, the BUG-GFX-0012 latch),
[app_render.rs](../../../src/koto-pico/src/firmware/app_render.rs)
(the `StaticRebuild` early return in `present_app_delta`),
[layer.rs](../../../src/koto-gfx/src/layer.rs) (`AppStaticLayer`, 80-command cap).

## Why now (what changed since GFX-0009)

GFX-0009 closed on the observation that KotoBlocks/KotoSnake rebuild their
static layer **once per gameplay entry**, so the whole-surface cost was
"legitimate, intrinsic, one-shot" and Stages 1–2 were deferred.

The 2026-07-05 retained-render migrations (KotoShogi, KotoMines, KotoRun,
KotoRogue) invalidated that premise: these apps keep their board/fog/HUD chrome
in the static layer and rebuild it **on gameplay actions** — KotoShogi on every
completed move, KotoRogue on every room entry/exit. Hardware (`phase=160`,
KotoRogue session 2026-07-05):

- `static_rebuilds=130+` over one session (vs. the "small constant" GFX-0009
  expected);
- each rebuild frame: `full=1 full_reason=StaticRebuild raster_us=100573
  transfer_us=77317 dirty_px=102400 lat_ms=199 fps=5` — a ~180 ms hitch per
  action-class event.

App-side mitigation has already been squeezed hard (rebuild gating, retained
HUD text items, run-merged fog): the remaining rebuilds are *genuine content
changes*, but tiny ones — a few fog-run rectangles out of ~50 commands. The
renderer still repaints all 320×320 for them.

## Key insight (supersedes one GFX-0009 conclusion)

GFX-0009 argued localization is pointless when the layer carries a full-screen
base (`draw_rect(0,0,320,320,…)` ⇒ damage bounds = whole panel). That holds for
*whole-layer bounding-box* damage — but not for **old-vs-new command
alignment**: across a mid-session rebuild the base fill, panel chrome, and most
runs are *byte-identical commands*; only the handful of changed commands differ.
Aligning the two lists (the GFX-0008 prefix/suffix technique, applied to static
lists) yields damage = union of the unmatched commands' footprints — for a
KotoRogue room transition, a few row bands instead of the whole panel.

## The hard invariant (BUG-GFX-0012)

The force-full latch exists because an incremental present once swallowed the
frame that had to *reveal* freshly built chrome (KotoSnake UI invisible until
the first pickup). Any change here must keep that guarantee:

1. **First paint** (`!has_previous`) stays an unconditional full repaint.
2. Diffed damage must cover **old ∪ new footprints of every unmatched command**
   — rebuilt content always reaches GRAM.
3. Alignment failure, command-count shifts beyond the aligned window, or any
   ambiguity ⇒ **fall back to the existing latch** (full repaint). The latch is
   never removed; it becomes the escape hatch instead of the only path.
4. Wide-but-bounded damage still goes through `FullRepaintPolicy` and may
   escalate to full exactly as immediate-list damage does today.

## Design sketch

No double buffer (KOTO-0136's ~6 KiB boot-stack cost stands). Retain instead a
compact shadow of the last **applied** static layer in its own `StaticCell`
(per the DeviceRuntimeHost-doubling lesson): per command a content fingerprint
plus its clipped footprint rect — 80 × (u32 hash + 4×i16) ≈ **1.3 KiB**.

On `game2d_static_end`:

- prefix/suffix-align new commands' fingerprints against the shadow;
- unmatched middle ⇒ damage = union of those entries' old rects + new rects;
- empty unmatched set ⇒ `Skip` (GFX-0009 Stage 1 dedup falls out for free);
- alignment window too wide / count shift too large ⇒ latch (full), as today.

## Stages

| Stage | Content | Gate |
| :---- | :------ | :--- |
| 0 | Retain the fingerprint shadow; **observe-only**: extend `phase=170` with `would_be_damage_px` vs the full repaint actually taken. No policy change. | Field parity of `phase=160/164/170`; golden frames byte-identical. |
| 1 | Act on the **identical-rebuild** case only: unmatched set empty ⇒ skip the latch. | koto-gfx unit tests; golden frames; BUG-GFX-0012 hardware smoke stays fixed. |
| 2 | Act on **bounded diffs**: feed the unmatched-union damage to `FullRepaintPolicy`. | Golden-frame parity for KotoBlocks/KotoSnake/Sokoban + the four migrated apps; hardware smoke below. |

## Implementation notes (2026-07-05)

- Stage 0 (`34a3f0f`): `koto_gfx::StaticLayerShadow` (80 × {u32 FNV-1a
  fingerprint + clipped-footprint i16 quad} ≈ 1 KiB, `size_of` guarded ≤ 1.5 KiB)
  in its own binary `StaticCell`; `collect_static_rebuild_dirty` (prefix/suffix
  alignment over fingerprints → `NoShadow | Identical | Bounded | Wide`);
  `phase=170` gains `align= region= would_rects= would_px=`.
- Stage 1 (`5d28624`): `align=identical` ⇒ no forced present, no reveal latch.
- Stage 2 (`ca3eea1`): `align=bounded` damage rides `present_app_delta`'s
  working set (`DIRTY_RECT_PROBE_CAP += STATIC_DAMAGE_CAP`, escalation owned by
  the normal coalesce-before-decide policy); `phase=170` gains
  `acted=skip|bounded|full`.
- The alignment window `STATIC_DAMAGE_CAP = 32` (BOARD_BAND_CAP-style headroom
  over `FULL_REPAINT_RECTS`); tune against Stage-0 `region=` hardware data if
  KotoRogue room transitions turn out wider.
- Beyond the sketch: a **base-overdraw ambiguity guard** — a bounded rebuild is
  demoted to the latch path when an immediate full-screen fill exists in either
  diffed frame while the static layer supplies the retained base. With one
  shared layer instance, neither the base-change check nor the immediate diff
  (which skips base fills) can see such a fill change, which is precisely the
  BUG-GFX-0012 overdraw; steady per-action rebuilds carry no immediate base
  fill, so the target workloads are unaffected (invariant 3's "any ambiguity ⇒
  latch").
- Deliberately not skipped in the shadow diff: a full-screen base fill in the
  unmatched region (a recolored static base can only surface here — it becomes
  whole-surface damage the policy escalates).
- KotoSim renders full-frame per present (no delta path), so sim golden output
  is structurally unchanged; `koto-gfx` (146) and `koto-sim` (121) tests green,
  `thumbv6m` firmware build + clippy clean (pre-existing warnings only).

## Acceptance criteria

- [x] koto-gfx unit tests: identical rebuild ⇒ skip; single changed run-rect ⇒
      damage is that band's old∪new union; inserted/removed command within the
      alignment window ⇒ bounded; reordered or widely shifted lists ⇒ full
      (latch), never under-damage. (A *bounded* reorder is positionally paired
      and damages both unions — covered, strictly better than full.)
- [x] Golden-frame pixel parity across all shipped apps (sim), and
      `cargo test -p koto-sim` / `-p koto-gfx` / firmware `thumbv6m` build green.
- [ ] **BUG-GFX-0012 hardware smoke re-run**: KotoSnake gameplay UI visible
      before the first pickup; KotoBlocks title→gameplay chrome paints cleanly.
- [x] Hardware (`phase=160`), KotoRogue (2026-07-05 device session): confirmed.
      `static_rebuilds=6` with `full=0 full_reason=none` across every sample —
      no mid-session rebuild fell back to a whole-surface repaint. Action-class
      frame: `dirty_px=15936 rects=7 raster_us=24968 transfer_us=13495
      lat_ms=81` (vs the pre-GFX-0013 `dirty_px=102400 … lat_ms=199` baseline;
      the remaining latency is `vm_us=37678`). Steady frames unchanged:
      `rects=1 dirty_px≈200–256 fps=36–42`, idle `rects=0`.
- [ ] Hardware (`phase=160`), KotoShogi move frames: same check pending.
- [ ] SRAM: free-SRAM re-measured on hardware (≥ ~80 KiB headroom per the
      stack-headroom baseline). Shadow cell ≤ ~1.5 KiB is asserted by a
      `size_of` test (`shadow_is_compact`); the probe working set grew by
      32 rects (+512 B future size) with `STATIC_DAMAGE_CAP`.

## Non-goals

- No hostcall/ABI/bytecode change; apps need no rebuild.
- No change to first-paint behaviour or `FullRepaintPolicy` thresholds.
- No PSRAM-backed pre-rasterized static layer (GFX-0009 Stage 3 stays excluded).
