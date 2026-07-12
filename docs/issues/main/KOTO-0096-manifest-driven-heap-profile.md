# KOTO-0096: Manifest-driven per-app heap profile

- Status: done
- Type: harness
- Priority: P1
- Requirements: FR-RT-3, NFR-MEM-2

## Goal

Stop every app requesting a fixed 4 KB heap (the drift introduced when KOTO-0060 set a
single `SIM_VM_HEAP_BYTES`). Each app should request only the heap it needs; the
simulator should size that app's VM heap to the request; and the package manifest's
declared budget should validate it.

## Acceptance Criteria

- [x] The VM heap is sized per app (no fixed `SIM_VM_HEAP_BYTES`): the `BytecodeVm`
  no longer owns a const-generic heap array; the heap is supplied to `execute_frame`
  as a `&mut [u8]` the simulator sizes from the program's KBC header.
- [x] The compiler writes each app's *actual* heap need (buffers + string data) to the
  header, floored at a small minimum, not a fixed 4 KB.
- [x] `RuntimeLimits::max_heap_bytes` is the device heap *ceiling* (16 KB); `verify_kbc`
  rejects an app requesting more.
- [x] The launch path rejects an app whose header heap exceeds the manifest's
  `sram_work_bytes` (`SimError::AppExceedsMemoryBudget`).
- [x] Tests cover a small app loading & running, and an over-budget app being rejected.
- [x] `python harness\check_all.py` passes.

## Resolution

- `koto-core` `BytecodeVm<STACK, CALLS>` (dropped the `HEAP` const generic). The heap is
  threaded as `&mut [u8]` through `execute_frame` → `step` → `exec_host_call` and the
  `STORE`/`LOAD` and `heap_slice` paths; bounds are checked against the slice length.
  This stays `no_std`/alloc-free in core — the *caller* owns the buffer.
- `koto-sim` `BytecodeAppSession` owns a `Vec<u8>` heap sized to
  `program.header().max_heap_bytes` and lends it each frame; `launch` validates the
  header against the manifest `sram_work_bytes`.
- `koto-compiler` floors the header heap at 64 B and otherwise emits the exact
  buffer+string total. Apps now request, e.g., 64 B (samples), 1111 B (memo),
  3966 B (KotoBlocks) instead of a uniform 4096.

A future per-app *fuel*/stack profile and a manifest-declared heap maximum (vs. the
compiler-computed need) can build on this; the heap source of truth today is the
compiler-computed header, with the manifest as the validating device budget.
