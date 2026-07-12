# KOTO-0042: Runtime Input And IME Host Calls

- Status: done
- Type: feature
- Priority: P0
- Requirements: FR-SDK-1, FR-SDK-4, FR-IME-1, FR-IME-2, FR-IME-3, FR-RT-4, NFR-REL-1

## Goal

Extend the `kotoruntime-bytecode` host ABI so a bytecode app can receive typed
character input and drive a host-side IME/text-buffer service. Today
`VmInputSnapshot` only carries the six PicoCalc button bits, and there is no way
for bytecode to reach the romaji/kana, Sticky Shift, SKK, or editor models. This
issue adds the smallest VM-neutral primitives an interactive text app needs,
keeping the romaji→kana and SKK conversion logic as a host service the VM drives.

## Acceptance Criteria

- [x] `VmInputSnapshot` carries a typed Unicode codepoint and edit-intent bits in
      addition to the existing held/pressed button bits, and `empty()` and all
      constructors are updated.
- [x] A `text_input` host call returns the frame-stable typed codepoint and intent
      bits using the documented ABI return convention.
- [x] IME/text-buffer host calls let bytecode feed a key into the host IME+editor,
      trigger SKK conversion, query the IME composition line, move the cursor,
      delete/backspace, load a document, and read back the document text and cursor.
- [x] New host-call IDs are reserved in the dispatch table and verifier, the host
      ABI minor version is bumped, and apps requesting a higher minor are rejected.
- [x] New `VmHost` methods have default implementations so existing hosts compile
      unchanged, and heap-writing calls validate app heap ranges like `file_read`.
- [x] `docs/RUNTIME_BYTECODE_ABI.md` documents the new IDs, intent bits, and
      version note; tests cover success and invalid-pointer/unknown-key failures.

## Notes

Prerequisite for KOTO-0041. The host calls must stay VM-neutral ("text
composition / text buffer" service, not "memo") so a later Wasm runtime can reuse
them. Romaji→kana and SKK lookup stay in `koto-core` host code; this issue only
exposes them across the bytecode boundary. Depends on KOTO-0034 and KOTO-0036.

Implemented in `src/koto-core/src/runtime.rs`: `HOST_ABI_MINOR` bumped to `1`;
`VmInputSnapshot` gains `text_codepoint`/`intent_bits`; new host-call IDs
`text_input` (0x21) and the `ime_*`/`edit_*` service (0x60–0x66) with `ime_key`,
`edit_dir`, `edit_delete`, and `text_intent` constant modules; `VmHost` gains
default-`Unsupported` trait methods so existing hosts compile unchanged; and the
verifier models each host call's `(args_popped, values_pushed)` stack effect so
looping interactive apps verify accurately. The host-side IME/editor wiring lands
in KOTO-0043.
