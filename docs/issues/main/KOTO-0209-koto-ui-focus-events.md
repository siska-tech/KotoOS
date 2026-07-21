# KOTO-0209: KotoUI keyboard events and focus routing

- Status: done
- Type: feature
- Priority: P1
- Requirements: FR-SHELL-2, FR-SDK-2, NFR-PERF-4, NFR-PORT-1
- Related: KOTO-0016, KOTO-0042, KOTO-0208
- Roadmap: [KotoUI GUI Component Roadmap](../../planning/KOTOUI_ROADMAP.md)

## Goal

Provide deterministic, allocation-free keyboard/gamepad event normalization and
focus traversal for a flat set of controls, including modal focus scopes.

## Acceptance Criteria

- [x] Normalize navigation, activate, cancel, character/text, backspace, delete,
  home, end, and submit actions without importing a device-specific key code.
- [x] Adapt KotoOS input snapshots and text intents to the normalized event model
  while preserving press/repeat semantics.
- [x] Implement fixed-capacity focus registration with stable `WidgetId` order,
  explicit initial focus, forward/backward traversal, and programmatic focus.
- [x] Hidden and disabled controls are skipped; removal of the focused control
  selects a documented deterministic successor.
- [x] Modal focus scopes trap traversal and restore the prior valid focus when
  closed.
- [x] Focus changes damage only the old and new focus indicators.
- [x] Dispatch returns semantic responses instead of invoking application
  callbacks or retaining application references.
- [x] Unit tests cover empty/full registries, disabled controls, repeat input,
  modal open/close, cancellation, focus restoration, and capacity failure.
- [x] Event processing is non-blocking and performs no allocation or I/O.
- [x] Workspace tests and `python harness/check_project.py` pass.

## Notes

Spatial nearest-neighbor navigation and pointer hit testing are outside this
issue. Initial traversal follows explicit caller registration order.

The detailed event ordering, repeat-mask boundary, focus successor policy, and
single-modal-scope behavior are documented in
[`KOTOUI_INPUT_FOCUS.md`](../../architecture/KOTOUI_INPUT_FOCUS.md).

## Validation Notes

- `cargo test`, `cargo test -p koto-ui -p koto-core`,
  `cargo clippy -p koto-ui -p koto-core --all-targets -- -D warnings`, and
  `cargo check` for both crates on `thumbv6m-none-eabi` pass on 2026-07-15.
- KotoUI has 19 passing unit tests; KotoCore has 168, including four adapter
  tests for repeat phase, Unicode, intent ordering, and capacity exhaustion.
- Requirement/link/issue-index checks and `git diff --check` pass.
- `python harness/check_all.py` passes after synchronizing the committed KPA
  packages on 2026-07-15.
