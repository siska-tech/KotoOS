# KOTO-0181: boot splash screen

- Status: DONE 2026-07-12 — device-confirmed. Implemented 2026-07-11 (shared
  `koto_core::boot_splash` painter + firmware integration + KotoSim
  parity/golden); device smoke confirmed the splash with real step cadence and
  no boot-time regression.
- Type: feature
- Priority: P2
- Related: KOTO-0081 (shell visual home — shares palette/typography), KOTO-0026
  (LCD init — the moment the panel first lights is where the splash goes).

## Goal

KotoOS boots straight into the shell with no identity moment. Implement a boot
splash in the spirit of the 2026-07 mock: dark background, pixel-art mascot
(cloaked cat, crescent moon, radio tower), large "KotoOS" wordmark with accent
color, tagline ("tiny system. big adventure."), then a boot checklist
(`[ok] init kernel / init memory / mount sd / init audio / init input / init
shell` with hex step codes) and a progress bar before handing off to the
shell.

## Notes

- The checklist should reflect **real** init phases (the `phase=` UART
  milestones already exist) rather than being cosmetic sleep()s — boot time
  must not regress noticeably; target the splash costing near-zero added wall
  time by rendering while init actually proceeds.
- Asset budget: one full-screen background is ~150 KiB RGB565 — prefer
  1-bit/indexed art + palette, or compose from glyph/tile draws, given SRAM
  and XIP pressure (KOTO-0176 context).
- Sim parity: KotoSim should show the same splash so screenshots/docs match
  the device.
- Failure states (SD missing, audio init failed) should surface on the
  checklist line rather than only in UART.

## Implementation (2026-07-11)

- `koto-core/src/boot_splash.rs`: shared `no_std` painter — `BootSplash`
  checklist state (6 steps, each mirroring a real `phase=` milestone and
  rendering its phase number as the hex step code), night-scene glyph art
  (cloaked cat / crescent moon / radio tower, 1 char per art pixel in flash —
  no RGB565 background asset), integer-scaled "Koto|OS" wordmark (accent
  tail), tagline, `[ok]`/`[ng]`/`[--]` checklist with `Failed(&'static str)`
  notes, and a resolved-steps progress bar. `paint_rect` is viewport-exact
  (host-tested against the full paint per 16-line device strip).
- `koto-pico`: `firmware/splash_render.rs` strips the painter over
  `write_rgb565_rect` (same shape as `shell_render`). `koto_firmware.rs`
  paints the splash right after `phase=12 lcd-init-ok` (`phase=18
  splash-render-start/ok`), **replacing the KOTO-0119 solid-blue test and its
  3 s of sleeps** — the splash's full paint is the panel bring-up proof now.
  Kernel/PSRAM/audio resolve up front from the already-run init results;
  `mount sd` resolves at `phase=14`, `init input` at the first power/bridge
  poll, `init shell` just before the first shell paint; each resolution
  repaints only its line band + progress bar. No cosmetic sleeps on the happy
  path; a failed step holds the splash 1.5 s so `[ng]` is readable. The
  one-shot panel diagnostics (SPI bench, pixel-blit) moved *before* the splash
  so they don't scribble on it.
- `koto-sim`: `--splash --image PATH` renders the completed splash BMP;
  `--window` opens on the splash (fixed cadence) before the shell;
  `tests/splash_golden.rs` pins sim-vs-core byte parity plus an FNV-1a golden
  of the completed frame with the shipped font.

## Acceptance Criteria

- [x] Device boot shows the splash with real init milestones and reaches the
      shell without measurable boot-time regression (compare `phase=14`
      catalog-ready timestamps before/after). — device-confirmed 2026-07-12;
      the splash replaced the 3 s solid-blue delay, so boot is net faster than
      the old baseline.
- [x] KotoSim renders the same splash (golden frame).
      (`splash_golden.rs`: byte parity with the koto-core painter + golden
      checksum; `--splash --image` for screenshots.)
- [x] Init failures render as visible `[ng]`/error lines on the splash.
      (Host-tested marker/note rendering; firmware maps PSRAM fallback, PCM
      diag error, missing SD, and a dead keyboard bridge onto their steps, and
      any failure holds the splash 1.5 s so the line is readable.)
