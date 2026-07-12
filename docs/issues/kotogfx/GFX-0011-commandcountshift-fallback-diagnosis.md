# GFX-0011: CommandCountShift fallback diagnosis and refinement

- Status: **Stage 0 implemented** (observe-only) — the investigation + staged plan, with
  the Stage-0 diagnostic landed: [`EditRegionShape`](../../../src/koto-gfx/src/derive.rs#L104)
  + [`probe_command_shift_coalesce`](../../../src/koto-gfx/src/derive.rs#L204) in `koto-gfx`,
  and the `phase=169` `app-cmdshift` line extended with the wide-edit-region shape, the
  observe-only coalesce probe, and a `fallback_reason` classification. No rendering-behaviour
  change: the live `collect_immediate_dirty` bail, the decision, and every transferred pixel
  are byte-identical; the probe reuses the dead full-repaint `dirty` buffer as scratch (no new
  buffer). Host tests green (`koto-gfx` 111), `thumbv6m` build green, `build_apps.py --check`
  clean, firmware clippy adds no new finding (11 lib warnings, unchanged), release
  `koto_firmware` `.bss` 177088 → 177152 (**+64 B**, the `command_shift` Option in the async
  future).
  **Stage 0b implemented** (diagnostics format only) — the combined `phase=169` line
  (~590 B, ~33 fields) overran the 352 B UART `LineBuffer` and truncated mid-field on hardware
  (`dirty_skipphase=160` corruption). It is split into two short lines that both transmit
  intact: `phase=169 app-cmdshift` keeps the edit-region *shape* summary (`prev_cmds`,
  `cur_cmds`, `prefix_len`, `suffix_len`, `edit_region_prev`, `edit_region_cur`,
  `max_edit_region`, `dirty_skipped`, `fallback_reason`), and a new sparse
  `phase=174 app-cmdshift-probe` carries the observe-only coalesce probe (`rects_pre`,
  `rects_coalesced`, `area_pre`, `area_coalesced`, `bbox`, `probe_truncated`,
  `would_incremental_after_probe_coalesce`). The budget/class correlation the original
  `phase=169` carried is dropped — `phase=168` already reports it, and GFX-0008 established
  these frames are not a budget population. Same one-shot-plus-throttled cadence; no behaviour,
  threshold, or `MAX_EDIT_REGION` change. `thumbv6m` build green, tests green, clippy unchanged.
  The behaviour-changing refinement (collecting the wide edit region so Stage 1B can rescue the
  coalescible count-shift frames — *Stage 1*) is **deferred and gated**: do not proceed until
  `phase=174` is confirmed readable on hardware. Hardware smoke on KotoSnake `CommandCountShift`
  frames (complete `phase=169` + `phase=174`) is the open gate.
  **Stage 1 implemented** (behaviour-changing collection-cap change) — the live
  [`present_app_delta`](../../../src/koto-pico/src/firmware/app_render.rs#L308) now passes
  `collect_immediate_dirty` the expanded `DIRTY_RECT_PROBE_CAP` (the structural sum every other
  layer already gets) instead of the smaller `MAX_EDIT_REGION`, so a wide count shift **collects**
  its edit-region union rects and feeds them into the existing GFX-0010 Stage-1B
  `coalesce_then_decide` path rather than bailing and force-escalating (`rects_pre=0`). A
  coalescible count shift (the smoothly-shifting snake body) now stays incremental at its
  post-coalesce pass count (`converted_to_incremental`, reported on `phase=171`); a region wider
  than even the expanded cap still bails and fails safe (case #4, `probe_truncated`); a
  genuinely wide-area count shift still full-repaints on the re-summed post-coalesce area
  (attributed `CommandCountShift`, the count having changed). No `FullRepaintPolicy` threshold,
  hostcall, ABI, bytecode, `APP_DRAW`, PSRAM, LCD, CodeWindow, audio, or CPU-ownership change;
  the equal-length path and the bounded (`region ≤ MAX_EDIT_REGION`) count shift are byte-
  identical (neither cap bails below the region span). The Stage-0 observe-only dry-run
  (`probe_command_shift_coalesce`) is retired from the live path — the wide region is now
  collected for real, so `outcome.pressure` is the actual contrast; `phase=169`/`phase=174` are
  re-sourced from it (`fallback_reason`, `dirty_skipped`, and the live cap on `phase=169`;
  `old_reason`/`new_reason`/`converted_to_incremental` on `phase=174`). `cargo test`
  `-p koto-gfx` (115, +4 Stage-1 collect-then-decide tests) / `-game2d` / `-core` / `-sim` (13
  golden) green; `thumbv6m` build green; `build_apps.py --check` clean; firmware lib clippy
  adds no new finding (11 warnings, unchanged); release `koto_firmware` `.bss` 177152 →
  177152 (**±0 B**, the expanded buffer is Stage-1A's and the retired probe frees its scratch
  use). **Hardware smoke confirmed** on KotoSnake: the former `full=1 CommandCountShift` frames
  now mostly stay incremental (coalescible wide count-shift edits convert to `full=0` with small
  dirty regions), while genuinely wide post-coalesce damage remains a full repaint — e.g.
  `area_coalesced` ≈ 77k–80k (over `FULL_REPAINT_AREA = 76800`) stays `CommandCountShift`,
  exactly as designed. GFX-0011 is complete. (Orthogonal follow-up, **out of this issue's
  scope**: with the present path no longer the bottleneck, the remaining major KotoSnake spike is
  now VM-side — e.g. `full=0` frames with `vm_us ≈ 165 ms` — a bytecode-execution cost, not a
  render one; it wants its own investigation.)
- Type: investigation / proposal (the Stage-0 diagnostic it proposes is observe-only by
  construction; the refinement is a deferred follow-up — see *Non-goals*).
- Priority: P3
- Requirements: NFR-PERF-1, NFR-DRAW-1

Source of truth:
[derive.rs](../../../src/koto-gfx/src/derive.rs)
([`collect_immediate_dirty`](../../../src/koto-gfx/src/derive.rs#L154),
[`MAX_EDIT_REGION`](../../../src/koto-gfx/src/derive.rs#L92),
the wide-region bail [derive.rs:191](../../../src/koto-gfx/src/derive.rs#L191),
the edit-region positional pairing [derive.rs:200](../../../src/koto-gfx/src/derive.rs#L200)),
[repaint.rs](../../../src/koto-gfx/src/repaint.rs)
([`coalesce_then_decide`](../../../src/koto-gfx/src/repaint.rs#L257),
the `probe_truncated` fail-safe [repaint.rs:295](../../../src/koto-gfx/src/repaint.rs#L295),
[`FullRepaintPolicy::decide`](../../../src/koto-gfx/src/repaint.rs#L150),
[`decision_snapshot`](../../../src/koto-gfx/src/repaint.rs#L344)),
[app_render.rs](../../../src/koto-pico/src/firmware/app_render.rs)
([`present_app_delta`](../../../src/koto-pico/src/firmware/app_render.rs#L308),
the `collect_immediate_dirty` call passing `MAX_EDIT_REGION`
[app_render.rs:420](../../../src/koto-pico/src/firmware/app_render.rs#L420),
`DIRTY_RECT_PROBE_CAP` [app_render.rs:84](../../../src/koto-pico/src/firmware/app_render.rs#L84)),
[diag.rs](../../../src/koto-pico/src/firmware/diag.rs)
([`log_app_cmdshift_correlation` / `phase=169`](../../../src/koto-pico/src/firmware/diag.rs#L504)),
[app_runtime.rs](../../../src/koto-pico/src/firmware/app_runtime.rs#L1019)
(`phase=169` emit gating).

Depends on: nothing new (reads signals GFX-0008 / GFX-0010 / KOTO-0159 already produce).

Relates to: [GFX-0008](GFX-0008-commandcountshift-policy-refinement.md) (the aligned
immediate diff whose wide-region *bail* is the mechanism studied here),
[GFX-0010](GFX-0010-rectsexceeded-pressure-investigation.md) (the coalesce-before-decide
reorder whose rescue this population is currently excluded from),
[KOTO-0159](../main/KOTO-0159-kotoblocks-dirty-rect-coalescing.md) (the `coalesce_rects`
pass), [KOTO-0143](../main/KOTO-0143-full-repaint-instrumentation-coalescing.md)
(full-repaint instrumentation / reasons).

> **Why this is separate from GFX-0008 and GFX-0010.** GFX-0008 fixed the *spurious*
> `CommandCountShift` repaints where a single localizable insert/remove had a **bounded**
> edit region — those now stay incremental. GFX-0010 Stage 1B fixed the coalescible
> `RectsExceeded` frames by coalescing the *collected* raw rect set before deciding. The
> **residual** `CommandCountShift` full repaints on KotoSnake are neither: they are frames
> whose edit region is **wide** (so GFX-0008 bails) and whose damage is therefore **never
> collected** (so GFX-0010 Stage 1B has nothing to coalesce). This is a third, distinct
> mechanism — a *collection* gap, not a *decision* gap — and gets its own investigation.

## Observed problem

Post-GFX-0008 / GFX-0010, KotoSnake steady play still shows heavy full-repaint frames:

```
phase=169 app-cmdshift ... prev_cmds=36 cur_cmds=35 ... rects_pre=0 rects_post=0
                           would_degrade=0 would_reject=0 first_pressure=none
phase=160 ... full=1 full_reason=CommandCountShift
```

The signature is precise and, at first read, paradoxical:

- **The immediate command count changed** (`36 → 35`) — so the frame takes the
  length-shift path in the diff and is attributed `CommandCountShift`.
- **`rects_pre=0`** — *no dirty rects were collected at all*, yet the frame still full
  repaints. A frame that changed nothing would `Skip`; a frame that changed a lot would
  show a high `rects_pre`. Zero collected rects on a full-repaint frame is the tell.
- **Budget is not the lever** — `would_degrade=0 would_reject=0 first_pressure=none`, the
  same GFX-0008 result: this is not an over-budget frame.

So the question is exactly the task's: *why does a count-shift frame full-repaint with
`rects_pre=0`, and are these edits truly unsafe or merely uncollected?*

## Where the count-shift fallback is produced

The whole sequence is [`present_app_delta`](../../../src/koto-pico/src/firmware/app_render.rs#L308).
In program order for a count-shift frame:

| Step | Site | What happens on a wide count shift |
|---|---|---|
| 1. Immediate diff | [`collect_immediate_dirty`, app_render.rs:420](../../../src/koto-pico/src/firmware/app_render.rs#L420) (passed `MAX_EDIT_REGION` = 24) | length differs → prefix/suffix align → **region > 24 → set `probe_overflow=true` and `return` without diffing** ([derive.rs:191](../../../src/koto-gfx/src/derive.rs#L191)). **No command rect is pushed.** |
| 2. Board / sprite / text | [app_render.rs:437-464](../../../src/koto-pico/src/firmware/app_render.rs#L437) | push their own rects (often *also* zero on a pure snake-move frame — the body is immediate commands, not board/sprite). |
| 3. Snapshot | [`decision_snapshot`, app_render.rs:481](../../../src/koto-pico/src/firmware/app_render.rs#L481) | `probe_overflow` folds into `decision_rect_overflow`. |
| 4. Coalesce+decide | [`coalesce_then_decide`, app_render.rs:489](../../../src/koto-pico/src/firmware/app_render.rs#L489) | `probe_truncated = probe_overflow \|\| board_overflow = true` → fail-safe forces `FullRepaint`; `command_count_changed=true` attributes `CommandCountShift`. |
| 5. Record | [app_render.rs:504](../../../src/koto-pico/src/firmware/app_render.rs#L504) | `geom` over `dirty[..probe_len]` = `dirty[..0]` → `rects_pre=0`, `rects_post=0`. |

### Finding 1 — the fallback is driven by `probe_overflow`, not by any collected rect

The escalation does **not** come from a rect count or an area crossing a threshold. It comes
from the truncation fail-safe in
[`coalesce_then_decide`](../../../src/koto-gfx/src/repaint.rs#L291): `rect_overflow:
probe_truncated`. On a wide count shift `probe_truncated` is set the instant
[`collect_immediate_dirty`](../../../src/koto-gfx/src/derive.rs#L191) bails. The decision
is "the raw set is incomplete, so I cannot trust it — repaint everything," and
`command_count_changed` supplies the attribution. **The count change itself never
escalates** (as GFX-0008 established); the *bail* does.

### Finding 2 — `rects_pre=0` because dirty collection of the edit region was *skipped*

This is the crux, and it is the opposite of "nothing changed." The wide-region branch
([derive.rs:191-196](../../../src/koto-gfx/src/derive.rs#L191)) sets `overflow` and returns
**before the edit-region diff loop** ([derive.rs:200-204](../../../src/koto-gfx/src/derive.rs#L200))
ever runs. So the union rects that *would* describe the damage are never pushed:
`probe_len` stays 0 (from the command layer), `geom.rects_pre=0`, and the throttled
`phase=169` line faithfully reports zero. The frame changed a great deal; we simply chose
not to measure it, on the GFX-0008 premise that a wide region "is not a localizable single
edit" and diffing it would cost more than a full repaint.

### Finding 3 — the immediate diff is the one layer GFX-0010 left truncating below the probe cap

GFX-0010 Stage 1A/1B widened the working set to `DIRTY_RECT_PROBE_CAP = 84`
([app_render.rs:84](../../../src/koto-pico/src/firmware/app_render.rs#L84)) so board,
sprite, and text collection feed the full structural dirty set into
`coalesce_then_decide`. But the **immediate command diff is still passed the *un*-widened
`MAX_EDIT_REGION = 24`** ([app_render.rs:420-430](../../../src/koto-pico/src/firmware/app_render.rs#L420)),
so it bails at 24 and sets `probe_overflow` — the one collection path that still truncates
short of the probe cap. Consequently:

- Stage 1B's rescue (coalesce a fragmented-but-collected set under threshold) **cannot fire
  for the count-shift population**: there is nothing in `dirty[..probe_len]` to coalesce.
  The `probe_truncated` fail-safe short-circuits to full repaint regardless.
- The `CommandCountShift` tail GFX-0010 explicitly left to a follow-up
  (["a wide command restructure keeps its `CommandCountShift` attribution"])
  is *structurally* every wide count shift, not just genuine restructures — because the
  bail happens before we can tell them apart.

This is the root cause. Like GFX-0010's `RectsExceeded`, the addressable case is an
**ordering/collection artifact**: we decide the region is "too wide to bother collecting"
*before* the coalescer that exists to collapse that width has seen it.

### Finding 4 — positional pairing over the wide region is pixel-*correct*, only potentially wasteful

The reason we can even consider collecting the wide region: the edit-region diff
([derive.rs:200-204](../../../src/koto-gfx/src/derive.rs#L200)) pairs `prev[prefix+k]`
with `cur[prefix+k]` and pushes the union of their footprints. For a length-shifted list
this is the *misaligned* pairing GFX-0008 called out — but misaligned pairing is **wasteful,
not wrong**: each slot's union rect covers whatever that slot painted in *both* frames, and
every painted pixel belongs to some slot in each frame, so the collected set always covers
the symmetric difference. Recompositing the union rects reproduces byte-identical pixels
(the same invariant that lets GFX-0010 coalesce and still match the full repaint). So
collecting the wide region and coalescing it is **pixel-safe**; the only open question is
whether the coalesced result is *cheaper* than a full repaint — a raster-cost question,
exactly as for GFX-0010's irreducible-vs-coalescible scatter. That is what Stage 0 measures.

### Why a snake-move frame lands here

KotoSnake draws its body as immediate commands. Frame to frame the whole body shifts one
cell (head advances, tail retracts), so **almost every command differs positionally** —
the common prefix and suffix are near-zero — and a length change (grow/shrink by one)
makes the edit region essentially the entire list. Region ≫ 24, so the diff bails
(`rects_pre=0`) instead of collecting ~35 adjacent union rects that, being contiguous along
the body, would very plausibly coalesce into a handful of bands. **Whether they actually do
is the empirical question Stage 0 answers** — and the honest reason to measure before
reordering.

## Classification of `CommandCountShift` cases

Mapping the task's five categories onto the mechanism above. The distinguishing evidence is
the alignment geometry (`prefix_len`, `suffix_len`, `edit_region_prev/cur`) plus the
dry-run coalesce count Stage 0 adds.

| # | Case | Signature | Current outcome | Addressable? |
|---|---|---|---|---|
| 1 | **Bounded insert/remove** | `region ≤ MAX_EDIT_REGION`; long common prefix/suffix | **Already incremental** (GFX-0008). Collected, diffed, under threshold. | Done — not a fallback. |
| 2 | **Wide edit region, currently fail-safe** | `region > MAX_EDIT_REGION`; short prefix/suffix; `rects_pre=0`; `probe_truncated=1` on the command layer only | **Full repaint** — the bail skips collection. | **Prime suspect.** Coalescibility unknown until we collect + dry-run (Stage 0). The KotoSnake population. |
| 3 | **Equal-length reorder / content shift** | `prev_len == cur_len` | **Not `CommandCountShift` at all** — the equal-length path never bails ([derive.rs:171](../../../src/koto-gfx/src/derive.rs#L171)); it diffs positionally and escalates (if at all) as `RectsExceeded`/`AreaExceeded`, which **GFX-0010 Stage 1B already handles**. | Out of scope here (covered by GFX-0010). Listed to rule it out. |
| 4 | **Truncated / overflow collection** | region (or an equal-length diff) genuinely exceeds `DIRTY_RECT_PROBE_CAP = 84`; `probe_truncated=1` even after widening | Full repaint. | **Must stay fail-safe** — an incomplete set cannot be trusted (GFX-0010's rule). Only widening the *cap* would move it, and 84 is already the structural max. |
| 5 | **Truly unsafe restructure** | near-zero prefix/suffix *and* the collected/coalesced damage is genuinely wide-area (title→gameplay, list rebuilt from scratch) | Full repaint. | **Full repaint is correct** — the area re-sum in `coalesce_then_decide` escalates it as `AreaExceeded`/`CommandCountShift` on its own merit. |

The whole refinement turns on separating **#2** (uncollected-but-coalescible) from **#4/#5**
(genuinely truncated or genuinely wide). Today they are indistinguishable because #2 is
never collected. Stage 0 makes them distinguishable **without changing any decision**.

## Does `CommandCountShift` always require a full repaint?

**No — for case #2 specifically, and only measurably so.** The escalation keys on a *bail we
chose* (region > 24) before collecting the damage the coalescer could reduce. Three
sub-cases mirror GFX-0010:

- **Uncollected-but-coalescible (#2):** the wide region's union rects, once collected and
  coalesced, stay ≤ threshold in count and area. A full repaint is *unnecessary*; the frame
  could stay incremental at the post-coalesce pass count. **The addressable population.**
- **Truncated (#4):** the region exceeds even the 84-rect probe cap — the set is genuinely
  incomplete, so the fail-safe is correct.
- **Genuinely wide (#5):** collected + coalesced area crosses `FULL_REPAINT_AREA` — a full
  repaint is the correct raster-pass tradeoff (attributed `AreaExceeded` or, with the count
  change, `CommandCountShift`).

Only the first is a defect; Stage 0 measures how large it is.

## Staged plan

Each stage is independently shippable. **Stage 0 is observe-only and is the only thing this
issue may land.** The refinement (Stage 1) is behaviour-changing and deferred, gated on the
Stage-0 evidence.

### Stage 0 — diagnostics only (this issue, recommended first safe step)

Close the two measurement gaps from *Findings 2–3*, without changing the escalation
decision, mirroring GFX-0006's observe mode and GFX-0010 Stage 0/1A exactly:

**(a) Surface the alignment geometry the bail currently discards.**
[`collect_immediate_dirty`](../../../src/koto-gfx/src/derive.rs#L154) computes `prefix`,
`suffix`, and `region` as internal locals and throws them away on the bail path. Return them
(e.g. via a small `EditRegionShape { prefix_len, suffix_len, edit_region_prev, edit_region_cur,
max_edit_region, bailed }` out-param, populated on both the diffed and bailed paths) so the
firmware can report them.

**(b) Dry-run collect-the-region-and-coalesce, observe-only.** On a `CommandCountShift`
frame, run a *scratch* collection of the wide edit region into a probe buffer — the same
positional pairing ([derive.rs:200](../../../src/koto-gfx/src/derive.rs#L200)) but capped at
`DIRTY_RECT_PROBE_CAP` instead of bailing at `MAX_EDIT_REGION` — then dry-run
`coalesce_rects` on the copy and record the hypothetical post-coalesce count and re-summed
area. The live decision is untouched (the scratch buffer is thrown away; the frame still
full-repaints exactly as today). This is the direct analog of GFX-0010 Stage 1A's expanded
probe, applied to the one layer that still truncates. Cost: one bounded collection + one
O(n²) coalesce over n ≤ 84 on the throttled cadence — negligible, and the buffer is consumed
before the `present_app_commands().await` so it stays stack-resident (watch the
`.bss`/stack `llvm-size` delta per the firmware stack-headroom discipline).

**(c) Emit two short lines** ([diag.rs](../../../src/koto-pico/src/firmware/diag.rs)) on a
`CommandCountShift` frame, on the existing throttled cadence. Stage 0 first appended every
field to `phase=169`, but the combined line (~590 B / ~33 fields) overran the 352 B
`LineBuffer` and truncated mid-field on hardware (`dirty_skipphase=160`). **Stage 0b** splits
it so each line transmits intact — the shape summary readers need to classify the fallback is
now independent of the longer probe payload:

`phase=169 app-cmdshift` — the edit-region **shape**:

- `prev_cmds`, `cur_cmds` — the immediate counts that differed.
- `prefix_len`, `suffix_len` — the common-anchor lengths (near-zero ⇒ smoothly-shifting
  list like the snake body; large ⇒ localizable edit that nonetheless overran the cap).
- `edit_region_prev`, `edit_region_cur` — the region span on each side (`> max_edit_region`
  on the case-#2 population).
- `max_edit_region` — the cap that was compared against (currently 24), the bail boundary.
- `dirty_skipped` — a boolean: was the edit-region diff loop skipped (the bail)? `1` on
  every case-#2 frame, and the direct explanation for `rects_pre=0` upstream.
- `fallback_reason` — the classification from the probe: `bounded` (region was diffed),
  `coalescible` (bailed but would coalesce incremental — case #2, the rescue candidate),
  `probe_truncated` (region > probe cap — #4), `area_exceeded` / `rects_exceeded` (genuinely
  wide / irreducible after coalescing — #5). Distinguishes #2 from #4/#5 on the short line.

`phase=174 app-cmdshift-probe` — the observe-only dry-run (b) results, sparse:

- `rects_pre`, `rects_coalesced`, `area_pre`, `area_coalesced`, `bbox` — what the wide region
  collects and coalesces to (`bbox ≫ area_pre` reads as scattered).
- `probe_truncated` — the region overflowed even the probe buffer (an incomplete set).
- `would_incremental_after_probe_coalesce` — `rects_coalesced ≤ FULL_REPAINT_RECTS &&
  area_coalesced < FULL_REPAINT_AREA && !probe_truncated`. The gate reading.

The budget/class correlation the original GFX-0006C `phase=169` carried is dropped: `phase=168`
already reports budget/class on its own cadence, and GFX-0008 established these frames are not
a budget population.

Reading it:

- **`fallback_reason=coalescible`** (equivalently `phase=174`
  `would_incremental_after_probe_coalesce=1` with `probe_truncated=0` and `dirty_skipped=1`) →
  confirmed case #2: the wide count shift *would* stay incremental if the region were collected
  before deciding. **Stage 1 is justified.**
- **`fallback_reason=area_exceeded`** → case #5, genuinely wide; full repaint correct.
- **`probe_truncated=1`** → case #4; region exceeds the 84-rect structural max; fail-safe
  correct.
- **`prefix_len`/`suffix_len` near-zero across the population** → confirms the
  smoothly-shifting-list mechanism (*"Why a snake-move frame lands here"*) rather than
  genuine restructures.

Observe-only: no policy change, no threshold change, no app-specific knowledge;
`FullRepaintPolicy` / `FullRepaintReason` / `MAX_EDIT_REGION` / hostcalls / bytecode all
untouched. Every pixel and every attribution stays byte-identical to today. **This is the
recommended first safe step.**

**Gate:** if Stage 0 shows the KotoSnake `CommandCountShift` population is dominated by
`dirty_skipped=1 & would_incremental=1` (uncollected-but-coalescible), proceed to Stage 1.
If dominated by `probe_truncated=1` or wide `region_area_coalesced`, close "intrinsic — the
wide-region bail is the correct raster-pass tradeoff" and the residue is a
cap/threshold-tuning question for a later issue.

### Stage 1 — collect the wide region so Stage 1B can decide it (behaviour-changing, **implemented**)

**Landed** (host-side; hardware smoke is the open gate). The live path passes
`collect_immediate_dirty` the expanded `DIRTY_RECT_PROBE_CAP` instead of `MAX_EDIT_REGION`, so
the wide edit region is collected and the existing GFX-0010 Stage-1B `coalesce_then_decide`
owns the decision on the collected + coalesced set. The design points below held as recorded:
the bail became the probe-cap overflow (case #4 fail-safe), the post-coalesce area re-sum
escalates a genuinely wide case #5 on its own merit, `command_count_changed` keeps a still-
escalating frame's `CommandCountShift` attribution, and no new buffer or threshold was added.
The observe-only Stage-0 probe is retired from the live path (the region is now collected for
real); `phase=169`/`phase=174` are re-sourced from the real `CoalescePressure`.

Remove the premature `MAX_EDIT_REGION` short-circuit from the *live* path: pass
`collect_immediate_dirty` the `DIRTY_RECT_PROBE_CAP` (as every other layer already gets),
so a wide count shift **collects** its edit-region union rects instead of bailing, and let
the existing GFX-0010 Stage 1B `coalesce_then_decide` make the call on the *collected +
coalesced* count and re-summed area. Design points to settle in that issue (recorded here,
not decided):

- **The bail becomes the probe-cap overflow.** `collect_immediate_dirty` still sets
  `overflow` if the region exceeds the (now larger) probe cap — that is case #4, which must
  stay fail-safe. Only the boundary moves (24 → 84), not the fail-safe itself.
- **Area re-sum is load-bearing.** The wide region's *misaligned* union rects can cover far
  more area than the true damage. Stage 1B's post-coalesce `area_coalesced`
  ([repaint.rs:287](../../../src/koto-gfx/src/repaint.rs#L287)) is the safety valve: a
  genuinely wide case #5 crosses `FULL_REPAINT_AREA` and full-repaints on its own merit
  (attributed `CommandCountShift`, the count still changed). No new threshold.
- **Attribution stays honest.** `command_count_changed` still reorders a *still-escalating*
  frame's reason to `CommandCountShift` — so a case-#5 frame keeps its label; only case-#2
  frames convert to incremental (`converted_to_incremental=1`, already surfaced on
  `phase=171`).
- **RAM.** Reuse the existing 84-rect probe buffer (`present_app_delta` already sizes it);
  the wide-region collection writes into the same `dirty` array. No new buffer — this is a
  *cap argument change* at the call site, not a new allocation. Re-run the `llvm-size`
  `.bss`/stack check per the firmware discipline.
- **No app-specific knowledge, no per-app profile, no `MAX_EDIT_REGION` retune as a policy
  knob** — `MAX_EDIT_REGION` stops being a *live* bail and survives only if Stage 0 shows a
  reason to keep a cheaper live cap distinct from the probe cap (a cost, not correctness,
  decision).

### Stage 2 — out of scope (recorded, not proposed)

- Index-stable / keyed immediate diffing (so a shifting body does not misalign at all,
  collecting the *true* small damage instead of wide union rects) — a larger derivation
  change, only worth it if Stage 1's post-coalesce counts show the misaligned unions leave
  rescuable frames on the table.
- Any threshold, `MAX_EDIT_REGION`, or per-app policy change.

## Non-goals (carried from the task framing)

- No rendering-behaviour change in this issue beyond the observe-only Stage-0 diagnostics
  (the dry-run feeds only the log line; the live decision and every transferred pixel stay
  byte-identical).
- No threshold change (`FULL_REPAINT_RECTS` / `FULL_REPAINT_AREA` stay as-is); no
  `MAX_EDIT_REGION` change this issue.
- No app-specific policy, no per-app profiles, no KotoSnake-specific constants.
- No budget enforcement (Stage 0 re-confirms budget is not the lever —
  `would_degrade=0`/`would_reject=0`).
- No hostcall ID / ABI / app-bytecode change; no `APP_DRAW` capacity change.
- No PSRAM / LCD / CodeWindow / audio / CPU-ownership change.
- `FullRepaintPolicy`, its thresholds, and the `FullRepaintReason` variants stay unchanged
  (the collection-cap change that would touch the live path is Stage 1, gated).

## Acceptance criteria

- [x] Identifies where the `CommandCountShift` fallback is produced — the wide-region bail
      in [`collect_immediate_dirty`](../../../src/koto-gfx/src/derive.rs#L191) setting
      `overflow`, escalated by the `probe_truncated` fail-safe in
      [`coalesce_then_decide`](../../../src/koto-gfx/src/repaint.rs#L295) and attributed by
      `command_count_changed` (*Finding 1*).
- [x] Explains `rects_pre=0` / `rects_post=0`: the edit-region diff loop is **skipped** by
      the bail, so no command rect is ever collected (*Finding 2*), and confirms the
      fallback occurs **before** collecting edit-region damage.
- [x] States that `prefix_len`, `suffix_len`, `edit_region_prev/cur`, and `max_edit_region`
      explain the fallback (short anchors + wide region on a smoothly-shifting list), and
      that the immediate diff is the one layer GFX-0010 left truncating below the probe cap
      (*Finding 3*).
- [x] States whether the GFX-0010 coalesce-before-decide path can safely handle some
      count-shift cases if the edit region is fully collected and non-truncated — **yes for
      case #2**, because positional pairing over the wide region is pixel-correct
      (*Finding 4*), gated on area re-sum and non-truncation (Stage 1).
- [x] Classifies the five `CommandCountShift` cases (table) and separates the addressable
      #2 from the fail-safe #4/#5 and the out-of-scope #3.
- [x] Recommends the first safe step: **Stage-0 observe-only alignment-geometry + dry-run
      coalesce on `phase=169`**, with the collection-cap change (Stage 1) deferred and gated.
- [x] **Stage 0:** the wide-edit-region shape + observe-only coalesce probe are computed on a
      `CommandCountShift` full repaint (`phase=169` / `phase=174`); it is observe-only (the probe
      reuses the dead full-repaint `dirty` buffer as scratch; the live full repaint is
      byte-identical), adds no app-specific knowledge, and leaves `FullRepaintPolicy` / reasons /
      the live `MAX_EDIT_REGION` bail / hostcalls / bytecode untouched. Core logic lives in
      `koto-gfx` ([`EditRegionShape::of`](../../../src/koto-gfx/src/derive.rs#L127),
      [`probe_command_shift_coalesce`](../../../src/koto-gfx/src/derive.rs#L204)) as a pure,
      host-tested helper; `collect_immediate_dirty`'s length-shift branch was refactored to
      reuse `EditRegionShape::of` (single source of truth, behaviour-identical).
- [x] **Stage 0b:** the diagnostics are split into two UART lines that fit the 352 B
      `LineBuffer` (the combined ~590 B line truncated mid-field on hardware):
      [`log_app_cmdshift_correlation`](../../../src/koto-pico/src/firmware/diag.rs) emits the
      short `phase=169 app-cmdshift` shape summary (`prev_cmds`, `cur_cmds`, `prefix_len`,
      `suffix_len`, `edit_region_prev`, `edit_region_cur`, `max_edit_region`, `dirty_skipped`,
      `fallback_reason`), and the new
      [`log_app_cmdshift_probe`](../../../src/koto-pico/src/firmware/diag.rs) emits the sparse
      `phase=174 app-cmdshift-probe` (`rects_pre`, `rects_coalesced`, `area_pre`,
      `area_coalesced`, `bbox`, `probe_truncated`, `would_incremental_after_probe_coalesce`).
      Budget/class fields dropped (covered by `phase=168`). Same cadence; no behaviour /
      threshold / `MAX_EDIT_REGION` change. `thumbv6m` build green; tests green; clippy unchanged.
- [x] **Stage 0:** host tests — bounded edit-region shape
      (`edit_region_shape_bounded_single_removal`), wide-shift bail reporting `dirty_skipped`
      (`edit_region_shape_wide_shift_bails` / `edit_region_shape_matches_live_bail`), and probe
      collection/coalescing without changing the live decision
      (`probe_collects_wide_region_and_coalesces_incremental`, `probe_scattered_region_not_incremental`,
      `probe_truncates_past_buffer_capacity`, `probe_does_not_change_live_decision`).
      `cargo test -p koto-gfx` (111) / `-game2d` / `-core` / `-sim` (13 golden) green;
      `thumbv6m` build green; firmware clippy adds no new finding (11 lib warnings, unchanged
      from baseline); `build_apps.py --check` clean; release `koto_firmware` `.bss`
      177088 → 177152 (**+64 B**, no new rect buffer — the probe reuses the dead `dirty` set).
- [ ] **Stage 0/0b:** hardware smoke shows **complete** `phase=169` *and* `phase=174` lines
      (no truncation) on KotoSnake `CommandCountShift` frames, classifies the population as
      case-#2 (coalescible) vs #4/#5, and confirms `phase=160` attribution counts are
      byte-identical to the pre-patch session (the diagnostic changes no decision). **Do not
      proceed to Stage 1 until `phase=174` is confirmed readable on hardware.**
- [x] **Stage 1:** the live `present_app_delta` passes `collect_immediate_dirty` the expanded
      `DIRTY_RECT_PROBE_CAP` instead of `MAX_EDIT_REGION`, so a wide count shift collects its
      edit-region union rects and feeds them into the existing `coalesce_then_decide` path; no
      `FullRepaintPolicy` threshold, hostcall, ABI, bytecode, `APP_DRAW`, PSRAM, LCD, CodeWindow,
      audio, or CPU-ownership change, and no app-specific constant.
- [x] **Stage 1:** host tests (`koto-gfx`) — a wide count-shift edit region that used to bail
      (`dirty_skipped`, forced repaint) becomes `Incremental` with `converted_to_incremental`
      after coalescing (`stage1_wide_count_shift_becomes_incremental_after_coalescing`); a
      genuinely wide-area count shift still full-repaints on the re-summed area
      (`stage1_wide_count_shift_area_exceeded_still_full_repaints`); a region past even the
      expanded cap still fails safe (`stage1_wide_count_shift_truncated_still_full_repaints`);
      and a bounded count shift collects byte-identically under either cap and stays incremental
      (`stage1_bounded_count_shift_is_identical`). Existing GFX-0008 and GFX-0010 tests remain
      green.
- [x] **Stage 1:** the Stage-0 observe-only dry-run is retired from the live path;
      `phase=169`/`phase=174` are re-sourced from the real `CoalescePressure`
      (`phase=169`: `fallback_reason`, `dirty_skipped`, live cap; `phase=174`:
      `old_reason`/`new_reason`/`converted_to_incremental` + raw/coalesced counts and areas).
      `cargo test -p koto-gfx`/`-game2d`/`-core`/`-sim` green; `thumbv6m` build green;
      `build_apps.py --check` clean; firmware lib clippy adds no new finding (11 warnings,
      unchanged); release `koto_firmware` `.bss` 177152 → 177152 (**±0 B**, no new buffer).
- [x] **Stage 1:** hardware smoke on KotoSnake — the former `full=1 CommandCountShift` frames
      now mostly become `full=0` incremental with small dirty regions (coalescible wide count
      shifts convert), while genuinely wide post-coalesce damage stays `full=1 CommandCountShift`
      (e.g. `area_coalesced` ≈ 77k–80k over `FULL_REPAINT_AREA = 76800`). Confirmed on device.
      *Orthogonal, out of scope:* the remaining major spike is now VM-side (`full=0`,
      `vm_us ≈ 165 ms`) — a bytecode-execution cost for a separate issue.
- [x] **Follow-up regression fixed:** together with GFX-0010 Stage 1B, this conversion of a
      wide count shift to incremental exposed a latent reveal bug — KotoSnake's initial static
      chrome, built on the title→play transition and then obscured, was no longer revealed by the
      first gameplay frame (which now stays incremental). Fixed generically by a one-shot
      full-repaint latch on the present following a static-layer rebuild — see
      [BUG-GFX-0012](BUG-GFX-0012-kotosnake-initial-ui-invisible.md). Steady-frame GFX-0011
      behaviour is unaffected.

## Acceptance tests (host-side, `cargo test`)

If the Stage-0 patch lands:

1. **`koto-gfx`** — `collect_immediate_dirty` returns the alignment shape on both paths: a
   single head-removal (case #1) reports `prefix_len=0`/large `suffix_len`, `region=1`,
   `bailed=false`; a whole-list shift (case #2) reports near-zero prefix/suffix,
   `region ≈ len`, `bailed=true`. Regression-guards the geometry the log reports.
2. **`koto-gfx`** — a wide count-shift whose region collects to contiguous union rects that
   coalesce ≤ `FULL_REPAINT_RECTS` with area < `FULL_REPAINT_AREA`: assert the dry-run
   reports `would_incremental=true` (the case-#2 rescue is real), while the same frame's
   *live* decision is unchanged `FullRepaint(CommandCountShift)` (observe-only proof).
3. **`koto-gfx`** — a wide count-shift whose collected region genuinely exceeds
   `DIRTY_RECT_PROBE_CAP` reports `probe_truncated=true` / `would_incremental=false`
   (case #4 stays fail-safe); a genuinely wide-area region reports `would_incremental=false`
   via area (case #5).
4. **`koto-pico` diag** — `phase=169` formatting test extended to assert the new fields are
   present, parseable, and emitted only on `CommandCountShift` frames; `dirty_skipped=1`
   iff the bail fired.
5. `cargo test -p koto-gfx`/`-game2d`/`-core`/`-sim` (golden frames) green; `thumbv6m`
   firmware build green; firmware-lib clippy adds no new finding (`check_all` does not lint
   `koto-pico` — run `-p koto-pico --target thumbv6m-none-eabi --bins` manually).

## Hardware smoke acceptance plan (KotoSnake / KotoBlocks)

- **KotoSnake — coalescibility of the count-shift frame.** Drive a long-snake session. On
  each `full_reason=CommandCountShift` frame read the extended `phase=169`: classify
  `dirty_skipped=1 & would_incremental=1` (case #2, Stage 1 justified) vs `probe_truncated=1`
  (#4) vs wide `region_area_coalesced` (#5). Confirm `prefix_len`/`suffix_len` are near-zero
  (the shifting-body mechanism).
- **Budget is not the lever.** Confirm the same frames still report
  `would_degrade=0 would_reject=0 first_pressure=none`.
- **KotoBlocks — no regression / contrast.** Confirm its count-shift frames (line clear /
  lock) stay incremental (GFX-0008 behaviour) and, where any `CommandCountShift` full
  repaint appears, capture its `phase=169` classification — a board/text-driven contrast to
  KotoSnake's body-driven one.
- **Observe-only confirmation.** Steady-play `phase=160` attribution counts (`full=1`
  frequency and reason mix) are **identical** to the pre-patch session. Any change means the
  dry-run leaked into the live path — a bug.
- **Post-Stage-1 (when it lands).** The frames Stage 0 flagged case-#2 flip to `full=0`
  (incremental) with a small `rects_post` on `phase=164` and `converted_to_incremental=1`
  on `phase=171`; #4/#5 stay `full=1 CommandCountShift`. Golden-frame identity between the
  collected-region incremental path and the prior full recompose for the rescued frames.

## Notes

GFX-0008 (bounded count shift), GFX-0010 (coalescible `RectsExceeded`), and this issue
(wide count shift) complete the account of the steady-play full-repaint populations.
GFX-0011's contribution is to show that the residual `CommandCountShift` fallback is, in its
addressable case, a **collection** artifact, not a decision one: GFX-0010 taught the present
path to coalesce before deciding, but the immediate command diff — the one layer that still
bails at `MAX_EDIT_REGION` — never hands it the wide region to coalesce. The first move is to
*measure* the gap (Stage 0, observe-only) — is the uncollected region coalescible or
genuinely wide? — not to widen the live cap blind; the collection-cap change (Stage 1) is a
small, behaviour-changing follow-up justified only once the hardware shows the population is
genuinely coalescible.
