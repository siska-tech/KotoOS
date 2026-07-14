# KOTO-0203: VS Code tilemap `.kspr` preview

- Status: done
- Type: feature
- Priority: P2
- Related: KOTO-0192, KOTO-0195, KOTO-0198, KOTO-0200, KOTO-0202

## Goal

Render editor-authored `.map` files with their actual `.kspr` tiles in the VS
Code tilemap editor. Define an explicit glyph-to-tile mapping in the app
descriptor so the editor never guesses which sprite belongs to a map, while
keeping `.map` files as headerless, portable grid data.

## Acceptance criteria

- [x] Extend the `app.json` `maps` contract with an optional app-relative
  `.kspr` tileset path and an explicit mapping from every configured map glyph
  to a numeric tile ID.
- [x] Document the descriptor shape and keep tileset metadata out of the
  `.map` payload, so KOTO-0202's packaged map format and runtime `asset_load`
  contract remain unchanged.
- [x] The app schema and map-config resolver reject a tileset outside the app
  directory, a non-`.kspr` source, duplicate or missing glyph assignments,
  non-integer or negative tile IDs, and references to absent `.kspr` tiles.
- [x] When a valid tileset mapping is present, the tilemap editor parses the
  `.kspr` palette and 16x16 tile rows and renders every map cell with its
  mapped tile image.
- [x] The editor palette shows each tile preview together with an unambiguous
  glyph label, including a visible label for the space glyph.
- [x] Painting, dragging, and eyedropping continue to edit glyphs in the
  backing `.map` text rather than writing sprite data or a derived map format.
- [x] Opening and saving an untouched graphically rendered map remains
  byte-identical; changing one cell still changes only its corresponding row.
- [x] If no tileset mapping is declared, the editor retains the current glyph
  grid without an error. If a declared tileset cannot be loaded or parsed, it
  shows an actionable diagnostic and provides a safe glyph-view fallback.
- [x] Changes to the referenced `.kspr` document are reflected after the
  tilemap editor is reopened or refreshed without modifying the `.map` file.
- [x] Both KOTO-0200 retained-tilemap sample descriptors declare their
  `sprites/tiles.kspr` mapping (`.` to tile 0, `#` to tile 1, `~` to tile 2,
  and `@` to tile 3) and display graphically in the editor.
- [x] Automated extension tests cover tileset resolution, path confinement,
  `.kspr` parsing, glyph-to-tile validation, rendered cell selection, fallback
  behavior, painting, and newline/minimal-diff preservation.
- [x] `tools/vscode-koto/README.md` documents graphical tilemap setup and the
  glyph-only fallback, and `python harness/check_project.py` plus all VS Code
  extension tests pass.
- [x] Manual confirmation in an Extension Development Host covers graphical
  rendering, palette selection, painting, eyedropping, undo/redo, `.kspr`
  refresh, fallback display, saving, and reopening.

## Design notes

- Preferred descriptor shape:

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

- Keep `glyphs` during the first implementation so the existing build and
  editor contract remains backward compatible; require `tiles` keys to match
  the unique configured glyph set whenever `tileset` is present.
- Reuse the line-preserving `.kspr` model from KOTO-0192 rather than adding a
  second sprite parser or compiling KIM1 inside the webview.
- A per-map tileset override or sidecar descriptor may be added separately if
  an app later needs multiple visual themes. It is outside this issue.
- Graphical preview is authoring metadata only. Runtime code remains free to
  choose a different tile upload strategy or compiled `.kim` output path.
