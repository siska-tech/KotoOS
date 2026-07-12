# KOTO-0011: KPA Manifest Validation in Core

- Status: done
- Type: feature
- Priority: P0
- Requirements: FR-PKG-1, FR-PKG-2, FR-SHELL-1

## Goal

Move manifest validation rules into reusable core code so KotoSim and future package tools agree.

## Acceptance Criteria

- [x] Core defines required manifest fields and limits.
- [x] Invalid app IDs, names, runtime names, and missing entries are rejected.
- [x] KotoSim uses the shared validation path.

## Notes

The current KotoSim parser extracts only `format`, `app_id`, and `name`.
