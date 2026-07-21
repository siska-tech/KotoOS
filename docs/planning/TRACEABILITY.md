# Requirement Traceability

This document maps research findings into requirement areas so design drift is visible.

| Research Topic | Requirement Coverage | Notes |
| :------------- | :------------------- | :---- |
| RP2040 SRAM limit | HC-1, NFR-MEM-1, NFR-MEM-2, NFR-MEM-3 | Full-screen framebuffer is forbidden on RP2040. |
| SPI LCD bandwidth | HC-2, NFR-PERF-1, NFR-PERF-2, NFR-DRAW-1, NFR-DRAW-2 | Dirty rectangles and scanline DMA are first-class paths. |
| PSRAM XIP unavailable on RP2040 | HC-3, FR-RT-2, FR-RT-5, NFR-MEM-4, NFR-MEM-5 | PSRAM is block storage, not pointer-addressable RAM. |
| Shell / VM resident SRAM overlap | HC-1, FR-RT-2, FR-RT-5, NFR-MEM-1, NFR-MEM-2, NFR-MEM-4 | Foreground-exclusive shell state and VM code tiles share one SRAM slot; the shell is preserved in a bounded PSRAM reservation as defined by [RP2040 Shell / Code-Window Resident Overlay](../architecture/RP2040_SHELL_CODE_RESIDENT_OVERLAY.md). |
| PWM slice sharing | HC-4, FR-MML-1, FR-MML-2, NFR-REL-3 | Audio uses a software PCM mixer. Hardware validation is tracked by [KOTO-0114](../issues/main/KOTO-0114-pico-probe-pwm-audio-output.md). |
| I2C keyboard speed | HC-5, NFR-PERF-4, NFR-PERF-5 | Polling must be non-blocking and initialized above 10kHz. |
| Keyboard matrix limits | HC-8, FR-SDK-6, NFR-PERF-6 | Default game mapping requires real hardware validation. |
| SPI SD storage | HC-6, FR-FS-1, FR-FS-3, FR-PKG-2, NFR-REL-2 | Sequential reads and package layout reduce stalls. |
| Fixed GPIO wiring | HC-7, NFR-PORT-4, section 2.3 | Backend pin ownership stays inside HAL. |
| Battery through STM32 | FR-SHELL-5, FR-SDK-7, NFR-REL-4 | Must degrade cleanly when unavailable; hardware validation is tracked by [KOTO-0115](../issues/main/KOTO-0115-pico-probe-battery-power-status.md). |
| Rust implementation policy | FR-SIM-5, NFR-PORT-1, NFR-PORT-5, NFR-DEV-4 | Rust is the primary language; C/C++ use is isolated behind FFI. |
| App-facing KotoUI components | FR-SDK-1, FR-SDK-2, FR-SDK-5, FR-SDK-9, FR-RT-3, FR-RT-4, NFR-PERF-1, NFR-DRAW-1, NFR-MEM-2, NFR-PORT-1, NFR-PORT-3, NFR-REL-1, NFR-I18N-1, NFR-I18N-2, NFR-I18N-3 | A versioned bounded UI ABI, locale-aware SDK, bilingual Gallery/File Note, and Shell localization are planned in [KOTOUI_APP_ABI_ROADMAP.md](KOTOUI_APP_ABI_ROADMAP.md). |
| PicoCalc international audience | FR-SHELL-6, FR-SDK-9, NFR-I18N-1, NFR-I18N-2, NFR-I18N-3 | English is the deterministic fallback; Japanese remains supported, and `qps-ploc` catches layout assumptions before device validation. |
| Central system configuration | FR-CONFIG-1, FR-CONFIG-2, FR-CONFIG-3, FR-SHELL-6, FR-SDK-9, NFR-REL-5 | KotoConfig owns mutation, ConfigService owns bounded persistence/change generations, and consumers receive allowlisted snapshots; planned in [KOTOCONFIG_ROADMAP.md](KOTOCONFIG_ROADMAP.md). |
| Koto symbolic integer domains | FR-PKG-3, FR-SDK-5, FR-RT-4, NFR-DEV-3, NFR-DEV-4, NFR-REL-1 | Compile-time integer enums organize SDK ABI constants and App state machines without VM/runtime cost; tracked by [KOTO-0225](../issues/main/KOTO-0225-koto-language-enum-sdk-domains.md). |
| Koto structured App state | FR-PKG-3, FR-RT-4, NFR-MEM-2, NFR-DEV-3, NFR-DEV-4, NFR-REL-1 | Top-level static heap records and statically resolved inline methods improve state organization without an allocator or VM/ABI change; tracked by [KOTO-0228](../issues/main/KOTO-0228-koto-static-records-inline-methods.md). |
| Koto SDK text/List resources | FR-SDK-1, FR-SDK-2, FR-SDK-5, FR-SDK-9, FR-RT-4, NFR-PERF-1, NFR-MEM-2, NFR-DEV-3, NFR-DEV-4, NFR-REL-1, NFR-I18N-1, NFR-I18N-2 | Caller-owned indexed UTF-8 resources and List row builders remove repeated App parsers and wire-layout arithmetic without allocation or ABI changes; tracked by [KOTO-0230](../issues/main/KOTO-0230-sdk-text-resources-list-row-builders.md). |
| KotoUI SDK transaction/locale completion | FR-SDK-1, FR-SDK-2, FR-SDK-5, FR-SDK-9, FR-RT-4, NFR-PERF-1, NFR-MEM-2, NFR-DEV-3, NFR-DEV-4, NFR-REL-1, NFR-I18N-1, NFR-I18N-2 | Explicit update submission, resource-to-widget text bridging, canonical locale matching, and focused SDK sources remove remaining common App boilerplate without hiding presentation or fallback policy; tracked by [KOTO-0231](../issues/main/KOTO-0231-koto-ui-sdk-transaction-resource-locale-ergonomics.md). |
| KotoUI compile-time packet capacity | FR-PKG-3, FR-SDK-1, FR-SDK-2, FR-SDK-5, FR-RT-4, NFR-MEM-2, NFR-DEV-3, NFR-DEV-4, NFR-REL-1 | SDK-backed compile-time helpers derive KUI1/KUP1 packet storage from semantic record/data capacities, reject protocol-bound violations, and keep wire-layout arithmetic out of Apps; tracked by [KOTO-0232](../issues/main/KOTO-0232-koto-ui-compile-time-packet-capacity-helpers.md). |
| KotoUI builder call-site capacity locality | FR-PKG-3, FR-SDK-1, FR-SDK-2, FR-SDK-5, FR-RT-4, NFR-MEM-2, NFR-DEV-3, NFR-DEV-4, NFR-REL-1 | Helper-sized local buffers and compile-time `len(buf)` keep one-use packet sizing beside its builder transaction without adding runtime buffer metadata; tracked by [KOTO-0233](../issues/main/KOTO-0233-koto-buffer-capacity-at-ui-builder-call-sites.md). |
| Optional Wi-Fi modules | FR-CONFIG-3, NFR-PORT-4, NFR-PORT-6, NFR-REL-5 | A board name alone does not expose Wi-Fi settings: radio HAL plus a compiled network service must advertise the capability. Current networking remains out of MVP scope. |
| Optional synchronized system time | FR-SHELL-5, FR-CONFIG-3, NFR-MEM-2, NFR-PORT-1, NFR-PORT-3, NFR-PORT-4, NFR-PORT-6, NFR-REL-3, NFR-DEV-5 | A bounded unauthenticated SNTP service may publish advisory Shell/filesystem time after DHCP without becoming a boot, security, or offline dependency; tracked by [KOTO-0244](../issues/main/KOTO-0244-bounded-sntp-time-service.md). |
| App-facing external data retrieval | FR-SDK-5, FR-RT-4, FR-PKG-1, FR-PKG-3, NFR-MEM-2, NFR-PORT-1, NFR-PORT-3, NFR-PORT-6, NFR-REL-1, NFR-REL-3, NFR-REL-5, NFR-DEV-3, NFR-DEV-4, NFR-DEV-5 | Sandboxed Apps use a default-denied, origin-allowlisted, fixed-capacity OS Fetch service rather than raw sockets; device authentication must not trust advisory SNTP time. Tracked by [KOTO-0245](../issues/main/KOTO-0245-bounded-app-fetch-service.md). |
| Bounded external data decoding | FR-SDK-5, FR-RT-3, FR-RT-4, FR-PKG-3, NFR-MEM-2, NFR-PORT-1, NFR-PORT-3, NFR-REL-1, NFR-DEV-3, NFR-DEV-4 | Incremental fixed-capacity JSON tokens let Apps process partial Fetch and MQTT payloads without a DOM or response-sized allocation; tracked by [KOTO-0246](../issues/main/KOTO-0246-bounded-json-data-decoder.md). |
| Weather Internet-data reference app | FR-SDK-4, FR-SDK-5, FR-SDK-9, FR-RT-4, FR-FS-2, FR-PKG-1, FR-PKG-3, NFR-MEM-2, NFR-PORT-3, NFR-REL-1, NFR-I18N-1, NFR-I18N-2, NFR-DEV-3, NFR-DEV-4 | A bilingual packaged Weather app validates bounded Fetch, JSON decoding, sandbox caching, advisory update time, and honest offline/stale states; tracked by [KOTO-0247](../issues/main/KOTO-0247-weather-internet-data-reference-app.md). |
| Application network credentials | FR-SDK-5, FR-RT-4, FR-FS-2, FR-PKG-1, NFR-MEM-2, NFR-PORT-1, NFR-PORT-3, NFR-REL-1, NFR-REL-5, NFR-DEV-3, NFR-DEV-4 | OS-owned opaque grants keep application service secrets out of packages, VM memory, sandboxes, settings, and logs; tracked by [KOTO-0248](../issues/main/KOTO-0248-app-network-credential-vault.md). |
| Live MQTT telemetry for Apps | FR-SDK-5, FR-RT-4, FR-PKG-1, FR-PKG-3, NFR-MEM-2, NFR-PORT-1, NFR-PORT-3, NFR-PORT-6, NFR-REL-1, NFR-REL-3, NFR-REL-5, NFR-DEV-3, NFR-DEV-4, NFR-DEV-5 | An OS-brokered, subscribe-only MQTT profile provides best-effort live telemetry with bounded queues, topic permissions, authenticated transport, and no background App execution; tracked by [KOTO-0249](../issues/main/KOTO-0249-bounded-app-mqtt-service.md). |
| IoT Dashboard reference app | FR-SDK-5, FR-SDK-9, FR-RT-4, FR-FS-2, FR-PKG-1, FR-PKG-3, NFR-MEM-2, NFR-PORT-3, NFR-REL-1, NFR-I18N-1, NFR-I18N-2, NFR-DEV-3, NFR-DEV-4 | A bilingual packaged dashboard validates bounded MQTT/JSON telemetry, burst coalescing, stale-state presentation, credential denial, and clean disconnect on App exit; tracked by [KOTO-0250](../issues/main/KOTO-0250-iot-dashboard-mqtt-reference-app.md). |
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
