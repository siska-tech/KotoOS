# Changelog

## KotoOS 0.2.0 — 2026-07-14

- Added hardware-validated RP2350A/Pico 2 W firmware and UF2 tooling while
  preserving the RP2040 profile.
- Added the KotoIDE VS Code extension foundation, Koto language server,
  include-aware compiler diagnostics, app wizard, and sprite/icon/tilemap/MML
  authoring tools.
- Added retained tilemap APIs, samples, asset packaging, and editor previews.
- Added a full-resolution 320×320 RGB565 image-streaming gallery with fade and
  wipe transitions plus looping SLD4 background music.
- Added persistent RGB565 drawing and ranged package-asset loading host calls.
- Unified packaged KotoAudio playback and long-clip streaming across KotoSim
  and PicoCalc.
- Hardened SD SPI startup with power stabilization, 400 kHz acquisition
  retries, CRC-validated transfer-clock promotion up to 25 MHz, and throughput
  diagnostics. The validated RP2350A card reached 915 KiB/s.
- Expanded RP2350A peripheral, PSRAM, CodeWindow, rendering, and audio
  validation coverage.

## KotoOS 0.1.0

- Established the Rust workspace, portable shell/runtime foundations, KotoSim,
  RP2040 PicoCalc bring-up, bytecode compiler/VM, Memo with SKK IME, package
  tooling, graphics pipeline, audio service, and initial bundled applications.
