# KOTO-0236: Compile-time asset sizes and SDK storage capacity helpers

- Status: done
- Type: feature
- Priority: P3
- Requirements: FR-PKG-3, NFR-MEM-2, NFR-DEV-3, NFR-DEV-4, NFR-REL-1
- Related: KOTO-0183, KOTO-0191, KOTO-0221, KOTO-0232, KOTO-0233, KOTO-0235

## Goal

Remove the last hand-maintained storage capacities from KotoUI-authored Apps by
(1) folding real asset byte sizes at compile time (`asset_len("path", ...)`)
and (2) adding two SDK storage capacity helpers
(`ui_text_resource_capacity`, `ui_list_rows_capacity`) in the KOTO-0232/0233
compile-time helper class. No VM/KBC/host ABI change; a runtime asset-size
host call was considered and rejected (see Notes).

## Motivation

An audit of the Gallery's three capacity constants split them into one
irreducibly App-owned fact and two leaks of SDK-private representation:

- `GALLERY_RESOURCE_RAW_BYTES = 384` bounds the largest locale asset
  (qps-ploc.txt, 343 bytes today). The number must track the App's own asset
  files by hand; an undersized bound is only caught when that locale actually
  loads (`asset_load` fails `NO_MEMORY`).
- `GALLERY_RESOURCE_BYTES = 416` is `22 lines x 4` table bytes plus a
  hand-tuned 328-byte payload arena — but `resources.koto` declares the
  4-bytes-per-line table stride *SDK-private*. The App cannot size this
  correctly without wire knowledge the SDK reserves the right to change.
- `buf rows_blob[68]` is `3 rows x 12` record bytes plus a label arena; the
  12-byte KUI1/KUP1 row record stride is likewise SDK-private, and the
  declared 68 carries 2 bytes of unexplained slack against the 66-byte
  mount-slot fact beside it — hand-summing drift in miniature.

File Note's `NoteStorage` (KOTO-0235) has the same shape: `raw: buf[384]`
tracks its locale assets by hand, and a text-resource region sized by a raw
literal.

With both pieces, a translation that grows re-folds the storage on the next
build instead of failing in CI or on device, and the SDK-private strides
return to the SDK. The *semantic* budgets stay explicit and App-owned: the
per-line display caps, mount arena slot sizes, and the runtime heap budget
gate are unchanged and still bound total growth.

## Proposed Contract

### `asset_len(...)` compile-time builtin

- `asset_len("path/in/package.txt")` folds to the named asset's byte size at
  compile time. With two or more arguments it folds to the **maximum** of the
  named assets' sizes (the multi-locale case).
- Valid exactly where capacity helpers are valid today: a top-level `const`
  initializer, a local `buf` size, a struct buffer-field size (KOTO-0235),
  and as an argument to a compile-time capacity helper.
- **Path-identity guarantee (the load-bearing rule):** `asset_len` and
  `asset_load` share one namespace — the package asset paths declared as
  `output` in the app manifest's `assets` block, which is exactly the set the
  runtime host checks (`asset_paths` gating in the sim host; the same
  manifest drives the device package). The same string literal means the
  same asset in both positions; resolution is position-independent (any
  source file in the program, including SDK includes, sees the same
  namespace), because that is how `asset_load` behaves at runtime. It is
  deliberately **not** include-style source-file-relative resolution — the
  proving targets reference `locales/...` from `src/main.koto`, where a
  file-relative rule would name a nonexistent sibling.
- A path not declared in the manifest is a compile diagnostic naming the
  path. Corollary of the identity guarantee: any literal `asset_len`
  accepts is a valid `asset_load` argument whose runtime permission check
  cannot fail, and the folded size is the size of the bytes that ship in
  the package.
- This is the KOTO-0232 pattern — a narrowly defined compiler-backed
  facility, not general const evaluation. There is no runtime form.

### SDK storage capacity helpers

- `ui_text_resource_capacity(line_capacity, payload_capacity)` folds to
  `line_capacity * 4 + payload_capacity`, with `parse`'s own bounds
  (`line_capacity` in `1..=16383`, total `<= 65535`) as compile diagnostics.
- `ui_list_rows_capacity(row_capacity, label_capacity)` folds to
  `row_capacity * 12 + label_capacity`, with `begin`'s bounds
  (`row_capacity` in `1..=UI_MAX_LIST_ROWS`, total `<= 65535`) as compile
  diagnostics.
- Both join `ui_mount_capacity`/`ui_update_capacity` in the shared
  `capacity_helper_call` path so `const`, `buf`, and buffer-field sites get
  identical diagnostics, and both get checked SDK runtime implementations for
  ordinary-expression use, mirroring KOTO-0232.
- Since `parse` strips delimiters, `payload_len <= raw_len` always holds, so
  `ui_text_resource_capacity(lines, asset_len(...))` is a safe derived
  sizing that needs no separate payload budget.

### Deliberate V1 restrictions

- `asset_len` covers manifest entries packaged **verbatim** (copy-through
  `assets` entries, where the mapped `source` file's bytes are the packaged
  bytes). Pipeline-transformed entries (PNG→`.kim`, KMML→KAQ1) have output
  sizes the compiler cannot see from the source file; naming one is a
  focused compile diagnostic. Extending `build_apps` to pass a generated
  output-size table is a possible follow-up, not part of this issue.
- No glob form; each path is named explicitly.
- String-literal paths only — no consts, no concatenation.

## Implementation Scope

- Compiler: `asset_len` in the const/buf/field-size grammar; the two new
  helpers in `capacity_helper_call`; diagnostics and tests (undeclared /
  transformed / unreadable asset, out-of-bounds helper arguments,
  multi-argument max, helper-in-helper composition).
- Asset-namespace plumbing: the compiler resolves `asset_len` against the
  manifest `assets` block, not the filesystem directly — discover the
  nearest enclosing `app.json` upward from the root source file, map each
  declared `output` path to its `source` file, and stat the source bytes.
  Tests and other drivers inject the table through a resolver alongside
  `IncludeResolver` (an on-disk `app.json` is not required in unit tests);
  the LSP gets the discovery for free because it compiles real on-disk
  paths.
- SDK: checked runtime implementations of the two helpers in
  `sdk/koto_ui/abi.koto`; docs in `KOTO_SDK.md`.
- Docs: `KOTO_APP_LANGUAGE.md` (asset_len contract, resolution rule, V1
  restrictions), `KOTO_SDK.md` cross-references.
- Tooling: LSP diagnostics flow through `compile_source` unchanged; verify
  hover/inlay paths tolerate the new folded forms. `dev_app --watch`
  (KOTO-0191) must also watch referenced assets so size changes re-fold.
- Repo hygiene: pin `eol=lf` (or `-text`) for text assets in
  `.gitattributes` so committed KBC bytes do not depend on checkout line
  endings once asset sizes fold into bytecode.
- Adoption: Gallery (`GALLERY_RESOURCE_RAW_BYTES`/`GALLERY_RESOURCE_BYTES`/
  `rows_blob`) and File Note (`NoteStorage.raw`, `resource`) as proving
  targets, at unchanged budget gates and golden frames.

## Acceptance Criteria

- [x] `asset_len` folds single and multi-argument (max) forms in `const`,
  `buf`, and buffer-field positions; undeclared, transformed, and unreadable
  asset paths have focused diagnostics.
- [x] Path identity with `asset_load` is proven by construction and by test:
  `asset_len` accepts exactly the manifest-declared `output` paths (the
  runtime `asset_paths` namespace), resolution is independent of which
  source file (root or include) names the path, and a fixture shows the
  same string literal folding at compile time and loading successfully at
  runtime with `loaded <= folded` (equality for the largest locale).
- [x] `ui_text_resource_capacity` / `ui_list_rows_capacity` fold with
  KOTO-0232-class boundary diagnostics at all three declaration sites and
  have checked SDK runtime forms.
- [x] Editing a locale asset's size changes the folded capacities on the next
  build (proven by a compiler test with an overlay/loader-provided asset).
- [x] Gallery and File Note declare their locale/text/rows storage via
  `asset_len` + helpers with zero hand-summed byte constants, passing their
  simulator suites with unchanged golden frames and budget gates (small heap
  deltas from exact sizing are expected and re-baselined explicitly).
- [x] `dev_app --watch` re-folds when a referenced asset changes.
- [x] Compiler, LSP, formatting, affected-crate Clippy, App build/package
  synchronization, runtime budget, and project consistency checks pass.

## Notes

Landed 2026-07-17. Implementation shape:

- Compiler: `AssetResolver` trait + production `ManifestAssets`
  (nearest-`app.json` discovery, lazy so hermetic compilations never touch
  the filesystem) in `tools/koto-compiler/src/assets.rs`, injected into the
  parser alongside include loading (`compile_with_resolvers` /
  `compile_source_with_assets` for tests). `asset_len` folds in the
  const/buf/field grammar; expression position is a focused "compile-time
  only" diagnostic; helper arguments compose (`asset_len` and
  helper-in-helper).
- The two storage helpers share `capacity_helper_call` with the KOTO-0232
  packet constructors (strides 4/12, total <= 65535, lines <= 16383,
  rows <= UI_MAX_LIST_ROWS); checked runtime forms live in
  `sdk/koto_ui/abi.koto`.
- Path identity fixture: `koto-sim` runtime test compiles against a real
  on-disk `app.json` and asserts `loaded == folded` through
  `SimRuntimeHost::asset_load`. LSP hover/inlay verified via a real-manifest
  tempdir test.
- `--watch` already rescans the whole app folder (locales included) and
  reruns `build_apps.py --app`, so asset edits re-fold without extra wiring.
- Adoption: Gallery folded `GALLERY_RESOURCE_RAW_BYTES` 384→343 (largest
  locale, qps-ploc), `GALLERY_RESOURCE_BYTES` 416→431 (22×4 + 343),
  `rows_blob` 68→66 (`ui_list_rows_capacity(3, 30)`, also the mount List
  slot); File Note folded `NoteStorage.raw` 384→197 and `resource` 384→249
  (13×4 + 197). Golden frames and budget thresholds unchanged; measured
  heap peaks only shrank. Device smoke after the next SD refresh is the
  remaining hardware step.
- SDK line-count growth shifts include-expanded KDBG line attribution for
  KotoUI apps, so only the two adopted apps' KBC changed; every other
  committed KBC/KPA stayed byte-identical (`build_apps.py --check`).

Filed from the 2026-07-17 Gallery capacity audit (KOTO-0235 follow-up
discussion).

A runtime `asset_size(path)` host call was considered and rejected: `buf` and
buffer-field capacities are compile-time constants with no runtime allocator,
so a runtime size cannot size storage — it would only pre-check what
`asset_load` already fails deterministically (`NO_MEMORY`), at the cost of a
full-stack ABI addition (koto-vm ID/dispatch/verifier, kbc-asm, compiler
intrinsic, sim host, koto-pico firmware + device verification, ABI minor
bump). Revisit only if a streaming consumer appears that cannot know sizes
statically; today's `asset_load_range` callers derive offsets from their
formats' own structure.

A build-time `max_bytes` assertion in the `apps.json` assets block was the
lighter alternative; folding was preferred because it makes undersized
storage unrepresentable rather than merely detected, and it composes with
KOTO-0235 buffer fields. The assertion idea remains useful for
pipeline-transformed assets, which `asset_len` V1 deliberately excludes.
