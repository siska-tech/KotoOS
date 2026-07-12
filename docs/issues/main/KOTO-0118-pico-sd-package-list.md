# KOTO-0118: Pico SD Package List

- Status: done
- Type: feature
- Priority: P0
- Requirements: FR-SHELL-1, FR-SHELL-2, FR-FS-1, FR-FS-3, NFR-REL-2

## Goal

Populate the PicoCalc firmware launcher from package manifests on the FAT SD
card instead of compiled-in fixture entries.

## Acceptance Criteria

- [x] Product firmware initializes the SD card with the validated SPI clock
  fallback.
- [x] The firmware enumerates `APPS/*.kpa.json` and reads each accepted
  manifest through a bounded sequential buffer.
- [x] Manifest `app_id` and `name` fields pass through the portable
  `PackageInfo` validation before entering `PackageList`.
- [x] The launcher reflects the discovered package count and supports
  navigation across more than one visible page.
- [x] Invalid manifests and unavailable storage degrade to a visible fallback
  state without blocking keyboard input.
- [x] The RP2040 build, Clippy, and project harness pass, and a UF2 is produced.

## Notes

This issue builds on KOTO-0068 and KOTO-0117. App launch and bytecode loading
remain separate work; confirmation still provides visual feedback only.

## Resolution

The product firmware now mounts FAT storage over SPI0, retries initialization
at 1 MHz when the validated 12 MHz path fails, enumerates long
`APPS/*.kpa.json` names, and reads manifests sequentially into a fixed 2304-byte
buffer. The bounded parser accepts KPA format/version 1 and sends `app_id` and
`name` through `PackageInfo` validation.

The launcher shows three package cards at a time, derives stable card colors
from app IDs, and adds page indicators for the discovered package count.
Navigation repaints the appropriate three-card group. Storage and manifest
failures produce one diagnostic package instead of blocking the frame loop.

Verified:

- `cargo build -p koto-pico --bin koto_firmware --target thumbv6m-none-eabi --offline`
- `cargo clippy -p koto-pico --bin koto_firmware --target thumbv6m-none-eabi --offline -- -D warnings`
- `python harness/check_project.py`
- `elf2uf2-rs target/thumbv6m-none-eabi/debug/koto_firmware target/thumbv6m-none-eabi/debug/koto_firmware.uf2`
