# KOTO-0085: Shell Command Bar Actions

- Status: done
- Type: feature
- Priority: P1
- Requirements: FR-SHELL-2, FR-SIM-2

## Goal

Make the shell command bar truthful and discoverable, with labels that match
available input routes.

## Acceptance Criteria

- [x] The command bar shows open/launch, the details-pane toggle, favorite, sort,
  and category actions as key-chip entries (power is omitted until a real route
  exists).
- [x] The details-pane toggle is bound to `Cancel` (Backspace, distinct from the
  launch/Enter route), flips KOTO-0083's pane visibility, and its label shows the
  current `ON`/`OFF` state.
- [x] Enabled entries (launch, toggle) map to real `confirm`/`cancel` input
  routes shared by window and scripted input; launch disables when no package is
  selectable.
- [x] Favorite, sort, and category are rendered visibly disabled (dimmed, no key
  chip) until KOTO-0086.
- [x] Tests cover the command-bar entries for a normal selection, the toggle
  state, and the empty-package (launch disabled) case.

## Notes

The command bar is data-driven via `ShellState::command_bar`, which returns
`ShellCommand` entries (key, label, optional state, enabled). `paint_command_bar`
draws key chips for enabled entries, dims disabled ones, and breaks at a command
boundary when space runs out. Favorite/sort/category become enabled in KOTO-0086.
