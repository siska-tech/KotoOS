# KOTO-0099: Memo IME candidate overlap avoidance

- Status: done
- Type: bug
- Priority: P1
- Requirements: FR-IME-3

## Goal

Prevent the IME candidate/composition UI from overlapping the editable text area
when the caret is near the bottom of the memo viewport.

## Acceptance Criteria

- [x] Converting text with the caret on the last visible row does not cover the active editing line.
- [x] Candidate display either flips above the caret or uses the fixed IME/status area.
- [x] Existing candidate navigation still works.
- [x] Add a scripted memo scenario that places the caret near the bottom and triggers conversion.

## Notes

The full conversion panel in [`apps/memo/src/main.koto`](../../../apps/memo/src/main.koto)
is drawn at a fixed `y = 210..274`, which sits inside the document area
(`y = 24..274`). With a 13px row pitch, rows 14–18 fall under the panel, so
converting on a bottom row painted over the line being edited.

Fix: keep the candidate panel pinned to the fixed bottom area and instead make
the *editing viewport* avoid it. A new host call `edit_reserve_rows` (id `0x71`,
host ABI minor 9) tells the host editor how many bottom rows an overlay covers;
the editor keeps the cursor scrolled above that reserved band
(`MemoEditor::set_reserved_bottom_rows`, applied in `ensure_cursor_visible`).
The memo app reserves 5 rows while the panel is showing (Converting) and clears
it to 0 otherwise. This avoids the earlier "flip to top" attempt, which merely
moved the overlap onto the top rows.

Verification:
- `reserved_rows_scroll_cursor_above_overlay` (`src/koto-core/src/memo.rs`)
  covers the editor scrolling and the clamp that leaves one editable row.
- `memo_app_scrolls_caret_above_conversion_panel_at_viewport_bottom`
  (`src/koto-sim/src/lib.rs`) drives 24 newlines to put the caret on the last
  visible row, starts a かさ conversion, and asserts the panel stays at
  `(2, 210, 316, 64)` while the caret rect clears `y = 210` and candidate
  navigation still reports `傘 1/2` → `笠 2/2`.
- Frame capture confirms the panel pinned at the bottom with the document
  scrolled so the caret sits above it.
