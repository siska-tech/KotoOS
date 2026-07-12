# GFX-0010: RectsExceeded pressure investigation (coalesce-ordering defect)

- Status: **Stage 0 implemented** (observe-only) — investigation + staged plan,
  with the Stage-0 diagnostic landed: `probe_coalesce_pressure` in
  [`koto-gfx`](../../../src/koto-gfx/src/repaint.rs) + a `phase=171` line emitted on
  `RectsExceeded`/`AreaExceeded` frames. No rendering-behaviour change; the reorder
  (Stage 1) remains deferred and gated on the hardware data this diagnostic produces.
  **Stage 1A implemented** (observe-only) — the *probe-only* dirty collection is now
  expanded to `DIRTY_RECT_PROBE_CAP` so the dry-run measurement stops being poisoned by
  truncation; see *Stage 1A* below. Stage 0 hardware data motivated it: `RectsExceeded`
  frames came back `rects_pre=25`, `truncated=1`, `rects_coalesced=2`, with
  `area_coalesced`/`bbox` well under full screen — the collected prefix collapses hard,
  but `would_incremental=false` was *forced* by truncation, so we could not say whether
  the frame was genuinely coalescible. Stage 1A removes that ambiguity without changing
  any decision: the policy still decides on the 25-cap snapshot
  (`koto_gfx::decision_snapshot`), the probe sees the full structural set, and the
  `phase=171` line now reports the decision snapshot and the probe set separately.
  Host-side proof landed (decision-identity property test over every raw count, expanded
  probe over/under threshold, structural-max non-truncation); `cargo test`
  `-p koto-gfx`/`-game2d`/`-core`/`-sim` green; `thumbv6m` build green; firmware clippy
  adds no new finding; `.bss` +960 B (the 84-rect probe lands in the async future, not
  the stack), leaving ≈91 KB free above `.bss` — far above the ~50 KB floor. Hardware
  boot-smoke + the coalescible/irreducible classification remain the open gate.
  **Stage 1B implemented** (behaviour-changing reorder) — the present path now
  *batch-coalesces the full expanded raw dirty set before* `FullRepaintPolicy::decide`,
  so a fragmented-but-coalescible `RectsExceeded` frame stays incremental at its
  post-coalesce pass count instead of escalating on the raw rect count. The decision
  reads the post-coalesce count and a re-summed post-coalesce area; the 25-cap
  `decision_snapshot` survives only to feed the `phase=171` *old-vs-new decision*
  contrast (`old_reason`/`new_reason`/`converted_to_incremental`). Truncation is the
  fail-safe: a probe-buffer overflow or `board_overflow` forces a full repaint regardless
  of the coalesced count, preserving the pre-coalesce overflow/`CommandCountShift` safety;
  a still-over-threshold or genuinely-wide-area coalesced set escalates exactly as before.
  `koto_gfx::coalesce_then_decide` owns the reorder (host-tested: coalescible >25 raw
  becomes incremental, irreducible scatter / truncated / `board_overflow` still
  full-repaint, command-count-shift attribution + fail-safe preserved, small and empty
  frames unchanged); the firmware `present_app_delta` calls it and transfers the coalesced
  survivors. `phase=171` becomes `app-coalesce-decide` and now fires for surviving
  rect/area escalations *and* converted-to-incremental frames. No threshold, hostcall,
  ABI, bytecode, or app-specific change. `cargo test`
  `-p koto-gfx`/`-game2d`/`-core`/`-sim` green; `thumbv6m` build green; firmware clippy
  adds no new finding; `build_apps.py --check` clean; release `koto_firmware` `.bss`
  177120 → 177088 (**−32 B**, no new buffers — the probe is Stage 1A's). Hardware smoke
  on KotoSnake (former `RectsExceeded` frame → `full=0`, small rect count, no artifacts)
  is the open gate.
- Type: investigation / proposal (the Stage-0 diagnostic it proposes is
  observe-only by construction; the behaviour-changing reorder is deferred to a
  follow-up gated on the Stage-0 data — see *Non-goals*).
- Priority: P3
- Requirements: NFR-PERF-1, NFR-DRAW-1

Source of truth:
[app_render.rs](../../../src/koto-pico/src/firmware/app_render.rs)
([`present_app_delta`](../../../src/koto-pico/src/firmware/app_render.rs#L285),
`DIRTY_RECT_CAP`/`DIRTY_COALESCE_MAX_WASTE`
[app_render.rs:61](../../../src/koto-pico/src/firmware/app_render.rs#L61)),
[repaint.rs](../../../src/koto-gfx/src/repaint.rs)
([`FullRepaintPolicy::decide`](../../../src/koto-gfx/src/repaint.rs#L136),
[`FULL_REPAINT_RECTS`](../../../src/koto-gfx/src/repaint.rs#L65)),
[dirty.rs](../../../src/koto-gfx/src/dirty.rs)
([`coalesce_rects`](../../../src/koto-gfx/src/dirty.rs#L111),
[`coalesce_dirty_tiles`](../../../src/koto-gfx/src/dirty.rs#L41)),
[derive.rs](../../../src/koto-gfx/src/derive.rs)
([`collect_immediate_dirty`](../../../src/koto-gfx/src/derive.rs#L154)),
[stats.rs](../../../src/koto-gfx/src/stats.rs)
([`DirtyRectGeometry::from_rects`](../../../src/koto-gfx/src/stats.rs#L51)),
[app_runtime.rs](../../../src/koto-pico/src/firmware/app_runtime.rs#L1110)
(`phase=164`/`phase=169` emit gating).

Depends on: nothing new (reads signals KOTO-0159 / GFX-0006 / GFX-0008 already
produce).

Relates to: [GFX-0008](GFX-0008-commandcountshift-policy-refinement.md) (sibling
fix for the *spurious CommandCountShift* population),
[GFX-0009](GFX-0009-staticrebuild-cost-investigation.md) (sibling investigation for
the *StaticRebuild* population),
[KOTO-0159](../main/KOTO-0159-kotoblocks-dirty-rect-coalescing.md) (the
`coalesce_rects` pass and the `phase=164` fragmentation line this issue studies),
[KOTO-0143](../main/KOTO-0143-full-repaint-instrumentation-coalescing.md)
(full-repaint instrumentation / reasons).

> **Why this is separate from GFX-0008 and GFX-0009.** GFX-0008 removed the
> *spurious* `CommandCountShift` repaints (a localizable single edit the positional
> diff failed to bound). GFX-0009 showed the `StaticRebuild` repaints on
> KotoBlocks/KotoSnake are *legitimate, intrinsic, one-shot*. The **remaining** heavy
> steady-play frames are attributed `RectsExceeded` (with a residual
> `CommandCountShift` tail). `RectsExceeded` is a *different* mechanism again — not a
> mis-derived diff and not a deliberate chrome build, but the **rect-count
> escalation deciding before the coalescer that exists to lower that very count has
> run**. It gets its own investigation.

## Observed problem

Post-GFX-0008 / GFX-0009, `phase=160` on KotoSnake shows the heavy steady frames as
`full=1 full_reason=RectsExceeded`, with:

- `would_degrade=0` / `would_reject=0` / `first_pressure=none` on the budget
  observation (`phase=168`) — so this is **not** a draw-budget problem; budget
  enforcement (GFX-0006C) would not bite, exactly as it does not for the
  `CommandCountShift` tail. Budget is correctly *not the first lever*.
- a dirty *area* well under the `AreaExceeded` threshold (else the frame would have
  been attributed `AreaExceeded`, which outranks `RectsExceeded` in
  [`decide`](../../../src/koto-gfx/src/repaint.rs#L136)).

So by construction every `RectsExceeded` frame is: *small-to-moderate area, high
rect count, immediate-command count unchanged* (a count change would be attributed
`CommandCountShift`). That is the precise signature of **fragmentation that should
coalesce** — which makes the ordering of collect / decide / coalesce the first thing
to inspect.

## Where the dirty set is produced, decided on, and coalesced

The whole sequence lives in
[`present_app_delta`](../../../src/koto-pico/src/firmware/app_render.rs#L285). In
program order:

| Step | Site | Produces |
|---|---|---|
| 1. Board cells → bands | [coalesce_dirty_tiles](../../../src/koto-pico/src/firmware/app_render.rs#L348) | board changes already merged into ≤`BOARD_BAND_CAP` (32) bands; `board_overflow` if it can't fit. |
| 2. Immediate command diff | [collect_immediate_dirty](../../../src/koto-pico/src/firmware/app_render.rs#L391) | one union rect per changed/moved command (prefix/suffix-anchored when length shifts, GFX-0008). |
| 3. Board bands → rects | [app_render.rs:408](../../../src/koto-pico/src/firmware/app_render.rs#L408) | one rect per band. |
| 4. Sprite diff | [app_render.rs:417](../../../src/koto-pico/src/firmware/app_render.rs#L417) | one old∪new rect per changed sprite. |
| 5. Text diff | [app_render.rs:429](../../../src/koto-pico/src/firmware/app_render.rs#L429) | one full-width row-band rect per changed text item. |
| **6. DECIDE** | **[FullRepaintPolicy::decide](../../../src/koto-pico/src/firmware/app_render.rs#L440)** | **skip / incremental / full-repaint, on the count from steps 1–5.** |
| 7. `rects_pre` snapshot | [from_rects, app_render.rs:482](../../../src/koto-pico/src/firmware/app_render.rs#L482) | `DirtyRectGeometry` over the collected set. |
| **8. COALESCE** | **[coalesce_rects, app_render.rs:483](../../../src/koto-pico/src/firmware/app_render.rs#L483)** | **merges scattered rects; `rects_post` = survivor count.** |

### Finding 1 — `RectsExceeded` is decided **pre-coalesce**

Step 6 runs `decide` on `dirty_rects: dirty_len` — the count from steps 1–5,
**before** `coalesce_rects` (step 8). `decide` escalates when
`dirty_rects > FULL_REPAINT_RECTS` (24). So the count that trips `RectsExceeded` is
the *raw fragmentation*, never the *post-coalesce pass count*. The coalescer — whose
entire stated purpose (KOTO-0159) is to "merge scattered dirty rectangles into fewer
recomposite passes" — runs only on the `Incremental` branch, **after** the decision
it could have changed.

### Finding 2 — `rects_pre`/`rects_post` answer this, but `rects_post` is never computed on the escalated frame

- `rects_pre` is produced by
  [`DirtyRectGeometry::from_rects`](../../../src/koto-gfx/src/stats.rs#L51) on *both*
  branches (incremental [app_render.rs:482](../../../src/koto-pico/src/firmware/app_render.rs#L482),
  full-repaint [app_render.rs:457](../../../src/koto-pico/src/firmware/app_render.rs#L457)).
- `rects_post` is only assigned on the **incremental** branch
  ([app_render.rs:484](../../../src/koto-pico/src/firmware/app_render.rs#L484), after
  `coalesce_rects`). On the `RectsExceeded` full-repaint branch it stays `0` — *the
  full-repaint path does not coalesce*, so we have **no measurement of what coalescing
  would have done** to the very frames it should have rescued.
- The `phase=164` dirty-rect line is additionally gated `!full_repaint()`
  ([app_runtime.rs:1115](../../../src/koto-pico/src/firmware/app_runtime.rs#L1115)), so
  a `RectsExceeded` frame emits **no fragmentation line at all** (unlike
  `CommandCountShift`, which GFX-0006C gave the `phase=169` correlation line). The
  recorded `dirty_geom` is latched into `PaintMetrics` but never shipped over UART.

**So the question "did coalescing already have enough information to reduce the rect
count?" cannot be answered from current hardware logs.** That gap is what Stage 0
closes.

### Finding 3 — the working-set cap is *also* pre-coalesce, so the coalescer is structurally dead on heavy frames

`DIRTY_RECT_CAP = FULL_REPAINT_RECTS + 1 = 25`
([app_render.rs:61](../../../src/koto-pico/src/firmware/app_render.rs#L61), explicitly
"one past the full-repaint rect threshold"). Steps 1–5 stop storing at 25 and set
`rect_overflow`. Consequences:

- The collected set handed to `decide` is **capped at 25 and pre-coalesce**, so a
  frame that fragments to, say, 40 raw rects is truncated at 25 with `rect_overflow`
  set — and escalates — *before* anyone counts how many passes those 40 would
  coalesce into.
- The cap is sized the way it is **because** the decision is pre-coalesce: there is
  no reason to collect past "one more than would stay incremental" if coalescing can
  never run on the over-cap frame. The cap and the decision are two halves of the same
  pre-coalesce assumption.
- Net: on exactly the fragmented frames `coalesce_rects` was written (KOTO-0159) to
  rescue, it is never invoked. It only ever runs on frames that were *already* going
  to stay incremental, where it trims raster passes but cannot change a `full=1`
  verdict.

This is the root cause. `RectsExceeded` is, in the fragmentable case, a
**coalesce-ordering defect**, not a genuine "too much changed" signal.

## Classification of `RectsExceeded` causes

Mapping the task's six hypotheses onto the mechanism above. The distinguishing
evidence is the recorded geometry: `rects_pre`, `area_pre`, `bbox_area`,
`max_area`/`min_area`, and the first-four-rect `sample` quads.

| # | Hypothesis | Signature in the geometry | Verdict |
|---|---|---|---|
| 1 | **Genuinely wide dirty damage** | `area_pre` near `FULL_REPAINT_AREA` (¾ surface) | **Not `RectsExceeded`.** Wide area is attributed `AreaExceeded` (outranks rects in `decide`). A `RectsExceeded` frame is by definition *not* area-wide. A genuine full repaint here would be correct, not a defect. |
| 2 | **Fragmented rects that should coalesce** | high `rects_pre`, modest `area_pre`, `bbox_area` ≫ `area_pre` (scattered), uniform small `max_area`≈`min_area` | **Prime suspect.** This is precisely the set `coalesce_rects` targets, and precisely the set the ordering defect prevents it from rescuing. **Confirmable only with a post-coalesce count (Stage 0).** |
| 3 | **Immediate effect / particle footprints** | many `sample` quads of equal tile-sized rects (e.g. 16×16), scattered | A *contributor to* #2: N changed equal-length effect commands → N rects. Adjacent/clustered ones are exactly what `DIRTY_COALESCE_MAX_WASTE` (≈4 tiles) collapses — *if* coalesce ran first. |
| 4 | **Retained HUD / status dirty regions** | a few `sample` quads with `w=320, h=17` (full-width text bands) | Drives **area**, not **count** (each is one wide rect). Few in number; on its own pushes toward `AreaExceeded`, not `RectsExceeded`. Relevant mainly in the *mixed* pattern below. |
| 5 | **Board / sprite / text layer derivation** | board rects already band-shaped (`w`=multiples of tile); sprite/text one-per-change | Low count contributors. Board is **already coalesced** into bands (step 1) before the count, so it is not a fragmentation source unless it overflows `BOARD_BAND_CAP` (→ `board_overflow`, a separate escalation). Sprites/text are one rect per changed index. |
| 6 | **Threshold too conservative** | `rects_pre` just over 24 **and** the post-coalesce count *also* > 24 | Only the residual after #2 is addressed. If coalescing already gets the frame under 24, the threshold is fine and must not be touched; if the post-count is still over, *then* the threshold (or the collection cap) is the conversation — a **later** issue, not this one. |

### The mixed pattern the task calls out

"One full-width HUD rect plus many tiny effect rects" (#4 + #3) is the expected
worst case: the HUD band (320×17) dominates `bbox_area` and inflates `area_pre`
modestly, while a cluster of effect/particle rects supplies the *count* that trips
24. Coalescing would merge the effect cluster (and leave the HUD band as its own
pass, since the bbox over HUD+effects wastes far more than `DIRTY_COALESCE_MAX_WASTE`
— correct), plausibly dropping the pass count under threshold. **This is the
hypothesis Stage 0 is designed to confirm or refute on hardware.**

## Does `RectsExceeded` always require a full repaint?

**No — and that is the whole point.** The escalation keys on a *count* the system
chose to measure before reducing it. Three sub-cases:

- **Coalescible fragmentation** (#2/#3): the post-coalesce pass count is ≤ threshold.
  A full repaint is *unnecessary*; the frame could stay incremental at the
  post-coalesce count. **This is the addressable population.**
- **Irreducible scatter** (checkerboard-like, far-apart changes): coalescing cannot
  merge them within the waste budget, post-count stays high. A full repaint is then a
  defensible *raster-pass* tradeoff (each pass re-walks the whole layer stack) — but
  note this is a *raster-cost* argument, not a *correctness* one.
- **Area-wide** (#1): not `RectsExceeded` at all (attributed `AreaExceeded`).

Only the first is a defect; Stage 0 measures how large it is.

## Staged plan

Each stage is independently shippable. Stage 0 **and Stage 1A** are observe-only and
the only things this issue may land. The reorder (Stage 1) is **behaviour-changing
and deferred**, gated on the Stage-1A evidence (Stage 0 produced a *truncated* lower
bound; Stage 1A is what actually settles the coalescible-vs-irreducible question).

### Stage 0 — diagnostics only (this issue, optional patch)

Close the measurement gap from *Finding 2*: on a `RectsExceeded`- (and
`AreaExceeded`-) attributed frame, **dry-run the coalescer on a scratch copy of the
collected dirty set and emit the hypothetical post-coalesce count**, without changing
the escalation decision. This mirrors the existing observe-only patterns exactly —
GFX-0006's budget *observe* mode and GFX-0009 Stage-0's `phase=170` — and answers the
one question current logs cannot: *would coalescing have kept this frame incremental?*

Concretely:

- In `present_app_delta`, on the `DeltaDecision::FullRepaint(RectsExceeded |
  AreaExceeded)` branch (where `dirty_geom` is already recorded,
  [app_render.rs:457](../../../src/koto-pico/src/firmware/app_render.rs#L457)), copy
  `dirty[..dirty_len]` into a stack scratch array, run `coalesce_rects` on the copy
  with the production `DIRTY_COALESCE_MAX_WASTE`, and stash the survivor count into
  `geom.rects_post`. The real recomposite is untouched — the copy is thrown away. Cost
  is one O(n²) pass over n ≤ 25 rects on a throttled cadence: negligible.
- Add a low-volume `phase=171 app-rects-exceeded` line (caller-gated to
  `RectsExceeded`/`AreaExceeded` frames, one-shot on first + throttled sample, same
  cadence machinery as `phase=169`) carrying: `reason`, `rects_pre`,
  `rects_post_hypothetical`, `rect_overflow` (was the raw count truncated at the
  cap?), `area_pre`, `bbox_area`, `max_area`, `min_area`, and the `sample` quads.

Reading it:

- **`rects_post_hypothetical ≤ 24` with `rect_overflow=0`** → confirmed cause #2: the
  frame *would* stay incremental if coalesce ran before `decide`. Stage 1 is justified.
- **`rects_post_hypothetical > 24`** → irreducible scatter or a genuinely high pass
  count; reorder alone won't help, and the threshold/cap conversation (#6) is a
  separate, later issue.
- **`rect_overflow=1`** → the raw set was truncated at `DIRTY_RECT_CAP`; the
  hypothetical post-count is a *lower bound* (there were >25 raw rects we never
  stored), so Stage 1 must also raise the collection cap, not just reorder.
- **`bbox_area ≫ area_pre` with full-width quad(s) in `sample`** → the mixed HUD +
  effects pattern; corroborates the qualitative classification above.

Observe-only: no policy change, no threshold change, no app-specific knowledge,
`FullRepaintPolicy` / `FullRepaintReason` / hostcalls / bytecode all untouched. **This
is the recommended first safe step.**

**Gate:** if Stage 0 shows the `RectsExceeded` population is dominated by
`rects_post_hypothetical ≤ 24` (coalescible), proceed to Stage 1. If it is dominated
by irreducible scatter, close this issue "intrinsic, full repaint is the correct
raster-pass tradeoff" and the residue becomes a threshold-tuning question for a
future issue. **But Stage 0 data came back truncated** (`rects_pre=25`,
`truncated=1`), so the gate cannot be read yet — that is exactly what Stage 1A fixes.

### Stage 1A — expanded coalesce probe / expanded dirty collection (observe-only)

**Why Stage 1A exists.** Stage-0 hardware shows the defect's signature
(`rects_pre=25`, `rects_coalesced=2`, small `area_coalesced`/`bbox`) but
`would_incremental_after_coalesce=false`, *forced* by `truncated=1`. The collection
stopped at `DIRTY_RECT_CAP = 25` (*Finding 3*), so the dry-run only ever saw the first
25 of an unknown-but-larger raw set. `rects_coalesced=2` is therefore a **lower-bound
artifact of a truncated prefix**, not the true post-coalesce pass count: the 26th..Nth
rects we never stored could merge into those 2 survivors (count unchanged — strong
coalescibility) or land far away (count climbs back over 24 — irreducible). We cannot
tell which, so the Stage-0 gate is unreadable. Stage 1A's sole job: **collect enough
raw dirty rects that the probe is no longer truncated, so `rects_coalesced` and
`would_incremental` become trustworthy** — still without touching any decision.

This stays observe-only by the same construction as Stage 0: the expanded buffer feeds
*only* the dry-run; the live `decide` call reads the unchanged 25-cap inputs, so every
pixel and every attribution is byte-identical to today.

**1. Can the collection buffer be larger than `FULL_REPAINT_RECTS + 1` for
decision/probe purposes?** For *probe* purposes, yes, safely — the 25 cap is sized to
the pre-coalesce decision (*Finding 3*: "no reason to collect past one-more-than-stays-
incremental if coalescing can never run on the over-cap frame"), and a probe is not the
decision. For *decision* purposes, **no — not in this issue**: enlarging the buffer the
policy reads is the behaviour-changing Stage 1, and would also silently move
attribution (see the invariant below). Stage 1A splits the two: the decision keeps its
25-cap snapshot; only a separate probe buffer grows.

**Invariant (the thing that keeps this observe-only).** The decision must consume the
*same* `DeltaInputs` it does today: `rect_overflow` set the instant the raw count
crosses `DIRTY_RECT_CAP`, and `dirty_area` **frozen at that cap**. This second half is
load-bearing: if the expanded collection kept summing area past 25 rects, a truncated
large-area frame that reports `RectsExceeded` today could cross `FULL_REPAINT_AREA` and
flip to `AreaExceeded` — a real attribution change. So Stage 1A keeps two accumulators:
a *decision* snapshot (count/area/overflow frozen exactly at the 25 boundary, fed to
`decide` unchanged) and a *probe* accumulator (count/area continuing to the probe cap,
fed only to `probe_coalesce_pressure`). `would_incremental` then reads the probe's full
area, which is the honest input for the hypothetical incremental decision.

**2. A separate cap — `DIRTY_RECT_PROBE_CAP`.** Add a probe-only constant distinct from
the decision's `DIRTY_RECT_CAP`. Size it from the *structural* upper bound on raw rects
a frame can produce, so it is app-agnostic (no KotoSnake constant) and the probe can
**never** truncate on a non-board-overflow frame:

| Source | Cap | Rects |
|---|---|---|
| Immediate command diff | `MAX_EDIT_REGION` (= `FULL_REPAINT_RECTS`) | 24 |
| Board bands | `BOARD_BAND_CAP` | 32 |
| Sprites | `GAME2D_MAX_SPRITES` | 16 |
| Text items | `GAME2D_MAX_TEXT_ITEMS` | 12 |
| **Total** | | **84** |

`DIRTY_RECT_PROBE_CAP = MAX_EDIT_REGION + BOARD_BAND_CAP + GAME2D_MAX_SPRITES +
GAME2D_MAX_TEXT_ITEMS = 84` is the smallest cap that is provably non-truncating for
every layer's worst case at once — a derived sum of existing caps, not a magic number,
and it tracks them if they change. A smaller round cap (e.g. 64) re-introduces exactly
the lower-bound ambiguity Stage 1A exists to remove, so full-fidelity sizing is the
right call for a *measurement* stage. (`board_overflow` is the one residue: when the
board fragments past `BOARD_BAND_CAP`, `coalesce_dirty_tiles` returns `None` and emits
*zero* board rects upstream, so no rect buffer can recover them — those frames keep
`truncated=1`, correctly, and Stage 1A leaves that honest.)

**3. Stack/RAM impact on the RP2040.** `Rect` is 4×`i32` = 16 B.

- Current: `dirty: [Rect; 25]` = 400 B, plus the Stage-0 `let mut scratch = dirty`
  copy on the escalation branch = another 400 B.
- Stage 1A: `probe: [Rect; 84]` = **1344 B**. Net change vs today is ≈ +944 B if the
  probe is a second buffer, but it can be **net-cheaper**: on the full-repaint branch
  the collected rects are dead after the probe (`present_app_commands` recomposites the
  whole surface and never reads them), so the dry-run may coalesce the probe buffer
  **in place** — the Stage-0 `scratch` copy is unnecessary defensive copying and can be
  dropped. Net delta then ≈ +1344 − 400 (old scratch) ≈ **+944 B**, or ≈ +544 B over
  the *un-probed* baseline.
- This sits against the firmware's silent-stack hazard (see the stack-headroom note:
  ~84.4 KB free above `.bss`, floor ~50 KB; the linker does not catch stack overflow).
  ≈1 KB is far inside the margin, **but**: `present_app_delta` is `async`, so locals
  that the compiler keeps across the `present_app_commands().await` land in the **task
  future** (a `StaticCell`-backed arena), not the down-growing stack — growing the
  future grows `.bss`, not the stack. The probe buffer is consumed *before* that await,
  so it *should* stay stack-resident, but the boundary is the compiler's call.
  **Validation discipline (from the note): build `thumbv6m-none-eabi --release`, run
  `llvm-size`, confirm `.bss` moved by ≲ the future-size delta and stays well above the
  ~50 KB floor; smoke-boot on hardware (the failure mode is a silent boot-time
  HardFault, e.g. UART stops after `phase=146`).** This is the one line item with a
  real-RAM cost, so it carries the heaviest validation.

**4. Stream/coalesce incrementally instead of storing all rects?** Possible, but the
wrong tradeoff *for a measurement stage*. An online accumulator (each `push_dirty`
attempt-merges into a bounded survivor set) keeps RAM at the small cap, but pairwise
online merging is order-dependent and strictly weaker than the batch `coalesce_rects`,
which re-scans each survivor's tail as it grows (`dirty.rs:118`). Online merging can
therefore report a *higher* post-count than the true batch result — biasing the probe
toward "irreducible", i.e. under-counting the very coalescibility we are trying to
detect. For a faithful measurement we want the batch coalescer's best answer, which
needs the whole raw set. So: **store-all + batch for Stage 1A** (fidelity first);
online/streaming is a *Stage-1 production* RAM optimisation to revisit only if the
84-rect buffer proves too heavy in practice.

**5. What should coalesce-before-decide operate on?** Three candidates, mapped to where
each belongs:

- **Expanded raw dirty rects + batch coalesce** → *Stage 1A* (this stage). Maximum
  measurement fidelity; the probe sees everything the batch coalescer needs.
- **Incremental online coalescing** → rejected for measurement (point 4); a possible
  Stage-1 RAM lever only.
- **Two-tier: small raw buffer + coalesced accumulator** → the *Stage-1 production*
  shape. Once Stage 1A confirms the population is coalescible, Stage 1 wants the
  decision path to stay cheap (small live working set) while still feeding the
  coalescer enough input — a small raw staging buffer flushed into a bounded coalesced
  accumulator is the natural structure, sized from Stage-1A's observed post-counts. It
  is **not** needed for Stage 1A (which optimises for a correct number, not for RAM).

Reading Stage-1A `phase=171` (now with `truncated=0` on non-board-overflow frames):

- **`rects_coalesced ≤ 24`, `would_incremental=1`, `truncated=0`** → confirmed
  coalescible on the *full* raw set. Stage 0's truncated `2` was real; **Stage 1 is
  justified.**
- **`rects_coalesced > 24`, `truncated=0`** → the rects Stage 0 never saw push the true
  post-count back over threshold: irreducible scatter. Reorder alone will not help;
  threshold/cap tuning (#6) becomes the separate later conversation.
- **`truncated=1` persists** → only the `board_overflow` residue; a band-buffer
  question, orthogonal to the rect-collection cap.

Observe-only: no policy change, no threshold change, no app-specific knowledge,
`FullRepaintPolicy` / `FullRepaintReason` / hostcalls / bytecode untouched. The single
behavioural risk is the decision-input invariant above; its acceptance test is golden
attribution identity.

**Gate (replaces the Stage-0 gate, which truncation left unreadable):** if Stage-1A
`phase=171` shows the `RectsExceeded` population dominated by `rects_coalesced ≤ 24`
with `truncated=0`, proceed to Stage 1. If dominated by `rects_coalesced > 24` (true,
untruncated), close "intrinsic, full repaint is the correct raster-pass tradeoff".

### Stage 1 — coalesce before deciding (behaviour-changing, deferred, gated on Stage 1A)

Reorder `present_app_delta` so `coalesce_rects` runs **before**
`FullRepaintPolicy::decide`, and feed the policy the *post-coalesce* count (and a
re-summed post-coalesce area). A fragmented-but-coalescible frame then stays
incremental at its true pass count; a frame that is still over threshold *after*
coalescing escalates exactly as today, with the same attribution. Design points to
settle in that issue (recorded here, not decided):

- **Collection cap.** With the decision moved after coalescing, `DIRTY_RECT_CAP` must
  be raised to admit the *raw* fragmentation the coalescer needs as input (else
  truncation pre-empts the merge again — *Finding 3*). Stage 1A already sizes and
  RAM-validates this buffer (`DIRTY_RECT_PROBE_CAP = 84`, the structural sum of the
  layer caps) for the probe; Stage 1 promotes that same buffer onto the decision path
  (or adopts the two-tier small-raw + coalesced-accumulator shape from Stage 1A point
  5 if the post-counts say the full 84-rect live buffer is more than the decision
  needs). Either way it is a bounded stack/future array sizing decision (watch the
  firmware stack-headroom note), not a policy knob.
- **Area re-sum.** `dirty_area` is currently the pre-coalesce summed area (overlaps
  double-counted). Coalescing into bounding boxes changes summed area
  non-monotonically (nested/overlapping shrink it, adjacent preserve it); the policy's
  `AreaExceeded` branch should read a post-coalesce re-sum so the area decision stays
  honest.
- **No threshold change.** `FULL_REPAINT_RECTS` (24) and `FULL_REPAINT_AREA` stay as
  they are; Stage 1 changes *what is counted*, not *the bound*. Threshold tuning, if
  Stage 0 shows residual pressure, is a distinct later issue.

### Stage 2 — out of scope (recorded, not proposed)

- Smarter coalescing (e.g. a larger waste budget, or merging by spatial clustering
  rather than pairwise bbox) — only worth it if Stage 1's post-counts show the current
  `DIRTY_COALESCE_MAX_WASTE` leaves rescuable frames on the table.
- Any threshold or per-app policy change.

## Non-goals (carried from the task framing)

- No rendering-behaviour change in this issue beyond the optional observe-only
  diagnostics (Stage 0, and Stage 1A's probe-only expanded collection — both feed only
  the dry-run; the live decision and every transferred pixel stay byte-identical).
- No threshold change (`FULL_REPAINT_RECTS` / `FULL_REPAINT_AREA` stay as-is).
- No app-specific policy, no per-app profiles, no KotoSnake-specific constants.
- No budget enforcement (Stage 0 confirms budget is not the lever — `would_degrade=0`).
- No hostcall ID / ABI / app-bytecode change; no `APP_DRAW` capacity change.
- No PSRAM / LCD / CodeWindow / audio change.
- `FullRepaintPolicy`, its thresholds, and the `FullRepaintReason` variants stay
  unchanged this issue (the reorder that would touch the call site is Stage 1, gated).

## Acceptance criteria

- [x] Document identifies where `rects_pre` is produced
      ([from_rects, app_render.rs:482/457](../../../src/koto-pico/src/firmware/app_render.rs#L482))
      and where `rects_post` is produced
      ([app_render.rs:484](../../../src/koto-pico/src/firmware/app_render.rs#L484), incremental branch only).
- [x] States that `RectsExceeded` is decided on the **pre-coalesce** count
      ([decide at app_render.rs:440](../../../src/koto-pico/src/firmware/app_render.rs#L440), before
      [coalesce at :483](../../../src/koto-pico/src/firmware/app_render.rs#L483)), and that the
      working-set cap is itself pre-coalesce (*Finding 3*).
- [x] Classifies the `RectsExceeded` causes against the six hypotheses (table above)
      and identifies the mixed HUD+effects pattern.
- [x] States whether coalescing *already had the information* to reduce the count
      (it did — *Finding 1/3* — but is never invoked on the escalated frame) and why
      that cannot be confirmed from current logs (*Finding 2*).
- [x] Recommends the first safe step: **Stage-0 observe-only dry-run coalesce +
      `phase=171` line**, with the reorder (Stage 1) deferred and gated on its data.
- [x] Stage-0 patch landed: `phase=171` fires only on `RectsExceeded`/`AreaExceeded`
      frames, carries `rects_coalesced` + `would_incremental_after_coalesce` + a
      `truncated` flag, is observe-only (dry-run on a scratch copy; recompose is
      byte-identical), adds no app-specific knowledge, and leaves `FullRepaintPolicy` /
      reasons / hostcalls / bytecode untouched. `cargo test -p koto-gfx`/`-game2d`/
      `-core`/`-sim` green, `thumbv6m` build green, `build_apps.py --check` clean,
      firmware clippy adds no new finding (error count unchanged at baseline).
- [ ] Hardware smoke (below) classifies the KotoSnake `RectsExceeded` population as
      coalescible vs irreducible, gating Stage 1.
- [x] **Stage 1A:** a probe-only `DIRTY_RECT_PROBE_CAP` (= 84, the structural sum
      `MAX_EDIT_REGION + BOARD_BAND_CAP + GAME2D_MAX_SPRITES + GAME2D_MAX_TEXT_ITEMS`)
      backs the dry-run while the decision keeps its `DIRTY_RECT_CAP` snapshot via
      `koto_gfx::decision_snapshot` (count capped at 25, overflow reconstructed, area
      already cap-independent). No app-specific constant.
- [x] **Stage 1A:** decision-identity proven host-side — `decision_snapshot` reproduces
      the pre-Stage-1A `(len, overflow)` and the same `decide` verdict for every raw
      count 0..`PROBE_CAP+5` (`expanded_probe_never_changes_the_decision`), and the
      summed area is asserted cap-independent. No `RectsExceeded`→`AreaExceeded`
      attribution drift is possible (area is identical; only the probe widens).
- [x] **Stage 1A:** `phase=171` now carries `decision_rects`/`decision_truncated`
      (the 25-cap snapshot) separately from `rects_pre`/`rects_coalesced`/
      `probe_truncated` (the expanded probe) and `would_incremental_after_probe_coalesce`.
      Probe over/under threshold and structural-max non-truncation are covered host-side.
- [x] **Stage 1A:** `.bss`/RAM delta measured (`llvm-size`): release `koto_firmware`
      `.bss` 176160 → 177120 (**+960 B**, the probe buffer in the async future, not the
      stack), ≈91 KB free above `.bss` (well above the ~50 KB floor). `cargo test`
      `-p koto-gfx`/`-game2d`/`-core`/`-sim` green; `thumbv6m` build green; firmware
      clippy adds no new finding; `build_apps.py --check` clean.
- [ ] **Stage 1A:** hardware boot-smoke (silent-HardFault failure mode) and the
      untruncated coalescible/irreducible classification on KotoSnake (`phase=171`,
      `probe_truncated=0`) — the open gate for Stage 1.
- [x] **Stage 1B:** `koto_gfx::coalesce_then_decide` batch-coalesces the full expanded raw
      dirty set before `FullRepaintPolicy::decide`, feeding the policy the post-coalesce
      count and a re-summed post-coalesce area. The firmware `present_app_delta` drives the
      present from its returned decision and transfers the coalesced survivors. No
      threshold (`FULL_REPAINT_RECTS`/`FULL_REPAINT_AREA`), hostcall, ABI, bytecode, or
      app-specific change.
- [x] **Stage 1B:** truncation fail-safe — a probe-buffer overflow or `board_overflow`
      forces a full repaint regardless of the coalesced count (host-tested
      `truncated_collection_still_full_repaints` / `board_overflow_still_full_repaints`),
      and a wide command restructure keeps its `CommandCountShift` attribution
      (`command_count_shift_attribution_preserved_on_truncation`). A still-over-threshold
      irreducible scatter still full-repaints (`scattered_overcount_still_full_repaints`).
- [x] **Stage 1B:** the headline rescue proven host-side — a coalescible >25 raw-rect set
      becomes `Incremental` with `converted_to_incremental` and the survivors matching a
      direct `coalesce_rects` (`coalescible_overcount_becomes_incremental`); small and
      empty frames are unchanged (`small_incremental_frame_is_unchanged` /
      `empty_frame_skips`), so today's incremental frames stay byte-identical.
- [x] **Stage 1B:** `phase=171` updated to `app-coalesce-decide`, reporting `old_reason` /
      `new_reason` / `converted_to_incremental` alongside the raw/coalesced counts and
      areas; it now fires for surviving rect/area escalations *and* converted frames.
- [x] **Stage 1B:** `cargo test -p koto-gfx`/`-game2d`/`-core`/`-sim` green; `thumbv6m`
      build green; firmware clippy adds no new finding; `build_apps.py --check` clean;
      release `koto_firmware` `.bss` 177120 → 177088 (−32 B, no new buffers).
- [ ] **Stage 1B:** hardware smoke on KotoSnake — the former `full=1 RectsExceeded` frame
      becomes `full=0` with a small rect count on `phase=164` and no visual artifacts;
      `phase=160` reason mix shifts `RectsExceeded`→incremental for the coalescible
      population, irreducible scatter stays `full=1`. Golden-frame identity (rescued
      incremental pixels == prior full recompose).

## Acceptance tests (host-side, `cargo test`)

If the Stage-0 patch lands:

1. **`koto-gfx`** — a fragmented dirty set (e.g. the KOTO-0159 clustered-fragments
   fixture) whose `rects_pre > FULL_REPAINT_RECTS` coalesces to a survivor count
   `≤ FULL_REPAINT_RECTS`: assert the dry-run count matches `coalesce_rects` run
   directly, proving the diagnostic measures the real reduction (no behaviour drift —
   the scratch copy and the production call agree).
2. **`koto-gfx`** — an irreducible-scatter set (checkerboard) stays over threshold
   after the dry-run coalesce, so the diagnostic would not mislabel it as coalescible.
3. **`koto-pico` diag** — `phase=171` line formatting test mirroring the
   `phase=169` test: fields present, parseable, emitted only for
   `RectsExceeded`/`AreaExceeded`.
4. `cargo test -p koto-gfx` / `-p koto-core` / `-p koto-sim` (golden frames) green;
   `thumbv6m` firmware build green; firmware-lib clippy adds no new finding (note:
   `check_all` does not lint `koto-pico` — run `-p koto-pico --target thumbv6m-none-eabi
   --bins` manually).

If the Stage-1A patch lands (still observe-only):

5. **`koto-gfx`** — a synthetic frame with > `DIRTY_RECT_PROBE_CAP`-worth of *raw*
   rects where the first 25 (the Stage-0 prefix) coalesce to a count under threshold
   but the full set does **not**: assert the probe over the full set reports
   `rects_coalesced > FULL_REPAINT_RECTS` / `would_incremental=false`, while the same
   probe over only the first 25 reports `≤ 24` — proving the expanded buffer changes the
   verdict and that the Stage-0 truncated reading was an artifact. The mirror case
   (full set still coalesces under threshold) reports `would_incremental=true`,
   `truncated=0`.
6. **`koto-pico` present-path invariant** — a host-level test (or assertion) that
   `decide` is called with `DeltaInputs` computed from the 25-cap snapshot regardless
   of how many rects the probe accumulated: the same frame yields the identical
   `DeltaDecision` and `FullRepaintReason` with the probe buffer present and absent. In
   particular a frame whose *uncapped* area would cross `FULL_REPAINT_AREA` but whose
   *capped* area does not still attributes `RectsExceeded`, not `AreaExceeded`.
7. **`koto-pico` diag** — `phase=171` formatting test extended to assert `truncated=0`
   on a non-overflow escalation and `truncated=1` is preserved on a `board_overflow`
   frame (the band-buffer residue a bigger rect buffer cannot recover).

## Hardware smoke acceptance plan (KotoSnake / KotoBlocks)

- **KotoSnake — coalescibility of the heavy frame.** Drive a long-snake session
  (the KOTO_KOTOSNAKE_LONG_SNAKE_BUDGET scenario). On each `full_reason=RectsExceeded`
  frame, read `phase=171`: classify `rects_post_hypothetical` ≤ 24 (coalescible — Stage
  1 justified) vs > 24 (irreducible). Capture whether `rect_overflow=1` (raw set
  truncated → cap must rise in Stage 1). Confirm `bbox_area`/`sample` show the expected
  HUD-band + effect-cluster mix.
- **KotoSnake — budget is not the lever.** Confirm the same frames still report
  `would_degrade=0 would_reject=0 first_pressure=none` on `phase=168`, so the decision
  to skip budget enforcement holds.
- **KotoBlocks — no regression / second data point.** Line-clear / hard-drop / game-
  over frames: confirm they stay incremental (GFX-0008/0159 behaviour) and, where any
  `RectsExceeded` appears, capture its `phase=171` classification. KotoBlocks board
  damage is band-coalesced (step 1), so its `RectsExceeded` frames, if any, should be
  effect/text-driven — a useful contrast to KotoSnake's sprite/body-driven ones.
- **Observe-only confirmation.** With the Stage-0 patch, steady-play `phase=160`
  attribution counts (`full=1` frequency and reason mix) are **identical** to the
  pre-patch session — the diagnostic changes no pixels and no decision. Any change in
  the `full=1` rate would mean the dry-run leaked into the real path: a bug.
- **Stage 1A — untruncated coalescibility verdict.** Re-run the long-snake session with
  the expanded probe. On each `full_reason=RectsExceeded` frame read `phase=171` and
  confirm `truncated=0` (the whole point — Stage 0 was `truncated=1`); then classify the
  now-trustworthy `rects_coalesced` ≤ 24 with `would_incremental_after_coalesce=1`
  (coalescible — **Stage 1 justified**) vs > 24 (irreducible). This is the reading that
  actually opens or closes the Stage-1 gate. Confirm `phase=160` attribution counts are
  **still byte-identical** to the pre-Stage-1A session (decision-input invariant held).
  Capture the `.bss`/stack `llvm-size` delta and a clean boot (the silent-HardFault
  failure mode) as the RAM sign-off.
- **Post-Stage-1 (when it lands).** The frames Stage 0 flagged coalescible flip to
  `full=0` (incremental) with `rects_post` ≤ 24 on `phase=164`; the irreducible ones
  still `full=1 RectsExceeded`. Golden-frame identity between the reordered
  incremental path and the prior full recompose for the rescued frames (same pixels,
  fewer passes).

## Notes

With GFX-0008 (spurious CommandCountShift), GFX-0009 (intrinsic StaticRebuild), and
GFX-0010 (RectsExceeded), the three steady-play full-repaint populations are
accounted for. GFX-0010's contribution is to show that `RectsExceeded` is, in its
addressable case, **an ordering artifact**: the count-based escalation runs before —
and the working-set cap is sized to pre-empt — the coalescer built to lower that
count. The first move is to *measure* the gap (Stage 0, observe-only), not to reorder
blind; the reorder (Stage 1) is a small, behaviour-changing follow-up justified only
once the hardware shows the population is genuinely coalescible.
