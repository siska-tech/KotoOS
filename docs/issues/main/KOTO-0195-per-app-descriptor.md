# KOTO-0195: per-app `app.json` descriptor (split apps/apps.json)

- Status: done (2026-07-13) — full absorption: each app is now
  `apps/<dir>/app.json` (build recipe + manifest fields) with its icon moved
  into the folder; the `.kpa.json` manifest is generated at build time.
  `apps/apps.json` deleted. The 8 shipped apps' `.kpa` stayed byte-identical
  (see criteria); the 8 samples' `.kpa` changed only by their icon moving
  from the shared `icons/sample.kicon` to per-package `icons/sample_*.kicon`
  (identical pixels).
- Type: feature
- Priority: P2
- Related: KOTO-0190/0192 (the VS Code layer this feeds), KOTO-0196 (schema +
  icon editing on top of this), KotoIDE roadmap
  (`docs/planning/KOTOIDE_ROADMAP.md`), scaffold assets-block regression
  (known wart: re-running the scaffold rewrites the shared registry and drops
  other apps' blocks).

## Goal

Every registered app is described by one shared `apps/apps.json`, so apps are
not self-contained (copying an app folder loses its registration) and every
tool that adds an app rewrites the same file. Move each entry into the app's
own **`apps/<dir>/app.json`** and make that file the app's single descriptor.

## Current state (post KotoAudio packaging rework, 894cf3b)

A registry entry is `app_id` / `kind` / `source` / `output`
(`package_inputs/bytecode/*.kbc`) / `manifest`
(`package_inputs/manifests/*.kpa.json`) plus optional `codegen` / `assets` /
`maps` / `images` blocks. Manifests and icons live in the shared
`package_inputs/` staging tree; `sdcard_mock/apps/` holds the packed `.kpa`.

## Design notes

- **Recommended end state: `app.json` absorbs the manifest source.** The
  per-app `.kpa.json` (name, icon, `shell_icon` palette, description,
  category, memory, assets) duplicates the "describe this app" role; folding
  it into `app.json` and *generating* the staged manifest at build time makes
  the descriptor the one file an author edits — which is what the icon
  editing goal (KOTO-0196) needs, since the icon path lives on the manifest
  side today. The icon source then moves into the app folder
  (`apps/<dir>/icon.kicon`), staged into `package_inputs/icons/` by the
  build. A conservative stage 1 may migrate only the registry entry, but it
  leaves two per-app descriptor files.
- **Discovery replaces the central list.** `build_apps.py` scans
  `apps/**/app.json` recursively (samples nest at `apps/samples/<name>/`),
  sorted for determinism. Duplicate `app_id` detection — a job the central
  file did implicitly — moves into the scanner, with missing-field validation
  erroring per file.
- **Paths**: in-app paths become app-root-relative (`"src/main.koto"`), so a
  descriptor is copy-paste portable; staging outputs (`package_inputs/…`)
  stay repo-relative and explicit.
- **Consumers to migrate**: `harness/build_apps.py` (`load_registry`),
  `harness/dev_app.py` (resolution becomes "nearest ancestor `app.json`"),
  `koto-app-scaffold` (writes the new file directly — the shared-file
  rewrite, and with it the assets-block-stripping wart, disappears
  structurally), `check_project`/`check_all` validations, and the App Dev
  Loop guide (registration section). `apps/apps.json` is deleted with no
  shim; its `comment` documentation moves to the guide and the KOTO-0196
  schema.

## Acceptance Criteria

- [x] All 16 registered apps (samples included) build from per-app
      `app.json`; `build_apps.py --check`, the packer, golden frames, and
      budgets stay green.
      → all green; whole workspace `cargo test` (36 suites) green. The 8
      shipped apps' `.kpa` are **byte-identical** to HEAD (verified: the
      compiler embeds the source path in KDBG, so the build passes the
      repo-relative posix path, keeping bytecode identical). The 8 samples'
      `.kpa` changed only because their icon moved from the shared
      `icons/sample.kicon` to per-package `icons/sample_<name>.kicon`
      (same pixels) — the one intended, benign non-identity of full
      absorption.
- [x] The scaffold generates `apps/<dir>/app.json` and no longer rewrites any
      shared registry file.
      → `scaffolding_leaves_other_app_descriptors_untouched` pins that a fresh
      scaffold leaves an existing descriptor byte-for-byte unchanged; the
      shared-registry write (and the assets-block-stripping wart) is gone.
      End-to-end smoke: scaffolded an app, built it via `build_apps --app`,
      launched clean, removed it.
- [x] Duplicate `app_id` and malformed/missing-field descriptors fail the
      scan with a per-file error message.
      → `discover_apps` errors per file on missing required fields and on a
      duplicate `app_id` (naming the prior descriptor); the scaffold's
      `refuses_duplicate_app_ids` covers the scaffold side.
- [x] `dev_app.py` resolves the app for any file under an app tree via its
      `app.json` (VS Code run/screenshot/watch tasks unchanged).
      → nearest-ancestor `app.json` walk; verified resolving from a nested
      sample source, a sample run, and a `kotorogue/audio/*.kmml` asset.
- [x] `docs/guides/APP_DEV_LOOP.md` documents the descriptor and the
      generated-manifest flow (new "The app descriptor (`app.json`)" section);
      `ASSET_PIPELINE.md` and `harness/README.md` updated.
