# KOTO-0102: KotoBlocks local slot reduction

- Status: done
- Type: harness
- Priority: P1
- Requirements: NFR-MEM-2

## Goal

Reduce KotoBlocks local-slot pressure before adding visual effects or more game
features, and make local-slot usage attributable so the reduction is data-driven
rather than guesswork.

## Acceptance Criteria

- [x] The budget report attributes local slots: `koto-compiler --slot-map` prints
  per-function user-slot blocks, and `check_budgets.py` reports `user_slots_used`
  against the user-slot cap.
- [x] KotoBlocks `user_slots_used` drops below 42 / 45 (44 -> 41 / 45).
- [x] No `VM_LOCAL_SLOTS` increase (stays 48; user cap stays 45).
- [x] KotoBlocks smoke, golden-frame, and budget scenarios still pass; the captured
  gameplay frame is byte-identical before/after.

## Findings (slot attribution)

`local_peak` in the runtime budget report is the highest VM slot touched, which is
**48 whenever a value-returning function runs**: the top three slots are codegen
scratch and slot 47 is the return slot, so any app with a value-returning helper
reads 48/48 regardless of user-code pressure (Memo, with no value-returning
functions, reads 18). The actionable number is the static **user-slot** total from
`koto-compiler --slot-map`, bounded by the 45 user slots below the scratch region.

KotoBlocks today (`user_slots_used=44 / 45`):

```text
fn pmid        slot_base=0  params=1 locals=0  slots=1   range=0..1
fn shape       slot_base=1  params=2 locals=1  slots=3   range=1..4
fn blit_piece  slot_base=4  params=4 locals=2  slots=6   range=4..10
fn main        slot_base=10 params=0 locals=34 slots=34  range=10..44
```

`main`'s 34 simultaneously-live locals dominate. Because the compiler gives each
inlined function a *disjoint* user-slot block (KOTO-0092 reuses slots only within a
function, not across the inline boundary), the helpers add 10 slots on top of
`main`.

## Resolution

`user_slots_used` 44 -> 41 / 45 with two behavior-preserving, readability-preserving
changes (the captured gameplay frame is byte-identical before/after):

1. **`shape(t, r)` -> `shape(k)`** taking the combined `piece * 4 + rotation` index.
   The function already computed `k = t * 4 + r` as its only local; passing it in
   drops both a parameter and that local, so `shape` goes 3 -> 1 slots. Call sites
   pass `ptype * 4 + rot` (no new locals).
2. **Removed the `cp` local.** `cp = text_input()` was read at only two sites in the
   play path; `text_input()` is an idempotent per-frame host snapshot, so it is read
   inline at the hold check and the hard-drop check. The in-loop `cp != 32` guard was
   redundant (hard drop already sets `full = 1` before the loop) and became an
   unconditional `full = 1`. `main` goes 34 -> 33 slots.

Per-function map after: `pmid` 1, `shape` 1, `blit_piece` 6, `main` 33 (total 41).
`check_budgets.py` `max_user_slots` threshold tightened 44 -> 41 to lock the headroom.

### Levers left on the table (deferred, readability over the last slot)

The slot map already maxed out the manual scratch packing, so the remaining cuts all
trade clarity for one slot each and were not taken: packing `huse`/`hashold` into one
bit-flag, sharing one slot between the disjoint-in-time `gen` (title) and `gtick`
(play) state vars, or a full block-scoping rewrite of `main`. If a future effect needs
more than the current 4 free user slots, take these (or the compiler inline-boundary
reuse below) then.

3. **Compiler-side reuse across the inline boundary** (a leaf function with no live
   caller locals at its call site could reuse the caller's free slots) — done in
   [KOTO-0104](KOTO-0104-inline-boundary-slot-reuse.md): inline expansions now take
   call-site-scoped slots above the caller's live locals, so disjoint helpers share
   physical slots and `user_slots_used` is the post-reuse peak.
