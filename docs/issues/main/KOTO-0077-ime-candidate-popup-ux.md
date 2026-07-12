# KOTO-0077: IME Candidate Popup UX

- Status: done
- Type: feature
- Priority: P0
- Requirements: FR-IME-1, FR-IME-3, FR-IME-4

## Goal

Render IME conversion state as a temporary candidate popup instead of blending it
into the permanent status bar.

## Acceptance Criteria

- [x] The popup appears only while composition, reading, candidate, or missing
  candidate state is active.
- [x] The popup is visually distinct from both document text and the permanent
  command/status bars.
- [x] The popup shows input mode, reading text, current candidate or missing
  candidate feedback, and a commit/cancel hint.
- [x] Commit or cancel clears the popup immediately.
- [x] Scripted or golden-frame checks cover popup visibility and disappearance.

## Notes

This issue keeps the current single-candidate model. Multiple candidate
navigation belongs in KOTO-0078.

Completed: the memo app draws an IME-only popup while `ime_display` reports an
active state, including `IME ON`, the current composition/candidate text, and
commit/cancel hints. The bytecode-session UI test checks popup visibility and
that Cancel removes it on the next frame.
