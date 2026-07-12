# KOTO-0020: KPA Packer Prototype

- Status: done
- Type: feature
- Priority: P1
- Requirements: FR-PKG-1, FR-PKG-3

## Goal

Create the first host-side tool that turns a manifest and assets into a deterministic package artifact.

## Acceptance Criteria

- [x] Tool reads a manifest fixture.
- [x] Tool emits a package artifact or dry-run layout report.
- [x] Tests verify deterministic output ordering.

## Notes

This can start as a dry-run layout tool before a binary `.kpa` format is finalized.
