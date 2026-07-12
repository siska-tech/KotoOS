# KOTO-0108: Memo input blocked after opening long document

- Status: done
- Type: bug
- Priority: P1
- Requirements: FR-TEXT-?, FR-FS-?

## Goal

Fix the memo app so text input still works after opening a document that is long
enough to require scrolling.

## Acceptance Criteria

- [x] Opening a long document does not block subsequent text input.
- [x] Text input works when the document is scrolled.
- [x] IME input works when the document is scrolled.
- [x] Cursor/caret remains visible after input.
- [x] Scroll row and cursor position remain consistent after editing.
- [x] Regression test opens a long document, scrolls or starts scrolled, types text, and verifies document mutation.
- [x] `python harness/check_all.py` passes.

## Notes

The simulator editor and memo document buffer now hold 1024 bytes. File opens
read at most 960 bytes, deliberately preserving 64 bytes of immediate editing
headroom instead of filling the editor exactly and making every insertion fail.
Saving can use the full 1024-byte buffer.

The regression test opens an 80-line file through the picker, verifies it starts
scrolled, appends ASCII and IME kana, and checks the cursor remains visible with
a consistent nonzero scroll row.
