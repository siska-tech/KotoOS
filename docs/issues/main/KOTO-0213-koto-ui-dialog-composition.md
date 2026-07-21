# KOTO-0213: KotoUI panel and modal dialog composition

- Status: done
- Type: feature
- Priority: P1
- Requirements: FR-SHELL-2, FR-SHELL-3, NFR-PERF-1, NFR-DRAW-1
- Related: KOTO-0056, KOTO-0080, KOTO-0209, KOTO-0210, KOTO-0211, KOTO-0212
- Roadmap: [KotoUI GUI Component Roadmap](../../planning/KOTOUI_ROADMAP.md)

## Goal

Provide bounded panel and modal-dialog composition for labels, lists, text
fields, and action buttons without introducing a general dynamic layout engine.

## Acceptance Criteria

- [x] Implement panel/frame rendering with title, content rectangle, border,
  padding, and optional dimmed-backdrop styling.
- [x] Provide deterministic inset, row, and button-row helpers that compute
  absolute child rectangles without allocation, recursion, or constraint
  solving.
- [x] A dialog owns a bounded list of child references/IDs, opens a modal focus
  scope, selects a documented initial action, and restores focus on close.
- [x] Accept, cancel, and close results identify the dialog and selected action;
  disabled actions cannot close it.
- [x] Opening and closing damage the complete backdrop/dialog region needed to
  restore pixels; child-only changes retain their smaller damage.
- [x] Dialogs clip safely at small surface sizes and reject impossible geometry
  without partial state changes.
- [x] Tests cover confirmation, list-picker, and text-prompt compositions,
  focus trapping/restoration, disabled default actions, clipping, and damage.
- [x] Examples show how existing Memo open/save behavior maps to the components
  without requiring Memo migration in this issue.
- [x] Fixed child/action capacities and component state sizes are documented.
- [x] Workspace tests and `python harness/check_project.py` pass.

## Notes

Nested modals, movable windows, asynchronous callbacks, and application-specific
file enumeration stay outside KotoUI.

`python harness/check_all.py` passes after synchronizing the committed KPA
packages on 2026-07-15.
