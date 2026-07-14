# KOTO-0193: koto-compiler library split with structured diagnostics

- Status: done (2026-07-13) — `CompileRequest` / `Compilation` now expose
  verified bytecode, assembly, structured diagnostics, slot maps, and mapped
  definition symbols from in-memory source. `OverlayLoader` resolves unsaved
  include buffers before its filesystem/fallback loader, and the CLI is a thin
  wrapper over this API. Compiler clippy/tests, all default-member tests, all
  16 `build_apps.py --check` builds, and the project harness pass.
- Type: refactor
- Priority: P3
- Related: KOTO-0046 (compiler MVP), KOTO-0183 (SourceMap file:line:col —
  the data this exposes), KOTO-0194 (koto-lsp, the consumer), KotoIDE roadmap
  Phase 3a (`docs/planning/KOTOIDE_ROADMAP.md`).

## Goal

`koto-compiler` is a binary; its diagnostics reach callers as formatted
strings. Split a library crate/target exposing:

- **compile-from-source API** (source text + include resolver, not just file
  paths, so an LSP can compile unsaved buffers);
- **structured diagnostics**: typed severity/message/`file:line:col` span
  (the SourceMap already computes these — surface them before formatting);
- **slot-map and symbol data** as values (per-fn footprints, definition
  sites), not just stdout text.

## Design notes

- The CLI keeps byte-identical output and flags; it becomes a thin wrapper.
  `build_apps.py`, `--check` drift detection, and all existing tests must
  pass unchanged.
- No new public-API stability promise: the library is an internal workspace
  interface for `koto-lsp` and tooling, documented as such.
- Include resolution today reads the filesystem in `preprocess.rs`; the API
  needs an injectable resolver (real FS for CLI, overlay of open editor
  buffers for LSP).

## Acceptance Criteria

- [x] Library API compiles source (with includes, including in-memory
      overlays) and returns structured diagnostics and slot/symbol data.
- [x] CLI output and behavior are byte-identical (existing tests +
      `build_apps.py --check` green).
- [x] A unit test exercises the overlay resolver (unsaved-buffer scenario).

## Implementation notes

- `compile_source(CompileRequest, &mut dyn IncludeResolver)` returns one
  `Compilation` containing optional verified bytecode/assembly/slot-map,
  structured `Diagnostic` values, and `Symbol` definitions. Existing
  `compile*` and `slot_map*` functions remain source-compatible.
- Diagnostics carry typed severity and half-open, 1-based `SourceSpan` values
  mapped through includes. Symbols cover constants, data, functions, and
  parameters with mapped definition spans; slot footprints remain the public
  `SlotMap` / `FnSlots` values.
- `OverlayLoader<L>` normalizes paths and checks editor-provided source before
  delegating to `L`; the regression test proves an unsaved include returning
  `42` overrides the saved fallback returning `1`.
- `cargo test --workspace` is not the host gate: it attempts to compile
  `koto-pico`/`embassy-rp` for Windows and fails on ARM-only `sev`, target-gated
  PSRAM imports, and the existing Pico audio size assertion. The repository's
  `cargo test` default-member suite is green.
