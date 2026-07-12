# KOTO-0079: Memo Command Bar Actions

- Status: done
- Type: feature
- Priority: P1
- Requirements: FR-SIM-2, FR-SDK-4, FR-SDK-5

## Goal

Make the bottom command bar truthful: every displayed action should have a
working input route and visible feedback.

## Acceptance Criteria

- [x] The command bar lists save, open, convert, back/cancel, and exit actions
  only when they are meaningful.
- [x] The displayed key labels match KotoSim window input and scripted app input.
- [x] Save updates the visible save state and dirty state.
- [x] Back/cancel behavior is defined for normal editing and active IME popup
  states.
- [x] Tests cover at least save, exit, convert, commit, and cancel routes through
  the bytecode app.

## Notes

Avoid adding a real file picker here. Opening files is tracked separately by
KOTO-0080.

Completed: the memo command bar now shows normal editing actions (`F2 Save`,
`F10 Exit`) or active IME actions (`Tab Convert`, `RShift Commit`,
`Ctrl Cancel`) according to state, and omits Open until the KOTO-0080 dialog
exists. Existing bytecode-session and memo validation tests cover save, exit,
convert, commit, and cancel behavior.
