# KOTO-0031: KotoSim Software Framebuffer and Image Output

- Status: done
- Type: feature
- Priority: P0
- Requirements: FR-SIM-1, FR-SHELL-3, NFR-DRAW-3

## Goal

Let KotoSim rasterize the shell into real pixels and dump them to an image file
so the UI can be inspected visually, instead of only printing render-command
text. This is the first step toward the FR-SIM-1 virtual window.

## Acceptance Criteria

- [x] Core has an RGB565 rasterizer (`fill_rect`, glyph/text blit) that clips to
      bounds and reuses the bitmap font from KOTO-0013.
- [x] `ShellState::paint` draws the header, the package launcher tiles, and the
      selection highlight.
- [x] KotoSim writes a BMP with `--image PATH` (font via `--font`, default
      `assets/fonts/mplus12.kfont`); combinable with `--script`.
- [x] Tests cover rect clipping, selected-row highlighting, and BMP output.

## Notes

Depends on KOTO-0005 (render model) and KOTO-0013 (font).

The full-screen framebuffer lives only in the simulator (host), honoring
NFR-MEM-3 which forbids a full framebuffer in RP2040 SRAM. The core rasterizer
([`koto_core::raster::Canvas`](../../../src/koto-core/src/raster.rs)) is region/clip
based, so the same primitives can paint individual dirty tiles on-device later.

BMP (24-bit) is hand-written with zero dependencies so it opens directly on
Windows; a live interactive window is split out to KOTO-0032.
