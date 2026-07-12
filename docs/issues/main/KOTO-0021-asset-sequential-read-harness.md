# KOTO-0021: Sequential Asset Read Harness

- Status: done
- Type: harness
- Priority: P1
- Requirements: FR-FS-3, FR-PKG-2, HC-6

## Goal

Verify that package asset layout and readers favor sequential access.

## Acceptance Criteria

- [x] Fixture package layout records asset offsets.
- [x] Harness detects non-monotonic asset offsets.
- [x] Reader API can request preload windows.

## Notes

This keeps SD-card performance constraints visible early.

Implemented by the KPA layout CSV fixtures in `harness/fixtures`, the
`check_project.py` monotonic offset check, `kpa_packer::validate_layout`, and
the `koto_core::KpaReader` preload-window API.
