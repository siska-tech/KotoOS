# KOTO-0025: Keyboard Matrix Validation Plan

- Status: done
- Type: research
- Priority: P1
- Requirements: FR-SDK-6, NFR-PERF-6, HC-8

## Goal

Define how to validate safe game button chords on real PicoCalc hardware.

## Acceptance Criteria

- [x] Test procedure lists candidate A/B/X/Y mappings.
- [x] Logging format records held and detected key states.
- [x] Default mapping decision is documented after hardware validation.

## Notes

The accepted plan lives in [KEYBOARD_MATRIX.md](../../hardware/KEYBOARD_MATRIX.md).
The default mapping decision record is intentionally `pending` until the
procedure is run on real hardware; do not ship a concrete embedded default
without the JSONL evidence log described there.
