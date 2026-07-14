# KOTO-0198: VS Code tilemap editor

- Status: in-progress
- Type: feature
- Priority: P2
- Related: KOTO-0192, KOTO-0195, KOTO-0196, KOTO-0054, KOTO-0135

## Goal

Provide a VS Code custom editor for the text tilemaps declared by an app's
`app.json` `maps` object, so authors can paint and validate map cells visually
without weakening the existing plain-text source-of-truth and build contract.

## Scope

- Add an optional-priority custom text editor for `*.map` map files.
- Resolve the nearest ancestor `app.json` and use its `maps.dir`, `width`,
  `height`, and `glyphs` values as the editor contract.
- Render the map as a fixed-size cell grid with a selectable glyph palette.
- Support click/drag painting and a glyph eyedropper.
- Surface the same structural errors enforced by `harness/build_apps.py`.
- Preserve ordinary VS Code text-document save, undo, redo, and dirty-state
  behavior.
- Keep direct text editing as a first-class fallback.

## Acceptance criteria

- [x] A `*.map` file under the `maps.dir` declared by its nearest ancestor
  `app.json` can be opened with **Koto Tilemap Editor** through VS Code's
  **Open With...** flow.
- [x] A text file that is not covered by a valid `app.json` `maps` declaration
  is not modified and shows an actionable explanation if the custom editor is
  selected.
- [x] The editor reads grid width, height, and allowed glyphs from `app.json`;
  it does not duplicate that metadata in the map file.
- [x] The grid renders every configured cell and the palette exposes every
  configured glyph, including a clearly visible and selectable representation
  of the space glyph.
- [x] Selecting a glyph and clicking or dragging paints cells, while the
  eyedropper selects the glyph already present in a cell.
- [x] Cell edits are applied through the backing VS Code `TextDocument`, so
  save, undo, redo, external-change handling, and dirty-state indicators behave
  like normal text editing.
- [x] Opening and saving an untouched map is byte-identical, including LF/CRLF
  choice and the presence or absence of a final newline; changing one cell
  changes only its corresponding row.
- [x] Live validation reports wrong row count, wrong row width, glyphs outside
  `maps.glyphs`, and a player-start count other than exactly one `@`, matching
  the current `harness/build_apps.py` map contract.
- [x] Invalid maps remain available through the text editor and are never
  silently normalized, truncated, or rewritten by the graphical editor.
- [x] Automated extension tests cover app-config resolution, palette creation,
  painting, newline preservation, minimal row edits, and each validation error
  using the Sokoban map format as a fixture.
- [ ] A proving edit to a Sokoban map can be rebuilt with
  `python harness/build_apps.py` and subsequently passes
  `python harness/build_apps.py --check`.
- [x] `tools/vscode-koto/README.md` documents how to open and use the editor,
  its validation behavior, and how to return to plain-text editing.
- [x] `python harness/check_project.py` passes.
- [ ] Manual GUI confirmation in an Extension Development Host covers opening,
  painting, eyedropping, undo/redo, saving, reopening, and viewing an invalid
  map.

## Design notes

- The first version is a glyph-oriented editor. The current `maps` descriptor
  defines only an allowed glyph alphabet and has no glyph-to-sprite/tile
  mapping, so an art-preview mode would have to guess. Add such a preview only
  after an explicit mapping contract is designed.
- Map filenames currently define stage ordering. Creating, renaming, deleting,
  or reordering map files remains an Explorer/text workflow and is outside this
  issue.
- The map text files remain the source of truth; KOTO-0202 packages them as
  read-only data assets instead of generating embedded Koto source.
- Follow the line-preserving custom-editor model established by KOTO-0192 and
  keep the extension dependency-free with no JavaScript build step.
