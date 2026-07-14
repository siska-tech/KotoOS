# Requirement Traceability

This document maps research findings into requirement areas so design drift is visible.

| Research Topic | Requirement Coverage | Notes |
| :------------- | :------------------- | :---- |
| RP2040 SRAM limit | HC-1, NFR-MEM-1, NFR-MEM-2, NFR-MEM-3 | Full-screen framebuffer is forbidden on RP2040. |
| SPI LCD bandwidth | HC-2, NFR-PERF-1, NFR-PERF-2, NFR-DRAW-1, NFR-DRAW-2 | Dirty rectangles and scanline DMA are first-class paths. |
| PSRAM XIP unavailable on RP2040 | HC-3, FR-RT-2, FR-RT-5, NFR-MEM-4, NFR-MEM-5 | PSRAM is block storage, not pointer-addressable RAM. |
| PWM slice sharing | HC-4, FR-MML-1, FR-MML-2, NFR-REL-3 | Audio uses a software PCM mixer. Hardware validation is tracked by [KOTO-0114](../issues/main/KOTO-0114-pico-probe-pwm-audio-output.md). |
| I2C keyboard speed | HC-5, NFR-PERF-4, NFR-PERF-5 | Polling must be non-blocking and initialized above 10kHz. |
| Keyboard matrix limits | HC-8, FR-SDK-6, NFR-PERF-6 | Default game mapping requires real hardware validation. |
| SPI SD storage | HC-6, FR-FS-1, FR-FS-3, FR-PKG-2, NFR-REL-2 | Sequential reads and package layout reduce stalls. |
| Fixed GPIO wiring | HC-7, NFR-PORT-4, section 2.3 | Backend pin ownership stays inside HAL. |
| Battery through STM32 | FR-SHELL-5, FR-SDK-7, NFR-REL-4 | Must degrade cleanly when unavailable; hardware validation is tracked by [KOTO-0115](../issues/main/KOTO-0115-pico-probe-battery-power-status.md). |
| Rust implementation policy | FR-SIM-5, NFR-PORT-1, NFR-PORT-5, NFR-DEV-4 | Rust is the primary language; C/C++ use is isolated behind FFI. |
| RP2040 HAL backend choice | NFR-PORT-3, NFR-PORT-5, NFR-DEV-1, NFR-DEV-2 | `embassy-rp` first backend; bring-up probes in [RP2040_BRINGUP.md](../hardware/RP2040_BRINGUP.md). |
| RP2350 / Pico 2 module compatibility | FR-SDK-8, NFR-PORT-4, NFR-PORT-6, NFR-DEV-5 | Active compatibility work is split into build profiles and device parity in [RP2350_SUPPORT_ROADMAP.md](RP2350_SUPPORT_ROADMAP.md). |
| Pico Plus 2(W) onboard QMI PSRAM | FR-RT-6, NFR-MEM-6, NFR-PORT-6 | Prefer module PSRAM behind the existing bounded HAL contract, with PicoCalc PSRAM fallback; tracked by [KOTO-0206](../issues/main/KOTO-0206-pico-plus-2-onboard-psram.md). |
| P/ECE and DOS/VGA influence | FR-DOS-1, FR-DOS-2, NFR-DRAW-3 | Treated as app/runtime modes, not full emulation goals. |
| Scanline sprite composition | FR-PM-1, FR-PM-2, HC-1, NFR-DRAW-1 | PicoMings uses scanline tile and sprite composition as defined in [PICOMINGS_SPRITE_MODEL.md](../spec/PICOMINGS_SPRITE_MODEL.md). |
| TiPO/BTRON influence | FR-IME-3, FR-FS-2 | Fixed IME line and data containment influence UI and FS. |
| PicoMite/REPL influence | FR-REPL-1, FR-REPL-2 | Deferred until KotoRuntime exists. |

## Open Traceability Questions

- ~~Which VM should be proven first: Wasm3, Lua, mruby, or a custom stack VM?~~ Resolved by [KOTO-0018](../issues/main/KOTO-0018-runtime-selection-spike.md): prototype a small custom stack VM first. See [RUNTIME_VM_SELECTION.md](../architecture/RUNTIME_VM_SELECTION.md).
- ~~Which Rust embedded HAL path should be proven first for PicoCalc: `embassy-rp`, `rp-hal`, or a thin Pico SDK FFI layer?~~ Resolved by [KOTO-0008](../issues/main/KOTO-0008-rp2040-bringup-plan.md): `embassy-rp` is the first backend, with the Pico C SDK kept only as an FFI escape hatch. See [RP2040_BRINGUP.md](../hardware/RP2040_BRINGUP.md).
- Which keys are safest for A/B/X/Y on the real keyboard matrix?
- Which embedded HAL probe results should become permanent release-gate
  fixtures after the first PicoCalc bring-up pass?
- ~~Which LCD controller variants require divergent initialization sequences?~~
  Resolved by [KOTO-0026](../issues/main/KOTO-0026-lcd-init-profiles.md): keep
  ILI9488, ST7365P-compatible, and unknown-compatible init data behind embedded
  HAL LCD profiles. See [LCD_INIT_PROFILES.md](../hardware/LCD_INIT_PROFILES.md).
- How reliable is battery reporting across PicoCalc firmware revisions?
