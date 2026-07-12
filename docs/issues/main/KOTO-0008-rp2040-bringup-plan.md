# KOTO-0008: RP2040 Bring-Up Plan and HAL Backend Decision

- Status: done
- Type: research
- Priority: P1
- Requirements: NFR-PORT-3, NFR-PORT-5

## Goal

Choose the first embedded Rust HAL path for RP2040/RP2350 and define the minimal device bring-up sequence.

## Acceptance Criteria

- [x] Compare `embassy-rp`, `rp-hal`, and Pico SDK FFI for the first backend.
- [x] Define first hardware probes for LCD, keyboard, SD, PSRAM, audio, and power.
- [x] Document flashing/debug workflow.

## Notes

This should happen before large embedded-specific code lands.

Outcome: [RP2040_BRINGUP.md](../../hardware/RP2040_BRINGUP.md) records the comparison, the bring-up probe
sequence, and the flash/debug workflow. The chosen first backend is `embassy-rp`, with the Pico C
SDK kept only as an FFI escape hatch. This resolves the HAL question in
[TRACEABILITY.md](../../planning/TRACEABILITY.md). The `embassy-rp` decision was accepted on 2026-06-13; the
bring-up probes are the first embedded work it authorizes.
