# KOTO-0032: KotoSim Live Interactive Window

- Status: done
- Type: feature
- Priority: P1
- Requirements: FR-SIM-1, FR-SIM-2

## Goal

Open a live 320×320 window for KotoSim that presents the framebuffer in real time
and feeds PC keyboard input into the shell, replacing the offline BMP dump from
KOTO-0031 for interactive debugging.

## Acceptance Criteria

- [x] A window displays the current frame and refreshes on input.
- [x] PC keyboard events map to the PicoCalc button/`InputState` model
      (arrows = move, Enter/Space = confirm, Backspace = cancel, Esc = quit).
- [x] The windowing backend is an optional feature so headless `cargo test` and
      CI stay dependency-free.

## Notes

Depends on KOTO-0031 (framebuffer + rasterizer).

Implemented behind the `window` cargo feature (optional `minifb` dependency); the
loop lives in [`koto_sim::window`](../../../src/koto-sim/src/window.rs) and reuses
`ShellState::paint` + `framebuffer_to_argb`. Run with:

```powershell
cargo run -p koto-sim --features window -- --window
```

Without the feature, `--window` prints how to rebuild, so the default build has no
GUI/native dependencies.

FR-SIM-1 names SDL2; this uses pure-Rust `minifb` to avoid a native SDL2
toolchain on Windows. The framebuffer/paint layer is backend-agnostic, so SDL2
can be re-targeted later if required; FR-SIM-1 should be annotated accordingly.
