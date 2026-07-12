# KOTO-0064: Pico HAL Crate Bootstrap

- Status: done
- Type: feature
- Priority: P0
- Requirements: NFR-PORT-3, NFR-PORT-4, NFR-DEV-1, NFR-DEV-2

## Goal

Add the first embedded PicoCalc HAL crate and build configuration without
pulling embedded dependencies into the host simulator path.

## Acceptance Criteria

- [x] The workspace contains an embedded HAL/probe crate targeting RP2040.
- [x] Host `cargo test` and `python harness\check_all.py` remain usable on a PC.
- [x] The crate structure keeps fixed PicoCalc pin ownership inside the embedded
  backend.
- [x] The README or bring-up document explains how to build the embedded target.

## Notes

Use the `embassy-rp` decision recorded in `docs/RP2040_BRINGUP.md` unless a new
research issue supersedes it.

## Resolution

Added the `koto-pico` workspace member with `embassy-rp` 0.10.0 and an RP2040
bootstrap binary. The host crates remain the workspace's `default-members`, so
normal simulator checks do not compile embedded-only dependencies.

The backend owns the PicoCalc LCD, keyboard, SD, PSRAM, and audio pin map in
`src/koto-pico/src/pins.rs`. Target-specific Cargo configuration and `memory.x`
link the standard Pico 1H layout (2 MB flash, 264 KB SRAM) without changing host
build settings.

Verified:

- `cargo build -p koto-pico --bin bootstrap --target thumbv6m-none-eabi --offline`
- `python harness/check_all.py`
