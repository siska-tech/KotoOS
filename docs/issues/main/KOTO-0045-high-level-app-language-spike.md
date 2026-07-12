# KOTO-0045: High-Level App Language Spike

- Status: done
- Type: research
- Priority: P0
- Requirements: FR-PKG-3, FR-RT-1, FR-RT-4, FR-SDK-1, FR-SDK-4, FR-IME-1, NFR-DEV-3, NFR-DEV-4

## Goal

Choose and specify the first high-level source language for KotoRuntime apps so
real applications do not need to be authored directly in bytecode assembly. The
result should be a small, ahead-of-time compiled language subset that can express
the memo app and future small PDA/game apps while keeping memory and runtime
costs predictable.

## Acceptance Criteria

- [x] A language design note defines syntax, primitive types, local variables,
      control flow, functions, string/buffer handling, and host-call error
      handling for the MVP.
- [x] The design compares a small Koto-specific language against at least one
      existing embeddable language approach and explains why the selected path is
      appropriate for RP2040 constraints.
- [x] The MVP subset can express a frame loop, drawing, typed input, IME/editor
      calls, file save/load, and app exit.
- [x] Out-of-scope features are explicitly listed, including dynamic objects,
      garbage collection, closures, generics, and a large standard library.
- [x] The decision is linked from the bytecode app development roadmap.

## Notes

This issue is intentionally a spike before compiler implementation. The likely
direction is a small Koto-specific language compiled on the host PC to `KBC1`,
with the assembler kept as a low-level IR/debug format rather than the main app
authoring surface.

Decision recorded in [docs/KOTO_APP_LANGUAGE.md](../../spec/KOTO_APP_LANGUAGE.md): a small,
Koto-specific, AOT-compiled procedural language (`int`/`bool`/`buf`, `let`/`const`,
`if`/`while`/`loop`, non-recursive `fn`, heap buffers, explicit `int` host-call
results) over the integer VM, with a worked memo sketch and an explicit out-of-scope
list. Compiler, SDK prelude, and build loop follow in KOTO-0046–0048.
