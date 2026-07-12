# KOTO-0093: Memo Save / Save As Filename Prompt

- Status: done
- Type: feature
- Priority: P2
- Requirements: FR-SDK-4, FR-SDK-5, FR-SIM-3

## Goal

Make F2 (Save) ask for a filename so Koto Memo can create new files, not only
overwrite the one it opened. This completes the open/save UX started in
[KOTO-0080](KOTO-0080-memo-open-save-dialog-baseline.md).

## Acceptance Criteria

- [x] Pressing F2 in the editor opens a filename prompt instead of saving
  silently, showing the current file as the default.
- [x] Confirming with an empty entry overwrites the current file.
- [x] Typing a different name and confirming saves the document under that name
  inside the sandbox and makes it the active file (Save As).
- [x] Backspace edits the entry; Ctrl cancels without saving.
- [x] Tests cover both the overwrite path and the save-as (new name) path, with
  no sandbox escape.

## Notes

KOTO-0112 supersedes the original empty-name overwrite shortcut: F2 now asks
`上書き保存しますか? (y/n)` inline. `y` overwrites and `n` opens Save As.

Reuses the existing `INTENT_SAVE` intent (no ABI change): the editor enters a
filename-entry mode and reads raw ASCII codepoints from `text_input` (letters,
digits, `.`, `-`, `_`) into a name buffer, bypassing the IME. Save As writes
through the same sandboxed `file_open` path as a normal save.

The richer app needed more VM heap; the simulator heap profile was right-sized
1024 → 2048 bytes (the memo app was already at ~1001/1024). See the deliberate
budget-sizing note alongside [KOTO-0092](KOTO-0092-compiler-local-slot-reuse.md).
