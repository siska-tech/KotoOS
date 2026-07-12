# GFX-0008: CommandCountShift policy refinement (bounded-damage relaxation)

- Status: in-progress — **derivation change implemented; hardware validation
  pending.** The aligned immediate diff + wide-region fallback are in
  [`koto-gfx`](../../../src/koto-gfx/src/derive.rs); `FullRepaintPolicy`, the
  thresholds, and the reason variants are unchanged. The budget/overflow safety
  gates are deferred (see *Implementation notes*) — they only bite once GFX-0006C
  enforcement can drop a command.
- Type: feature (behaviour-changing: a count-shift frame with bounded damage now
  stays incremental instead of full-repainting)
- Priority: P3
- Requirements: NFR-PERF-1, NFR-DRAW-1

Source of truth: [KOTO_BUDGET_OBSERVE_MODE.md](../../devlog/KOTO_BUDGET_OBSERVE_MODE.md),
[repaint.rs](../../../src/koto-gfx/src/repaint.rs)
([`FullRepaintPolicy`](../../../src/koto-gfx/src/repaint.rs)),
[app_render.rs](../../../src/koto-pico/src/firmware/app_render.rs)
([`present_app_delta`](../../../src/koto-pico/src/firmware/app_render.rs#L284)).

Depends on: [GFX-0006](GFX-0006-game2d-api-and-budgeted-immediate.md) (the
`phase=168`/`phase=169` observe-mode diagnostics this proposal reads).

Relates to: [KOTO-0143](../main/KOTO-0143-full-repaint-instrumentation-coalescing.md)
(full-repaint instrumentation), [KOTO-0159](../main/KOTO-0159-kotoblocks-dirty-rect-coalescing.md)
(dirty-rect coalescing).

> **Why this is separate from GFX-0006C.** GFX-0006C wires up *budget enforcement*
> (admitting the immediate list through `DrawBudget`, dropping/degrading over-cap
> commands). The `phase=169` hardware logs show that the remaining heavy
> `CommandCountShift` frames are **not over budget**, so enforcement would not
> address them. They are a *dirty-derivation* artifact, not a draw-volume problem.
> This issue proposes the orthogonal fix. **Do not fold it into GFX-0006C.**

## Observed problem (phase=169, hardware)

`CommandCountShift` full-repaint frames show, on the budget-correlation line:

- `would_degrade=0`
- `would_reject=0`
- `first_pressure=none`
- small command-count deltas — e.g. `prev_cmds=29 cur_cmds=28`, `43 → 42`.

Per the GFX-0006B observe data, these frames sit **comfortably inside** the
immediate-draw budget. Budget enforcement (GFX-0006C) drops/degrades only classes
already showing pressure below cap; with no pressure here, it has nothing to act on.
Yet the frame still pays a whole-surface `present_app_commands` recompose.

## Root cause

The immediate-command diff in
[`present_app_delta`](../../../src/koto-pico/src/firmware/app_render.rs#L384) is
**positional**: it compares `command_at(prev, i)` against `command_at(cur, i)` for
`i in 0..prev.len.max(cur.len)`. When the list length shifts by even one command at
the front or middle (one command inserted/removed), **every later index is
misaligned** — each compares two unrelated commands and emits the union of their two
footprints as a dirty rect. A single logical change fragments into a tail of
spurious, near-arbitrary union rects.

That fragmentation balloons `dirty_rects` and `dirty_area`, which trips the existing
[`FullRepaintPolicy`](../../../src/koto-gfx/src/repaint.rs#L136) thresholds
(`FULL_REPAINT_RECTS = 24`, `FULL_REPAINT_AREA = ¾·320·320`). The escalation is then
*attributed* to `CommandCountShift` because `command_count_changed` outranks
`AreaExceeded`/`RectsExceeded` in the reason priority. Crucially:

- `command_count_changed` **alone does not escalate** — it is only the attribution
  label once a threshold is independently tripped. So the real trigger is the
  *spurious damage the positional diff invents*, not the count change itself.
- The genuine on-screen damage from a one-command edit is **bounded** (that single
  command's footprint, plus whatever truly changed). The positional misalignment is
  an artifact of the diff strategy, not a property of the frame.

This is exactly the case KOTO-0143 already removed for *text* by moving per-frame
`draw_text` into the index-keyed retained text layer (see
[config.rs](../../../src/koto-pico/src/firmware/config.rs#L150)). The residue is the
remaining *immediate* list, which is still positionally diffed.

## Goal

Avoid the full repaint for **safe** `CommandCountShift` frames — frames where dirty
derivation can still produce **bounded damage** — without any app-specific
knowledge. The visible result must be byte-identical to the full-repaint path; only
the cost changes (incremental rects instead of a 320×320 recompose).

Explicit non-goals (carried from the task framing):

- No KotoSnake-specific constants; no app-specific profiles.
- No change to `APP_DRAW` capacity.
- No budget enforcement (that stays GFX-0006C).
- No rendering-behaviour change in *this* issue — this is the proposal + plan.

## Proposed approach

**Primary: anchored prefix/suffix alignment of the immediate diff (recommended).**

Fix the derivation so the count-shift case yields bounded damage, then let the
*existing, unchanged* `FullRepaintPolicy` thresholds keep the frame incremental on
their own merit. No new threshold, no new reason variant, no policy special-case.

Replace the single positional walk with a three-segment alignment over the two
immediate lists `prev[0..prev.len]` and `cur[0..cur.len]`:

1. **Common prefix** `p` = longest run where `prev[i] == cur[i]` from the front.
2. **Common suffix** `s` = longest run where `prev[prev.len-1-k] == cur[cur.len-1-k]`
   from the back (not overlapping the prefix).
3. **Edit region** = `prev[p .. prev.len-s]` vs `cur[p .. cur.len-s]`. Only this
   bounded span is diffed — old footprints in the region are erased, new footprints
   painted, exactly as `command_dirty_rect` already does. Commands outside the
   region are byte-identical between frames and contribute no damage.

A one-command insertion/removal collapses to: prefix = everything before it, suffix
= everything after it, edit region = the single changed command. The tail no longer
misaligns; `dirty_rects`/`dirty_area` reflect only real damage; the frame stays
`Incremental` through the existing policy. This is generic — it is plain list-edit
alignment, no app palette, no per-game branch — and reuses `command_at` /
`command_dirty_rect` unchanged.

Bound the work to keep it `no_std`/firmware-cheap and to fail safe:

- Cap the edit-region span at a small constant `MAX_EDIT_REGION` (app-agnostic, e.g.
  the existing `FULL_REPAINT_RECTS` order of magnitude). If the region is larger than
  the cap — a genuinely wide restructure, not a single edit — **fall back to the
  current full repaint**, attributed `CommandCountShift` as today. This makes the
  relaxation a strict subset of current behaviour: it only ever *removes* spurious
  repaints, never adds risk on a real wide change.
- Prefix/suffix scan is O(prev.len + cur.len); the region diff is O(region) — both
  bounded by the `APP_DRAW` cap, well under one positional pass today.

**Safety gate (must all hold to take the incremental path):**

Using only signals already computed/diagnosed (`phase=168`/`phase=169`):

| Signal | Source | Condition to relax |
|---|---|---|
| `command_count_changed` | `prev.len != cur.len` | the case we target |
| count delta | `prev_cmds` vs `cur_cmds` | `|Δ| ≤ MAX_EDIT_REGION` |
| edit-region damage | new prefix/suffix diff | `dirty_rects ≤ rect_threshold` and `dirty_area < area_threshold` (the *existing* policy gate — no new constant) |
| budget pressure | `BudgetObservation` | `would_degrade == 0 && would_reject == 0 && first_pressure == none` (no command is being dropped, so the diffed list is the list that paints) |
| app-draw overflow | `overflow_count` / draw `peak` vs cap | `== 0` — a truncated draw list is an unreliable diff; never relax |
| `static_rebuilt` | `AppStaticLayer::rebuilt` | already an earlier early-return (`StaticRebuild`); independent of this path |

The budget-pressure and overflow gates are the correctness backstops: if the
immediate list was truncated (overflow) or would be degraded/rejected under the
budget, the painted pixels would not match the diffed list, so we must not trust the
bounded-damage derivation — fall back to full repaint.

**Alternative considered (not recommended): a policy-level relaxation in
`FullRepaintPolicy::decide`** — keep positional diffing but, when
`command_count_changed` is the only escalation cause and a new `bounded_damage` input
is set, downgrade to `Incremental`. Rejected because it would require the call site
to compute the bounded damage *anyway* (i.e. do the alignment), so the alignment is
the real work; threading it back through the policy as an extra `DeltaInputs` field
adds surface without removing any. Keeping `FullRepaintPolicy` untouched and fixing
the derivation upstream is simpler and keeps the policy a pure threshold function.

## Acceptance Criteria

- [x] The immediate-command diff aligns by common prefix/suffix + bounded edit
      region; a single-command insert/remove dirties only the changed footprint(s),
      not the misaligned tail. Logic lands in `koto-gfx`
      ([`collect_immediate_dirty`](../../../src/koto-gfx/src/derive.rs), with
      [`is_full_screen_base`](../../../src/koto-gfx/src/derive.rs) and
      [`MAX_EDIT_REGION`](../../../src/koto-gfx/src/derive.rs) lifted from the
      firmware), with `present_app_delta` as the thin call site (GFX-0003
      methodology); the firmware's `command_dirty_rect`/`is_full_screen_base`
      wrappers are removed as the call moves to the aligned collector.
- [x] `FullRepaintPolicy`, its thresholds, the `FullRepaintReason` variants, and the
      `phase=168`/`phase=169` diagnostics are **unchanged**. A wide edit region flags
      the existing `rect_overflow` signal, so the unchanged policy full-repaints it as
      `CommandCountShift` (the count still changed); the relaxation is a strict subset
      — equal-length frames diff byte-identically to the pre-GFX-0008 walk.
- [x] No app-specific constants/profiles; no `APP_DRAW` capacity change; no budget
      enforcement; no hostcall/ABI change; no bytecode rebuild (`build_apps.py
      --check` clean).
- [ ] Golden frames prove visual identity between the relaxed incremental path and
      the prior full-repaint output for the count-shift cases. *(koto-sim's 13 golden
      frames pass unchanged; a dedicated 29→28 count-shift fixture is still TODO.)*

## Acceptance tests (host-side, `cargo test`)

Add to `koto-gfx` alongside the existing `derive.rs`/`repaint.rs` tests:

1. **Single removal at head stays bounded** — `prev` = `[A,B,C,…]` (len 29),
   `cur` = `[B,C,…]` (len 28); assert the derived dirty set covers only `A`'s
   footprint (and any genuinely changed command), `dirty_rects` ≤ a small constant,
   not the whole tail.
2. **Single insertion mid-list stays bounded** — symmetric to (1), one command added.
3. **Wide restructure still full-repaints** — edit region > `MAX_EDIT_REGION`
   ⇒ `DeltaDecision::FullRepaint(CommandCountShift)`, unchanged from today.
4. **No-op reorder of equal-length lists** — `prev.len == cur.len`, content identical
   ⇒ `Skip` (regression guard: alignment must not invent damage).
5. **Identity vs full repaint (golden)** — for a recorded 29→28 KotoBlocks-style
   frame, the composited output of the relaxed incremental path is byte-identical to
   `present_app_commands` full recompose. (Fixture frame only — no app constants in
   `koto-gfx`.)
6. **Overflow/budget gate** — with `overflow=true` or a budget observation showing
   `would_reject > 0`, the relaxation is refused and the frame full-repaints.
7. `cargo test -p koto-gfx` / `-p koto-game2d` / `-p koto-core` / `-p koto-sim`
   (golden frames) all green; `thumbv6m` firmware build green; firmware-lib clippy
   adds no new finding.

## Implementation notes

- **Where it landed.** [`collect_immediate_dirty`](../../../src/koto-gfx/src/derive.rs)
  replaces the inline positional walk in
  [`present_app_delta`](../../../src/koto-pico/src/firmware/app_render.rs). It takes
  the two `commands[..len]` slices directly, so a slot past `len` is simply absent —
  this folds in the old `command_at` Empty-padding (the KOTO-0141 stale-overlay fix)
  without a helper. The firmware test helper `immediate_dirty_rects` now routes
  through the same function, so the existing firmware invalidation tests
  (disappeared overlay/text, moved command, stable empty list) became regression
  tests for the aligned path.
- **Wide-region fallback reuses the existing escalation signal.** Rather than a new
  `DeltaDecision` variant or a policy change, a region wider than `MAX_EDIT_REGION`
  sets the `rect_overflow` flag the working set already uses. The unchanged
  `FullRepaintPolicy::decide` then escalates, and because the firmware still passes
  `command_count_changed: previous.len != current.len`, the attributed reason is
  `CommandCountShift` — byte-identical decision and attribution to today for genuine
  wide restructures.
- **Budget/overflow safety gates deferred (intentional).** The proposal listed a
  budget-pressure gate (`would_degrade/reject == 0`) and an app-draw-overflow gate as
  correctness backstops. Both are **moot in the current observe-only regime**: GFX-0006B
  budget metering drops nothing, and the diff runs over `commands[..len]` — exactly the
  list that paints — so the diffed list always equals the painted list, overflow or
  not. These gates become necessary only when **GFX-0006C enforcement** can actually
  drop/degrade a command (making the painted list differ from the emitted one); they
  are recorded here as a precondition to wire up alongside that work, and deliberately
  omitted now to avoid coupling the derivation to budget state prematurely.
- **Verification (host).** `cargo test -p koto-gfx` (93, +7 GFX-0008 tests:
  equal-length identity & positional-parity, single head-removal, single mid-insertion,
  wide-region overflow, at-cap boundary, full-screen-base skip) / `-p koto-game2d` (7) /
  `-p koto-core` (132) / `-p koto-sim` (13 golden frames) all pass; `thumbv6m` firmware
  build green; firmware-lib clippy adds no new finding (pre-existing `psram.rs` /
  `probe_keyboard` lints untouched); `build_apps.py --check` clean, no bytecode rebuild.
  Firmware `#[cfg(test)]` tests are host-incompatible (embassy-rp ARM asm), as before.

## Hardware validation plan

- **Before/after `phase=169` rates.** On KotoBlocks (and any app that shifts its
  immediate count), confirm the count of `full_reason=CommandCountShift` frames with
  `would_degrade=0 would_reject=0 first_pressure=none` drops toward zero, while
  genuinely wide frames still report `CommandCountShift`.
- **`phase=160` peak/timing parity.** `raster_us`/`transfer_us` for the relaxed
  frames fall from the full-recompose cost to the incremental cost; no new `ovf` or
  draw-peak regression. No change on frames that were already incremental.
- **Visual smoke.** KotoBlocks / KotoSnake `phase=160/164` smoke shows no flicker,
  tearing, or stale overlay on line-clear / piece-lock / score-change frames (the
  count-shift triggers). Compare a captured frame against the golden output.
- **Fallback intact.** Force a wide restructure (e.g. title→gameplay transition) and
  confirm it still takes the full repaint — the relaxation must not swallow a real
  wide change.

## Notes

This closes the loop opened by the GFX-0006C decision: enforcement handles
*over-budget* frames; this handles *count-shift* frames, which the `phase=169`
correlation proved are a distinct, non-budget population. After both land, a
`CommandCountShift` full repaint should mean what its name says — a genuinely wide
immediate-list restructure — rather than a one-command edit the positional diff
failed to localize.
