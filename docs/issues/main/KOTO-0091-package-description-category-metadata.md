# KOTO-0091: Package Description And Category Metadata

- Status: done
- Type: feature
- Priority: P0
- Requirements: FR-SHELL-1, FR-PKG-1

## Goal

Extend the `.kpa` manifest and the in-core package model with the `description`
and `category` fields the home-screen details pane needs, so KOTO-0083 and
KOTO-0086 can show real data instead of placeholders.

## Acceptance Criteria

- [x] `ManifestFields` and `PackageInfo` carry an optional short `description` and
  a `category`, with documented length limits (`MAX_DESCRIPTION_LEN` = 128,
  `MAX_CATEGORY_LEN` = 32).
- [x] The manifest JSON parser reads `description` and `category` when present and
  degrades cleanly when absent.
- [x] Bundled sample/fixture manifests (`sdcard_mock/apps/*.kpa.json`) set the new
  fields.
- [x] `docs/KPA_FORMAT.md` documents the new optional fields.
- [x] Tests cover manifests with and without the new fields, and length-limit
  enforcement.

## Notes

This is the foundation (phase 0b) for the Shell UX details pane. It is the
manifest extension KOTO-0086 asks to split out before the organization UI. The
selected-app size is derived from package bytes and last-opened time is held in
shell state, so neither is part of this manifest extension.
