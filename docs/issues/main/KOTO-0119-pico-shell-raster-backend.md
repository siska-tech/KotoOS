# KOTO-0119: Pico Shell Raster Backend

- Status: in-progress
- Type: feature
- Priority: P0
- Requirements: FR-SHELL-2, FR-SHELL-3, FR-SHELL-4, FR-SDK-1, NFR-PERF-1, NFR-PORT-3

## Goal

Render the same portable KotoShell visual implementation used by KotoSim on
the PicoCalc LCD, replacing the temporary device-only card UI.

## Acceptance Criteria

- [x] `Canvas` can rasterize a logical 320×320 surface into bounded horizontal
  windows without allocating a full-screen framebuffer.
- [x] The Pico LCD backend accepts RGB565 windows and converts them to the
  panel's validated RGB666 wire format.
- [x] Product firmware calls `ShellState::paint` with the same M+ bitmap font
  used by KotoSim.
- [x] Header, launcher grid, selection, details pane, status strip, command bar,
  page navigation, and manifest names are rendered by portable shell code.
- [x] The RP2040 build, Clippy, core tests, and project harness pass, and a UF2
  is produced.

## Notes

This issue establishes visual parity and the bounded raster transport. Further
hardware timing measurements may split dirty-region scheduling into a focused
performance issue.

## Resolution

`Canvas::new_viewport` now keeps logical 320×320 coordinates while clipping
and translating output into a compact caller-owned window. The Pico firmware
uses 16-line RGB565 strips (10 KiB) and reruns the portable shell painter for
each strip, avoiding the forbidden 204,800-byte framebuffer.

`PicoCalcLcd::write_rgb565_rect` converts the portable little-endian RGB565
output into the ILI9488 RGB666 stream during DMA-backed row transfers. Product
firmware embeds the existing `mplus10.kfont`, parses package description and
category metadata, and calls the same `ShellState::paint` path as KotoSim.

Verified:

- `cargo test -p koto-core --offline`
- `cargo build -p koto-pico --bin koto_firmware --target thumbv6m-none-eabi --offline`
- `cargo clippy -p koto-pico --bin koto_firmware --target thumbv6m-none-eabi --offline -- -D warnings`
- `python harness/check_project.py`
- `elf2uf2-rs target/thumbv6m-none-eabi/debug/koto_firmware target/thumbv6m-none-eabi/debug/koto_firmware.uf2`

Physical validation on 2026-06-21 exposed an unacceptable device-only
performance failure: the 16-line implementation reran the full shell painter
20 times per frame and repeated that work for animation frames. The LCD showed
in-progress bands as visual noise and input appeared stalled. The issue was
reopened for bounded-pass tuning.

The first diagnostic attempt incorrectly used the RP2040 module's native USB
CDC, while the PicoCalc mainboard Type-C connector exposes UART0 TX on GP0.
Diagnostics now use that validated path at 115200 8N1 and emit each startup
phase synchronously before LCD initialization, SD scanning, and raster output.
The bounded raster strip is 32 lines (20 KiB), and animation frames do not
schedule repeated full redraws.
