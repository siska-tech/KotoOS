# KOTO-0080: Memo Open/Save Dialog Baseline

- Status: done
- Type: feature
- Priority: P2
- Requirements: FR-SDK-4, FR-SDK-5, FR-SIM-3

## Goal

Add a small open/save dialog baseline for Koto Memo so the app can move beyond a
single hard-coded `memo.txt` file.

## Acceptance Criteria

- [x] The memo app can show a modal or full-screen file selection surface.
- [x] The dialog lists sandbox-visible memo files (real directory enumeration via
  the new `dir_list` host call, not a placeholder).
- [x] Opening a file updates the document text, filename display, cursor, and
  dirty state.
- [x] Saving writes to the selected filename inside the app sandbox.
- [x] Tests cover opening a second file and saving edits without escaping the
  sandbox.

## Notes

Rather than fake host paths, the save-data directory listing was implemented
directly: host call `dir_list` (host ABI minor 7, id `0x70`) enumerates the app's
own sandbox directory, surfaced to Koto as `dir_count`/`dir_name`. A new `open`
input intent (bit 16) mapped to F4 raises the picker; F2 saves to the selected
file. The memo app tracks the active filename in a buffer and passes it to
`file_open`/`save_document`.

Enabling work landed alongside this issue:

- The `1<<16` intent constant forced proper 32-bit constant materialization in the
  compiler (`push_i16` only carries a sign-extended 16-bit immediate).
- The richer app needed more VM local slots; `VM_LOCAL_SLOTS` was right-sized
  16 → 48 and the operand-stack profile 8 → 16. The structural fix (per-scope slot
  reuse) is tracked in
  [KOTO-0092](KOTO-0092-compiler-local-slot-reuse.md).
