# KOTO-0081: Shell Visual Home

- Status: done
- Type: feature
- Priority: P0
- Requirements: FR-SHELL-2, FR-SHELL-3, FR-SHELL-5, FR-SIM-1

## Goal

Turn KotoShell into a recognizable home screen with a top status bar, main
launcher area, details area, secondary status strip, and bottom command bar.

## Acceptance Criteria

- [x] The shell draws a top home/status bar with a home label and system status
  area.
- [x] The shell uses a light theme palette consistent with the memo light theme
  (KOTO-0088).
- [x] The app launcher area and selected-app details area are visually separated.
- [x] The layout defines two modes shared with KOTO-0082/KOTO-0083: a pane-shown
  mode (left grid + right details pane) and a pane-hidden mode (full-width grid).
  Grid geometry (`shell_grid_columns`, `shell_visible_items`, tile/icon rects) is
  derived per mode rather than from a single fixed constant.
- [x] A bottom command bar lists the currently available shell actions, with a
  secondary status strip (free memory, selected item / page count).
- [x] The layout remains within the 320x320 surface without overlap in both modes.
- [x] Golden-frame and raster tests cover the expected shell chrome in both layout
  modes.

## Notes

This is the shell counterpart to KOTO-0074. Keep behavior mostly unchanged; this
issue establishes the screen frame and visual hierarchy. Real device is 320x320
square, so the pane-shown mode is width-constrained. The two-mode layout geometry
established here is consumed by the grid (KOTO-0082) and the toggleable details
pane (KOTO-0083).

Deferred to follow-ups: grid label fit/centering and pagination (KOTO-0082),
details-pane text wrapping and metadata slots (KOTO-0083), real status-bar
indicators/clock (KOTO-0084), command-bar action wiring and key chips
(KOTO-0085), and colored launcher icons (KOTO-0087). The pane-toggle method
(`ShellState::toggle_detail_pane`) exists but is wired to input in KOTO-0085.
