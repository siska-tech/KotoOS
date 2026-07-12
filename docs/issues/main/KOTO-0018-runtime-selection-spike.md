# KOTO-0018: Runtime VM Selection Spike

- Status: done
- Type: research
- Priority: P0
- Requirements: FR-RT-1, FR-RT-3

## Goal

Choose the first runtime VM candidate to prototype for KotoRuntime.

## Acceptance Criteria

- [x] Compare custom stack VM, Wasm interpreter, Lua, and mruby at a high level.
- [x] Estimate SRAM footprint and integration risk.
- [x] Pick one candidate for the first executable prototype.

## Notes

This is a decision point; do not build all candidates deeply.

Decision recorded in [RUNTIME_VM_SELECTION.md](../../architecture/RUNTIME_VM_SELECTION.md):
prototype a small custom stack VM first.
