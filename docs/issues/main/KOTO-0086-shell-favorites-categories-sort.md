# KOTO-0086: Shell Favorites Categories And Sort

- Status: done
- Type: feature
- Priority: P2
- Requirements: FR-SHELL-1, FR-SHELL-2, FR-SIM-3

## Goal

Add basic organization features to KotoShell: favorite state, categories, and
sorting.

## Acceptance Criteria

- [x] Favorite status is a `PackageInfo` flag toggled via
  `ShellState::toggle_selected_favorite` (F2).
- [x] Packages can be filtered by category; `cycle_category` (F4) walks all ->
  each distinct category -> all, and the launcher view shows only matches.
- [x] Sorting mode (`既定`/`名前`/`★優先`) is cycled by `cycle_sort` (F3),
  shown in the status strip and command bar, and applied deterministically by a
  stable insertion sort.
- [x] Favorites, sort, and category persist to the save-data area
  (`data/dev.koto.shell/prefs.txt`) via `save_shell_prefs`/`apply_shell_prefs`;
  the window restores them on launch and saves on change.
- [x] The details-pane visibility is intentionally session-only for now (allowed
  by the MVP note).
- [x] Tests cover favorite toggle, name/favorite sort order, category
  navigation, and a save/restore roundtrip.

## Notes

The category metadata manifest extension is split out as KOTO-0091. The shell
keeps a filtered/sorted `order` view over `packages`; `selected` indexes the
view. Favorite/sort/category mutators preserve the selected package across the
relayout. Sort/category state is visible in the status strip (`並び:` / `分類:`)
and the F2/F3/F4 command-bar entries; the favorite star shows on the tile and in
the details pane.
