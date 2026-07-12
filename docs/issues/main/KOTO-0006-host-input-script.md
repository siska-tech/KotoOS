# KOTO-0006: Scripted Host Input Harness

- Status: done
- Type: harness
- Priority: P1
- Requirements: FR-SIM-2, NFR-PERF-4

## Goal

Allow KotoSim to replay simple scripted input sequences for deterministic shell and IME tests.

## Acceptance Criteria

- [x] A text input script can express up/down/confirm/cancel.
- [x] KotoSim can run a script and print resulting shell actions.
- [x] Tests cover navigation over multiple package entries.

## Notes

This gives useful automation before a graphical SDL2 backend exists.
