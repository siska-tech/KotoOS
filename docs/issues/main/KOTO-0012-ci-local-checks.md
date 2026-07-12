# KOTO-0012: Local CI Command and Check Script

- Status: done
- Type: harness
- Priority: P0
- Requirements: NFR-DEV-4

## Goal

Create one local command that runs formatting, tests, Clippy, and project harness checks.

## Acceptance Criteria

- [x] A documented script or task runs all standard checks.
- [x] The script exits non-zero on any failed check.
- [x] README uses the single command for normal validation.

## Notes

This keeps the project pleasant before external CI exists.
