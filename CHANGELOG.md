# Changelog

## KotoOS 0.3.0-alpha — 2026-07-22

OpenAI Build Week preview release.

- Added an OS-owned, capability-gated HTTPS Fetch service for sandboxed apps,
  with bounded request/response storage, certificate validation, KotoSDK and
  KotoSim parity, and the `app_fetch_https` reference app.
- Hardware-validated authenticated HTTPS on RP2040 Pico W and RP2350A Pico 2 W.
  The RP2040 profile uses TLS-scoped audio exclusion and a guarded crypto stack;
  normal PCM/SLDPCM and game audio resume after networking ends. RP2350A keeps
  independent TLS memory and concurrent audio playback.
- Added bounded MQTT subscribe, SNTP time, JSON data decoding, credential-vault,
  and Wi-Fi configuration foundations for networked applications.
- Added RP2350A `app_fetch_https` firmware/UF2 generation through
  `tools/build-rp2350a.ps1 -AppFetchHttps`.

### Known issues

- Pico W's saved-network **forget** operation remains broken on hardware
  ([KOTO-0251](docs/issues/main/KOTO-0251-pico-w-product-network-wiring.md));
  scan, connect, DHCP, persistence, Fetch, and post-network audio are validated.
- Some RP2350A PicoCalc peripheral-parity acceptance criteria remain tracked in
  [KOTO-0205](docs/issues/main/KOTO-0205-rp2350-picocalc-peripheral-parity.md).
- The KotoUI Gallery SKK candidate integration test currently fails to select
  the expected `傘` candidate in KotoSim; other host suites and all embedded
  cross-build profiles complete successfully.
- Network and app APIs may change before the stable 0.3.0 release.

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
