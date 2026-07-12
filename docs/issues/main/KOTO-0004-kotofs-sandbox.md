# KOTO-0004: KotoFS Sandbox Path Resolver

- Status: done
- Type: feature
- Priority: P0
- Requirements: FR-FS-2, NFR-REL-1

## Goal

Add a core KotoFS path resolver that maps app-visible virtual paths into sandbox-relative paths and rejects traversal outside the sandbox.

## Acceptance Criteria

- [x] `koto-core` exposes a filesystem module.
- [x] App paths like `/data/save.dat` resolve inside the app sandbox.
- [x] Traversal paths like `../other_app/save.dat` and `/../x` are rejected.
- [x] Unit tests cover valid paths, normalization, and rejection cases.

## Notes

This is core-only logic and should remain `no_std` friendly.
