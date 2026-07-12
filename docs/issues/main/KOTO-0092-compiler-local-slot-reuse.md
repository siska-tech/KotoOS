# KOTO-0092: Compiler Per-Scope Local Slot Reuse

- Status: done
- Type: harness
- Priority: P2
- Requirements: FR-SDK-4

## Goal

Let the Koto compiler reuse VM local slots across disjoint block scopes so a
program's *total* `let` count is no longer the limit — only the peak number of
simultaneously live locals. This keeps `VM_LOCAL_SLOTS` a predictable, bounded
register file without growing it each time an app gains a feature.

## Background

The VM shares one local file across all of an app's functions (each function gets
a non-overlapping `slot_base`). The compiler allocates a fresh slot per `let` and
never frees it, so `count_lets` over the whole program must fit in
`USER_LOCAL_SLOTS`. The memo app hit this wall when the open/save dialog
([KOTO-0080](KOTO-0080-memo-open-save-dialog-baseline.md)) was added: the editor
branch and the dialog branch consume distinct slots even though they never run in
the same iteration. The interim fix was to right-size `VM_LOCAL_SLOTS` (16 → 48);
this issue is the structural fix so it does not need raising again.

## Acceptance Criteria

- [x] A `let` declared inside an `if` / `while` / `loop` body frees its slot when
  that block ends, so a later disjoint block can reuse it.
- [x] Block scoping for `let` is honoured (a `let` does not leak out of its
  block), with clear diagnostics for use-after-scope.
- [x] The limit reported to apps is the peak live-local count, not the static
  total; the precheck and `alloc_local` reflect that.
- [x] Existing sample apps and the memo app still compile and pass the gate.

## Resolution

In `tools/koto-compiler/src/codegen.rs`:

- `Codegen::emit_block` now opens a lexical scope: it snapshots the current scope's
  `locals` map and `next_slot` on entry and restores them on exit, so a block's
  `let`s drop out of scope and free their slots for the next disjoint block. (`buf`
  declarations are heap-allocated globally and are intentionally not scoped.)
- The per-function precheck reserves `params + peak_lets(body)` slots, where the new
  `peak_lets` walker computes the maximum simultaneously-live local count (an `if`'s
  arms are disjoint; a nested block's locals are live only within it). `count_lets`
  (static total) is removed.
- Use-after-scope now reports `undefined name` like any other unbound reference.

Tests in `tests.rs`: `reuses_slots_across_disjoint_blocks` (60 total `let`s, peak 15,
compiles), `rejects_too_many_simultaneously_live_locals`, `let_does_not_leak_out_of_block`,
and `reused_slots_keep_disjoint_block_values_correct` (runtime correctness of a reused
slot). `VM_LOCAL_SLOTS` stays at the deliberate 48 — reuse simply gives apps more
headroom under it, so it does not need shrinking.
