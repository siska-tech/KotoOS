# KOTO-0034: Cooperative Bytecode VM Core

- Status: done
- Type: feature
- Priority: P0
- Requirements: FR-RT-1, FR-RT-2, FR-RT-3, FR-RT-4, FR-SDK-1, FR-SDK-2, NFR-REL-1

## Goal

Implement the first cooperative `kotoruntime-bytecode` interpreter over verified
`KBC1` code. The VM should run a small integer-first instruction subset with a
bounded operand stack, bounded call stack, per-frame fuel, deterministic traps,
and a host-call dispatch boundary.

## Acceptance Criteria

- [x] The VM can execute `nop`, `halt`/exit, `push_i16`, stack movement, basic
      integer arithmetic, conditional/unconditional branches, `call`, `ret`, and
      `host_call` for the minimal fixture subset.
- [x] Operand stack, call depth, heap access, branch target, division by zero,
      and fuel exhaustion failures return `VmError` values instead of panicking.
- [x] A host-call trait reserves the ABI IDs from `docs/RUNTIME_BYTECODE_ABI.md`
      and implements at least `exit`, `yield_frame`, `draw_rect`, and
      `input_snapshot` for tests.
- [x] The VM samples input once per frame, consumes a caller-provided fuel budget,
      and preserves VM state across `yield_frame` and `FuelExhausted`.
- [x] Tests include a draw/input/exit fixture and a fuel exhaustion fixture that
      leaves the VM resumable.

## Notes

Depends on KOTO-0033. This issue should stay focused on the portable core, not on
package loading or simulator CLI behavior. File and asset host calls can remain
reserved until KOTO-0035 or a later storage-focused issue wires them to KotoFS.

Implemented in `koto-core::runtime` with `BytecodeVm`, `VmHost`,
`VmInputSnapshot`, `VmRunResult`, and deterministic `VmError` traps. The first
host-call subset handles `exit`, `yield_frame`, `draw_rect`, and
`input_snapshot`; text and file calls remain reserved for KOTO-0036.
