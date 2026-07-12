# KOTO-0128: Pico App Runtime Frame Flicker

- Status: done
- Type: feature
- Priority: P1
- Requirements: FR-SDK-1, NFR-PERF-1, NFR-DRAW-1, NFR-DRAW-2

## Goal

Make animated bytecode apps repaint without visible flicker on the PicoCalc LCD,
matching the KotoSim app rendering, by giving the device app-present path the
same retained-GRAM / off-screen-composite discipline that KOTO-0120 gave the
shell paint path.

## Context

Follow-up to KOTO-0120 (deviation D6 in
[PICO_KOTOSHELL_PARITY.md](../../hardware/PICO_KOTOSHELL_PARITY.md)). KOTO-0125 hardware
validation found the Dirty Rects SDK sample (`dev.koto.samples.dirty-rects`)
shows the correct image — a white square sliding left-to-right with a status
line — but flickers on the current device LCD adapter only. KotoSim renders the
same command list without flicker.

Root cause: the app present path in
`src/koto-pico/src/bin/koto_firmware.rs` only computes a minimal dirty rect when
the frame's command list contains a *full-screen* base rect
(`full_screen_base_color`). The Dirty Rects sample's background is a partial band
(`draw_rect(0, 72, 320, 40, 0)`), not a full-screen clear, so:

- `present_app_delta` finds no full-screen base in the previous frame and falls
  back to `present_app_commands` every frame.
- `present_app_commands`, with no full-screen base, replays each command as a
  direct `lcd.fill_rect` GRAM write: it fills the 320×40 band black, then draws
  the 24×24 square on top, and separately clears and redraws the text band.

Because the erase and the redraw hit GRAM directly with no off-screen
compositing or genuine pixel diff, the momentarily-black band is visible as
flicker. KotoSim composites the frame and presents it atomically, so the
intermediate erase is never shown.

This is a rendering-optimization defect in the device present path, not a
runtime/lifecycle defect: the app runs, reads input, and exits correctly
(KOTO-0125). It does not affect the shell, which already uses the KOTO-0120
bounded `paint_rect` path.

## Acceptance Criteria

- [x] Animated apps with a partial (non-full-screen) background, including the
  Dirty Rects sample, repaint without visible flicker on physical hardware.
- [x] The app present path composites each frame's changed region off-screen (or
  diffs against the retained previous frame) and transfers only genuinely
  changed pixels, instead of issuing erase-then-redraw `fill_rect` writes
  straight to GRAM.
- [x] The per-frame transfer stays within the existing bounded strip budget
  (`RASTER_STRIP_BYTES` / `RGB666_STRIP_BYTES`); no full-screen SRAM framebuffer
  is introduced.
- [x] Apps with a full-screen base (Actor Array, Hello Text) keep their current
  correct rendering and do not regress.
- [x] KotoSim and PicoCalc keep the same portable app command model; only the
  device transport changes.

## Notes

The KOTO-0120 shell fix (retained GRAM + bounded `paint_rect` dirty rectangles)
is the template. The app case differs in that the app command list, not
`ShellState`, defines the frame; the device should treat the previous frame's
composited pixels as retained GRAM and update only the rectangle whose composed
pixels actually changed, regardless of whether a full-screen base rect is
present. Sizing of any added scratch must follow the project's VM-budget-sizing
guidance and stay within the resident OS-core budget.

## Resolution

`present_app_delta` in `src/koto-pico/src/bin/koto_firmware.rs` no longer
requires a full-screen base rect to take the composited dirty-rect path. It now
derives the retained baseline as `full_screen_base_color(previous)` when present,
or black (`0`) otherwise — black matches the launch-time `lcd.fill(BLACK)`
baseline. As long as the current frame's base matches the previous frame's
(both present and equal, or both absent), each changed command's union rectangle
is composited off-screen into the existing `RASTER_STRIP` (clear to the baseline,
replay all current commands clipped to the rect) and transferred in one DMA, so
GRAM never shows an erase-then-redraw intermediate. A base appearing/disappearing
or a single dirty rect exceeding the strip still falls back to a full present.

This keeps the per-changed-command (small) transfers, so full-screen-base apps
(Actor Array, Hello Text) do not regress to a single large union transfer, while
partial-background apps (Dirty Rects) now composite their ~32×24 moving region
atomically instead of erasing the whole 320×40 band directly in GRAM. No new
SRAM buffer is introduced; the change is device-transport only and the portable
app command model is unchanged.

Verified:

- `cargo build -p koto-pico --bin koto_firmware --target thumbv6m-none-eabi --release --offline`
- `cargo clippy -p koto-pico --bin koto_firmware --target thumbv6m-none-eabi --release --offline -- -D warnings`
  (also fixed a pre-existing `manual_is_multiple_of` lint surfaced by the
  toolchain in the app-heartbeat counter)
- `cargo test -p koto-core --offline` (164 passed)

### Hardware validation (2026-06-22)

Confirmed on the validated PicoCalc: the Dirty Rects sample's moving square
repaints without the visible flicker, and the full-screen-base samples are
unchanged. All acceptance criteria are met.
