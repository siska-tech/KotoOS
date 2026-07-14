# KOTO-0202: Package-authored map assets

- Status: done
- Type: feature
- Priority: P2
- Requirements: FR-RT-5, NFR-RT-2
- Related: KOTO-0116, KOTO-0130, KOTO-0195, KOTO-0198, KOTO-0200

## Goal

Treat editor-authored tilemaps as read-only package assets instead of copying
their complete contents into generated Koto source. Standardize `.map` as the
authoring extension, validate and package each declared map, and let bytecode
apps load map bytes explicitly with `asset_load` into bounded heap buffers.

## Acceptance criteria

- [x] Define and document the text `.map` resource contract, including UTF-8
  glyph rows, line-ending handling, declared width and height, allowed glyphs,
  and the maximum byte count an app must reserve for `asset_load`.
- [x] Change the `app.json` `maps` build flow to discover and validate `.map`
  files rather than `.txt` files, and report actionable errors for malformed
  dimensions, glyphs, encoding, or duplicate package paths.
- [x] Every validated map is staged into its app's KPA as a manifest-declared,
  read-only asset with deterministic ordering and a stable package-local path.
- [x] Map staging is isolated per app: two app folders may both author a file
  such as `maps/world.map` without one package receiving the other app's data.
- [x] `build_apps.py` no longer emits map contents or a generated `stage_data`
  string into Koto source. If multiple stages need generated metadata, it is
  limited to paths and dimensions rather than map payload bytes.
- [x] Rename the KOTO-0200 sample sources from `maps/world.txt` to
  `maps/world.map` and update their descriptors, documentation, and editor
  integration accordingly.
- [x] Both KOTO-0200 samples allocate bounded map buffers, call `asset_load`
  for their declared `.map` asset, check the returned byte count or error, and
  decode the documented row format without relying on a bytecode string
  pointer.
- [x] The static and scrolling samples preserve their current retained-tilemap
  behavior, including marker lookup, camera clamping, staged viewport upload,
  and idle-frame avoidance of redundant `game2d_set_tile` calls.
- [x] Package and simulator tests prove that the `.map` bytes loaded at runtime
  match the authored file, undeclared map paths remain inaccessible, and a
  malformed or truncated load fails safely rather than reading beyond the
  initialized buffer.
- [x] A regression check inspects the generated Koto/KBC path and proves that
  the flattened world payload is not embedded in app bytecode.
- [x] The VS Code tilemap editor recognizes `.map` resources and continues to
  validate and edit both KOTO-0200 sample maps.
- [x] `python harness/build_apps.py --check`,
  `python harness/check_project.py`, and the relevant compiler, package,
  simulator, and VS Code extension tests pass.

## Design notes

- Use the existing one-shot `asset_load(path, len, buf, max)` security model;
  maps are package assets, not save-sandbox files and not a new filesystem API.
- Keep source maps readable as rows. The build may canonicalize line endings
  for deterministic packaging, but it must not flatten the payload into source
  code or bytecode.
- Runtime parsing must account for row separators explicitly. Direct
  `world + y * width + x` indexing is valid only if the packaged format is
  defined as headerless packed cells; otherwise a bounded decoder should copy
  glyph cells into a flat buffer before tile upload.
- The descriptor remains the single authoring surface. Map asset declarations
  should be derived from or represented by its `maps` block rather than
  duplicated manually in the generated manifest.

## Implementation notes

- `build_apps.py` validates app-local `.map` files and stages them through a
  per-app temporary asset root, so identical package paths remain isolated.
- KOTO-0200 and Sokoban now load authored maps with `asset_load`; no flattened
  map payload or generated `stage_data` function is emitted into bytecode.
- `check_project.py` compares packaged map payloads byte-for-byte with their
  app-local sources, while simulator fixtures cover missing/truncated loads and
  retained rendering behavior.
