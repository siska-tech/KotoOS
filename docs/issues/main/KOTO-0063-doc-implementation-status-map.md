# KOTO-0063: Documentation Implementation Status Map

- Status: done
- Type: docs
- Priority: P0
- Requirements: NFR-DEV-4

## Goal

Create a compact status map that tells readers which requirements are simulated,
implemented in core, planned only, or pending real hardware validation.

## Acceptance Criteria

- [x] A status document or section defines labels such as `simulated`,
  `core-implemented`, `planned`, and `hardware-pending`.
- [x] Hardware-pending items include LCD, keyboard matrix, SD, PSRAM, audio, and
  power validation.
- [x] README links to the status map.
- [x] The map references the relevant issue IDs rather than duplicating long
  design text.
- [x] `python harness\check_project.py` passes.

## Notes

This is meant to reduce context load before implementation sessions.

## Resolution

Added `docs/IMPLEMENTATION_STATUS.md` with stable status labels, a compact
simulator/core baseline, and an explicit hardware-pending table. The validated
KOTO-0065 blink/CDC result is separated from the pending LCD, keyboard, SD,
PSRAM, audio, and power probes.
