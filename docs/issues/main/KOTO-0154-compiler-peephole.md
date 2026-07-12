# KOTO-0154: Conservative compiler peephole pass

- Status: done (peephole pass runs in `koto-compiler` codegen, KOTO-0154 section of `tools/koto-compiler/src/codegen.rs`)

A small, conservative peephole optimization in the host-side `koto-compiler`,
applied to the emitted `kbc-asm` instruction stream **after** code generation and
**before** final bytecode assembly. It targets the two compiler-only optimization
targets called out in [KOTO_VM_PROFILE_KOTOBLOCKS.md](../../devlog/KOTO_VM_PROFILE_KOTOBLOCKS.md)
(stack-shuffle and constant-materialization pressure) by removing redundant stack
juggling and folding/eliminating constant pushes, without touching the VM.

This is a bytecode-cleanup and headroom task, not a VM change. KotoBlocks already
has ~84% per-frame fuel headroom; the goal is a smaller, cleaner instruction stream
for free, with a mechanically-obvious correctness story.

## What it does NOT change

Preserved exactly: opcode values, the `KBC1` bytecode format/ABI, hostcall IDs,
`RuntimeLimits`, the verifier rules, and the interpreter (`koto-vm` is untouched).
No PSRAM, CodeWindow, firmware, graphics, audio, or input code is touched. Source
semantics are unchanged: the optimizer only rewrites the instruction stream into a
shorter sequence that computes the identical result. Implementation is entirely in
[tools/koto-compiler/src/codegen.rs](../../../tools/koto-compiler/src/codegen.rs)
(`peephole` and helpers), called from `Codegen::finish`.

## The rewrite rules

Each rule is justified mechanically against the exact interpreter semantics in
`koto_vm::BytecodeVm::step` / `exec_binary`:

| # | Pattern | Rewrite | Justification |
| --- | --- | --- | --- |
| 1 | `push_i16 X; push_i16 Y; <binop>` | `push_i16 (X op Y)` | Constant fold; `fold_binop` mirrors `exec_binary` (same wrapping, masked-shift, logical `shr`). |
| 2 | `push_i16 v; <binop>` where `v` is the op's identity | (deleted) | `x+0`, `x-0`, `x\|0`, `x^0`, `x<<0`, `x>>0`, `x*1`, `x&-1` all `== x`. |
| 3 | `push_i16 X; drop` | (deleted) | A literal push has no side effect; pushing then dropping is a no-op. |
| 4 | `dup; drop` | (deleted) | Duplicate-then-discard returns the stack to its prior state. |
| 5 | `swap; swap` | (deleted) | Two adjacent swaps cancel. |

Rules are applied to a fixpoint within each run, so folds and deletions cascade
(`push 2; push 3; mul; push 4; add` → `push 10`).

Constant folding only re-emits a single `push_i16` when the folded value
round-trips through the 16-bit sign-extended immediate (`-32768..=32767`), so the
materialized constant is bit-identical to executing the three opcodes; otherwise the
sequence is left intact. Division by zero is never folded, preserving the runtime
`DivisionByZero` trap.

## Safety contract

- **Straight-line runs only.** Optimization happens strictly within maximal runs of
  plain stack/arithmetic/`load*`/`store*` instructions. Any label, directive
  (`.loc`/`.rodata`/header), branch (`br`/`br_if_zero`), `call`/`ret`/`halt`,
  `host_call`, `store_str`, or other mnemonic **ends the run and is never crossed or
  reordered**. The pass therefore cannot disturb control flow, branch targets, the
  debug line table, or host-call side effects, and never reorders a memory access.
  Because codegen emits `.loc` per *statement*, intra-expression instruction runs
  are `.loc`-free, so this barrier rule loses very little.
- **Depth-consistent.** Every rewrite is net stack-neutral (rules 2–5) or `+1 → +1`
  (rule 1). The verifier's single linear operand-depth scan stays consistent at
  every later point; the only effect on depth is to *lower* intermediate peaks. The
  header's `.stack` request is computed before the pass and is a ceiling the
  optimized code only ever stays under, so verification is unaffected.
- **No new liveness analysis.** No aggressive cross-block or liveness-based reuse was
  introduced. Ambiguous patterns are skipped (see below).

## Deliberately skipped (ambiguous / not mechanically obvious)

- **The boolean-normalization and comparison idioms** (`dup; push 0; swap; sub_i32;
  or_i32; push 31; shr_i32`, `0 - x` negate, `1 - b` logical-not, the `%` lowering)
  are *not* redundant — the `swap`/`dup` there are load-bearing to the branchless
  result. The pass correctly leaves them intact (e.g. it must not mistake `push 0;
  swap; sub` for the `x - 0` identity — the intervening `swap` makes it `0 - x`).
  These dominate the residual `swap`/`dup`/`or`/`shr` counts and would need a
  template rewrite, not a peephole, to shrink. Out of scope here.
- **`host_call; drop`** (ignored Status results) stays: the host call's result push
  is mandated by the VM ABI, so the `drop` is required to balance it.
- **`store_local N; load_local N`** could become `dup; store_local N` but trades two
  ops for two — no count win — so it is not done.

## Results

Captured through the `koto_blocks_runs_and_reports_metrics` profiler
(`cargo test -p koto-sim --features opcode_stats --test fixture_runner koto_blocks
-- --nocapture`), 8 frames, empty input (the immediate-draw title screen — the same
canonical capture as the base profile doc).

### Instruction count

| Metric | Before | After | Δ |
| --- | --- | --- | --- |
| Cumulative instructions (8 frames) | 68,217 | 66,081 | **−2,136 (−3.1%)** |
| Busiest frame (f0, warm-up) | 9,718 | 9,452 | −266 |
| Steady-state frame (f1) | 8,315 | 8,049 | −266 |
| `koto_blocks.kbc` size | 41,502 B | 40,622 B | −880 B (−2.1%) |
| Static asm instruction lines | 6,633 | 6,413 | −220 |

### Opcode histogram delta (executed opcodes)

| Opcode | Before | After | Δ |
| --- | --- | --- | --- |
| `push_i16` | 19,665 | 18,597 | **−1,068** |
| `sub_i32` | 9,960 | 8,912 | **−1,048** |
| `shl_i32` | 46 | 26 | −20 |
| `swap` | 7,256 | 7,256 | 0 |
| `or_i32` | 6,715 | 6,715 | 0 |
| `load_local` | 6,069 | 6,069 | 0 |
| `shr_i32` | 5,833 | 5,833 | 0 |
| `dup` | 5,156 | 5,156 | 0 |
| (all others) | — | — | 0 |

The win is concentrated in constant folding of `push; push; sub_i32` (and other
`push; push; <binop>`) chains — chiefly recomputed tile/cell geometry and constant
comparison subexpressions in the per-frame loop, which now materialize as a single
`push`. `swap`/`dup`/`or`/`shr` are unchanged because, as noted above, they come
from the data-dependent boolean-normalization templates, which the conservative pass
deliberately does not rewrite. This realizes the profile's optimization target #2
(constant materialization) while leaving target #1 (template-level shuffle removal)
to a future, less mechanical change.

All 15 committed app fixtures rebuilt smaller (e.g. `kotorogue` 96,066→93,802 B,
`kotorun` 39,570→37,882 B, `memo` 22,007→21,047 B), so the cleanup benefits every
Koto app, not just KotoBlocks.

## Verification

- `cargo test -p koto-vm` — ok (VM untouched).
- `cargo test -p koto-core -p koto-sim` — ok (incl.
  `koto_blocks_play_uses_retained_game2d_layers`, proving gameplay still drives the
  retained Game2D layers).
- `cargo test -p koto-sim --features opcode_stats` — ok.
- `cargo test -p koto-compiler` — ok; 7 new peephole tests assemble each body both
  optimized and unoptimized and run both through the real interpreter, asserting an
  identical `Exited` result (plus direct `fold_binop`/`push_value`/barrier unit
  tests).
- `cargo build -p koto-pico --target thumbv6m-none-eabi --bins` — ok.
- `python harness/check_golden_frames.py` — **golden-frame output identical**.
- `python harness/build_apps.py --check` — no source/bytecode drift.
- `python harness/check_budgets.py` — ok; user-slot and heap budgets unchanged
  (folding does not touch slot allocation).
