# KOTO-0214: KotoUI rendering and platform integration

- Status: done
- Type: feature
- Priority: P1
- Requirements: FR-SDK-1, FR-SDK-5, NFR-DRAW-1, NFR-MEM-2, NFR-PORT-2, NFR-PORT-3
- Related: KOTO-0119, GFX-0004, GFX-0005, KOTO-0208, KOTO-0213
- Roadmap: [KotoUI GUI Component Roadmap](../../planning/KOTOUI_ROADMAP.md)

## Goal

Connect KotoUI's painter and damage contracts to the existing KotoCore/KotoGFX
rendering path with identical simulator and firmware behavior and no dependency
cycle.

## Acceptance Criteria

- [x] Implement the painter adapter in the owning integration layer so
  `koto-ui` remains independent of `koto-core`, device HALs, and font storage.
- [x] Map fills, borders, text measurement/rendering, clipping, and focus marks to
  existing RGB565 and bitmap-font primitives.
- [x] Convert KotoUI damage into existing render/damage requests, preserving
  clipping and documented overflow fallback behavior.
- [x] Simulator and Pico paths use the same component painting logic; only their
  established render/HAL backends differ.
- [x] An idle component frame emits no redraw; focus, selection, checkbox, text,
  and modal transitions repaint no more than their declared damage.
- [x] Integration tests compare recording-painter operations with rendered pixel
  output for representative component states.
- [x] Document crate dependency direction and verify it with workspace metadata
  or an equivalent automated check.
- [x] Record release-build code size and component-state SRAM deltas against the
  pre-integration baseline and flag unexplained regressions.
- [x] `cargo check -p koto-pico --target thumbv6m-none-eabi`, workspace tests,
  and `python harness/check_project.py` pass.

## Notes

This issue adapts existing rendering facilities; it does not add a new
framebuffer, font cache, display service, or VM host-call family.

The Pico debug/release builds and `python harness/check_all.py` pass on
2026-07-15.
