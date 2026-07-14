# KOTO-0197: VS Code new-app project wizard

- Status: in-progress — implemented 2026-07-13. The extension exposes a
  three-step, validated new-app wizard in the command palette and Explorer
  title, delegates creation to `koto-app-scaffold`, and opens the generated
  `main.koto`. Pure wizard-model tests and the existing scaffold end-to-end
  tests pass; awaiting final VS Code reload/UI confirmation.
- Type: feature
- Priority: P2
- Requirements: NFR-DEV-3, NFR-DEV-4
- Related: KOTO-0053 (scaffold implementation), KOTO-0190 (VS Code extension),
  KOTO-0195 (`app.json` per-app descriptor), KOTO-0196 (descriptor/icon tools).

## Goal

Let an app author create a complete KotoOS app project without leaving VS
Code or remembering scaffold CLI flags. The flow must retain the existing
scaffold's validation, deterministic layout, and refusal to overwrite files.

## Acceptance Criteria

- [x] **Koto: Create New App Project** is contributed to the command palette
      and the Explorer title as a visible new-folder action.
- [x] The wizard collects a reverse-DNS app ID, display name, and project
      directory, provides a derived `apps/<slug>` default, validates each
      value inline, and shows a final confirmation screen.
- [x] Confirmation invokes `koto-app-scaffold` without a shell, streams output
      to a dedicated channel, supports cancellation, and surfaces validation,
      duplicate-ID, and existing-path errors without overwriting anything.
- [x] A successful creation opens the generated `src/main.koto` and offers to
      open `app.json`.
- [x] Dependency-free Node tests pin ID/slug/directory validation and exact CLI
      argument construction; existing Rust scaffold tests continue to prove
      that generated projects compile and launch.
- [x] Extension and app-development documentation describe the wizard and its
      generated files.
- [ ] After **Developer: Reload Window**, confirm the Explorer button and full
      wizard create an app and open its generated source.

## Notes

- The wizard deliberately wraps the Rust scaffold instead of duplicating file
  templates or manifest validation in JavaScript.
- Project directories are restricted to relative `apps/` paths in the UI.
  The underlying scaffold remains the authority for duplicate and overwrite
  rejection.
