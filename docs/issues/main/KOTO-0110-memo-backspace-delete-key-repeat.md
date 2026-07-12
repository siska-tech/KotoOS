# KOTO-0110: Memo Backspace/Delete key repeat

- Status: done
- Type: UX
- Priority: P2
- Requirements: FR-TEXT-?, FR-SIM-?

## Goal

Allow Backspace and Delete to repeat naturally while held in the simulator memo.

## Acceptance Criteria

- [x] Backspace repeats while held.
- [x] Delete repeats while held.
- [x] Repeated intents continue to use existing document and IME deletion semantics.
- [x] Regression coverage verifies repeated Backspace and Delete frames.
- [x] `python harness/check_all.py` passes.

## Notes

KotoSim now requests the window backend's standard key-repeat events for
Backspace and Delete, matching the existing arrow-key behavior. No editor or IME
state-machine changes were needed.

