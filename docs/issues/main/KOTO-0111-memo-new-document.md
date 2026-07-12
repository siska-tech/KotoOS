# KOTO-0111: Memo new document

- Status: done
- Type: feature
- Priority: P2
- Requirements: FR-TEXT-?, FR-FS-?

## Goal

Create a named empty memo without overwriting the currently open document.

## Acceptance Criteria

- [x] F5 immediately switches to an unnamed empty document.
- [x] The filename is requested when the new document is saved.
- [x] The new document is marked unsaved and is not written until Save.
- [x] Cancel returns to the current document unchanged.
- [x] The new filename is used by subsequent Save.
- [x] `python harness/check_all.py` passes.

## Notes

`INTENT_NEW` uses intent bit 17 and is mapped to F5 in KotoSim. Creation only
switches editor state and displays `(新規)`; F2 then opens Save As and the sandbox
file appears after a non-empty filename is confirmed.
