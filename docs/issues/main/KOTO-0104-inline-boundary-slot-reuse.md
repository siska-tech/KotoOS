# KOTO-0104: Inline-boundary local slot reuse

- Status: done
- Type: feature
- Priority: P1
- Requirements: NFR-MEM-2

## Goal

Let user local slots be reused across inlined function boundaries so helper
functions and future heap-backed data accessors do not unnecessarily grow
`user_slots_used`. Do not increase `VM_LOCAL_SLOTS`.

## Background

The compiler inlines every function (no `call`/`ret`), but it previously gave each
function a *disjoint* user-slot block accumulated in source order (KOTO-0092 reused
slots only *within* a function). So even though only one inlined helper is ever live
at a time, helpers stacked their slots on top of `main`'s, and `user_slots_used` was
the sum of every function's footprint. KotoBlocks reached 41/45 after source cleanup
(KOTO-0102) and 44/45 after the game-feel effects (KOTO-0103), with readable helper
abstractions threatening to push past 45.

## Approach (Option B: call-site scoped reuse)

Each inline expansion is treated as a call-site scope. When a function is inlined,
its parameter and body slots are allocated starting at the **caller's current
`next_slot`** (above the caller's live locals) rather than a fixed per-function
base, and are released when the expansion ends. The caller resumes at its pre-call
cursor, so a later disjoint inline call reuses the same physical slots — exactly the
mechanism KOTO-0092 already used for lexical blocks, now extended across the inline
boundary.

Correctness details:

- **Nested-argument calls**: each bound parameter slot is reserved on the caller
  scope before the next argument is evaluated, so a nested inline call inside a later
  argument allocates above the parameters it must not clobber.
- **Value-returning functions / return slot**: unchanged — the return value still
  routes through scratch slot 47 (`SCRATCH_RET`); only the *user* slots move.
- **Overflow**: `alloc_local` and the inline param binding both check the live
  high-water against the 45 user-slot cap (the old static cross-function check is
  gone; the dynamic peak is the real bound).
- `main` is the root expansion and starts at user slot 0.

`user_slots_used` is now the **post-reuse physical peak** (`max_slot`, the highest
`next_slot` reached during codegen). The slot map obtains it by running codegen, so
it cannot drift from the real allocation; per-function lines report each function's
own footprint (`params + peak_lets`) as a guide, with no fixed ranges.

## Results

- KotoBlocks `user_slots_used` 44 → **42 / 45** (helpers reuse slots above `main`).
- Memo `user_slots_used` 18 → **15 / 45**.
- Future helpers called where fewer locals are live no longer grow the peak.

## Acceptance Criteria

- [x] Inline helper locals reuse physical user slots across inline boundaries
  (test: two 4-slot helpers sum to 9 disjoint but peak at 5 reused).
- [x] Scratch slots 45/46/47 unchanged; `VM_LOCAL_SLOTS` remains 48; user cap 45.
- [x] Runtime `local_peak` remains informational; compiler `user_slots_used` is the
  primary metric, now the post-reuse peak.
- [x] Slot map reflects the post-reuse physical allocation.
- [x] KotoBlocks `user_slots_used` decreases (44 → 42) while allowing helper
  abstractions without unnecessary slot growth.
- [x] KotoBlocks gameplay frame byte-identical before/after the slot-reuse rebuild;
  golden frame validation green.
- [x] Memo and KotoBlocks budget scenarios green; `check_all.py` green.
- [x] Compiler tests prove slot reuse across inline boundaries.
- [x] Regression tests for value-returning inline functions (return-slot behavior)
  and nested-argument inline calls.

## Notes

`main`'s own footprint (36 user slots in KotoBlocks) is the floor; reuse cannot go
below it. The remaining lever for `main` itself is source-level scoping (KOTO-0102),
not the compiler. Option A (lifetime-based allocation on lowered IR) and Option C
(intrinsic lowering for generated accessors) remain available if a future need
exceeds what call-site reuse provides.
