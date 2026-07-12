# KOTO-0068: Pico Probe: SD Mount And Read

- Status: done
- Type: feature
- Priority: P1
- Requirements: HC-6, FR-FS-1, FR-FS-3, NFR-REL-2

## Goal

Prove that the embedded backend can mount a FAT SD card over SPI0 and read a
Koto package manifest sequentially.

## Acceptance Criteria

- [x] The probe mounts at least one known-good FAT SD card.
- [x] The probe lists `apps/` and reads a `.kpa.json` manifest.
- [x] SPI clock fallback behavior is logged for failures or unsupported cards.
- [x] Bring-up notes record card model and observed result.

## Notes

This corresponds to probe 4 in `docs/RP2040_BRINGUP.md`.

Implementation started on 2026-06-20 using `embassy-rp` SPI0 and
`embedded-sdmmc`. The probe will log card-detect state, try a fast SPI clock
followed by a compatibility fallback, list `apps/`, and stream the first
`.kpa.json` manifest through a bounded buffer over USB CDC.

Physical validation on 2026-06-20 mounted a 7,822,376,960-byte card. The 12 MHz
attempt returned `CardNotFound`, the 1 MHz fallback mounted successfully,
`apps/` listed 15 long manifest names, and `kotomines.kpa.json` began a
sequential 1,545-byte read. Terminal output now converts manifest LF bytes to
CRLF for readability without changing the bytes read from storage.

The final run used a TOSHIBA 8GB SDHC Memory Card. It completed the entire
1,545-byte `kotomines.kpa.json` read and printed `manifest read end bytes=1545`.
