# BUG-GFX-0012: KotoSnake initial UI invisible until first food pickup

- Status: **Fixed** (host-side; hardware validation pending). Root cause identified as a
  render-path regression from GFX-0010/GFX-0011: the frame that must *reveal* KotoSnake's
  retained static chrome is converted to an incremental present, so the chrome (built a frame
  earlier and then obscured) never reaches GRAM. Minimal fix: a one-shot latch that forces a
  full repaint on the first present *following* a static-layer rebuild
  ([`force_full_repaint_after_static_rebuild`](../../../src/koto-gfx/src/repaint.rs)), wired
  through [`PresentRequest.force_full`](../../../src/koto-pico/src/firmware/display_service.rs)
  and the frame loop. Steady frames keep the GFX-0010/0011 incremental behaviour.
  `cargo test -p koto-gfx` (117, +2 latch tests) / `-sim` (13 golden) green; `thumbv6m` build
  green. Hardware smoke (launch KotoSnake → start play → UI visible *before* eating) is the
  open gate.
- Type: bug (visible correctness regression)
- Priority: P1 (visible on every KotoSnake gameplay start)
- Requirements: NFR-DRAW-1 (correct retained composition)

Source of truth:
[app_runtime.rs](../../../src/koto-pico/src/firmware/app_runtime.rs)
(the present trigger + BUG-GFX-0012 latch,
[app_runtime.rs:907](../../../src/koto-pico/src/firmware/app_runtime.rs#L907)),
[display_service.rs](../../../src/koto-pico/src/firmware/display_service.rs)
([`PresentRequest.force_full`](../../../src/koto-pico/src/firmware/display_service.rs#L60),
the delta-vs-full routing [display_service.rs:99](../../../src/koto-pico/src/firmware/display_service.rs#L99)),
[app_render.rs](../../../src/koto-pico/src/firmware/app_render.rs)
([`present_app_delta`](../../../src/koto-pico/src/firmware/app_render.rs#L309),
the `StaticRebuild` early return [app_render.rs:349](../../../src/koto-pico/src/firmware/app_render.rs#L349),
[`full_screen_base_color`](../../../src/koto-pico/src/firmware/app_render.rs#L757),
the GFX-0010 [`coalesce_then_decide`](../../../src/koto-pico/src/firmware/app_render.rs#L490) call),
[repaint.rs](../../../src/koto-gfx/src/repaint.rs)
([`force_full_repaint_after_static_rebuild`](../../../src/koto-gfx/src/repaint.rs)),
[apps/kotosnake/src/main.koto](../../../apps/kotosnake/src/main.koto)
(static build [main.koto:231](../../../apps/kotosnake/src/main.koto#L231), the title's
full-screen fill [main.koto:249](../../../apps/kotosnake/src/main.koto#L249), the two
`yield_frame()`s [main.koto:273](../../../apps/kotosnake/src/main.koto#L273) /
[main.koto:603](../../../apps/kotosnake/src/main.koto#L603)).

Introduced by: [GFX-0010](GFX-0010-rectsexceeded-pressure-investigation.md) Stage 1B
(coalesce-before-decide *converts* a would-be full repaint to incremental) and
[GFX-0011](GFX-0011-commandcountshift-fallback-diagnosis.md) Stage 1 (the wide
`CommandCountShift` edit region is now *collected* instead of force-escalated). Together they
removed the accidental full repaint that previously masked this latent app/renderer
interaction.

## Observed behavior

- On first KotoSnake gameplay start, the UI/chrome (HUD bars, playfield border + grid, the
  fixed labels スコア/ながさ/ベスト/操作) is **not** displayed — the field shows the flat page
  background with the moving snake, no chrome.
- After the snake eats one food item, the full UI appears.
- Appeared after the GFX-0010 / GFX-0011 dirty-rect / coalescing changes.

## Root cause

KotoSnake captures its chrome into the retained **static layer** once, on the title→play
transition, and relies on the host to composite it beneath every gameplay frame
([main.koto:223-246](../../../apps/kotosnake/src/main.koto#L223)). The transition is driven by
a state machine whose title and play branches each end in their own `yield_frame()`, so **the
frame that builds the static layer and the frame that first renders gameplay are two different
frames**:

| Frame | `state` | What the app does | `static_layer.rebuilt` | Present path |
|---|---|---|---|---|
| **N — transition** | `0` (F1 pressed) | `game2d_static_begin/end` builds the chrome, then draws the **title** (incl. a full-screen `C_BG` fill, [main.koto:249](../../../apps/kotosnake/src/main.koto#L249)) and `yield_frame`s. Sets `state=1`. | **1** | `StaticRebuild` full repaint |
| **N+1 — reveal** | `1` | `clear_frame` (resets `rebuilt`), draws the snake + retained HUD text, `yield_frame`s. | **0** | delta → **coalesce_then_decide** |

### Finding 1 — the static-rebuild full repaint is *obscured* on frame N

Frame N is a `StaticRebuild` full repaint (`rebuilt=1` → the early return in
[`present_app_delta`](../../../src/koto-pico/src/firmware/app_render.rs#L349)). But
`present_app_commands` composites in layer order **static → board → sprites → text → immediate
commands**, so frame N's own immediate commands — the title, including its full-screen `C_BG`
fill — are painted **on top of** the freshly composited chrome. GRAM after frame N is the title
screen; it reflects the static chrome **nowhere**.

### Finding 2 — the reveal frame (N+1) is now converted to incremental

Frame N+1 has `rebuilt=0` and the **same** full-screen base color on both sides
(`C_BG`: the title emits it as an immediate fill; gameplay has it in the static layer — see
[`full_screen_base_color`](../../../src/koto-pico/src/firmware/app_render.rs#L757), which reads
the static layer first), so it is **not** a `BaseChange` and **not** a `StaticRebuild`. It takes
the delta path. The immediate list changes wholesale (title art → snake body), a wide
count shift. **Before GFX-0010/0011** that wide change escalated to a full repaint
(`CommandCountShift`/`RectsExceeded`), which recomposited the whole surface and thereby revealed
the chrome. **After GFX-0011 Stage 1** the wide edit region is *collected*, and **after GFX-0010
Stage 1B** it *coalesces under threshold and converts to incremental*. So only the diffed rects
transfer.

### Finding 3 — the chrome region is never in the reveal frame's dirty set

The incremental soundness invariant is *"retained GRAM already equals a full repaint outside the
dirty rects."* It is **false** across this transition: GRAM holds the title (flat `C_BG` where
the chrome should be), while a full repaint would show the static chrome. The chrome's region is
not in the dirty set because (a) it is in the retained static layer, not in any changed
immediate/board/sprite/text slot, and (b) the title's disappearing full-screen `C_BG` fill is a
base fill, which the immediate diff **skips** (`is_full_screen_base`, the clear-to-base owns it)
— so the pixels under it are never re-derived. Those chrome pixels are therefore never
composited: the HUD bars, playfield border/grid, and fixed labels stay flat background. **This
is the bug.**

### Why eating food "fixes" it

Every changed retained-text value dirties its own row band, and its incremental recomposite
pulls in the static chrome **clipped to that band**. Eating food updates the score
(`T_SCORE`, [main.koto:400](../../../apps/kotosnake/src/main.koto#L400)), length
(`T_LEN`, [main.koto:580](../../../apps/kotosnake/src/main.koto#L580)) and best
(`T_BEST`, [main.koto:583](../../../apps/kotosnake/src/main.koto#L583)) — top and bottom HUD
rows — plus the "+10" popup and eat-flash, so multiple chrome bands get recomposited at once and
the UI "appears." It is a **symptom of piecemeal reveal**, not a fix: the chrome fills in
wherever play happens to dirty, which is why it looks like it snaps in on the first bite.

## Answers to the investigation questions

1. **Does the first gameplay UI frame produce `static_rebuilt=1`?** No. The rebuild happens on
   the *previous* frame (the title→play transition, still `state==0`); the first gameplay frame
   (`state==1`) has `static_rebuilt=0`.
2. **Does `present_app_delta` take the `StaticRebuild` early return?** On the transition frame,
   yes (`rebuilt=1`). On the reveal frame, **no** — it takes the delta path.
3. **Is the first gameplay frame converted to incremental when it should be full?** **Yes — this
   is the regression.** GFX-0011 collects the wide count shift and GFX-0010 converts the coalesced
   result to `Incremental`; pre-change it full-repainted and revealed the chrome.
4. **Are static/text layers present but not included in dirty rects?** The static chrome is
   present and correct in the retained layer but its region is **not** in the reveal frame's dirty
   set (and the removed title base fill is skipped by the diff), so it is never composited.
   Retained text *is* dirtied on first set, so text rows do reveal chrome locally.
5. **Does eating food trigger a score/text update that incidentally dirties the UI?** Yes — that
   is exactly the piecemeal-reveal symptom (see above), not the cause.

## Minimal fix

The coalesce rescue is correct for *steady* frames; it is unsound only for the **first present
after a static-layer rebuild**, because the rebuild frame's compose is not guaranteed to have
reached GRAM (the app may overdraw it, as KotoSnake does with its title). So force a full repaint
on exactly that one following present, and leave every other frame on the incremental path.

A pure one-shot latch owns the decision (host-tested in koto-gfx, the present-policy crate):

```rust
// koto-gfx repaint.rs
pub fn force_full_repaint_after_static_rebuild(static_rebuilt: bool, latched: bool) -> (bool, bool) {
    (latched, static_rebuilt) // (force_full_this_present, latch_for_next_present)
}
```

- The frame loop ([app_runtime.rs](../../../src/koto-pico/src/firmware/app_runtime.rs#L907))
  carries the `latched` bit across presents, forces a present when it is set (so a reveal frame
  that changed nothing else still flushes the chrome), and passes `force_full` into the request.
- The service ([display_service.rs:99](../../../src/koto-pico/src/firmware/display_service.rs#L99))
  routes `has_previous && !force_full` to `present_app_delta` (unchanged) and everything else to
  `present_app_commands(StaticRebuild)` — so the reveal frame becomes a full compose that blits
  the retained chrome to GRAM. The rebuild frame itself is still handled by the existing
  `StaticRebuild` early return; the latch adds only the one following present.

This is **not app-specific** (any app that rebuilds its static layer gets the correct reveal),
touches **no hostcall / ABI / bytecode / `APP_DRAW`**, and adds **no static RAM** (one `bool`
in the frame loop, one `bool` field in `PresentRequest`). GFX-0010/GFX-0011 remain intact:
steady frames leave `force_full=false` and take the coalesce path.

### Why one following present is enough

The reveal frame (N+1) does not full-screen-overdraw (gameplay's base lives in the static
layer), so its forced full compose shows the chrome, and every frame after it is soundly
incremental against a GRAM that now holds the chrome. A pathological app that overdrew the reveal
frame too would have been broken pre-GFX-0011 as well (the accidental full only ever fell on the
wide-change frame); one latched present matches the real failure and the one-shot-per-gameplay-
entry cost.

## Regression test

Host-side (`koto-gfx`), since the firmware present path is not exercised by the koto-sim VM
harness:

- `static_rebuild_forces_exactly_the_following_present_full` — drives the latch across a
  steady → rebuild → reveal → steady sequence and asserts the reveal present is forced full
  while the rebuild frame is not (it uses the existing `StaticRebuild` path) and steady frames
  stay incremental.
- `consecutive_static_rebuilds_keep_latching` — back-to-back rebuilds re-arm the latch, so a run
  of rebuilds never leaves a stale incremental reveal.

The existing `kotosnake_play_uses_retained_static_and_text_layers` (koto-sim) continues to guard
that the app *builds* the static chrome on the title→play transition (the precondition this bug
sits downstream of). An end-to-end pixel test of the reveal would require running the firmware
present path under simulation (out of scope here); the latch unit tests plus the hardware smoke
below cover the fix.

## Hardware validation plan

1. Launch KotoSnake, cross into gameplay (F1) — **before eating any food**, confirm the full
   chrome is visible: top/bottom HUD bars, playfield border + grid, and the fixed labels.
2. Confirm `phase=160` reports the reveal frame (first `state==1` frame) as `full=1
   full_reason=StaticRebuild` (was `full=0` incremental before the fix), and that the frame
   after it returns to `full=0` incremental — i.e. exactly one forced full, not a per-frame full
   repaint.
3. Confirm steady play is unchanged: the GFX-0010/0011 populations (`RectsExceeded`/
   `CommandCountShift` → incremental) still convert, so the fix did not regress performance.
4. Regression-adjacent: enter game over → return to title → start again, and confirm the chrome
   reveals on every fresh gameplay entry (the latch re-arms each rebuild).

## Non-goals / constraints honored

- Correctness fixed first; no performance tuning in this change.
- No app-specific renderer policy — the latch is generic over "static layer was just rebuilt."
- No hostcall / ABI / bytecode / `APP_DRAW` / PSRAM / LCD / CodeWindow / audio change.
- GFX-0010 / GFX-0011 incremental behaviour preserved on all steady frames.
