# KOTO-0072: Memo Editor Usable UI

- Status: done
- Type: feature
- Priority: P0
- Requirements: FR-IME-3, FR-SIM-1, FR-SIM-2

## Goal

Make the memo app feel like a small editor rather than a raw text string painted
onto the framebuffer.

## Acceptance Criteria

- [x] The memo view renders visible lines from the editor model instead of a
  whole document blob.
- [x] A cursor or caret is visible and tracks horizontal/vertical movement.
- [x] Scrolling is visible and keeps the cursor in the content area.
- [x] The IME line is visually separated from document text.
- [x] Save/exit status or feedback is visible in the simulator.
- [x] Scripted or golden-frame checks cover multiline text, cursor movement,
  and IME composition display.

## Notes

The core editor now exposes viewport queries through the bytecode host ABI, and
the memo app renders visible rows, caret, scroll hint, save feedback, and a
separated IME/status band.
