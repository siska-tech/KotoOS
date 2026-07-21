# KOTO-0210: KotoUI label, button, and checkbox controls

- Status: done
- Type: feature
- Priority: P1
- Requirements: FR-SHELL-2, FR-SHELL-3, NFR-PERF-1, NFR-DRAW-1
- Related: KOTO-0208, KOTO-0209
- Roadmap: [KotoUI GUI Component Roadmap](../../planning/KOTOUI_ROADMAP.md)

## Goal

Implement the first reusable KotoUI controls—label, button, and checkbox—with
consistent theme, focus, disabled, activation, and dirty-region behavior.

## Acceptance Criteria

- [x] Implement a non-focusable label with start/center/end alignment and
  deterministic clipping or truncation inside its assigned rectangle.
- [x] Implement a button with normal, focused, pressed, and disabled visuals and
  one activation response per accepted press.
- [x] Implement a checkbox with checked/unchecked, focused, pressed, and disabled
  visuals and a value-changed response carrying the new value.
- [x] Text and accessible semantic labels are borrowed or caller-owned; controls
  do not allocate or retain font/backend resources.
- [x] Controls render exclusively through the KOTO-0208 painter contract.
- [x] Focus-only changes damage only focus/border bounds; value or label changes
  damage the component bounds; idle updates produce no paint request.
- [x] Theme tokens give every state a visible distinction in RGB565, including a
  non-color-only focus mark.
- [x] Recording-painter tests assert draw order, clipping, responses, and damage
  for every state transition.
- [x] Size measurements and maximum borrowed text behavior are documented.
- [x] Workspace tests and `python harness/check_project.py` pass.

## Notes

Icons, images, radio groups, toggle switches, and animated transitions are not
part of the basic-control issue.

The ownership, paint ordering, damage, borrowed-text, and host/embedded size
contracts are documented in
[`KOTOUI_BASIC_CONTROLS.md`](../../architecture/KOTOUI_BASIC_CONTROLS.md).

## Validation Notes

- `cargo test`, `cargo test -p koto-ui -p koto-core`,
  `cargo clippy -p koto-ui -p koto-core --all-targets -- -D warnings`, and
  `cargo check` for both crates on `thumbv6m-none-eabi` pass on 2026-07-15.
- KotoUI has 31 passing unit tests, including recording-painter coverage for
  state colors, draw order, clipping, semantic responses, damage, narrow
  geometry, repeat suppression, and Pressed/Released behavior.
- KotoCore has 169 passing tests, including native release-phase adaptation.
- Requirement/link/issue-index checks and `git diff --check` pass.
- `python harness/check_all.py` passes after synchronizing the committed KPA
  packages on 2026-07-15.
