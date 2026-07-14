# koto-lsp

`koto-lsp` is KotoOS's dependency-light stdio language server (KOTO-0194).
It recompiles the owning app through the `koto-compiler` library after a
debounced full-document update and provides:

- live include-aware diagnostics from unsaved editor overlays;
- go-to-definition for functions, constants, and data definitions;
- function signature / slot-footprint and constant-value hover;
- a `user_slots_used / 45` inlay, prefixed with `⚠` at the harness's 90%
  warning threshold.

It intentionally does not provide completion, rename, formatting, or semantic
tokens. Run it directly over stdio with:

```powershell
cargo run -q -p koto-lsp --
```

The VS Code extension starts that command by default. To use a prebuilt binary,
set `koto.languageServer.path` to its absolute path; set
`koto.languageServer.enabled` to `false` to disable the server. Full-document
updates are debounced by `koto.languageServer.debounceMs` (default 150 ms).
