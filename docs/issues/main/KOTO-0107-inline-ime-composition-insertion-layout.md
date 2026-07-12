# KOTO-0107: Inline IME composition insertion layout

- Status: done
- Type: bug
- Priority: P1
- Requirements: FR-TEXT-?, FR-SIM-?

## Goal

Make inline IME composition/candidate display behave like temporary inserted
text at the caret, instead of overlaying existing document text.

## Acceptance Criteria

- [x] Inline composition text shifts following text to the right on the same visual line.
- [x] Inline candidate text shifts following text to the right on the same visual line.
- [x] Cursor is shown at the end of the inline composition/candidate.
- [x] Rendering works when the caret is in the middle of an existing line.
- [x] Rendering works near the right edge, with wrapping or clipping handled intentionally.
- [x] Existing document contents are not mutated until commit.
- [x] Golden frames are updated intentionally.
- [x] `python harness/check_all.py` passes.

## Notes

The memo renderer now splits the active visual row at the editor cursor and
draws prefix, temporary IME text, and suffix separately. The suffix is shifted
by the composition/candidate width and the caret is painted at the temporary
text's end. At the right edge the preedit is intentionally clipped and the
caret is clamped inside the document region.

Regression coverage checks composing and candidate states in the middle of
`abcd`, confirms the document remains unchanged, and checks right-edge clipping.
The existing golden trace does not include this memo state and remains
intentionally unchanged; golden validation passes.
