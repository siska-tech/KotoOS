# KOTO-0083: Shell Selected App Details Pane

- Status: done
- Type: feature
- Priority: P1
- Requirements: FR-SHELL-1, FR-SHELL-2, FR-PKG-1

## Goal

Show useful selected-app information beside the icon grid in a right-side details
pane that the user can **toggle on or off** (not permanently displayed).

## Acceptance Criteria

- [x] The selected app name and short (wrapped) description render in a right-side
  details pane.
- [x] The pane shows metadata slots for last opened time, size, category, and
  favorite state. Category is real (KOTO-0091); the rest use deterministic
  placeholders until their data sources exist.
- [x] The selected-app pane updates when selection changes
  (`render_selection_change` repaints the pane).
- [x] The pane can be toggled shown/hidden; `ShellState` carries `detail_pane`
  with `toggle_detail_pane`/`set_detail_pane_visible`. `Cancel` toggles it.
- [x] Toggling relayouts the screen: the grid uses the full width when the pane is
  hidden; the caller repaints the full surface (the simulator paints fully each
  frame).
- [x] After a toggle the selection index is re-clamped and the page is recomputed
  (`current_page`) for the new grid geometry.
- [x] The old full-screen details view (`ShellView`) is removed and folded into
  the pane.
- [x] Tests cover detail text/pane bounds, the toggle path and selection re-clamp,
  and both shown/hidden layouts.

## Notes

KOTO-0057 added a details view. This issue upgrades it into a toggleable
home-screen information pane like the target mockup. Real device is 320x320
square, so the pane-shown layout is width-constrained (left grid ~190px + pane
~130px); the pane-hidden layout reclaims the full width for a wider/denser grid.
Layout-mode geometry is shared with KOTO-0081 and KOTO-0082; the toggle command
and key binding live in KOTO-0085; optional persistence of the last visibility
state is part of KOTO-0086.
