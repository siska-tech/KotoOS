# KOTO-0129: Pico Device Game2D `draw_pixels_rgb565` Support

- Status: in-progress
- Type: feature
- Priority: P1
- Requirements: FR-SDK-1, NFR-DRAW-1, NFR-DRAW-2

## Goal

Make tile/sprite-rendered games (KotoBlocks, and later kotorogue/kotosnake/etc.)
actually render their playfield on the PicoCalc, by implementing the device side
of the PicoMings tile/sprite model — `draw_pixels_rgb565` — and giving the app
present path enough draw-command budget for a full board.

## Context

Follow-up to KOTO-0127, which validated that large-app bytecode streams from PSRAM
and runs on hardware. KOTO-0127 hardware bring-up of `dev.koto.games.koto-blocks`
showed the VM running correctly from PSRAM — the UART heartbeat advances in real
time and the program counter progresses from the title-screen yield to the
gameplay yield with healthy fuel and no trap:

```
phase=156 app-staged backing=psram code_size=65528
phase=152 app-started
phase=154 app-heartbeat frame=180 pc=6064  fuel=5218    # title screen loop
phase=154 app-heartbeat frame=300 pc=16378 fuel=26665   # gameplay loop
```

But the game visually "stops while composing the screen": the title renders (it
uses `draw_rect` / `draw_text_color`) while the board never appears. Root cause:
KotoBlocks draws its board and pieces with `draw_pixels` (the tile/sprite model
from KOTO-0094 / KOTO-0097 — see `apps/koto_blocks/src/main.koto`, `blit_piece`),
but the device `DeviceHost` in `src/koto-pico/src/bin/koto_firmware.rs` does **not**
implement `draw_pixels_rgb565`; it falls back to the `VmHost` default that returns
`HostErrorCode::UNSUPPORTED`. The app swallows that status, so the VM keeps running
but nothing blits. Two device-side gaps:

- `DeviceHost` does not implement `draw_pixels_rgb565` (KotoSim's host does).
- `DeviceRuntimeHost` has no pixel draw-command variant (only `Rect` / `Text`),
  and `MAX_APP_DRAW_COMMANDS` is `16` — far below a full board's blit count.

This is a device rendering-capability gap, distinct from KOTO-0127's bytecode/heap
budget. It surfaced only once large games could launch.

## Acceptance Criteria

- [x] `DeviceHost::draw_pixels_rgb565` is implemented (decoding the little-endian
  RGB565 block from the app heap) instead of returning `UNSUPPORTED`, mirroring
  KotoSim's host so the portable app command model is unchanged. The block is
  referenced by its heap offset (recovered from the resolved slice pointer)
  rather than copied, so a full board's tiles stay within the command list.
- [x] `DeviceRuntimeHost` carries pixel draw commands through to the present path
  within the existing bounded strip budget (`RASTER_STRIP_BYTES` /
  `RGB666_STRIP_BYTES`); no full-screen SRAM framebuffer is introduced. The blit
  source bytes stay resident in the app heap and are re-read at compose time.
- [x] The per-frame draw-command budget (`MAX_APP_DRAW_COMMANDS`) is sized so a
  full KotoBlocks board (playfield cells + piece + side panel) renders (16 → 384),
  with the sizing justified inline per the project's VM-budget-sizing guidance.
- [~] A minimal 16x16 RGB565 tile diagnostic blits correctly on hardware (a known
  pattern at a known position), to isolate the blit path from game logic.
  (`present_pixel_diagnostic` added — four quadrant colours at (152, 152), logged
  as `phase=157`; pending on-device confirmation.)
- [~] KotoBlocks' board and pieces render correctly on physical hardware and the
  game is visibly playable (matching KotoSim). (Device blit path implemented;
  pending on-device confirmation.)
- [x] KOTO-0127's PSRAM `CodeSource` / bytecode-streaming path is unchanged.

## Notes

The portable command model and KotoSim host are the reference for decoding and
semantics (`draw_pixels_rgb565` takes `(x, y, w, h, ptr, len)` with `len == w * h
* 2` little-endian RGB565 bytes from the app heap). Compositing should reuse the
KOTO-0128 retained-GRAM / off-screen-strip discipline rather than writing blits
straight to GRAM. Any added scratch and the raised draw-command bound must stay
within the resident OS-core budget.

On-device confirmation follows the step-by-step checklist in
[KOTO-0129-HARDWARE-VALIDATION.md](KOTO-0129-pico-device-draw-pixels.md) (flash
command, expected UART phases, the `phase=157` pixel diagnostic, KotoBlocks
launch/visual/input checks, and the failure logs to capture).

KotoMemo's separate launch failure (out of memory; it does not use `draw_pixels`)
is **not** part of this issue — collect its final UART phase/error and track it on
its own.
