# KOTO-0062: Manifest JSON Parser Cleanup

- Status: done
- Type: feature
- Priority: P1
- Requirements: FR-PKG-1, FR-SIM-5, NFR-DEV-4

## Goal

Replace the simulator's hand-written manifest JSON field extraction with a
structured parser in host-side crates.

## Acceptance Criteria

- [x] KotoSim parses app manifests through a JSON parser instead of ad hoc string
  scanning.
- [x] Existing valid and invalid manifest tests continue to cover required
  fields, optional fields, and malformed values.
- [x] Core package validation remains no-std friendly and is not coupled to
  serde or host-only dependencies.
- [x] `python harness\check_all.py` passes.

## Notes

This issue applies to host-side simulator/tooling code. It should not make
`koto-core` depend on `std`.

## Resolution

KotoSim now parses manifests with host-only `serde_json` in `manifest.rs`.
Required root fields and nested `permissions` / `memory` objects are read
structurally before the existing no-std `PackageManifest` validation runs.
Tests cover malformed JSON and ensure nested duplicate field names cannot be
mistaken for required root fields. `koto-core` has no serde dependency.
