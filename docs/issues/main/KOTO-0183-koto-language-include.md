# KOTO-0183: Koto language source-file splitting (include/module support)

- Status: done (2026-07-12) — flat textual `include "file.koto";` landed; see
  the design note `docs/KOTO_LANGUAGE_INCLUDE.md` and the language section in
  `docs/KOTO_APP_LANGUAGE.md` ("Source-file splitting").
- Type: feature
- Priority: P2
- Related: KOTO-0046 (compiler MVP), KOTO-0092/KOTO-0104 (local slot reuse —
  the constraint system any include mechanism must respect), KOTO-0127 (large
  bytecode budget — inlining across files multiplies code size).

## Goal

A Koto app today is a single `src/main.koto`; there is no `include` or module
mechanism. Real apps (KotoRogue, KotoShogi) are thousands of lines in one
file. Add a way to split sources.

## Design constraints (what makes this non-trivial)

- The compiler **fully inlines every call into main** — helpers imported from
  a shared file multiply bytecode per call site, and the 2-tile code-window
  budget (KOTO-0173) punishes code-span growth. An include feature that makes
  it *easier* to write code the device can't afford is a net loss; the design
  note must address this (e.g. compile-time warnings from KOTO-0142's
  diagnostics).
- Local-slot budget (45 user slots, helpers stack on main's at the call site)
  is global; includes don't change it but hide it — error messages must
  attribute slot pressure to the right file/line.
- Keep it a **textual/compile-time** mechanism (no runtime linking): flat
  `include "util.koto"` with duplicate-definition errors is likely enough for
  the current app scale.

## Acceptance Criteria

- [x] Design note choosing the mechanism (include vs modules) with the
      code-size and slot-budget story written down.
      → `docs/KOTO_LANGUAGE_INCLUDE.md`: textual include, no modules; codegen
      unchanged (inlining costs are identical to the unsplit file), per-app
      budget gates stay the backstop, `--slot-map` gains `src=file:line`.
- [x] Compiler + `apps.json` build pipeline support multi-file apps; scaffold
      tool updated.
      → `koto-compiler` expands includes itself (`preprocess.rs`), so
      `apps.json`/`build_apps.py` needed no changes (`source` stays the root
      file; `--check` recompiles and catches include drift). The scaffold
      starter is now `main.koto` + `helpers.koto` via `include`, so every new
      app exercises the multi-file path.
- [x] One shipped app actually split as the proving case, byte-identical (or
      justified-diff) bytecode.
      → KotoRogue: `main.koto` (1,047 lines) → `main.koto` (555) +
      `dungeon.koto` (252: reveal/carve/gen_level/monsters_act) +
      `render.koto` (243: draw_msg…render_over). Byte-identical: committed
      `kotorogue.kbc` untouched, `build_apps.py --check` passes, and the
      compiler test `include_compiles_identically_to_the_unsplit_source` pins
      the property.
- [x] Compile errors report file:line across include boundaries.
      → every lexer/parser/codegen `Diag` is remapped through the `SourceMap`
      (expanded line → file, line); covered by compiler tests
      (`errors_in_included_files_report_their_own_file_and_line`,
      `duplicate_definitions_across_files_attribute_the_second_site`, ...).
