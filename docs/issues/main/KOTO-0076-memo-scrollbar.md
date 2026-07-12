# KOTO-0076: Memo Scrollbar

- Status: done
- Type: feature
- Priority: P1
- Requirements: FR-IME-3, FR-SIM-1

## Goal

Add a right-side scrollbar to Koto Memo so long documents show their scroll
position and visible range.

## Acceptance Criteria

- [x] The editor model exposes or derives total logical line count.
- [x] The memo app draws a right-side scrollbar track and thumb.
- [x] The thumb position changes when the visible row changes.
- [x] Cursor movement and editing keep the cursor visible while updating the
  scrollbar.
- [x] Tests cover a multiline document where scrolling changes the visible rows
  and scrollbar state.

## Notes

The attached target image uses arrow buttons and a segmented track. The first
implementation can use a simpler track/thumb as long as it is deterministic and
does not overlap text.

Completed: `MemoEditor::total_logical_lines` and host ABI minor 6
`edit_total_lines` expose total rows, and the memo app draws a deterministic
right-side track/thumb for long documents. The bytecode-session scrollbar test
drives a long document and verifies the thumb moves when the visible range
changes.
