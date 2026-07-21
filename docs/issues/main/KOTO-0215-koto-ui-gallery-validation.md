# KOTO-0215: KotoUI component gallery and regression harness

- Status: done
- Type: harness
- Priority: P1
- Requirements: FR-SDK-5, FR-SIM-1, FR-SIM-2, FR-SIM-5, NFR-PERF-1
- Related: KOTO-0058, KOTO-0209, KOTO-0214
- Roadmap: [KotoUI GUI Component Roadmap](../../planning/KOTOUI_ROADMAP.md)

## Goal

Add an interactive simulator gallery and deterministic harness that exercises
every KotoUI component state, keyboard path, and repaint boundary.

## Acceptance Criteria

- [x] Provide a documented simulator-accessible gallery containing label,
  button, checkbox, list, text field, panel, and dialog examples.
- [x] The gallery is navigable using the same mapped directional, activation,
  cancel, and text input events used by KotoOS.
- [x] A deterministic scenario reaches normal, focused, pressed/activated,
  checked, disabled, scrolling, editing/composition, and modal states.
- [x] Golden frames cover the default theme and modal backdrop at 320x320 and
  fail on unintended pixel or layout changes.
- [x] Trace assertions verify focus order, semantic responses, and exact damaged
  rectangles for the deterministic scenario.
- [x] A repeated idle-frame step proves that the gallery emits no component
  damage or new paint work.
- [x] Capacity/overflow scenarios for focus, damage, list, text, and dialog
  children fail deterministically without panic or memory growth.
- [x] Gallery launch and scenario usage are documented in the harness README.
- [x] The gallery does not become a shipped end-user app unless separately
  enabled as a developer feature.
- [x] Workspace tests, golden-frame checks, and
  `python harness/check_project.py` pass.

## Notes

The gallery is the visual contract for component work. Golden updates require
an intentional component/theme change and review of the associated damage
trace.

`python harness/check_all.py`, including the Gallery tests and simulator golden
validation, passes on 2026-07-15.
