# KOTO-0075: Memo Font Metrics And Caret Accuracy

- Status: done
- Type: bug
- Priority: P0
- Requirements: FR-IME-3, FR-SIM-1, FR-SIM-2

## Goal

Make the editor caret line up with the actual rendered glyph positions for ASCII,
kana, kanji, and mixed-width text in KotoSim.

## Acceptance Criteria

- [x] The memo editor viewport uses the same cell metrics as the font used for
  rendering.
- [x] Caret X/Y position matches rendered text for half-width and full-width
  characters.
- [x] Horizontal movement advances by one Unicode scalar while caret position
  advances by the rendered glyph width.
- [x] Vertical movement preserves the nearest rendered column across mixed-width
  lines.
- [x] Tests cover mixed ASCII/kana/kanji cursor movement and caret placement.

## Notes

Recent fixes reduced byte-offset mistakes, but the current path still risks drift
between editor metrics, compiled memo drawing constants, and the live `.kfont`
metrics. This should become one shared source of truth.

Completed: host ABI minor 4 adds `edit_view_metrics`, the Koto SDK exposes
`edit_cell_width()` / `edit_cell_height()`, and the memo app derives row and
caret placement from those host metrics. `MemoEditor::cursor_rect` and unit tests
cover mixed ASCII/kana/kanji movement using layout cell metrics.
