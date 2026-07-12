# KOTO-0090: Memo Line Wrap And Horizontal Scroll

- Status: done
- Type: feature
- Priority: P1
- Requirements: FR-SIM-1, FR-SIM-2

## Goal

Make long lines and vertical scrolling behave correctly in Koto Memo: the caret
must always stay in the painted region, and lines wider than the viewport must
either soft-wrap or scroll horizontally, toggled by the user.

## Acceptance Criteria

- [x] The host editor viewport row count matches the rows the app paints, so the
  caret never scrolls behind the command bar.
- [x] Long logical lines soft-wrap onto multiple visual rows by default; the
  vertical scrollbar and caret track visual rows.
- [x] A no-wrap mode scrolls the cursor's line horizontally to keep the caret in
  view and draws a horizontal scrollbar.
- [x] Wrap mode is toggleable (F3 in the simulator window) and shown in the
  command bar (`折返ON` / `折返OFF`).
- [x] `Ln N Col M` reports the logical document position regardless of wrapping.
- [x] Tests cover wrapping, horizontal scrolling, the caret staying visible after
  vertical scroll, and the toggle.

## Notes

Wrapping is modeled as a host editor mode (like a terminal's), toggled host-side
and queried by the app via `edit_wrap` / `edit_hscroll_view`, so it needs no new
input-intent bit. `edit_total_lines` now returns visual rows. The memo editor
layout is sized to `MEMO_CONTENT_ROWS` (19) and `MEMO_CONTENT_COLS` (49) to match
the painted area before the scrollbar.
