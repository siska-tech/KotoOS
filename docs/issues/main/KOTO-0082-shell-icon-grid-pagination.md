# KOTO-0082: Shell Icon Grid And Pagination

- Status: done
- Type: feature
- Priority: P0
- Requirements: FR-SHELL-1, FR-SHELL-2, FR-SHELL-3

## Goal

Render packages as a paged icon grid instead of a simple list, matching the PDA
home-screen direction.

## Acceptance Criteria

- [x] Packages render as large icon tiles with labels (labels are clipped to the
  tile width and centered).
- [x] Directional input moves selection within a grid, using the column count of
  the current layout mode (`shell_grid_columns`).
- [x] Selection clamps predictably at row, column, and page edges and advances
  across pages when moving past the last visible row.
- [x] Page indicators show current page and total pages (left/right triangles
  flanking page numbers, current page accented).
- [x] Tests cover navigation across rows, columns, and pages, plus the page-flip
  repaint path.

## Notes

Use existing package icons where available. Placeholder icons are acceptable
until KOTO-0087 provides a stronger icon asset set.

Pagination renders the page containing the selection. `render_selection_change`
repaints only the two affected tiles within a page, and the whole grid area on a
page flip. Per-page item counts come from `ShellState::items_per_page`, which
depends on the layout mode.
