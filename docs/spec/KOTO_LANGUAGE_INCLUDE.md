# KOTO-0183: Koto Source-File Splitting — Design Note

- Status: implemented (this note is the KOTO-0183 acceptance design note)
- Decision: **flat, textual include** resolved by the compiler front-end before
  lexing. Quoted paths are source-relative; angle-bracket `sdk/` paths name
  standard SDK libraries. No module system, namespaces, or runtime linking.

## Mechanism chosen: textual include (not modules)

A directive line

```koto
include "dungeon.koto";
```

is replaced, before lexing, by the contents of the named file (path relative to
the *including* file's directory). The expanded text is then compiled exactly
as a single-file program. A side table (`SourceMap`) records, per expanded
line, the originating file and line, and every diagnostic — lexer, parser, and
codegen — is remapped through it, so errors report the real `file:line:col`
across include boundaries.

SDK standard libraries use a workspace-rooted form instead:

```koto
include <sdk/koto_ui.koto>;
```

This form is independent of the App source directory, so moving an App does
not require rewriting its SDK imports.

### Why not modules

- The VM has **no `call`/`ret` and a single shared 16-slot local file**; the
  compiler fully inlines every call into `main` (see `KOTO_APP_LANGUAGE.md`).
  There is no linkage unit for a module to map onto — a module system would
  only buy namespacing, paid for with name mangling and a resolution pass, in
  a language whose largest shipped program is ~1,000 lines.
- Textual inclusion has a property nothing else offers at this scale: a split
  is **provably free**. A faithful split (extracted lines land in the include
  file verbatim, the directive line takes their place) reproduces the original
  source byte-for-byte after expansion, so the emitted assembly and the `.kbc`
  bytecode — including the KDBG `.loc` line/col entries — are byte-identical.
  The proving case (KotoRogue, split into `main.koto` + `dungeon.koto` +
  `render.koto`) is verified byte-identical by
  `python harness/build_apps.py --check`, and
  `include_compiles_identically_to_the_unsplit_source` pins the property in
  the compiler's own tests.
- Duplicate top-level definitions already fail in codegen ("function `x` is
  already defined"); with the SourceMap those collisions now point at the
  right file and line, which is the flat-include failure mode that matters.

## Semantics

- **Directive form:** a whole line whose trimmed content is either
  `include "relative/path.koto";` or `include <sdk/library.koto>;`, optionally
  followed by a `//` comment. Recognized only when `include` is followed by
  whitespace and a `"` or `<`; an identifier like `include_x` or a call
  `include(x)` is untouched. Malformed directives are compile errors.
- **Quoted paths** are relative to the including file, use `/` separators, and
  may not be absolute. `..` is allowed (see the code-size story for why sharing
  across apps is legal but discouraged).
- **Angle-bracket paths** resolve from the workspace root and are reserved for
  `sdk/`. Every component must be a normal path component: `.`, `..`, absolute
  paths, and backslashes are rejected. The canonical KotoUI import is
  `include <sdk/koto_ui.koto>;`.
- **Nesting** is allowed (an included file may include others), capped at
  depth 16.
- **Each file may be included at most once** per program (the root file
  counts). A second include of the same file — including any cycle — is an
  error at the second include site. This keeps the semantics predictable
  (pure textual splice, no include-guard magic) while turning the
  diamond-include foot-gun into an immediate, attributed error instead of a
  cascade of duplicate-definition errors.

## The code-size story (design constraint #1)

`include` deliberately changes **nothing** about code generation. The compiler
still inlines every call into `main`; a helper defined in `util.koto` costs
exactly what the same helper costs when pasted into `main.koto` — its body
multiplied by its call-site count (see memory: `koto-compiler full inlining`).
What `include` changes is *visibility*: bloat authored in another file is
easier to stop seeing.

Mitigations, in order of force:

1. **Per-app gates are unchanged and remain the backstop.** The committed
   bytecode budget (`harness/check_budgets.py`, code-window/tile analyses) is
   computed from the expanded program, so include-hidden growth still fails
   the same gates it fails today.
2. **Slot/footprint reporting is file-attributed.** `--slot-map` `fn` lines
   now carry `src=<file>:<line>`, so per-function inline footprints point into
   the file that defines them, not just "somewhere in the app".
3. **KOTO-0142 (inline-expansion report, loop-straddle warning) is the real
   answer** and stays a separate issue. It operates on the expanded program
   and can use the same SourceMap for attribution; nothing in this design
   blocks or duplicates it.
4. `KOTO_APP_LANGUAGE.md` documents the multiplication rule next to the
   `include` syntax, where an author deciding to split will actually read it.

Cross-app sharing (`include "../../shared/util.koto"`) is possible because the
mechanism is textual and the compiler cannot know the app root. It is
*discouraged* in the docs: the shared body is re-inlined into **every** app at
**every** call site, and each app pays it against its own code-window budget.

## The slot-budget story (design constraint #2)

The 45-user-slot budget is global and unchanged; includes cannot alter it,
only obscure it. Attribution is handled the same way as all other
diagnostics:

- Codegen slot errors ("function uses too many locals", scratch overflow,
  cap violations) carry the AST node's line, which the SourceMap remaps to
  the defining file — so slot-pressure errors name the file that owns the
  pressure, per the issue requirement.
- The static `--slot-map` report (`harness/check_budgets.py`) attributes each
  function's own footprint via `src=<file>:<line>` (mitigation 2 above).

## What did NOT need to change

- **`apps.json` / `build_apps.py`:** an app's `source` stays the root file;
  the compiler resolves includes itself. `--check` recompiles from source, so
  edits to included files fail the drift gate exactly like edits to
  `main.koto`. (Includes are inputs the same way the root is; no manifest of
  include files is needed because the compile reads them fresh every run.)
- **Bytecode format / verifier / VM:** nothing. `.debug_file` still names the
  root source, and KDBG `.loc` entries still carry include-expanded lines —
  runtime trap attribution is exactly as informative as before a faithful
  split (the expanded line *is* the pre-split line). If per-file trap
  attribution is ever wanted, `kbc-asm` already supports multiple
  `.debug_file` entries and codegen could emit remapped `.loc`s through the
  same `SourceMap` — deliberately not done here because it would change
  committed bytecode and forfeit the byte-identical proving case.
- **`maps` assets:** `build_apps.py` validates `.map` files under `maps.dir` and
  packages them as read-only data. It does not rewrite the entry source or any
  included file.

## Implementation map

- `tools/koto-compiler/src/preprocess.rs` — directive scan, path resolution,
  duplicate/cycle/depth errors, `SourceMap` (expanded line → file, line).
  File IO is injected (`IncludeLoader`), so unit tests run hermetically and
  callers passing sources with no `include` never touch the filesystem
  (keeps `koto_compiler::compile(name, source)` valid for in-memory tests).
- `tools/koto-compiler/src/lib.rs` — all public entry points expand first,
  then remap every `Diag` through the `SourceMap` instead of stamping the
  root filename on it.
- `tools/koto-app-scaffold` — the starter app is generated as
  `main.koto` + `helpers.koto` with an `include`, so every scaffolded app
  exercises the multi-file path end-to-end.
- Proving case: `apps/kotorogue/src/main.koto` split into `main.koto`
  (state, input, main loop) + `dungeon.koto` (generation + monster AI) +
  `render.koto` (all drawing), byte-identical committed bytecode.
