# Validation Plan

KotoOS should grow with checks that can run before real hardware is ready, then add hardware probes as soon as PicoCalc is available.

## Phase 0: Repository Harness

Run:

```powershell
python harness\check_all.py
```

Checks:

- Rust formatting is clean.
- Clippy passes with warnings denied.
- Rust tests pass.
- Requirement IDs are unique.
- Local Markdown links resolve.
- Sample package metadata is structurally valid.

## Phase 1: KotoSim Host Checks

Current simulator checks cover:

| Area  | Check                                     | Pass Criteria                                                        |
| :---- | :---------------------------------------- | :------------------------------------------------------------------- |
| Video | Draw dirty rect test pattern              | Only requested rect changes                                          |
| Input | Map PC keys to normalized state           | Held/pressed/released are stable                                     |
| FS    | Mount `sdcard_mock`                       | Files open through virtual paths                                     |
| Font  | Render sample Japanese text               | Glyphs fit expected cells                                            |
| IME   | Sticky Shift SKK trigger                  | One-shot shift affects one stroke                                    |
| Memo  | Launch, edit through IME, save, reload    | Saved content survives relaunch and includes committed Japanese text |
| Rust  | `cargo fmt`, `cargo clippy`, `cargo test` | No formatting drift, lints clean, tests pass                         |

## Phase 2: Package Harness

Current package and asset checks cover:

- Build a minimal `.kpa` fixture from metadata and assets.
- Validate package table offsets are monotonic.
- Validate assets marked sequential are contiguous.
- Validate app sandbox paths cannot escape their root.

## Phase 3: Device Bring-Up

Hardware checks should be logged through USB-CDC:

| Area     | Probe                                                                                                           | Pass Criteria                                                  |
| :------- | :-------------------------------------------------------------------------------------------------------------- | :------------------------------------------------------------- |
| LCD      | [Profile probes](../hardware/LCD_INIT_PROFILES.md) for ID, orientation, color format, rects, scanline DMA, and partial mode | Stable image, no orientation mismatch; selected profile logged |
| Keyboard | I2C poll rate                                                                                                   | 100kHz minimum when firmware allows                            |
| Keyboard | [Chord matrix test](../hardware/KEYBOARD_MATRIX.md)                                                                         | Default game mapping has no blocking                           |
| SD       | Mount and fallback                                                                                              | Known-good card mounts; fallback path logs                     |
| PSRAM    | Block read/write                                                                                                | Pattern round-trip succeeds                                    |
| Audio    | PWM callback tone                                                                                               | No underrun under idle shell                                   |
| Power    | Battery poll                                                                                                    | Value available or clean unsupported state                     |

## Phase 4: PicoCalc KotoShell Parity Gate

The release gate for a PicoCalc KotoShell that is behaviorally and visually
equivalent to the accepted KotoSim shell is defined and recorded in
[PicoCalc KotoShell Parity Validation](../hardware/PICO_KOTOSHELL_PARITY.md) (KOTO-0126,
the final gate for KOTO-0119 through KOTO-0125).

Recorded results (2026-06-22, `thumbv6m-none-eabi` release, SPI 62.5 MHz):

| Gate metric | Result |
| :---------- | :----- |
| Parity checklist (order, pagination, selection, pane, command bar, favorites, sort, categories, status, launch, recovery, return) | Passing with documented deviations D1–D5 (D6 resolved in KOTO-0128) |
| Release firmware flash image | 451.8 KiB (462,688 B) |
| Static SRAM (`.bss`) | 143.0 KiB of 264 KiB |
| Maximum working buffer | 30,720 B (no full framebuffer) |
| Same-page selection latency | 24 ms (< 33 ms target) |
| Initial paint (full) | 106 ms (fallback icons) / ~165 ms (15 themed icons) |
| Page movement | 112–147 ms (`dirty_px=90240`, all tiles change) |
| App launch / return | launch immediate; return 165 ms (full themed repaint) |
| Pane toggle | 279–280 ms (full repaint + bounded `SHELLPRF.TXT` write) |

## Phase 5: RP2350 / Pico 2 Compatibility Gate

The cross-build and hardware matrix is defined in the
[RP2350 / Pico 2 Support Roadmap](RP2350_SUPPORT_ROADMAP.md). RP2350 support is
not declared from compilation alone: each board profile needs an identifiable
UF2, a recorded boot, applicable peripheral probes, the shell/app/audio stress
session, and a board-specific stack-canary result.

The first active gate is the RP2350A Pico 2 W currently available for device
testing (KOTO-0204 and KOTO-0205). Pico Plus 2(W) is the subsequent KOTO-0206
gate and additionally requires module QMI PSRAM identity, boundary, and soak
tests plus a forced fallback to the separate PicoCalc baseboard PSRAM.

## Phase 6: Optional Wi-Fi Configuration Gate

Networking remains optional. Before device tests, simulator tests replay
[`network_service_v1.json`](../../harness/fixtures/network_service/network_service_v1.json)
without host network access and require exact ordered snapshots for capability
absence, scan/connect/disconnect, authentication failure, cancellation, radio
loss, stale completion rejection, and forget commit.

The exact focused command for all fake-service replay and KotoConfig Wi-Fi
controller scenarios is:

```powershell
cargo test -p koto-sim --test koto_network_service
```

Run the portable service boundary suite (queue/event overflow, exact timeout,
retry schedule, empty/full/deduplicated scans, invalid input, corrupt secret
store, and generation/request wrap) alongside it:

```powershell
cargo test -p koto-core net::tests
```

`python harness/check_project.py` additionally parses every checked-in fake
fixture and rejects nondeterministic fields, host-network dependencies, and
host-network/wall-clock/RNG APIs in the fake execution path.

Device promotion then requires the fixed capacities and SRAM ceilings in the
[KotoConfig Wi-Fi extension contract](../architecture/KOTOCONFIG_WIFI_EXTENSION.md),
release ELF accounting on RP2040 and RP2350, controlled-AP tests, secret
corruption/factory-reset checks, 100 lifecycle transitions, and the KOTO-0227
Wi-Fi-plus-stream soak. Any Wi-Fi failure must leave boot, language settings,
KotoShell, and offline app launch operational.

## Release Gates

MVP is not considered complete until:

- [x] KotoSim runs KotoShell with mock package listing.
- [x] Real PicoCalc shows KotoShell at usable speed. (24 ms same-page selection
  at SPI 62.5 MHz; see the parity gate above.)
- [x] SD card package listing works through KotoFS on hardware. (15 manifests +
  15 icons loaded on the validated card; KOTO-0121/0122.)
- [x] KotoIME can enter and convert a short Japanese phrase in simulator tests.
- [x] KotoSim can launch the memo app, commit IME text into it, save it, and
  reload the saved memo.
- [x] A minimal VM-hosted app can draw, read input, and exit without corrupting
  the shell in KotoSim.
- [x] PicoCalc KotoShell parity gate fully green: all six interaction captures
  recorded and the parity checklist passes with documented deviations D1–D5
  ([PICO_KOTOSHELL_PARITY.md](../hardware/PICO_KOTOSHELL_PARITY.md)).
