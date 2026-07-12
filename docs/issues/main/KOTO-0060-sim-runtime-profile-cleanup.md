# KOTO-0060: KotoSim Runtime Profile Cleanup

- Status: done
- Type: bug
- Priority: P0
- Requirements: FR-RT-3, FR-SIM-5, NFR-MEM-2

## Goal

Make KotoSim use one runtime profile for bytecode verification and VM
instantiation so accepted programs cannot exceed the session's actual stack,
call-depth, or heap capacity.

## Acceptance Criteria

- [x] A single simulator runtime profile defines stack slots, call depth, heap
  bytes, and frame fuel.
- [x] `verify_kbc` and `BytecodeVm` construction use matching limits.
- [x] Tests cover a program rejected because it exceeds the simulator profile.
- [x] `python harness\check_all.py` passes.

## Resolution

`RuntimeLimits::simulator_default()` (in `koto-core`) is now the single source of
the profile: stack slots (16), call depth (4), heap bytes (4096), and a new
`frame_fuel` field (60000). It was previously more permissive than the VM (256
slots / 16 KB heap) than the `SIM_VM_*` constants the VM was built from.

- `koto-sim` derives `SIM_VM_STACK_SLOTS`, `SIM_VM_CALL_DEPTH`, `SIM_VM_HEAP_BYTES`,
  and `SIM_FRAME_FUEL` from the profile, and `verify_kbc` runs against it.
- `koto-compiler` derives its `MIN_STACK` / `CALL_PROFILE` / `HEAP_PROFILE` request
  floors from the same profile.
- `runtime::tests::rejects_program_exceeding_simulator_heap_profile` covers a
  program rejected for requesting more heap than the profile allows.

Heap was raised 2 KB -> 4 KB and frame fuel 10000 -> 60000 to host the first
full-screen tile/sprite game (KotoBlocks, dev.koto.games.koto-blocks), whose
per-frame board repaint exceeds the old 10000-instruction budget. A future per-app
heap/fuel profile driven by the package manifest (4-16 KB) is left as follow-up.
