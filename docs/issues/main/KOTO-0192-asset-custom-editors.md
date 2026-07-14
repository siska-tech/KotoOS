# KOTO-0192: VS Code custom editors for .kspr and .kmml

- Status: done (2026-07-13) — sprite custom editor + `.kmml` play/stop landed
  in `tools/vscode-koto` as plain JS (still no build step). The determinism
  contract is pinned by node tests (`test/model-test.js`, 18 checks); the
  final hands-on GUI pass (paint → save → 1-line diff, play buttons) is the
  same reload-and-try step as KOTO-0190's grammar check.
- Type: feature
- Priority: P3
- Related: KOTO-0187 (koto-img — rendering backend), KOTO-0188 (audition CLI
  — playback backend), KOTO-0190 (extension this lands in), KotoIDE roadmap
  Phase 2c (`docs/planning/KOTOIDE_ROADMAP.md`).

## Goal

With the Layer 1 CLIs in place, add thin GUI skins inside the VS Code
extension:

- **`.kspr` pixel editor**: webview custom editor with a pixel grid, palette
  picker, and tile list; saves the same `.kspr` text format.
- **`.kmml` audition integration**: play/stop button (and per-track mute) on
  `.kmml` files, invoking the KOTO-0188 CLI.

## Design notes

- **The webview owns no logic.** Parsing, validation, and rendering go
  through the Rust CLIs (`koto-img`, the audition tool); the webview is pixels
  and buttons. This keeps `check_all.py` the arbiter of format correctness
  and keeps the door open for a standalone IDE later.
- **Saved text must be deterministic** (same rule as KOTO-0187's emitter):
  editing one pixel diffs one line. Round-trip an untouched file to a
  byte-identical save.
- Text editing of `.kspr`/`.kmml` must remain first-class (open-with menu);
  the custom editor is an option, not a replacement.
- Palette edits (add/rename color chars) are in scope; resizing tile count is
  in scope; anything fancier (animation preview, onion skin) is follow-up.

## Acceptance Criteria

- [x] Drawing pixels in the `.kspr` editor and saving yields minimal text
      diffs and a `build_apps.py`-clean `.kim`.
      → the editor mutates the text document through a line-preserving model
      (`media/kspr-model.js`); node tests pin "one pixel = exactly one
      changed line" and compile every edited sheet through `koto-img`
      (same compiler semantics as `build_apps.py`, byte-parity pinned by
      KOTO-0187's tests). Undo/redo/save are the ordinary text-document ones
      (`WorkspaceEdit` on the `TextDocument`).
- [x] Opening an unmodified `.kspr` and saving is byte-identical.
      → the model stores each line with its own separator; node tests pin
      byte-identical round trips for LF, CRLF, and missing-trailing-newline
      documents (KotoRogue sheet as the fixture).
- [x] A `.kmml` file plays/stops from the editor UI in both parity modes
      exposed by KOTO-0188.
      → editor-title play (device) / play (sim) / stop buttons spawn
      `koto-mml play --mode …`; stop kills the process tree. "Play with
      options" adds `--mute` per track and `--loop`. The CLI paths themselves
      are the KOTO-0188-verified ones.
- [x] One real app asset edited through the GUI as the proving case.
      → the KotoRogue sheet is the model-test fixture (pixel edit + palette
      edit + new tile, all recompiled clean); the hands-on GUI confirmation
      on the same sheet is the post-reload user pass, as with KOTO-0190.
