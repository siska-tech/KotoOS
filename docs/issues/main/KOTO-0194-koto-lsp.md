# KOTO-0194: koto-lsp — live diagnostics, definitions, and budget inlays

- Status: in-progress — implemented 2026-07-13. The `koto-lsp` workspace
  binary and dependency-free VS Code client provide debounced unsaved-buffer
  diagnostics, include-aware definitions, signature/slot and constant hover,
  and a 90%-aware slot-budget inlay. Eight Rust server/protocol tests, LSP
  clippy, all default-member tests, all 16 app build-sync checks, extension
  model tests, and the project harness pass. Awaiting the final VS Code reload
  and manual UI confirmation.
- Type: feature
- Priority: P3
- Related: KOTO-0193 (compiler library — hard dependency), KOTO-0190 (the
  extension hosting the client), KOTO-0102/KOTO-0104 (slot budget semantics),
  KotoIDE roadmap Phase 3b (`docs/planning/KOTOIDE_ROADMAP.md`).

## Goal

A Rust LSP server for `.koto` built on the KOTO-0193 library:

- **Live diagnostics** on change (unsaved buffers via the overlay resolver),
  with the same file:line:col quality as the CLI.
- **Go-to-definition** for functions and consts across `include` boundaries.
- **Hover**: fn signature + slot footprint; const values.
- **Budget inlays**: `user_slots_used / 45` surfaced in-editor, warning as it
  approaches the cap (same thresholds as the harness budget gate).

## Design notes

- Scope is deliberately this list. No completion, rename, formatting, or
  semantic tokens in the first cut — each is a follow-up once the plumbing is
  proven.
- Full recompilation per keystroke is fine at this language's scale (largest
  app ~1,000 lines); debounce rather than incrementalize.
- Client side is a few lines in the KOTO-0190 extension; the server is a
  workspace binary (`tools/koto-lsp`), found via a documented setting.

## Acceptance Criteria

- [ ] Typing an error into an app source shows a squiggle at the right
      file:line:col without saving; fixing it clears the diagnostic.
- [ ] Definition jump works from a call in `main.koto` to a helper defined in
      an included file and back.
- [ ] Hover shows fn footprint; the slot inlay matches `--slot-map` output on
      the same source.
- [x] Server logic covered by Rust tests against the library API (no
      editor-driven test harness required).

## Implementation notes

- `tools/koto-lsp` implements the LSP stdio framing directly with
  `serde_json`, keeping dependencies small. It supports initialize/shutdown,
  full document sync, publishDiagnostics, definition, hover, and inlayHint.
- The server finds the owning app's root source through the nearest
  `app.json`, overlays every open `.koto` buffer, and recompiles all open app
  roots after each debounced update. Previously published diagnostics are
  explicitly cleared after a successful fix.
- The VS Code client is plain JavaScript with no npm dependency. It runs
  `cargo run -q -p koto-lsp --` by default; `koto.languageServer.path` selects
  a prebuilt server, `enabled` disables it, and `debounceMs` controls the
  default 150 ms delay.
- Rust tests cover unsaved diagnostic publish/clear, include-crossing
  definition, function signature and slot-footprint hover, constant hover,
  the exact slot-map inlay values and 90% warning threshold, UTF-16 positions,
  file URI conversion, and JSON-RPC framing.
