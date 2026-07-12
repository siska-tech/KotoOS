# KOTO-0033: KBC1 Bytecode Verifier

- Status: done
- Type: feature
- Priority: P0
- Requirements: FR-RT-1, FR-RT-3, FR-RT-4, FR-RT-5, NFR-MEM-1, NFR-MEM-5

## Goal

Add the first `koto-core::runtime` verifier for the `KBC1` bytecode format so
malformed or oversized app bytecode is rejected before execution. The verifier
should implement the bounded header, resource, offset, opcode, branch target,
and host-call checks defined in `docs/RUNTIME_BYTECODE_ABI.md`.

## Acceptance Criteria

- [x] `koto-core` exposes a `runtime` module with fixed-size `KBC1` header parsing
      that does not allocate and does not panic on malformed input.
- [x] The verifier rejects bad magic, unsupported version, bad header size,
      reserved flags, invalid offset/size ranges, zero or unaligned code, and
      entry points outside the code region.
- [x] Runtime resource requests are capped against caller-provided limits for
      operand stack slots, call depth, and app heap bytes.
- [x] Unknown opcodes, invalid branch/call targets, simple static stack
      underflow, and unknown host-call IDs return deterministic verifier errors.
- [x] Unit tests cover valid minimal bytecode plus representative rejection
      cases from the ABI document.

## Notes

This is the next foundation step after the runtime selection and ABI documents.
Keep the implementation portable and `no_std` compatible; fixtures can live in
tests, but the verifier path itself should only read caller-provided byte slices.

Implemented in `koto-core::runtime` with public opcode and host-call constants,
`KbcHeader`, `RuntimeLimits`, `VerifiedProgram`, and `verify_kbc`.
