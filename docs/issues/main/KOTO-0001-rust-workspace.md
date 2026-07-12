# KOTO-0001: Rust Workspace Bootstrap

- Status: done
- Type: feature
- Priority: P0
- Requirements: NFR-PORT-1, NFR-DEV-4

## Goal

Create the initial Rust workspace and enough core code to compile, test, and run a host harness.

## Acceptance Criteria

- [x] Workspace contains `koto-core` and `koto-sim`.
- [x] `koto-core` exposes initial HAL traits and shell/package primitives.
- [x] `cargo fmt --check`, `cargo test`, and `cargo clippy --all-targets -- -D warnings` pass.

## Notes

Completed as the first implementation step.
