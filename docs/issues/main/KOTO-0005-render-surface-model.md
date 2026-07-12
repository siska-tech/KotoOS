# KOTO-0005: Core Render Surface and Dirty Rectangle Harness

- Status: done
- Type: feature
- Priority: P0
- Requirements: NFR-DRAW-1, NFR-DRAW-2

## Goal

Introduce a core render command model that can describe dirty rectangle updates before a real SDL2 or Pico backend exists.

## Acceptance Criteria

- [x] Core render commands can represent full, rect, and scanline updates.
- [x] Tests verify invalid rectangles are rejected.
- [x] KotoSim can print or record render commands for shell redraws.

## Notes

This should prepare the path toward a real SDL2 window while keeping the first pass dependency-free.
