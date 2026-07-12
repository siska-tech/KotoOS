# KOTO-0129 PicoCalc Hardware Validation Checklist

On-device validation for `draw_pixels_rgb565` and the raised draw-command budget.
Run on a physical PicoCalc (RP2040) with a UART0 terminal at 115200 8N1 attached
to GP0. Pair this with the issue: [KOTO-0129](../issues/main/KOTO-0129-pico-device-draw-pixels.md).

> Refresh the physical SD `.kbc` before debugging: an app that reacts in KotoSim
> but appears dead on hardware is usually running a stale board card.

## 1. Flash the firmware

Build and flash the UF2 (the cargo runner is `elf2uf2-rs -d`; hold BOOTSEL while
plugging in so the RP2040 enumerates as a mass-storage device):

```
cargo run -p koto-pico --bin koto_firmware --target thumbv6m-none-eabi --release
```

Or build the ELF and convert/flash separately:

```
cargo build -p koto-pico --bin koto_firmware --target thumbv6m-none-eabi --release
elf2uf2-rs -d target/thumbv6m-none-eabi/release/koto_firmware
```

## 2. Expected UART boot phases

In order, on a healthy boot:

- `phase=10 uart-ready baud=115200 format=8N1` (repeats ~6x while the bridge enumerates)
- `phase=11 lcd-init-start` → `phase=12 lcd-init-ok` → `phase=12 solid-blue-test`
- `phase=16 psram-ready capacity=8388608` (or `phase=198 psram-unavailable fallback=sram-window`)
- `phase=13 sd-scan-start` → `phase=14 catalog-ready packages=<n>`
- `phase=157 pixel-diagnostic ok x=152 y=152 w=16 h=16` (the blit self-test below)

On app launch:

- `phase=150 launch-request app=<id>`
- `phase=156 app-staged backing=psram code_size=<n>` (or `backing=sram`)
- `phase=152 app-started`
- `phase=155 app-draw frame=1 used=<n>/384 rect=<n> text=<n> pixels=<n>` (frame 1, then every 60)
- `phase=154 app-heartbeat frame=<n> pc=<n> fuel=<n>` (every 60 frames)
- `phase=153 app-exited code=<n>` on clean exit

## 3. Pixel diagnostic visual expectation

At boot, before the shell, a known 16x16 tile is blitted at panel `(152, 152)`
through the real `draw_pixels` command + present path (logged `phase=157`). It is
four 8x8 quadrants and holds for ~1 s:

- top-left: **red**, top-right: **green**
- bottom-left: **blue**, bottom-right: **white**

Wrong colours / swapped quadrants ⇒ RGB565 byte-order or blit-stride bug.
Nothing drawn ⇒ blit path or strip compose failure (look for `phase=257
pixel-diagnostic-error`).

## 4. KotoBlocks launch / visual / input checks

Launch `dev.koto.games.koto-blocks` and confirm against KotoSim:

- **Launch**: reaches `phase=152 app-started` with no `phase=25x` error.
- **Visual**: title screen renders (rects/text), then the **playfield board and
  pieces render** (the `draw_pixels` blits) — the gap this issue closes.
- **Draw budget**: `phase=155 app-draw` shows `pixels=<n>` rising on the gameplay
  frames and `used=<n>/384` staying **below 384** (no ` overflow`).
- **Input**: arrow keys move/rotate; the board updates and heartbeat `pc`/`fuel`
  stay healthy across frames.

## 5. Failure logs to capture

If anything fails, capture the full UART transcript and note specifically:

- The final `phase=` line before the hang/exit.
- Any `phase=25x` launch/VM error (`253` oversize, `254` verify, `255` memory
  budget, `257` app-vm-error with `pc`/`error`, `258` draw-error).
- `phase=155 app-draw ... overflow` — the draw-command cap (384) was hit and tail
  commands were dropped; record the `used`/`pixels` counts at that point.
- `phase=257 pixel-diagnostic-error` — the blit self-test itself failed.
- For the KotoMemo OOM (tracked separately, not part of this issue): its final
  `phase=255 launch-memory-budget-error` and the requested heap size.
