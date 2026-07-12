# KOTO-0009: Host Filesystem HAL Adapter

- Status: done
- Type: feature
- Priority: P0
- Requirements: FR-SIM-3, FR-FS-1, FR-FS-2

## Goal

Implement a host-side filesystem adapter that mounts `sdcard_mock` and routes file access through the same core-facing shape as future device storage.

## Acceptance Criteria

- [x] `koto-sim` exposes a host FS adapter.
- [x] Paths are resolved through `SandboxPath` before host file access.
- [x] Tests prove sandboxed reads cannot escape `sdcard_mock`.

## Notes

This is the bridge from the current direct manifest scanner toward KotoFS-backed package listing.
