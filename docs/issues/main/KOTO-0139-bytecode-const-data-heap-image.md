# KOTO-0139: Bytecode Const Data / Initial Heap Image

- Status: done
- Type: feature
- Priority: P1
- Requirements: NFR-RT-2

Source of truth: [GAME2D_RETAINED_RENDER_ARCHITECTURE.md](../../architecture/GAME2D_RETAINED_RENDER_ARCHITECTURE.md) §5.

## Goal

Replace runtime `heap_set_u16` table baking with a const **initial heap image** loaded
once at app start. KOTO-0137/0138 proved tables are the right representation, but baking
them in VM code at startup produced KOTO-0138's frame=1 stall and a startup-only tile
0↔1 PSRAM code-window ping-pong. This removes that class of startup cost structurally —
no table-bake code exists to run or to straddle a code-window boundary.

First in the post-0138 roadmap: low risk, and it lets stamp/tile/palette data (KOTO-0140/
0141) live in const memory cleanly.

## Key finding

`KbcHeader` already carries `rodata_offset` / `rodata_size`, and the verifier already
range-checks them against `bytecode_size` — they are simply **unused for heap
initialization** today. Reuse `rodata` as the initial image of `heap[0..rodata_size]`.

## Scope

- **KBC model:** `rodata` is the initial image of `heap[0..rodata_size]`. Const-
  initialized buffers are placed at the bottom of the heap (offsets already known at
  compile time); mutable buffers sit above the const region.
- **Compiler:** a `const`/`data` buffer initializer (or auto-promotion of a `buf`
  initialized from a literal array) emits bytes into `rodata` instead of a `heap_set_u16`
  sequence.
- **Runtime loader:** at app start, before `entry_word`, do **one `memcpy`** of `rodata`
  into `heap[0..rodata_size]`; the rest stays zeroed. No VM execution, no code-window
  activity.
- **KPA / KBC format:** no KPA change — `rodata` is inside the `.kbc` asset already. Only
  the runtime loader and compiler change.
- **Validation:** add `rodata_size <= max_heap_bytes` to the verifier; const buffer
  offsets must lie within `rodata_size`.
- **Migration:** KotoBlocks' shape table (KOTO-0137) and CELLS table (KOTO-0138) become
  const data; the ~56 `heap_set_u16` calls in `main` are deleted.

## Non-goals

- A new package-level data segment (not needed; `rodata` lives in the `.kbc`).
- Compressed or lazily-paged const data.

## Acceptance criteria

- KotoBlocks loads with its tables as const data and **zero `heap_set_u16` bakes**.
- frame=1 VM time and startup code-window refills drop to the steady-state range.
- The verifier rejects a header with `rodata_size > max_heap_bytes`.
- Pixel-parity with the baked version (KOTO-0137/0138 `ImageChops` method).

## Implementation (done)

- **Language:** new top-level `data NAME = u16[...]` / `u8[...]` declaration
  ([lexer](../../../tools/koto-compiler/src/lexer.rs), [parser](../../../tools/koto-compiler/src/parser.rs)).
  `data` names resolve globally as their heap offset, so a table is readable from
  any function (KotoBlocks' `shape()`/`blit_piece()` lost their hand-computed
  `SHAPE_TBL`/`CELLS` offset consts entirely).
- **Compiler** ([codegen.rs](../../../tools/koto-compiler/src/codegen.rs)): const `data` is
  laid out **above** the mutable `buf`s (so an app that addresses a buffer by an
  absolute cross-function offset — KotoBlocks' tile cache at `0`, read as `t*512`
  by `blit_piece` and the host — keeps that offset). The heap image (`rodata`) is
  the heap *prefix* up to the end of the const region; the mutable prefix is a zero
  fill. Emitted as a single `.rodata <hex>` directive.
- **kbc-asm** ([lib.rs](../../../tools/kbc-asm/src/lib.rs)): `.rodata` directive; the
  segment is placed after code (before debug) and `rodata_offset`/`rodata_size`
  written into the header.
- **Verifier** ([runtime.rs, since moved into koto-vm](../../../src/koto-vm/src/lib.rs)): rejects
  `rodata_size > max_heap_bytes` (`VerifyError::RodataExceedsHeap`); new
  `VerifiedProgram::rodata_range()` for loaders.
- **Loaders:** sim ([session.rs](../../../src/koto-sim/src/runtime/session.rs),
  [package.rs](../../../src/koto-sim/src/runtime/package.rs)) copy `rodata` from the
  resident slice; device ([app_runtime.rs](../../../src/koto-pico/src/firmware/app_runtime.rs))
  reads `rodata` straight from the SD `.kbc` into the heap while the file is open
  (it is not staged into the PSRAM code window), then zeroes only `heap[rodata_size..]`.
- **KotoBlocks:** `shapes`/`cells` are now `data`; the ~56 `heap_set_u16` bake
  calls are deleted. Compiled `code_size` dropped 30272 → 28640 bytes (−1632);
  `max_heap_bytes` unchanged.
- **Tests:** kbc-asm `.rodata` round-trip + odd-length rejection; verifier
  accept/reject around `rodata_size`; compiler `data` round-trip (u8/u16, indexing,
  no-bake) with the shared run-helper now heap-initializing from `rodata`.
- **Parity:** a 37-frame KotoBlocks scenario (title showcase blitting all 7 pieces
  via `cells`, then play + 6 down frames exercising `shape()`/active blit) is
  **byte-for-byte identical** between the baked baseline and the const-data build.

The frame=1 hardware metric (vm_us / `cw_trans` 0↔1 ping-pong) is structurally
removed: with no table-bake VM code, there is nothing to run or to straddle a code-
window boundary at startup. (On-hardware UART confirmation pending a device run.)
