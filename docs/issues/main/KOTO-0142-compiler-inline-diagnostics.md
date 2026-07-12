# KOTO-0142: Compiler Inline Diagnostics (Short-Term)

- Status: todo
- Type: feature
- Priority: P2
- Requirements: NFR-RT-2

Source of truth: [GAME2D_RETAINED_RENDER_ARCHITECTURE.md](../../architecture/GAME2D_RETAINED_RENDER_ARCHITECTURE.md) §8 "Compiler roadmap split".

## Context

The VM has **no `call`/`ret` and a single shared 16-slot local file**; `koto-compiler`
inlines every function at its call site, and the verifier is a single linear, non-CFG
pass. Real out-of-lining is therefore a deep change. This issue covers the **short-term,
diagnostics-only** slice — no ABI change — that gives authors visibility into the inline
bloat and code-window locality problems that KOTO-0137/0138 had to find by hand.

## Scope (short-term, this issue)

- **Inlined-expansion report:** per-function total inlined bytes and share of the code
  segment (the report that would have flagged `shape` at 40,880 B / 61.9% before
  KOTO-0137), plus per-call-site expansion.
- **Code-layout map:** function / source-line → code word range.
- **Loop-straddle warning:** warn when a loop back-edge body straddles an 8 KiB
  (`CODE_WINDOW_BYTES`) boundary — automates the KOTO-0137/0138 hand analysis.
- **Table-lowerable hint:** flag a constant-return branch chain that could become a heap/
  const table.

## Deferred (tracked here, not implemented)

- **Medium-term:** `#[noinline]` / `cold` annotations + real `call`/`ret`. Requires a VM
  calling convention with per-frame local windows **and** a control-flow-aware verifier —
  a substantial rewrite. Not required: the retained-render work (KOTO-0139/0140/0141)
  removes the hot VM code (blit loops, table bakes) that motivated out-of-lining, so this
  buys little until apps are far larger.
- **Long-term:** optimizer auto-table-lowering of constant chains; hot/cold code layout
  packing hot loops away from tile boundaries; basic-block alignment.

## Non-goals

- Any change to bytecode, the verifier's stack model, or the host ABI.

## Acceptance criteria

- The build emits the inlined-expansion report and the code-layout map.
- The loop-straddles-tile-boundary warning fires on a deliberately-bloated test app and
  is silent on KotoBlocks (post-0138, where no hot loop straddles a boundary).
