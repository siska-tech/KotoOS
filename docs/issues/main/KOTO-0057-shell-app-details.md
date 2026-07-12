# KOTO-0057: Shell App Details View

- Status: done
- Type: feature
- Priority: P2
- Requirements: FR-SHELL-1, FR-SHELL-2, FR-SHELL-5, FR-PKG-1, FR-RT-4

## Goal

Extend KotoShell so users can inspect app details before launch, including
runtime type, manifest metadata, permissions, memory request, and save-data
presence.

## Acceptance Criteria

- [x] Package metadata exposed by the manifest model includes the fields needed
      by an app details view.
- [x] KotoShell can show selected app details in the simulator without launching
      the app.
- [x] The view includes runtime, entry, permissions, memory request, and save-data
      status when available.
- [x] Rendering tests verify the details view stays within the 320x320 shell
      surface.

## Notes

This makes the package model visible to users and gives KotoOS more of an
operating environment feel, not just a launcher list.

## Implementation Notes

Completed in KotoShell by adding a details view opened with cancel/Backspace
from the launcher list. The manifest-backed package model now exposes runtime,
entry, permissions, memory requests, and simulator-loaded save-data presence.
The details view renders inside the existing 320x320 shell surface; Enter still
launches the selected app.
