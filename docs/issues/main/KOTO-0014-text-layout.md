# KOTO-0014: Text Grid and IME Line Layout

- Status: done
- Type: feature
- Priority: P0
- Requirements: FR-IME-3, NFR-DRAW-3

## Goal

Define the screen regions for normal text content, status, and the fixed IME input line.

## Acceptance Criteria

- [x] Core layout computes content and IME rectangles for 320x320.
- [x] Layout supports at least one 8x12 or 8x16 font cell size.
- [x] Tests verify that regions do not overlap.

## Notes

This prepares both KotoShell and KotoIME drawing.
