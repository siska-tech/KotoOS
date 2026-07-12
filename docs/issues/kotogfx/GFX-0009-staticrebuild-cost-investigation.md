# GFX-0009: StaticRebuild cost investigation and reduction plan

- Status: **proposed** — investigation + staged plan only. No rendering-behaviour
  change in this issue; an optional diagnostics-only patch is described under
  *Stage 0* and is the sole thing this issue may land in code.
- Type: investigation / proposal (the implementation stages it proposes are
  behaviour-preserving by construction; see *Non-goals*).
- Priority: P3
- Requirements: NFR-PERF-1, NFR-DRAW-1

Source of truth:
[app_render.rs](../../../src/koto-pico/src/firmware/app_render.rs)
([`present_app_delta`](../../../src/koto-pico/src/firmware/app_render.rs#L284),
[`present_app_commands`](../../../src/koto-pico/src/firmware/app_render.rs#L135)),
[layer.rs](../../../src/koto-gfx/src/layer.rs)
([`AppStaticLayer`](../../../src/koto-gfx/src/layer.rs#L185)),
[repaint.rs](../../../src/koto-gfx/src/repaint.rs)
([`FullRepaintReason::StaticRebuild`](../../../src/koto-gfx/src/repaint.rs#L29)),
[app_host.rs](../../../src/koto-pico/src/firmware/app_host.rs#L836)
(`game2d_static_begin`/`_end`).

Depends on: nothing new (reads signals GFX-0005/0006/0008 already produce).

Relates to: [GFX-0008](GFX-0008-commandcountshift-policy-refinement.md) (sibling
fix for the *other* full-repaint population — `CommandCountShift`),
[KOTO-0136](../main/KOTO-0136-game2d-static-layer.md) (the static layer this issue
studies), [KOTO-0143](../main/KOTO-0143-full-repaint-instrumentation-coalescing.md)
(full-repaint instrumentation / reasons),
[KOTO-0159](../main/KOTO-0159-kotoblocks-dirty-rect-coalescing.md) (dirty-rect
coalescing).

> **Why this is separate from GFX-0008.** GFX-0008 closed the *spurious*
> `CommandCountShift` full repaints (a one-command edit the positional diff failed
> to localize). With that landed, KotoBlocks line-clear / score-update / piece-lock
> frames stay incremental. The **remaining** worst frame on KotoBlocks reports
> `static_rebuilt=1 full_reason=StaticRebuild` with high `raster_us`/`transfer_us`.
> That is a *different* mechanism — a deliberate whole-surface chrome build, not a
> mis-derived diff — so it gets its own investigation rather than being folded into
> GFX-0008.

## Observed problem

On the `phase=160` line, KotoBlocks' heaviest frame is:

- `static_rebuilt=1`, `full_reason=StaticRebuild`,
- `raster_us` and `transfer_us` near their whole-surface maxima,
- and (post-GFX-0008) it is essentially the *only* remaining `full=1` frame in
  steady gameplay.

The question this issue answers: **why is that frame expensive, is the expense
intrinsic, and which (if any) static-rebuild frames can be made cheaper without
changing what's drawn?**

## Lifecycle of `static_rebuilt` (where it is set, cleared, read)

The signal is a single `bool` on the single retained
[`AppStaticLayer`](../../../src/koto-gfx/src/layer.rs#L185) instance — there is no
double buffer (KOTO-0136 kept it single-instance to save ~6 KiB of stack that a
double buffer cost at boot). Because there is no previous copy to diff, a rebuild is
signalled by this explicit flag rather than an old-vs-new comparison.

| Stage | Site | Effect |
|---|---|---|
| **Set** | [`AppStaticLayer::begin`](../../../src/koto-gfx/src/layer.rs#L201), reached via the `game2d_static_begin` hostcall ([app_host.rs:836](../../../src/koto-pico/src/firmware/app_host.rs#L836)) | `len = 0`, `rebuilt = true`; subsequent `draw_*` route into the layer until `game2d_static_end`. |
| **Read (present)** | [`present_app_delta`](../../../src/koto-pico/src/firmware/app_render.rs#L325) | `if static_layer.rebuilt { return present_app_commands(.., StaticRebuild) }` — an **early return before any dirty diff is collected**. |
| **Read (trigger + diag)** | [app_runtime.rs:866/878](../../../src/koto-pico/src/firmware/app_runtime.rs#L866) | `static_rebuilt` forces a present even if `*host.draw == *previous_draw`, and is reported on `phase=160`. |
| **Clear** | [`DeviceHost::clear_frame`](../../../src/koto-pico/src/firmware/app_host.rs#L637) (per frame) and [`DeviceHost::new`](../../../src/koto-pico/src/firmware/app_host.rs#L226) (app start) | `rebuilt = false` — so the flag reflects only a `game2d_static_begin` issued *this* frame. |

**Note on attribution overlap.** `full_reason=StaticRebuild` is emitted for *two*
distinct events: (a) the `static_layer.rebuilt` delta frame above, and (b) the
**first paint of the session** (`!has_previous`) in
[`display_service::present`](../../../src/koto-pico/src/firmware/display_service.rs#L113),
which passes `StaticRebuild` as its reason too. Today these are distinguishable only
by frame number (first paint is frame 1). The Stage-0 diagnostic below separates
them explicitly.

## Why StaticRebuild is expensive (root cause)

The `rebuilt` early return goes to
[`present_app_commands`](../../../src/koto-pico/src/firmware/app_render.rs#L135),
which — when the layer carries a full-screen base (it does; see below) — does a
**whole-surface strip compose**:

1. **Raster cost is `O(strips × commands)`.** The surface is composed in
   `RASTER_STRIP_LINES = 16`-row strips ([config.rs:25](../../../src/koto-pico/src/firmware/config.rs#L25)),
   so **20 strips** cover the 320-row panel. For *each* strip,
   [`paint_app_commands`](../../../src/koto-pico/src/firmware/app_render.rs#L731)
   re-walks the **entire** layer stack clipped to that strip — the full static
   command list (KotoBlocks builds ~40 commands: a page fill, the well frame/fill,
   ~30 grid lines, four panel frames, and the fixed labels;
   [main.koto:296–335](../../../apps/koto_blocks/src/main.koto#L296)) plus the
   board, sprites, retained text, and the immediate list. So the chrome is
   rasterized ~20×.
2. **Transfer cost is a full 320×320 blit.** `320·320·2 = 200 KiB` over the LCD bus,
   the fixed whole-panel cost.

Both are "very high" precisely because this is, by construction, a *legitimate*
whole-surface repaint: the static layer's first command is
`draw_rect(0, 0, 320, 320, C_PAGE)` — a full-screen base. KotoSnake is the same
(`draw_rect(0, 0, 320, 320, C_BG)`,
[main.koto:232](../../../apps/kotosnake/src/main.koto#L232)).

### Critical insight: the KotoBlocks transition damage is genuinely full-screen

Because both apps' static layers **contain a full-screen base fill**,
[`full_screen_base_color`](../../../src/koto-pico/src/firmware/app_render.rs#L678)
resolves from the static layer and the bounding box of the static damage *is* the
whole panel. Any "bound the damage to what the static rebuild touched" optimisation
would compute a ~320×320 rect and still full-repaint. **For KotoBlocks / KotoSnake
specifically, the cost of the one title→gameplay StaticRebuild is intrinsic and not
reducible by localization.** The honest conclusion: the lever for these apps is
*frequency* (it must happen exactly once per gameplay entry, never recur), not
per-frame cost.

This reframes the goal: GFX-0009 is mostly about **proving the rebuild is one-shot**
and **eliminating redundant/accidental rebuilds app-agnostically**, with damage
localization reserved for hypothetical partial-chrome apps.

## Classification of static-rebuild causes

| # | Cause | Trigger | Requires full repaint? | Localizable? |
|---|---|---|---|---|
| 1 | **First frame / app start** | `!has_previous` in `present` (frame 1) | **Yes** — nothing on the panel yet; the whole surface must be built. | No (no prior pixels). |
| 2 | **Retained static capture (title→gameplay)** | `state==1` → `game2d_static_begin` ([koto_blocks:296](../../../apps/koto_blocks/src/main.koto#L296), [kotosnake:231](../../../apps/kotosnake/src/main.koto#L231)) | **Yes for KotoBlocks/KotoSnake** (full-screen chrome). Only *partial-chrome* apps could be localized. | Only if new layer has no full-screen base. |
| 3 | **Menu/gameplay → title transition (degenerate rebuild)** | KotoSnake's empty `game2d_static_begin(); game2d_static_end()` on return-to-title ([kotosnake:372–374](../../../apps/kotosnake/src/main.koto#L372)) | **Redundant.** The new static layer is *empty*; the title screen already redraws a full-screen immediate base every frame, so that frame is a `BaseChange` full repaint *anyway*. The `rebuilt` flag adds nothing but a misattributed reason. | N/A — the damage is already covered by the base change. |
| 4 | **Board / static-layer refresh** | An app re-running `static_begin` with **identical** content (e.g. game-over→retry rebuilding the same chrome) | **No** in principle — old layer == new layer ⇒ zero damage ⇒ should `Skip`. Currently always full-repaints. | Yes (dedup). |
| 5 | **Accidental rebuild during normal play** | An app calling `game2d_static_begin` every frame (bug) | Currently **yes, every frame** — a silent perpetual full repaint. KotoBlocks/KotoSnake guard with `state==1`, so this is a *latent* failure class, not a current one. | Detectable; the cheapest fix is the app guard, but dedup (Stage 1) would also neutralize an identical re-begin. |

## Does StaticRebuild always require a full repaint?

**No.** The flag is binary, but the *damage* of a rebuild is bounded by `old static
footprint ∪ new static footprint`. The current code cannot derive that because there
is no retained previous static layer to diff against. Three sub-cases are provably
cheaper than a full repaint:

- **Identical rebuild** (case 4): damage is empty → `Skip`.
- **Empty/degenerate rebuild** (case 3): new layer empty; damage is the *old*
  footprint only, which on these apps coincides with a `BaseChange` the present path
  already detects.
- **Partial-chrome rebuild** (a future app whose static layer has *no* full-screen
  base): damage is the union of the two non-full-screen footprints.

The cases that **genuinely require** a full repaint are 1 and 2-for-full-screen-chrome
(KotoBlocks/KotoSnake) — and those are exactly the ones whose damage *is* the whole
surface, so the full repaint is correct, not wasteful.

## Staged reduction plan

Each stage is independently shippable and a strict subset of current behaviour (only
ever *removes* a repaint, never adds risk).

### Stage 0 — diagnostics only (this issue, optional patch)

Establish, on hardware, **whether StaticRebuild is one-shot or recurring**, and
separate first-paint from mid-session rebuild. Low-volume:

- A **cumulative `static_rebuilds=N`** counter carried on the existing throttled
  `phase=160` line (so it costs no extra emit; a healthy KotoBlocks session shows
  this pinned at a small constant — first paint + one per gameplay entry).
- A **one-shot `phase=170` line emitted *on* a mid-session rebuild frame**
  (`static_rebuilt && frame > 1`), reporting `frame`, the running count, and
  `static_cmds`. Rebuilds are rare by design, so this never floods UART; if it fires
  every frame, that is case 5 (an app bug) caught immediately.

This is the only code change GFX-0009 may land. It is observe-only — no policy, no
reason change, no app-specific knowledge. **Gate:** if the diagnostic shows
KotoBlocks/KotoSnake rebuild exactly once per gameplay entry (expected), Stages 1–2
are *deferred as unnecessary for these apps* and the issue closes as "intrinsic,
one-shot, acceptable" — the high-cost frame is a legitimate one-time transition.

### Stage 1 — dedup identical / empty rebuilds (behaviour-preserving)

Refuse the StaticRebuild full repaint when the rebuilt layer is **byte-identical to
the layer that produced the currently-displayed pixels** (catches case 4 and an
accidental identical re-begin in case 5). Two implementation options, both
respecting the single-instance memory rule (no double buffer):

- **(1a) Content hash.** Keep a `u32`/`u64` FNV hash of the static command bytes,
  updated at `game2d_static_end`. On the next rebuild, compare the new hash to the
  retained one; equal ⇒ clear `rebuilt` and take the normal incremental path. ~8
  bytes of state, no ABI change.
- **(1b) Transient old-copy.** `begin()` zeroes `len` but does **not** clear the
  command array, so the previous commands physically survive until overwritten.
  Diffing against them is possible but fragile (depends on push order); (1a) is
  preferred.

`AppStaticLayer` and `FullRepaintReason` are unchanged; this only adds an early
"nothing actually changed" exit ahead of the existing `rebuilt` early return.

### Stage 2 — bounded damage for partial-chrome apps (future, optional)

For a rebuilt layer **without** a full-screen base, derive damage as
`old-bounds ∪ new-bounds` (a bounding rect, or the GFX-0008 prefix/suffix alignment
applied to the static lists) and feed it to the existing
[`FullRepaintPolicy`](../../../src/koto-gfx/src/repaint.rs#L136) — wide damage still
escalates (attributed as today), bounded damage stays incremental. **No benefit to
KotoBlocks/KotoSnake** (full-screen base ⇒ full damage), so this is scoped to future
apps and only worth doing once such an app exists.

### Stage 3 — out of scope (recorded, not proposed)

- Retaining a pre-rasterized static layer in a buffer to skip the per-strip re-walk
  → memory/PSRAM change (excluded).
- Larger `RASTER_STRIP_LINES` to amortize the re-walk → strip-budget memory tradeoff
  (excluded).
- App-side: ensure `game2d_static_begin` is issued once per gameplay entry; drop
  KotoSnake's redundant empty `begin/end` on return-to-title (case 3). This is
  *app-specific* and explicitly **not** part of GFX-0009's renderer scope — noted for
  the app owners.

## Non-goals (carried from the task framing)

- No rendering-behaviour change in this issue beyond the optional Stage-0 diagnostic.
- No app-specific policy or constants; no per-app profiles.
- No hostcall ID / ABI / app-bytecode change; no `APP_DRAW` capacity change.
- No PSRAM, LCD, CodeWindow, or audio change.
- `FullRepaintPolicy`, its thresholds, and the `FullRepaintReason` variants stay
  unchanged.

## Acceptance criteria

- [ ] Document classifies every static-rebuild cause and states, per cause, whether a
      full repaint is required and whether it is localizable (table above).
- [ ] Establishes that the KotoBlocks heavy frame is a *legitimate, intrinsic,
      one-shot* full-screen chrome build — not a derivation artifact — with the
      lifecycle (set/clear/read) of `static_rebuilt` documented.
- [ ] (If Stage-0 patch landed) `phase=160` carries a cumulative `static_rebuilds=`
      count and a one-shot `phase=170` fires only on mid-session rebuilds; both are
      observe-only, add no app-specific knowledge, and leave `FullRepaintPolicy` /
      reasons / hostcalls / bytecode untouched (`build_apps.py --check` clean).
- [ ] Hardware smoke (below) confirms one-shot rebuild behaviour for KotoBlocks and
      KotoSnake, gating whether Stages 1–2 are pursued.

## Acceptance tests (host-side, `cargo test`)

If the Stage-0 patch lands, add to `koto-pico` firmware diag tests (host-compatible
formatting tests, mirroring the `phase=168`/`phase=169` line tests):

1. **Cumulative counter monotonic** — `static_rebuilds=` increments once per rebuilt
   frame, stays flat on incremental frames.
2. **`phase=170` first-paint suppression** — frame 1 (`!has_previous`) does *not*
   emit `phase=170`; a mid-session `static_rebuilt && frame>1` does.
3. `cargo test -p koto-gfx` / `-p koto-game2d` / `-p koto-core` / `-p koto-sim`
   (golden frames) all green; `thumbv6m` firmware build green; firmware-lib clippy
   adds no new finding.

If Stage 1 is later pursued, add `koto-gfx` tests: identical-rebuild ⇒ `Skip`
(no damage); empty rebuild over a full-screen-base previous ⇒ handled by the existing
`BaseChange` path; changed rebuild ⇒ unchanged full repaint.

## Hardware smoke acceptance plan (KotoBlocks / KotoSnake)

- **Rebuild frequency.** Capture `phase=160` `static_rebuilds=` across a full session:
  launch → title → gameplay → line clears / level up → game over → retry → quit.
  Expect the counter to step **once on first paint, once per title→gameplay entry**,
  and (today) once per KotoSnake return-to-title (case 3). It must **not** climb
  during steady play — a per-frame climb is case 5 and a bug.
- **`phase=170` cadence.** Confirms mid-session rebuilds are sparse (the transition
  frames only), never per-frame.
- **Cost attribution.** On the rebuild frame, `raster_us`/`transfer_us` are at the
  whole-surface level (expected, intrinsic); on all *other* gameplay frames they stay
  at the incremental level GFX-0008 restored (no regression — StaticRebuild must not
  leak into steady frames).
- **Visual smoke.** `phase=160/164` smoke shows the title→gameplay transition paints
  the chrome cleanly once (no flicker, no stale title pixels), and steady play shows
  no whole-screen flashes (which would indicate an unexpected recurring rebuild).
- **If Stage 1 lands later.** A forced identical re-begin (e.g. game-over→retry)
  no longer emits a StaticRebuild full repaint; the genuine title→gameplay rebuild
  still does. Golden-frame identity between the deduped path and a full recompose.

## Notes

With GFX-0008 (CommandCountShift) and GFX-0009 (StaticRebuild) understood, the two
remaining full-repaint populations are accounted for: GFX-0008 removed the *spurious*
ones; GFX-0009 shows the StaticRebuild ones on KotoBlocks/KotoSnake are *legitimate,
intrinsic, and one-shot*. The actionable residue is app-agnostic dedup of
redundant/accidental rebuilds (Stage 1) and localization for future partial-chrome
apps (Stage 2) — both deferred behind the Stage-0 diagnostic that proves whether they
are needed at all.
