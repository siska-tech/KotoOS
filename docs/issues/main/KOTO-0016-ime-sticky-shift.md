# KOTO-0016: Sticky Shift State Machine

- Status: done
- Type: feature
- Priority: P0
- Requirements: FR-IME-2

## Goal

Implement one-shot Shift behavior for thumb typing and SKK-style conversion triggers.

## Acceptance Criteria

- [x] Shift press arms the next character only.
- [x] The state clears after one non-shift key.
- [x] Tests cover repeated shift and cancellation behavior.

## Notes

Implemented in `koto_core::ime` as an allocation-free `StickyShift` state
machine. `Shift` arms one stroke, `Character` emits the shifted ASCII character
and clears the state, and non-text/cancel inputs clear the state without output.
Dictionary lookup and SKK conversion are still separate follow-up work.
