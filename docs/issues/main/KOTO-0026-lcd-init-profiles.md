# KOTO-0026: LCD Controller Init Profiles

- Status: done
- Type: research
- Priority: P1
- Requirements: HC-2, NFR-DRAW-2, NFR-PORT-4

## Goal

Capture LCD initialization differences for ILI9488 and ST7365P-compatible panels.

## Acceptance Criteria

- [x] Document known controller variants.
- [x] Define profile data needed by the device HAL.
- [x] List bring-up probes for orientation, color format, and partial updates.

## Notes

This keeps panel variation from leaking into core rendering code.

The accepted profile contract and probe plan live in
[LCD_INIT_PROFILES.md](../../hardware/LCD_INIT_PROFILES.md). Hardware-specific init command
tables remain pending until the PicoCalc LCD probe is run on real panels.
