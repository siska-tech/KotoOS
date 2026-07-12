# KOTO-0043: KotoSim Interactive Bytecode Session

- Status: done
- Type: feature
- Priority: P0
- Requirements: FR-SIM-2, FR-SIM-3, FR-SIM-5, FR-RT-4, NFR-DEV-4

## Goal

Give KotoSim a reusable, per-frame bytecode app session that keeps a verified VM
alive across frames, routes the frame input snapshot into the running VM, and
paints the VM's real draw output. This replaces the current window-mode shortcut,
which runs the VM once to fuel exhaustion through `launch_package` and then paints
recorded text with native Rust drawing.

## Acceptance Criteria

- [x] A `BytecodeAppSession` owns the verified program, the VM, and a runtime host,
      and exposes a `step_frame(input)` call that runs one bounded frame and reports
      yielded/exited/fuel-exhausted state.
- [x] The session host implements the KOTO-0042 IME/text-buffer calls over a
      host-side IME, editor, and loaded SKK index, with the dictionary read from a
      sandboxed simulator path rather than hard-coded into the binary.
- [x] KotoSim window mode launches `dev.koto.memo` through the session, paints the
      VM's `draw_rect`/`draw_text` output each frame, and returns to the shell on
      app exit instead of substituting native app logic.
- [x] Window input maps typed characters and edit-intent keys (Enter, Backspace,
      arrows, Home/End, Shift, convert/commit/cancel) into the VM input snapshot.
- [x] Tests drive a scripted session that draws and exits without panicking, show
      the captured draw output, and confirm input intents reach the host.

## Notes

Prerequisite for KOTO-0041. The session is app-agnostic; the memo app is just its
first user. Depends on KOTO-0035 and KOTO-0042. Keep the per-frame fuel and VM
size limits from the existing `SIM_*` constants.

Implemented in `src/koto-sim/src/lib.rs` and `src/koto-sim/src/window.rs`:
`SimRuntimeHost` now owns a `MemoEditor`, `KotoMemoIme`, and an optional `SkkSession`
(dictionary loaded best-effort from the host-privileged `dict/skk_min.skk` path,
shipped at `sdcard_mock/dict/skk_min.skk`) and implements the KOTO-0042 trait
methods; `BytecodeAppSession::{launch, step_frame}` keeps the VM alive across
frames; window mode drives the live session, paints its per-frame `draw_rect`/
`draw_text` output, and returns to the shell when the app exits. Three session
tests cover the smoke draw/save path, romaji→kana committed through the VM, and a
Sticky Shift + SKK conversion candidate produced through the VM.
