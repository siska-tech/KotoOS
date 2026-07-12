# KOTO-0053: App Scaffold Tool

- Status: done
- Type: feature
- Priority: P2
- Requirements: FR-PKG-1, FR-PKG-3, FR-SIM-5, NFR-DEV-3, NFR-DEV-4
- Prerequisites: KOTO-0048

## Goal

Provide a tool or command that creates a new Koto app skeleton with source,
manifest metadata, icon placeholder, build configuration, and a starter scenario.

## Acceptance Criteria

- [x] A documented command creates a new app directory using the repository's
      chosen app source layout.
- [x] Generated files include source, manifest metadata, icon placeholder or
      reference, build/package config, and a minimal scenario.
- [x] Generated app IDs and names are validated against package manifest rules.
- [x] A generated app builds and launches in KotoSim without manual file
      rearrangement.

## Notes

This can be a small host-side tool. It should reuse manifest validation rather
than duplicating package rules.

Implemented as `cargo run -p koto-app-scaffold -- --app-id APP_ID --name NAME`.
The tool writes the app source tree, smoke scenario, manifest, icon placeholder,
and `apps/apps.json` entry. Manifest field validation is delegated to
`koto-core::PackageManifest`.
