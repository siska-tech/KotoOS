# KOTO-0044: Bytecode Assembler And IR Target

- Status: done
- Type: feature
- Priority: P1
- Requirements: FR-SDK-1, FR-RT-4, FR-PKG-2, NFR-DEV-4

## Goal

Provide a readable text-to-`.kbc` assembler and low-level IR target so generated
bytecode is reproducible and inspectable rather than committed as an opaque
binary blob. This is not the preferred authoring format for real apps; durable
apps should be written in the high-level language selected by KOTO-0045 and
compiled through this layer or directly to `KBC1`.

## Acceptance Criteria

- [x] An assembler accepts a text source with mnemonics for the `KBC1` opcode set,
      labels and label-relative branch/call targets, string/byte data directives, and
      header directives (stack/call/heap/abi), emitting a `verify_kbc`-valid `.kbc`.
- [x] The assembler is checked in with a CLI or command to regenerate a `.kbc` from
      a source file.
- [x] A readable low-level fixture source is checked into the repository and at
      least one committed `.kbc` fixture is produced from it.
- [x] The project harness fails if the committed `memo.kbc` does not match
      re-assembling its source or does not pass `verify_kbc`.
- [x] Tests cover assemble→verify round-trips, label resolution, and rejection of
      undefined labels and unknown mnemonics.

## Notes

Prerequisite for KOTO-0041 and KOTO-0046. Generalizes the in-test `insn`/
`kbc_with_heap`/`emit_bytes` helpers in `koto-sim` into a real tool. High-level
language ergonomics are out of scope here and belong to KOTO-0045 and KOTO-0046.
Depends on KOTO-0033 and KOTO-0034.

Implemented as the `tools/kbc-asm` crate: `assemble(&str) -> Result<Vec<u8>, AsmError>`
plus a CLI (`kbc-asm SOURCE OUTPUT`, and `--check SOURCE EXPECTED` for drift). Source
supports `.stack`/`.calls`/`.heap`/`.abi`/`.entry` directives, own-line labels with
label-relative `br`/`br_if_zero`/`call`, decimal/hex/char immediates, and a
`store_str OFFSET, "text"` pseudo-instruction that materializes constant bytes into
the app heap. The committed `sdcard_mock/bytecode/memo.kbc` is reproduced
byte-for-byte from `apps/memo/memo.kbc.asm`; a `cargo test` drift guard and the
`harness/check_all.py` "Bytecode fixture sync" step (`kbc-asm --check`) fail on any
mismatch or verifier rejection.
