# KOTO-0117: Pico Firmware Main Loop

- Status: done
- Type: feature
- Priority: P0
- Requirements: FR-SIM-4, NFR-PORT-3, NFR-PORT-4, NFR-PERF-6

## Goal

Replace the isolated peripheral-probe shape with the first product-firmware
slice: run the portable KotoShell state machine on PicoCalc, feed it normalized
keyboard input, and paint bounded LCD updates from a frame loop.

## Acceptance Criteria

- [x] A dedicated `koto_firmware` binary initializes the validated LCD and
  keyboard paths.
- [x] PicoCalc key events are converted into `koto_core::InputState`, including
  held, pressed, and released transitions.
- [x] The firmware updates a real `koto_core::ShellState` on a bounded 16 ms
  frame cadence.
- [x] Selection changes are visible through bounded LCD rectangles without a
  full-screen framebuffer.
- [x] The RP2040 target build and dependency-free project harness pass.

## Notes

This is the first post-bring-up firmware issue. SD package discovery, font/text
rasterization, app launch, audio, and power integration remain separate
follow-ups. The initial package entries may be compiled-in fixtures so this
issue can validate the core/HAL boundary independently of storage.

## Resolution

Added the `koto_firmware` RP2040 binary. It initializes the validated
DMA-backed LCD and STM32 keyboard bridge, drains at most four FIFO events per
16 ms frame, converts held keys into portable `InputState` edges, and updates a
real `ShellState` containing three compiled-in package fixtures.

The first visual shell slice uses bounded card rectangles. Arrow keys move the
portable shell selection and Z/Enter confirms it with visible feedback. No
full-screen framebuffer is allocated.

Verified:

- `cargo build -p koto-pico --bin koto_firmware --target thumbv6m-none-eabi --offline`
- `python harness/check_project.py`

The full `harness/check_all.py` run reaches the existing runtime-budget gate
but currently fails on KotoBlocks (`heap_peak` 4191 over its 4096 threshold).
That app-budget drift is outside this firmware issue.
