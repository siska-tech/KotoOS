# KOTO-0196: app.json authoring + .kicon editor in VS Code

- Status: done (2026-07-13) — reopened and implemented. The icon editor
  now has six-color palette controls which apply to `app.json`, and the
  descriptor editor has an app-local resource picker for `.kspr`, `.kmml`,
  and `.kacl`. Node model/edit tests and the project harness pass; the linked
  extension's palette panel and resource picker were confirmed in VS Code.
- Type: feature
- Priority: P3
- Related: KOTO-0195 (the per-app descriptor this edits — hard dependency),
  KOTO-0187 (koto-img, gains the icon conversions), KOTO-0192 (the
  line-preserving text-bitmap editor pattern this reuses), KOTO-0054
  (asset pipeline `convert-icon`).

## Goal

Make `apps/<dir>/app.json` a first-class editing surface in VS Code —
validation and completion while typing, and the app icon viewable and
editable next to it — so registering assets or reskinning an app never means
reverse-engineering or manually restructuring another app's descriptor.

## Design notes

- **JSON Schema, declaratively.** The extension contributes
  `jsonValidation` for `apps/**/app.json` with a schema file shipped in
  `tools/vscode-koto/schemas/`. The schema is also the descriptor's format
  documentation: every field `build_apps.py` consumes gets a description,
  enums (`kind`), and required/optional structure. No TypeScript needed.
- **`.kicon` is a text bitmap** (`KICON1` magic + 40×40 ASCII mask rows) —
  the same editable-text family as `.kspr`. Reuse the KOTO-0192 approach: a
  line-preserving model and a custom editor (`priority: option`) with the
  same determinism contract (untouched save byte-identical, one pixel = one
  line diff). Render the mask through the app's `shell_icon` palette from
  its descriptor when resolvable, else neutral mask colors.
- **`koto-img` gains `png2kicon` / `kicon2png`** (mask threshold on import)
  so icons round-trip with paint tools, mirroring the `.kspr` paths. Decide
  during implementation whether this replaces
  `harness/asset_pipeline.py convert-icon` (the roadmap's
  "consolidate the Python converters" note) — replacing it needs the
  `check_project` icon fixtures moved too.
- **Linking**: an editor-title command on `app.json` ("Koto: Open App Icon")
  opens the descriptor's icon in the editor.
- **Palette authoring**: the icon editor exposes the six `shell_icon` colors
  (`background`, `primary`, `secondary`, `accent`, `highlight`, `shadow`) as
  color controls. Changes preview immediately; applying them edits only the
  sibling `app.json` palette through a VS Code workspace edit, so normal
  dirty-state, undo, and save behavior remains visible to the author. If the
  descriptor has no `shell_icon`, applying a palette inserts a complete
  `style: "mask"` block with documented defaults.
- **Resource registration**: an editor-title command on `app.json` ("Koto:
  Add App Resource") picks a file below that app directory and adds the
  matching descriptor entry. `.kspr` sources become `images` entries with a
  suggested `sprites/<stem>.kim` output; `.kmml` and `.kacl` sources become
  `audio` entries with a suggested `audio/<filename>` output. The author can
  confirm or change the package-local output before applying the edit.
  Unsupported/out-of-tree files are rejected with a useful message rather
  than producing a descriptor the build cannot consume. Map registration is
  excluded because its width, height, glyph alphabet, and generated-source
  markers require a separate configured workflow rather than a file alone.
- **Minimal JSON edits**: palette and resource actions preserve unrelated
  descriptor fields, array order, indentation, and the final newline. They do
  not rewrite the whole file via `JSON.stringify`. A duplicate source/output
  is detected before editing and does not create a second entry.

## Acceptance Criteria

- [x] Opening any `apps/**/app.json` gives schema completion, field hovers,
      and validation squiggles; the schema covers every field the build
      consumes.
      → `schemas/app.schema.json` (draft-07) contributed via `jsonValidation`
      (`fileMatch **/apps/**/app.json`); covers every field `build_apps.py`
      reads (app_id/kind/package/name/description/category/runtime/source/icon/
      shell_icon/memory/permissions/codegen/maps/images/audio) with enums,
      hex-color and id patterns, and `additionalProperties:false`. Validated
      all 16 committed descriptors against it (0 failures).
- [x] `.kicon` opens in the icon editor: untouched save is byte-identical, a
      painted pixel diffs one line (model contract pinned by node tests).
      → `koto.kiconEditor` custom text editor (`priority: option`) over a
      line-preserving `media/kicon-model.js`; `test/kicon-model-test.js`
      (11 checks) pins LF/CRLF/no-trailing byte-identical round trips and
      one-line-per-pixel diffs. The mask is recolored with the app's
      `shell_icon` (resolved from the sibling `app.json` by the host).
- [x] `koto-img` converts PNG ↔ `.kicon` round-trip-stable; one shipped icon
      re-exported and re-imported as the proving case.
      → `png2kicon` / `kicon2png` (luminance-over-white threshold); crate test
      `kotorogue_icon_png_round_trip_reproduces_identical_kicon` and a CLI
      proving run on `apps/memo/icon.kicon` both reproduce the mask
      byte-identically.
- [x] The icon opens from its `app.json` via the editor affordance.
      → **Koto: Open App Icon** editor-title button on `app.json` reads the
      descriptor's `icon` and opens it in the icon editor.
- [x] The icon editor shows editable controls for all six `shell_icon` colors;
      `background` and `primary` update the mask preview live, and applying a
      valid palette updates or inserts only `shell_icon` in the sibling
      `app.json` through an undoable VS Code edit.
- [x] Palette input accepts only `#RRGGBB`, reports invalid values without
      changing the descriptor, and works for descriptors that initially omit
      `shell_icon`.
- [x] **Koto: Add App Resource** on an `app.json` registers an in-app `.kspr`,
      `.kmml`, or `.kacl` file under the correct `images`/`audio` array with a
      sensible, confirmable package output; it can create a missing array.
- [x] Resource registration rejects files outside the app, unsupported file
      types, and duplicate source/output entries with an actionable message.
- [x] Automated extension tests pin palette update/insertion and resource
      array update/insertion while preserving unrelated JSON, indentation,
      array order, CRLF/LF, and the final-newline state.
- [x] The extension README and App Development Loop guide document palette
      editing, resource registration, supported file types, output defaults,
      and the explicit exclusion of map registration.
- [x] After **Developer: Reload Window**, confirm the six palette rows and
      apply button appear beside a `.kicon`, and the resource-picker button
      appears in the `app.json` editor title.
