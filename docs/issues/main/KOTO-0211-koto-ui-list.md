# KOTO-0211: KotoUI bounded list and selection control

- Status: done
- Type: feature
- Priority: P1
- Requirements: FR-SHELL-2, FR-SHELL-3, NFR-PERF-1, NFR-DRAW-1
- Related: KOTO-0208, KOTO-0209, KOTO-0210
- Roadmap: [KotoUI GUI Component Roadmap](../../planning/KOTOUI_ROADMAP.md)

## Goal

Implement a keyboard-first list control that displays a bounded viewport over a
caller-owned model and updates only rows affected by selection or scrolling.

## Acceptance Criteria

- [x] Define a model/row-rendering contract that borrows item data and does not
  copy an arbitrary collection into the widget.
- [x] Support empty, one-item, partially filled, and model-larger-than-viewport
  states with fixed row height and absolute bounds.
- [x] Up/down, page-up/page-down, home/end, and activation have deterministic
  selection and viewport behavior.
- [x] Selection clamps when the model shrinks and exposes a semantic
  selection-changed or activated response with the selected index.
- [x] Disabled rows are visibly distinct, skipped by navigation, and cannot be
  activated; all-disabled and empty models remain safe.
- [x] A selection move within the viewport damages only the old and new rows;
  viewport scrolling damages only the list viewport.
- [x] Optional scroll position indication uses the painter contract and no
  separately focusable child controls.
- [x] Tests cover boundary navigation, repeated keys, disabled gaps, resize,
  clipping, model shrink/growth, viewport scroll, and no-op updates.
- [x] Component state size and maximum supported item count/index type are
  documented.
- [x] Workspace tests and `python harness/check_project.py` pass.

## Notes

Variable-height rows, nested lists, drag scrolling, and grid views are outside
this issue.

The model ownership, navigation, viewport, damage, scrollbar, and memory
contracts are documented in
[`KOTOUI_LIST.md`](../../architecture/KOTOUI_LIST.md).

## Validation Notes

- `cargo test`, `cargo test -p koto-ui`,
  `cargo clippy -p koto-ui -p koto-core --all-targets -- -D warnings`, and
  `cargo check` for both crates on `thumbv6m-none-eabi` pass on 2026-07-15.
- KotoUI has 40 passing unit tests. List coverage includes empty/all-disabled
  models, repeated navigation, disabled gaps, page/home/end, activation,
  selection clamping, growth, resize, clipping, row versus viewport damage,
  scrollbar painting, and unchanged updates.
- Requirement/link/issue-index checks and `git diff --check` pass.
- `python harness/check_all.py` passes after synchronizing the committed KPA
  packages on 2026-07-15.
