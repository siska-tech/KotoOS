# KOTO-0010: KotoShell Render Model Integration

- Status: done
- Type: feature
- Priority: P0
- Requirements: FR-SHELL-2, FR-SHELL-3, NFR-DRAW-1

## Goal

Make KotoShell produce render commands for package list display and selection changes.

## Acceptance Criteria

- [x] Shell can render a package list into core render commands.
- [x] Moving selection marks only the old and new rows dirty.
- [x] Text-mode simulator can print the render command log.

## Notes

Depends on KOTO-0005.
