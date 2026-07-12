# KOTO-0106: Inline memo IME candidate display

- Status: done
- Type: UX
- Priority: P2
- Requirements: FR-TEXT-?, FR-SIM-?

## Goal

Follow up on KOTO-0078 by replacing the large bottom IME candidate panel with
inline composition/candidate display at the memo caret position.

## Background

KOTO-0078 added candidate list navigation, but the current bottom candidate
panel is too large for the 320x320 PicoCalc-style memo UI. The memo editor should
prefer inline IME display inside the text viewport, with the bottom bar reduced
to compact key hints and IME status.

## Acceptance Criteria

- [x] Composing text is displayed inline at the caret.
- [x] Active conversion candidate is displayed inline at the caret.
- [x] Candidate index/status is shown compactly in the bottom status bar.
- [x] The large bottom IME candidate panel is removed or reduced to status-only.
- [x] Memo viewport regains vertical space.
- [x] Candidate display remains visible near the bottom of the viewport.
- [x] Existing IME behavior and candidate navigation are preserved.
- [x] Golden frames are updated intentionally.
- [x] `python harness/check_all.py` passes.

## Notes

This is a visual/layout follow-up to KOTO-0078. Do not change the IME conversion
state machine or candidate selection semantics unless required by rendering.

Implemented entirely in the memo app renderer. Pending romaji and conversion
readings are underlined at the caret; an active candidate is painted there with
a compact pale highlight. The fixed `y = 210..274` conversion panel and its
five-row editor reservation were removed, restoring all 19 document rows.
Candidate position (`候補 n/m`) now lives in the bottom status bar.

Regression coverage verifies inline composing, inline candidate cycling, removal
of the old panel, and candidate visibility on the last viewport row. The memo
bytecode was rebuilt. The existing golden trace is intentionally unchanged
because its scenarios do not render the memo conversion UI; its validation still
passes.
