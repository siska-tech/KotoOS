# KOTO-0066: Pico Probe: LCD Fill

- Status: done
- Type: feature
- Priority: P0
- Requirements: HC-2, NFR-DRAW-1, NFR-DRAW-2, NFR-PERF-2

## Goal

Bring up the PicoCalc LCD enough to initialize a known profile, fill the screen,
and update a small rectangle or scanline band.

## Acceptance Criteria

- [x] The probe selects and logs an LCD init profile.
- [x] Solid color fills display with correct orientation and color order.
- [x] A partial rectangle or scanline update changes only the requested region.
- [x] Bring-up notes record controller/profile observations.

## Notes

This corresponds to probe 1 in `docs/RP2040_BRINGUP.md`.

The `lcd_fill` probe contains the ClockworkPi reference ILI9488 initialization
profile, RGB666 DMA fills, bounded rectangle updates, a scanline band update,
and USB-CDC progress logging.

Hardware observation on 2026-06-21 confirmed correct landscape orientation and
RGB color order. The four corner markers and centered rectangle had clean,
unclipped boundaries, and the cyan DMA band was continuous at y=200 with the
expected height of eight scanlines. No visible corruption or address-window
errors were observed.

## Resolution

The manually selected `ili9488-spi` profile passed physical LCD validation at
20 MHz using RGB666 (`COLMOD=0x66`) and `MADCTL=0x48`. Full-screen fills,
bounded rectangle writes, and the DMA scanline-band path are validated.
