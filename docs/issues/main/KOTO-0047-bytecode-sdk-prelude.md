# KOTO-0047: Bytecode SDK Prelude

- Status: done
- Type: feature
- Priority: P0
- Requirements: FR-SDK-1, FR-SDK-2, FR-SDK-4, FR-IME-1, FR-IME-2, FR-IME-3, FR-FS-2, FR-RT-4
- Prerequisites: KOTO-0042, KOTO-0045

## Goal

Define a small KotoSDK prelude for high-level bytecode apps so app code uses
named drawing, input, IME, file, lifecycle, and error APIs instead of raw numeric
host-call IDs.

## Acceptance Criteria

- [x] SDK functions are specified for drawing text/rects, yielding a frame,
      reading text input, feeding/querying IME state, moving/editing a text
      buffer, reading/writing sandboxed text files, and exiting.
- [x] Each SDK function maps to documented host ABI calls and describes success
      and failure return values.
- [x] Buffer ownership and maximum sizes are explicit in the SDK documentation.
- [x] Example source snippets show a frame loop, save/load, and IME commit flow.
- [x] Compiler or assembler tests prove SDK wrappers emit the expected host-call
      sequences.

## Notes

This prelude is the app-facing contract. It should stay small and stable enough
that Koto Memo, KotoVN prototypes, and later PDA utilities can share the same
runtime concepts.

Specified in [docs/KOTO_SDK.md](../../spec/KOTO_SDK.md) and implemented in
`tools/koto-compiler`: the host-call wrappers are built-in compiler intrinsics and
the `MODE_*`/`IME_*`/`DIR_*`/`DELETE_*`/`INTENT_*` constants are predefined (sourced
from `koto_core::runtime` so they cannot drift; a user `const` of the same name
overrides). Compiler tests assert the wrappers emit the expected `host_call`
sequences, the constants fold to the right immediates, and `text_intent` aliases
the `text_input` host call.
