# vscode-koto — KotoOS Development extension

Editor support for KotoOS app development:

- **Koto Tilemap Editor** (KOTO-0198/0202/0203): open a configured `maps/*.map` file
  with *Open With… → Koto Tilemap Editor* (or **Koto: Open in Tilemap
  Editor**). The editor resolves the nearest `app.json`, reads `maps.width`,
  `maps.height`, and `maps.glyphs`, and provides click/drag painting plus a
  right-click eyedropper. Add `maps.tileset` and a complete glyph-to-tile
  `maps.tiles` mapping to render the palette and grid with the actual 16x16
  `.kspr` art:

  ```json
  "maps": {
    "dir": "maps",
    "width": 20,
    "height": 20,
    "glyphs": ".#~@",
    "tileset": "sprites/tiles.kspr",
    "tiles": { ".": 0, "#": 1, "~": 2, "@": 3 }
  }
  ```

  Tileset paths are app-relative and confined to the app folder. Every unique
  configured glyph must have a non-negative `.kspr` tile ID. Without this
  optional mapping the editor uses its glyph palette (including a visible `␠`
  space); an unreadable or invalid declared tileset reports the problem and
  safely falls back to the same glyph view. It
  reports row-count, row-width, invalid-glyph, and `@` start-count errors using
  the same contract as `harness/build_apps.py`. Map text remains the source of
  truth: save/undo/redo use the normal text document, untouched newline bytes
  are preserved, and **Reopen Editor With… → Text Editor** always returns to
  plain text. A `.map` outside the configured `maps.dir` is refused without
  modification. Saving the referenced `.kspr` refreshes an open tilemap editor;
  reopening also reloads the latest sprite source.
- **New app project wizard** (KOTO-0197): click the new-folder action in the
  Explorer title or run **Koto: Create New App Project**. The three-step flow
  validates the app ID, display name, and `apps/` directory, confirms the
  summary, runs `koto-app-scaffold`, and opens the generated `src/main.koto`.
  It creates `app.json`, `src/main.koto`, `src/helpers.koto`, `icon.kicon`, and
  a smoke scenario without rewriting another app.
- **Koto language intelligence** (KOTO-0194): the dependency-free client
  starts the Rust `koto-lsp` server and provides live diagnostics from unsaved
  buffers, include-aware go-to-definition, signature/slot hover, constant
  hover, and a `slots used/45` inlay (`⚠` at 90%). By default it runs
  `cargo run -q -p koto-lsp --` in the workspace; set
  `koto.languageServer.path` to use a prebuilt executable, or disable it with
  `koto.languageServer.enabled`. Changes are debounced for 150 ms, configurable
  through `koto.languageServer.debounceMs`.
- **Syntax highlighting** for `.koto` (app language), `.kmml` (KotoMML
  scores), and `.kspr` (ASCII sprite sheets).
- **`$koto` problem matcher**: `koto-compiler` diagnostics
  (`file:line:col: message`, including inside `include`d files) become
  in-editor squiggles when a task compiles apps.
- **File types / comment toggling** (`//` for Koto, `#` for the asset
  formats).
- **`app.json` schema** (KOTO-0196): a JSON schema
  ([`schemas/app.schema.json`](schemas/app.schema.json)) validates every
  `apps/**/app.json` descriptor with field completion, hovers, and
  enum/pattern checks — covering every field the build consumes.
- **Koto Icon Editor** (KOTO-0196): open a `.kicon` with *Open With… → Koto
  Icon Editor* for a 40×40 mask editor (left-click/drag sets, right-click
  clears). Its six-color palette panel previews `background` / `primary` and
  applies the complete `shell_icon` palette to the sibling `app.json` through
  an undoable edit. From an `app.json`, the editor-title **Koto: Open App
  Icon** button opens the descriptor's icon. The neighboring **Koto: Add App
  Resource** button selects an app-local `.kspr`, `.kmml`, or `.kacl` and adds
  it to `images` or `audio`; the suggested package output can be changed before
  applying. Map registration remains manual because it also needs dimensions,
  glyphs, and generated-source configuration. Same line-preserving contract as
  the sprite editor ([`media/kicon-model.js`](media/kicon-model.js)):
  byte-identical untouched saves, one-line diffs per pixel.
- **Koto Sprite Editor** (KOTO-0192): right-click a `.kspr` → *Open With…* →
  Koto Sprite Editor (or the editor-title button / "Koto: Open in Sprite
  Editor"). Pixel grid with click/drag paint, right-click eyedropper, palette
  select / double-click recolor, `+ color`, `+ tile`, tile list. It edits the
  **text document itself** through a line-preserving model
  ([`media/kspr-model.js`](media/kspr-model.js)): an untouched file saves
  byte-identically, one painted pixel diffs one line, and undo/redo/save are
  the ordinary text-document ones. The plain text editor stays the default.
- **`.kmml` play/stop** (KOTO-0192): editor-title buttons on `.kmml` files
  run the KOTO-0188 audition CLI (`koto-mml play`) with native KotoAudio;
  "Koto MML: Play with options" adds per-track mute and looping. Output lands
  in the "Koto MML" channel. First use compiles `koto-mml --features play`
  (cpal), which takes a minute once.

Model regression tests (run from the repo root, requires node):

```powershell
node tools/vscode-koto/test/model-test.js
node tools/vscode-koto/test/kicon-model-test.js
node tools/vscode-koto/test/app-json-edits-test.js
node tools/vscode-koto/test/project-model-test.js
node tools/vscode-koto/test/tilemap-model-test.js
```

The workspace tasks that use this (build / run current app / screenshot /
check all) live in the repo's [`.vscode/tasks.json`](../../.vscode/tasks.json)
and resolve "current app" from the edited file path via
[`harness/dev_app.py`](../../harness/dev_app.py).

## Install (from source, no build step)

The extension uses plain JavaScript with no TypeScript, `npm`, or `vsce` build
step. Its small LSP client has no external Node dependencies.
Installing is linking this folder into your VS Code extensions directory and
reloading:

```powershell
New-Item -ItemType Junction `
  -Path "$env:USERPROFILE\.vscode\extensions\koto.vscode-koto" `
  -Target "tools\vscode-koto"
```

(macOS/Linux: `ln -s "$PWD/tools/vscode-koto" ~/.vscode/extensions/koto.vscode-koto`.)

Then run **Developer: Reload Window**. Because it is a junction/symlink,
grammar edits in the repo take effect on the next reload — there is nothing
to rebuild or re-install.

To uninstall, delete the junction:

```powershell
Remove-Item "$env:USERPROFILE\.vscode\extensions\koto.vscode-koto"
```

## Design constraints

Per the [KotoIDE roadmap](../../docs/planning/KOTOIDE_ROADMAP.md), this layer
stays thin and dumb: regex grammars, a problem matcher, and plain-JS webview
editors whose format correctness is still owned by the Rust toolchain — the
sprite editor edits `.kspr` text lines (compiled/validated by
`koto-img`/`build_apps.py`, KOTO-0187), and MML playback shells out to
`koto-mml` (KOTO-0188). Nothing here parses Koto; language intelligence
arrives as `koto-lsp` (KOTO-0193/0194).
