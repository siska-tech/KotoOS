# KOTO-0055: Save Data Management

- Status: done
- Type: feature
- Priority: P2
- Requirements: FR-FS-2, FR-SHELL-2, FR-SIM-3, NFR-REL-1, NFR-DEV-3

## Goal

Add a way to inspect and manage sandboxed app save data during simulator
development, with a path that can later become a small KotoShell management UI.

## Acceptance Criteria

- [x] A simulator command or harness helper can list save data namespaces by app
      ID without exposing paths outside the mock SD root.
- [x] Save data for a selected app can be cleared or reset through a documented
      development command.
- [x] The command refuses paths that would escape the sandbox or mock SD root.
- [x] Documentation explains how save data is laid out and how to reset test
      state before running scenarios.

## Notes

Start with CLI support. A shell UI can be added later once the user-facing app
details screen exists.

Completed with `koto-sim --save-list` and `koto-sim --save-clear APP_ID`.
The simulator library now lists save-data namespaces as app IDs with file/byte
counts, validates app IDs with manifest rules before clearing, and tests reject
traversal-like values. The save layout and reset flow are documented in
[APP_DEV_LOOP.md](../../guides/APP_DEV_LOOP.md).
