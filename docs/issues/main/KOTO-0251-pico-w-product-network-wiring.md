# KOTO-0251: Pico W product NetworkService wiring on the residency arena

- Status: in-progress (9/10 ACs closed 2026-07-21; sole remaining item is
  AC-6 **forget**, which is broken on hardware and needs a fix — every other
  criterion is hardware-confirmed or delegated to the KOTO-0243 fault matrix)
- Type: firmware integration
- Priority: P1
- Requirements: FR-CONFIG-3, NFR-MEM-1, NFR-MEM-2, NFR-MEM-4, NFR-MEM-5, NFR-PORT-4, NFR-PORT-6, NFR-REL-3, NFR-REL-5
- Related: KOTO-0224, KOTO-0227, KOTO-0239, KOTO-0240, KOTO-0241, KOTO-0243
- Roadmap: [KotoConfig Roadmap](../../planning/KOTOCONFIG_ROADMAP.md)

## Goal

Wire the product NetworkService, the SD-backed credential store, and the
capability-gated `network.wifi` KotoConfig page into the RP2040 Pico W product
firmware, placing all network runtime state in the KOTO-0227 switchable
residency arena. A Wi-Fi-enabled Pico W image then scans, connects, and holds
a DHCP lease like the Pico 2 W `-WifiConfig` artifact while keeping stream
audio available and staying inside the frozen KOTO-0227 SRAM thresholds.

This is the implementation prerequisite for KOTO-0243's Pico W validation
half. Every component already exists; what is missing is the Pico W product
binary wiring, which today is gated on `board-picocalc-pico2w` throughout
`koto_firmware.rs` (network runtime start, `WifiConfigInputs`, `WIFI_CONFIG`
capability union, Wi-Fi page loop, and credential-store load).

## Background

- KOTO-0227 (done) proved the switchable arena on Pico W hardware: 100
  physical round trips, the five-minute alternating PCM16/SLDPCM4 stream soak
  against bounded packet activity, and the post-Wi-Fi full-Audio regression
  all passed with zero fault counters. The release threshold is frozen at
  CPU0 `phase=176 free_min >= 8 KiB` and `core1_stack_free_min >= 4 KiB`
  (measured 11,856 B / 6,588 B).
- Pico 2 W's product profile allocates an independent 36 KiB
  `NETWORK_SERVICE_ARENA` static because it runs Audio and Wi-Fi
  concurrently. RP2040 must not copy that shape: its network state borrows
  the rich-audio arena (23,568 B runner/network reserve behind the
  12,688 B `cyw43::State` and the 604 B Fetch/TLS coordinator), and `.bss`
  must not grow for network state.
- `firmware::network` (the NetworkService command loop), the KOTO-0239
  `cyw43_network_future` product lifecycle, `firmware::secret_store`, and
  the KOTO-0241 page controller are already board-agnostic or
  `any(picow, pico2w)`-gated.

## Residency Policy

- Enabling the radio (from the Wi-Fi page) drives
  `FullAudio -> WifiStreamAudio` through the proven KOTO-0227 quiesce
  handshake before `cyw43_network_future` is installed; disabling the radio
  cancels/joins the runner, powers down GP23, and rebuilds rich audio.
- Wi-Fi residency persists after leaving the page while the radio stays
  enabled, so network time and future app networking can operate; rich audio
  (BGM, synthesized SFX, runtime cues, owned clips) stays explicitly
  temporarily-unavailable until radio-off. PCM16/SLDPCM4 stream audio remains
  available throughout, as guaranteed by KOTO-0227.
- Capability loss or a transition fault lands in the safe `Offline` state and
  restores `FullAudio`; no failure blocks boot, language settings, KotoShell,
  or offline app launch.

## Acceptance Criteria

- [x] Add a Pico W wifi-config build profile (feature set and board-named UF2
  output documented in the koto-pico README); the default offline Pico W
  artifact keeps its accepted behavior and links no network stack, sockets,
  or DHCP/DNS state.
- [x] Construct the NetworkService transport — `cyw43::State`, the
  `cyw43_network_future` runner future, embassy-net resources, and the Fetch
  transport mailbox — inside the borrowed KOTO-0227 arena through the
  generation-owned `WifiRuntime`. Network state adds no new `.data`/`.bss`
  beyond bounded control words, and the machine-readable SRAM report proves
  arena placement and the static-span delta.
- [x] One mode owner performs `FullAudio -> WifiStreamAudio` on radio enable
  and the reverse on radio disable, capability loss, and fault, reusing the
  KOTO-0227 generation handshake; stale generations are rejected and an
  interrupted transition lands `Offline` without arena reuse. Enable and
  disable halves hardware-confirmed (2026-07-20/21); stale/out-of-order
  generation-ack rejection unit-proven (`check_arena_future`). The fault-arm
  hardware injection (capability loss, interrupted-transition-lands-`Offline`)
  is delegated to the KOTO-0243 fault/persistence matrix.
- [x] Composite `WIFI_CONFIG` for Pico W follows the KOTO-0224 inputs
  (declared transport, DriverReady behind a resolved region policy, live
  service generation, credential provider ready). The page registers only
  when true, and the KOTO-0241 `RadioUnavailable`/escape flows are preserved,
  including cancellable quiescing on page entry with no keyboard trap.
  Hardware: the capability computed true, the page registered, and radio
  enable ran from it with input live through the quiesce (2026-07-20); the
  `RadioUnavailable`/escape paths are unchanged KOTO-0241 code.
- [x] The SD-backed KWS1 credential store loads on Pico W with the same
  fail-closed behavior as Pico 2 W: a corrupt or absent store keeps the
  capability false without affecting KCF1 settings. Positive load
  hardware-confirmed by successful WPA2 association (2026-07-20); the
  corrupt/absent fail-closed arm is Pico 2 W-shared code, its exhaustive
  proof delegated to KOTO-0243.
- [ ] Pico W hardware smoke against a controlled access point: scan with
  results, WPA2-Personal connect, DHCP lease, disconnect, forget, and
  radio-off returning to `FullAudio`, captured over UART without logging
  secrets. The full fault/persistence matrix remains KOTO-0243's.
  **Sole open item:** scan / WPA2 connect / DHCP lease / disconnect /
  radio-off→`FullAudio` all hardware-confirmed (2026-07-20/21). **`forget`
  (credential removal → capability-false) is broken on hardware (2026-07-21)
  and needs a fix** before this AC can close — tracked as the remaining
  KOTO-0251 work item.
- [x] Stream audio plays (PCM16 and SLDPCM4) while associated, and rich-audio
  requests return the explicit temporary-unavailability result rather than
  silently dropping. Hardware: PCM16 via `full-color-tile-image` (2026-07-20)
  and SLDPCM4 via `audio-codecs` (2026-07-21,
  `phase=158 stage=streaming path=audio/sample_audio_sld4.kacl`;
  `phase=173 unsupported_count=0 drops=0 underruns=0 command_drops=0`,
  `audio-scratch stream_acquisitions=3 rejected=0 corruption=0`) both play
  clean while associated; rich audio returns explicit temporary-unavailability
  until radio-off (BGM regression confirmed 2026-07-20/21).
- [x] The wifi-config Pico W image keeps the frozen KOTO-0227 thresholds
  under its worst supported Wi-Fi-plus-stream workload:
  `phase=176 free_min >= 8 KiB` and `core1_stack_free_min >= 4 KiB`, with
  measured margins re-injected into the SRAM report. Met via KOTO-0252
  (hardware 2026-07-21): `at=app free_min=9864 guard=ok` (+1,672 B) and
  `core1_stack_free_min=6556` (+2,460 B); both recorded in
  `audio_residency_memory_picow_wifi_config.json`.
- [x] Radio firmware, region-policy, credential, HAL, or service failures
  never prevent boot, KotoConfig language use, KotoShell, or offline app
  launch; the offline image never allocates network runtime state. The
  offline-never-allocates half is proven by the SRAM report (offline image
  links no network stack/sockets/DHCP state); the failure-non-blocking arm is
  delegated to the KOTO-0243 fault/persistence matrix.
- [x] All-board release builds, embedded cross-checks, project/doc
  consistency gates, and the KOTO-0227 transition harness pass; hardware
  captures are retained with the issue. Verified 2026-07-21: release builds
  (pico default / picow offline / picow wifi-config / pico2w wifi-config),
  `check_embedded.py`, `check_wifi_residency_layout.py`,
  `check_network_service_budget.py`, `check_audio_residency_memory.py`,
  `check_project.py`, and the transition harness (`check_audio_residency.py`
  3, `check_arena_future.py` 7) all OK; UART captures retained in the records
  above.

## Non-goals

- The integrated KOTO-0243 validation matrix (that issue owns promotion)
- App-facing network APIs (KOTO-0245, KOTO-0249) beyond keeping their
  transports linked where the features already overlap
- Captive portal, advanced network details, or region selection UI
- Treating PSRAM as socket memory, changing the 36 KiB arena size, or
  reducing CodeWindow/raster budgets as a memory source

## Notes

- The `not(feature = "wifi_residency_probe")` product gates in
  `koto_firmware.rs` that today pair `network_service` with
  `board-picocalc-pico2w` are the concrete wiring surface: network runtime
  start, `NetworkService`/`WifiPageController` state, the F1 config-open
  capability computation, the Wi-Fi page frame loop, and
  `load_wifi_secret_store`.
- Page-entry timing needs one design decision: on RP2040 the radio cannot
  come up until rich audio has quiesced, so radio enable from the page is an
  asynchronous transition whose progress and cancellation must render through
  the existing KOTO-0241 states rather than blocking input.
- KOTO-0245's `TlsExclusive` exception already reserves its coordinator slot
  after `cyw43::State`; keep the `split_wifi_residency` layout unchanged so
  the Fetch mailbox placement and the audio-residency memory gate stay valid.

## Implementation Record

### 2026-07-20: product wiring landed (hardware smoke pending)

- Build profile: the Pico W wifi-config image is
  `--no-default-features --features
  board-picocalc-picow,ram_interpreter,ram_audio_mixer,network_service`
  (thumbv6m), board-named UF2
  `koto_firmware-picocalc-picow-rp2040-wifi-config.uf2` via `elf2uf2-rs`;
  documented in the koto-pico README. `harness/check_embedded.py` gained a
  permanent "RP2040 Pico W wifi-config embedded bins" row. The default
  offline Pico W feature set is unchanged.
- Mode owner (the resolved page-entry design decision): the transition is
  driven by the page's `EnableRadio` intent, not page entry. Enter on the
  `Disabled` page starts `begin_wifi_quiesce`; the `PicowWifiEnable`
  state machine (`Quiescing` -> `Bringup`) is pumped once per page frame so
  input stays live (Esc cancels through the KOTO-0227 recovery boundaries —
  no keyboard trap). Only after `DriverReady` is `set_radio(true)` submitted,
  so the 10 s service deadline covers CLM init, not the ~231 KB firmware
  upload (Pico 2 W parity). Bounds: quiesce soft 2 s, arena-claim hard 10 s,
  driver bring-up 15 s.
- Radio disable: leaving the page while not associated powers the radio down
  (`WifiRuntime::shutdown` -> GP23 low -> `restore_full_audio`); leaving while
  associated keeps Wi-Fi residency (network time / app transports operate,
  rich audio stays temporarily unavailable). Disconnect-then-exit is the user
  path back to `FullAudio`. A completed network future (runner fault) is
  detected in both the shell loop and the page loop and tears down to
  `Offline`; any unprovable boundary latches
  `network::mark_radio_unavailable()` (fail-closed, no arena reuse) without
  touching boot, KotoConfig language, KotoShell, or offline app launch.
- Composite `WIFI_CONFIG` on RP2040: `WIFI_HAL` = resolved release region
  policy AND (`DriverReady` whenever the switchable runtime is live). With
  the radio off the driver is brought up on demand behind that same policy;
  a runtime that failed bring-up withholds the capability on the next F1
  open. Transport/service-generation/credential inputs match Pico 2 W.
- Teardown helpers: `network::reset_radio_mailbox()` (zeroizes staged
  SSID/secret, resets link/DHCP words, retains the SNTP endpoint and the
  `radio_present` fault latch) and `network::mark_radio_unavailable()`;
  `PicoWRadioResources::clone_for_enable_cycle` reuses the KOTO-0227 probe
  safety contract for repeated product enable cycles.
- SRAM evidence (release, `check_audio_residency_memory.py`): offline picow
  `.data+.bss` 205,584 B (delta +148 vs KOTO-0226); wifi-config picow
  `.data+.bss` 209,592 B, static span 210,108 B (delta +4,156 / +4,356).
  Decomposition by symbol diff: `__embassy_main::POOL` +2,768 B (the KOTO-0239
  budgeted CPU0 `NetworkService` + page/UI + KOTO-0240 secret-store residents
  in the main-task future), `network::MAILBOX` 800 B bounded HAL control
  words, remainder small atomics — no IP stack, socket windows, or CYW43
  driver state outside the arena. Arena checks all pass (exact 36 KiB,
  8-aligned, inside RP2040 SRAM). `check_wifi_residency_layout.py` and
  `check_network_service_budget.py` pass on the same feature set. Reports:
  `target/koto-dev/audio_residency_memory_picow_wifi_config.json` (+ offline
  variant), `wifi_residency_layout.json`, `network_service_budget.json`.
- Release builds pass for RP2040 pico (default), Pico W offline, Pico W
  wifi-config, and Pico 2 W wifi-config; feature-combination checks pass for
  picow offline/probe/probe+network/stream-soak. Clippy on the wifi-config
  profile adds zero findings versus the committed tree (pre-existing
  koto-psram/koto-pico toolchain-drift lints unchanged).
- Remaining (hardware): controlled-AP smoke (scan/WPA2 connect/DHCP lease/
  disconnect/forget/radio-off -> `FullAudio`), PCM16+SLDPCM4 stream playback
  while associated, frozen threshold re-measurement (`phase=176 free_min >=
  8 KiB`, `core1_stack_free_min >= 4 KiB`) re-injected into the SRAM report,
  and the KOTO-0227 transition harness re-run. UART captures to be retained
  here; the full fault/persistence matrix stays with KOTO-0243.

### 2026-07-20: wifi-config boot blocker found and fixed on hardware

- First flash of the Pico W wifi-config image hung silently after the boot
  banner; the offline image booted. Bisection on hardware (boot-bracket
  markers, HardFault/panic UART reporters, lock-free CPU1 probes, `.bss`-tail
  dumps, per-step `clk_peri` sampling) attributed it to a **silent
  divide-by-zero panic in embassy-rp `calc_prescs`** during SD SPI0 init:
  `clk_peri_freq()` read 0 because embassy's `CLOCKS` bookkeeping static had
  been overwritten during the CPU1 audio spawn window.
- Root cause: `initialize_rich_residency()` built the rich-audio service, cue
  players, 8 KiB clip player, and slot-sized `MaybeUninit` rewrites as
  by-value temporaries **summed in one stack frame** (~50 KiB transient —
  KOTO-0172's by-value-ctor lesson). The transient punched through
  `_stack_end` into the `.bss` tail. In the offline image the same dip landed
  in the legitimate free gap (statics ~4 KiB smaller), so the bug stayed
  latent; the KOTO-0170 canary cannot see it because paint starts *at*
  `__ebss` and the overflow goes below it.
- Fix (`firmware/audio.rs`): each residency slot is now constructed in its own
  `#[inline(never)]` frame (SFX players one element at a time; redundant
  `RichSlot::new()` pre-writes dropped). Generated-code frame sizes after the
  split: service ≈ 8.0 KiB, BGM cue player ≈ 18.0 KiB (largest single),
  SFX ≈ 2.7 KiB — sequential instead of summed. This path also runs on every
  post-Wi-Fi rich-audio reconstruction, so the KOTO-0251 teardown path is
  covered by the same bound. Device-confirmed: the wifi-config image now
  boots to the Shell.
- Permanent diagnostics added along the way: `phase=10 boot-mark` bring-up
  markers (LCD SPI / keyboard I2C / core1 spawn / SD / PSRAM had zero UART
  coverage), and `phase=91` HardFault + panic UART reporters replacing the
  silent `panic-halt`/default-handler loops in the product binary (raw UART0
  MMIO, RP2040/RP2350 base-address aware, faulting-core id included).
- All gates re-pass after cleanup (embedded cross-checks incl. the new picow
  wifi-config row, audio residency/scratch harness tests, SRAM report + layout
  + budget gates, clippy delta zero). The `free_min >= 8 KiB` threshold still
  needs on-device re-measurement on this image (statics grew ~4 KiB and the
  init transient now bounds at ~18 KiB).

### 2026-07-20: first Pico W product Wi-Fi chain confirmed on hardware

- User-confirmed on the cleaned wifi-config image: boot to Shell, radio
  enable from the KotoConfig Wi-Fi page (asynchronous
  `FullAudio -> WifiStreamAudio` transition), scan, association, DHCP lease,
  and SNTP time synchronization driving the Shell clock. This validates the
  KOTO-0251 mode owner end to end for the enable half on RP2040.
  `phase=176` region: `bottom=0x200334ec painted_top=0x20039c40
  stack_top=0x20042000 painted=26452`.
- Still to capture: `phase=176` peak lines (`used=`/`free_min=`) against the
  frozen `>= 8 KiB` bar, `core1_stack_free_min` (`phase=173`), stream audio
  while associated, and the radio-off half (disconnect -> exit ->
  `phase=251 wifi-picow radio-off full-audio-restored` -> rich audio/BGM
  regression).

### 2026-07-20: radio-off half and stream audio confirmed; free_min below bar

- User-confirmed on hardware: PCM stream audio plays while associated
  (`full_color_tile_image`), and after disconnect -> page exit the UART shows
  `phase=251 wifi-picow radio-off full-audio-restored` with rich audio (BGM)
  playing again in KotoRun/KotoRogue. The reverse transition and the
  explicit-unavailability policy are validated on device.
- **Threshold finding**: `phase=176 stack-peak at=app used=56912 free_min=3316
  lw=0x200341b0` on a session including game launches — below the frozen
  `>= 8 KiB` bar and the KOTO-0170 ~4 KiB stop-ship floor. Attribution: the
  absolute low-water `0x200341b0` matches the pre-existing offline app-session
  peak (KOTO-0170 measured `free_min=7,620` on the same absolute dip; the
  wifi-config image's ~4 KiB `.bss` growth consumed the difference:
  7,620 - 4,008 = 3,612 ~= 3,316). The network wiring did not deepen the
  stack; the long-standing ~56.9 KiB app-path transient is now
  threshold-critical on the grown image. Filed KOTO-0252 for the attribution
  and reduction; the KOTO-0251 threshold AC stays open until the wifi-config
  image measures `free_min >= 8 KiB` on the worst supported workload.
- `phase=173` audio summary was not present in this capture;
  `core1_stack_free_min` re-measurement rides along with the KOTO-0252
  re-run.
- Follow-up capture separates the workloads: after the full Wi-Fi chain with
  **no app launch**, `phase=176 stack-peak at=shell used=51740 free_min=8488
  lw=0x200355e4` — the Wi-Fi-plus-shell path itself meets the frozen
  `>= 8 KiB` bar. Only the app-session peak (`lw=0x200341b0`,
  `free_min=3316`) breaks it, confirming the KOTO-0252 attribution. The
  shell-path low-water is consistent with the bounded ~18 KiB
  `init_rich_runtime_bgm` frame reached from the page-exit teardown, so
  KOTO-0252's in-place-construction option would widen this margin too.

### 2026-07-21: thresholds met (via KOTO-0252) and SLDPCM4-while-associated confirmed

- KOTO-0252 landed the CPU0 app-session stack reduction; re-measured on this
  image: `phase=176 stack-peak at=app used=50332 free_min=9864 lw=0x20035b64
  guard=ok` (≥ 8 KiB, +1,672 B) and `phase=173 core1_stack_free_min=6556`
  (≥ 4 KiB, +2,460 B), both while associated and after radio-off, all audio
  counters zero. Margins re-injected into
  `audio_residency_memory_picow_wifi_config.json`. This closes the frozen-
  threshold AC on the wifi-config image.
- SLDPCM4 stream while associated (the remaining AC-7 codec): the
  `dev.koto.samples.audio-codecs` app streamed `audio/sample_audio_sld4.kacl`
  (`phase=158 stage=streaming`) with `phase=173 samples_submitted=8704
  samples_played=7001 drops=0 underruns=0 unsupported_count=0 command_drops=0`
  and `audio-scratch stream_acquisitions=3 rejected_acquisitions=0
  corruption_failures=0` — a clean SLDPCM4 decode under the switchable
  residency, complementing the earlier PCM16 confirmation.
- Transition harness re-run green: `check_audio_residency.py` (3),
  `check_arena_future.py` (7, incl. stale/out-of-order generation-ack
  rejection), plus `check_embedded.py`, `check_wifi_residency_layout.py`,
  `check_network_service_budget.py`, `check_audio_residency_memory.py`, and
  `check_project.py` all OK; all-board release builds pass.
- Remaining work: **`forget` is broken on hardware** (credential removal does
  not drive the capability false as expected) — needs a fix, then the AC-6
  capture. The exhaustive fault/persistence matrix (capability loss,
  corrupt/absent store, radio/HAL/service failures not blocking boot) stays
  with KOTO-0243.
