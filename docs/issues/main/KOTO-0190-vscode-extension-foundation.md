# KOTO-0190: VS Code extension foundation (grammars, tasks, problem matcher)

- Status: done (2026-07-13) — `tools/vscode-koto` landed as a **declarative
  only** extension (grammars + problem matcher + language configs; zero
  TypeScript, zero npm — install is a junction/symlink into
  `~/.vscode/extensions`). Tasks live in the committed `.vscode/tasks.json`
  with app resolution in `harness/dev_app.py`; one scope trim vs. the filing:
  "run current app" is a headless `--app --inspect` report because the sim
  has no windowed direct-app launch — that lands with KOTO-0191's watch
  loop, and a plain "sim window" task covers window mode meanwhile.
- Type: feature
- Priority: P3
- Related: KotoIDE roadmap Phase 2a (`docs/planning/KOTOIDE_ROADMAP.md`),
  KOTO-0183 (file:line:col diagnostics the problem matcher consumes),
  KOTO-0192 (custom editors build on this), KOTO-0194 (LSP client lands here
  later).

## Goal

Editor integration is currently zero. Ship a workspace-local VS Code
extension that makes the existing CLI loop one keystroke, without any new
Rust code:

- **TextMate grammars** for `.koto`, `.kmml`, `.kspr` (keywords,
  comments, palette/tile headers, MML commands).
- **Tasks**: build apps (`build_apps.py`), run current app in the sim
  (window mode), capture a screenshot (`--image`), run `check_all.py`.
- **Problem matcher** mapping `koto-compiler` `file:line:col` diagnostics to
  in-editor squiggles from the build task output.

## Design notes

- Keep the TypeScript layer thin and dumb (principle 3 of the roadmap): no
  parsing beyond regex grammars; all future intelligence arrives via the LSP
  (KOTO-0194) and CLI tools.
- Home: `tools/vscode-koto/`, installed from source (`npm` + dev install or a
  committed build script); no marketplace publication needed.
- Deriving "current app" from the edited file path (`apps/<dir>/src/…` →
  `apps.json` entry) keeps the run task zero-config.

## Acceptance Criteria

- [x] The four grammars highlight their fixtures sensibly (spot-checked, no
      golden needed).
      → patterns written against the real token sets (compiler `lexer.rs`
      keywords, `KOTOMML_FORMAT.md` commands, `build_apps.py` `.kspr` parse,
      Native KotoAudio directives); all grammar JSON validated and every regex
      compile-checked. Final visual pass is in the editor after reload.
- [x] Build/run/screenshot/check tasks work from a fresh clone following a
      documented install step.
      → `.vscode/tasks.json` (committed) + `harness/dev_app.py`; smoke-run:
      `run apps/memo/src/main.koto` resolved `dev.koto.memo` and printed the
      inspect report, `screenshot apps/kotorogue/src/dungeon.koto` (an
      *included* file) resolved KotoRogue and wrote
      `target/koto-dev/kotorogue.bmp` (title screen verified). Install step
      documented in `tools/vscode-koto/README.md` and executed (junction).
- [x] A compile error in a `.koto` file (including inside an `include`d file)
      appears as an in-editor diagnostic at the right file and line.
      → the `$koto` problem matcher regex was verified against a real
      compiler error line (`file.koto:2:11: unexpected token…`; group
      extraction pinned by a scripted check). Include-file paths come from
      the KOTO-0183 SourceMap through the same `file:line:col` format.
- [x] `docs/guides/APP_DEV_LOOP.md` gains an "editor setup" section.
      → new section 6, with the task list and the KOTO-0191 pointer.
