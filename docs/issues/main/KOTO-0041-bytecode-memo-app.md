# KOTO-0041: Bytecode Memo App

- Status: done
- Type: feature
- Priority: P0
- Requirements: FR-SIM-3, FR-SIM-5, FR-IME-1, FR-IME-2, FR-IME-3, FR-FS-2, NFR-DEV-4
- Prerequisites: KOTO-0042, KOTO-0043, KOTO-0044, KOTO-0045, KOTO-0046, KOTO-0047, KOTO-0048, KOTO-0049

## Goal

Implement the real `dev.koto.memo` application as a `kotoruntime-bytecode`
program rather than a native KotoSim-only shortcut. The app should run through
the same KPA manifest, bytecode VM, input host calls, text/file host calls, and
sandboxed save path that a distributed app will use.

## Acceptance Criteria

- [x] Launching `dev.koto.memo` in KotoSim window mode enters an interactive
      bytecode-driven memo screen, not a native simulator-only memo view.
- [x] The bytecode app can load `memo.txt`, edit text, move the cursor, delete
      text, save, exit, and relaunch with saved content intact.
- [x] The app exposes IME state through bytecode-visible app state or host calls
      so romaji/kana composition, Sticky Shift, SKK conversion, and candidate
      commit can be exercised without bypassing the VM.
- [x] `sdcard_mock/bytecode/memo.kbc` is generated from readable high-level app
      source, and the source is checked into the repository.
- [x] KotoSim window mode renders the bytecode app's real draw output and routes
      keyboard input into the running VM instead of replacing it with native
      Rust app logic.
- [x] The scripted memo validation path covers the bytecode app end-to-end,
      including save/reload and sandbox containment.

## Notes

KOTO-0039 proves that a memo package fixture can launch and call text/file host
APIs. KOTO-0040 proves the portable memo editor and IME behavior in a
deterministic simulator validation path. This issue closes the remaining product
gap: the interactive memo app itself must be bytecode-driven.

Do not satisfy this issue by special-casing `dev.koto.memo` into a native
KotoSim app view. If the current runtime ABI lacks enough input, drawing, or
state primitives, add the smallest bytecode/runtime extension issues needed and
keep this issue blocked on those prerequisites rather than hiding the gap.

The runtime ABI did lack the needed primitives, so this issue is blocked on three
prerequisite extensions and proceeds only after they land:

- [KOTO-0042](KOTO-0042-runtime-input-ime-host-calls.md): typed-character input
  and IME/text-buffer host calls so the VM can drive composition and editing.
- [KOTO-0043](KOTO-0043-sim-interactive-bytecode-session.md): a per-frame KotoSim
  bytecode session that routes input into a live VM and paints its real draw output.
- [KOTO-0044](KOTO-0044-bytecode-assembler.md): a text bytecode assembly/IR
  target so generated `memo.kbc` is reproducible and inspectable.
- [KOTO-0045](KOTO-0045-high-level-app-language-spike.md): a selected minimal
  high-level language for real app authoring.
- [KOTO-0046](KOTO-0046-koto-language-compiler-mvp.md): a compiler from that
  high-level source to `KBC1`.
- [KOTO-0047](KOTO-0047-bytecode-sdk-prelude.md): named SDK calls for drawing,
  input, IME, file, and lifecycle behavior.
- [KOTO-0048](KOTO-0048-app-build-package-loop.md): a reproducible source to
  bytecode/package build loop.
- [KOTO-0049](KOTO-0049-sim-app-dev-experience.md): simulator commands,
  diagnostics, and scripted app scenarios suitable for app development.

The IME stays a host service driven through KOTO-0042 host calls; the VM owns
control flow, input routing, drawing, and the save/exit path, so composition and
conversion are exercised without bypassing the VM.

The memo app should not be authored primarily in hand-written assembly. Assembly
is useful as a low-level IR/debug layer, but the durable memo source should use
the high-level app language defined by the bytecode app development roadmap.

Completed: the app is authored in `apps/memo/src/main.koto` and compiled to
`sdcard_mock/bytecode/memo.kbc` through the build loop (`python
harness/build_apps.py`, drift-checked by `check_all.py`). It runs as a live
`BytecodeAppSession`: KotoSim window mode paints its real `draw_rect`/`draw_text`
output and routes typed characters and edit intents into the VM (the native
`AppView` is gone). `run_memo_validation` now drives the VM frame by frame —
romaji か, Sticky Shift + SKK conversion to 傘 (candidate captured before commit),
commit, cursor move, backspace, save, exit, relaunch, and reload — asserting the
saved file stays inside `data/dev.koto.memo/`. The whole loop is verified by
`cargo test`, the `--memo-validation` CLI, and `harness/check_all.py`.
