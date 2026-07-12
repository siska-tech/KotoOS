# KOTO-0087: Shell Icon Asset Set

- Status: done
- Type: feature
- Priority: P2
- Requirements: FR-SHELL-1, FR-SHELL-3, FR-PKG-1

## Goal

Provide a coherent icon set for built-in shell categories and sample apps so the
home screen reads visually at a glance.

## Acceptance Criteria

- [x] Built-in/sample apps get semantically distinct colored icons via
  `icon_kind_for` (category, then app-id keyword, then a stable hash).
- [x] The placeholder icon set is visually consistent (one palette, 40x40) and
  documented (see Notes).
- [x] The shell renders icons at the `SHELL_ICON_SIZE` (40x40) tile icon area
  without clipping.
- [x] Asset pipeline checks still cover `.kicon` dimensions and manifest icon
  references (`harness/check_project.py`, `harness/asset_pipeline.py`).
- [x] A raster test covers an icon-rich page (distinct colored icons paint
  non-background pixels).

## Notes

The icon set is drawn in code (`IconKind` + `draw_*_icon` in `shell.rs`) rather
than as color asset files, avoiding a new color icon format for now. The eight
kinds — Notepad, Calendar, Folder, Calculator, Gear, Music, Game, Terminal —
share the light-theme palette and fit the 40x40 icon area. `icon_kind_for` maps a
package to a kind by category keyword (e.g. `カレンダー`), then app-id keyword
(e.g. `memo`, `file`, `term`), then a stable hash so unmatched sample apps still
get distinct icons. Real color icon assets and a color `.kicon` format remain
future work; the monochrome `.kicon` pipeline checks are unchanged.
