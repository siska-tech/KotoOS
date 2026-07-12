# KOTO-0002: KotoSim Package Manifest Scan

- Status: done
- Type: feature
- Priority: P0
- Requirements: FR-SHELL-1, FR-SIM-3

## Goal

Load simulator package metadata from `sdcard_mock/apps/*.kpa.json` instead of hard-coded package entries.

## Acceptance Criteria

- [x] `sdcard_mock/apps` contains sample manifests.
- [x] `koto-sim` scans manifests and populates `PackageList`.
- [x] Harness validates simulator manifests.

## Notes

The current parser intentionally extracts only the manifest fields required by the launcher.
