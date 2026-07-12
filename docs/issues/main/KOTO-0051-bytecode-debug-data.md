# KOTO-0051: Bytecode Debug Data And Source Map

- Status: done
- Type: feature
- Priority: P1
- Requirements: FR-PKG-3, FR-RT-4, FR-SIM-5, NFR-DEV-3, NFR-DEV-4
- Prerequisites: KOTO-0044, KOTO-0046

## Goal

Define and generate minimal debug data that maps bytecode instruction positions
back to high-level source locations. Runtime and compiler diagnostics should be
able to show useful `file:line:column` information instead of only a bytecode PC.

## Acceptance Criteria

- [x] A debug data format maps bytecode instruction indexes to source file, line,
      and column information.
- [x] The compiler can emit debug data into the `KBC1` debug section or a paired
      deterministic sidecar used by KotoSim.
- [x] KotoSim diagnostics use the debug map when reporting VM errors or host-call
      failures.
- [x] Tests cover debug map parsing, missing debug data fallback, and at least
      one runtime error that reports a source location.

## Notes

Keep the first format simple. Function names, local variable names, and stack
inspection are useful but not required for the MVP.

Implemented with an in-band `KDBG` debug section referenced by the existing
`KBC1` `debug_offset` / `debug_size` header fields. `kbc-asm` accepts
`.debug_file "path"` and `.loc LINE COL`; it writes one PC-to-source entry
whenever the active location changes. `koto-compiler` emits those directives for
Koto source statements, so compiled bytecode carries deterministic source
locations. `koto-core::debug_map` parses the section without allocation, and
KotoSim app diagnostics include `source file:line:column` when debug data is
present while preserving the PC-only fallback for old or hand-authored bytecode.
