# KotoConfig Roadmap

- Status: planned

## Purpose

Create one bounded system-settings surface for KotoOS. Language selection is the
first delivered setting; optional Wi-Fi configuration is designed as a
capability-gated extension without bringing networking into the current MVP.

## Delivery

| Order | Issue | Outcome |
| --: | :-- | :-- |
| 1 | KOTO-0223 | Native KotoConfig, shared ConfigService, language page, persistence, and locale publication |
| 2 | KOTO-0222 | KotoShell consumes the shared locale and supplies complete English/Japanese resources |
| 3 | KOTO-0224 | Freeze the optional Wi-Fi page, NetworkService, capability, and credential boundaries before network implementation |
| 4 | KOTO-0227 | Reorganize Pico W SRAM for runtime full-Audio/Wi-Fi switching while retaining PCM16/SLDPCM4 stream audio; keep Pico 2 W concurrent |
| 5 | KOTO-0239 | Implement bounded CYW43/Embassy NetworkService and enforce target SRAM/flash ceilings |
| 6 | KOTO-0240 | Implement separate bounded Wi-Fi credential persistence, zeroization, forget, and factory reset |
| 7 | KOTO-0242 | Execute deterministic fake NetworkService scenarios in KotoSim without host networking |
| 8 | KOTO-0241 | Add the bilingual capability-gated `network.wifi` KotoConfig page |
| 9 | KOTO-0251 | Wire the product NetworkService and Wi-Fi page into the Pico W build on the switchable residency arena |
| 10 | KOTO-0243 | Pass integrated Pico W switched-residency and Pico 2 W concurrent hardware validation |
| 11 | KOTO-0244 | Add optional bounded SNTP time synchronization and publish the real Shell/filesystem clock |

KOTO-0223 may build its native UI while KOTO-0218 implements the app ABI, but
its `KUC1` publication/event acceptance depends on KOTO-0218. KOTO-0222 removes
Japanese-only Shell strings after the shared setting exists. KOTO-0224 is a
future-readiness design gate and does not make Wi-Fi a release dependency.

KOTO-0240 and KOTO-0242 can proceed while KOTO-0227/KOTO-0239 complete the
device service foundation. KOTO-0241 develops against the fake service and
integrates with real providers after their contracts pass. KOTO-0251 opens
the Pico W product build's network wiring on the KOTO-0227 arena now that
those thresholds are frozen; the Pico 2 W wiring landed with KOTO-0241/0243
preflight. KOTO-0243 remains the only gate that promotes the complete
optional feature on supported hardware.

## Completion gate

The first milestone is complete when KotoConfig changes language on KotoSim and
PicoCalc, survives reboot and corrupted settings, Shell and apps observe one
locale generation, and unsupported pages consume no runtime state. The Wi-Fi
extension is ready for implementation only after KOTO-0224 records measurable
capacities and a secret-storage threat model.

The optional Wi-Fi milestone is complete only when KOTO-0227, KOTO-0239
through KOTO-0243, and KOTO-0251 pass. Wi-Fi-disabled builds remain supported throughout and
must not allocate network runtime state or lose offline functionality.

KOTO-0244 is a follow-on network-time feature, not part of the KOTO-0243 Wi-Fi
promotion gate. Its absence or failure leaves the clock visibly unknown and
does not weaken offline behavior.
