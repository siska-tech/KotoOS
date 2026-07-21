# KOTO-0243: PicoCalc Wi-Fi configuration validation

- Status: in-progress
- Type: hardware validation
- Priority: P1
- Requirements: FR-CONFIG-3, NFR-MEM-1, NFR-MEM-2, NFR-PORT-4, NFR-PORT-6, NFR-REL-1, NFR-REL-3, NFR-REL-5, NFR-DEV-5
- Related: KOTO-0204, KOTO-0205, KOTO-0224, KOTO-0227, KOTO-0239, KOTO-0240, KOTO-0241, KOTO-0242
- Roadmap: [KotoConfig Roadmap](../../planning/KOTOCONFIG_ROADMAP.md)

## Goal

Validate the complete optional Wi-Fi configuration feature on Pico W/RP2040
and Pico 2 W/RP2350A after its component issues pass independently. Prove the
bounded service, secret behavior, page flow, failure isolation, and different
Audio/Wi-Fi residency policies using a controlled access point.

## Acceptance Criteria

- [ ] Build board-identifiable release artifacts with Wi-Fi enabled and
  disabled for Pico W and Pico 2 W; disabled artifacts preserve the accepted
  offline flash/SRAM behavior and omit `network.wifi`.
- [ ] Record the selected regulatory region/channel set and scan, connect,
  DHCP, disconnect, reconnect, forget, and radio-disable results against a
  controlled Open and WPA2-Personal AES access point without logging secrets.
- [ ] Exercise all KOTO-0224/KOTO-0241 states and English/Japanese keyboard
  flows on device, including invalid credentials, AP absence, timeout,
  cancellation, retry, radio/firmware failure, and capability loss while open.
- [ ] Verify credential reboot persistence, corrupt/torn slot fallback,
  redaction, forget commit, and factory reset on physical storage. Captures
  contain no passphrases, PSKs, or complete secret records.
- [ ] Pico W passes at least 100 `FullAudio -> WifiStreamAudio -> FullAudio`
  transitions and the KOTO-0227 five-minute alternating PCM16/SLDPCM4 plus
  bounded packet-activity soak with zero guard, stale-reference, task, DMA,
  transition, command-drop, and audio-underrun failures.
- [ ] After Pico W returns to FullAudio, BGM, SFX, runtime cue/clip, PCM stream,
  worker heartbeat, app launch/exit, Shell, language settings, SD, and PSRAM
  regressions pass with accepted CPU0/CPU1 stack margins.
- [ ] Pico 2 W demonstrates concurrent full Audio and Wi-Fi without inheriting
  the RP2040 service restriction and passes the same configuration/fault flows.
- [ ] Release ELF reports satisfy the KOTO-0224 36 KiB RP2040 and 64 KiB
  RP2350 ceilings, named allocation rows, flash/blob accounting, and the
  KOTO-0170 stop-ship SRAM/stack floor under worst supported workload.
- [ ] Removing/corrupting radio firmware, invalidating region policy, failing
  HAL/service/credential initialization, and forcing network teardown never
  prevent boot, KotoConfig language use, KotoShell, or offline app launch.
- [ ] Retain machine-readable memory reports, UART logs, fixture/test versions,
  board/module identity, artifact hashes, AP configuration, and a pass/fail
  matrix with the issue before completion.
- [ ] Project, embedded cross-build, focused fake/service/security/UI tests,
  formatting, and applicable full workspace checks pass.

## Dependencies

Implementation acceptance requires KOTO-0227 and KOTO-0239 through KOTO-0242.
This issue owns integrated hardware evidence; it does not absorb defects back
into the validation harness when their owning component issue remains open.

## Validation Record

### 2026-07-19: preflight started

- Working tree base: `bf6db8b` (KOTO-0241). Record the final implementation
  revision and rebuild all hashes when this product-integration slice commits.
- Controlled AP policy: one isolated Open AP and one WPA2-Personal AES AP;
  record only test aliases, channel, band, and regulatory region. Never retain
  SSIDs, passphrases, PSKs, or complete secret records in repository evidence.
- Artifact matrix:

  | Board              | Wi-Fi    | Profile                            | Build                            | Device          |
  | :----------------- | :------- | :--------------------------------- | :------------------------------- | :-------------- |
  | Pico W / RP2040    | disabled | product                            | pass                             | pending         |
  | Pico W / RP2040    | enabled  | switched-residency product         | blocked by KOTO-0227 integration | pending         |
  | Pico 2 W / RP2350A | disabled | concurrent product                 | pass                             | pending         |
  | Pico 2 W / RP2350A | enabled  | concurrent KotoShell Wi-Fi product | pass                             | basic flow pass |

- Preflight artifacts (SHA-256, provisional until final device-test build):

  | Artifact                                                |     Bytes | SHA-256                                                            |
  | :------------------------------------------------------ | --------: | :----------------------------------------------------------------- |
  | `koto_firmware-picocalc-picow-rp2040-offline.uf2`       | 1,831,936 | `6fbaf3a8891512a70b7994487313cf817711b13a168a89d37d7fbe3830570bc0` |
  | `koto_firmware-picocalc-pico2w-rp2350a.uf2`             | 1,846,272 | `19f5e768801467d2b0bbcbb9ebc056360cc2288bb3f5417c05bc202a7812fb49` |
  | `koto_firmware-picocalc-pico2w-rp2350a-wifi-config.uf2` | 2,585,088 | `21fdd1ad64b5634b9b3f7bb30da2e6a85d15ced0ae1b143ec30e7f312daf22af` |

- RP2040 release gates pass: 36,864-byte aligned switchable arena;
  13,296-byte CYW43 driver storage; 23,568-byte future/network reserve;
  9,264-byte IP stack against the 12,288-byte ceiling; and 1,770-byte
  NetworkService plus page controller against the 4,096-byte ceiling. The
  offline product reports `.data + .bss` 205,584 bytes and static span 205,896
  bytes. CPU0/CPU1 hardware margins remain uncaptured.
- `python harness/check_embedded.py` passes all retained RP2040, Pico W, and
  RP2350A binaries. The RP2350A enabled artifact now keeps the NetworkService
  runtime alive in the normal Shell path and opens the LCD Wi-Fi page from
  KotoConfig; its basic device flow is confirmed in the record below.
- Focused host gates pass: `koto-core` 305 tests, deterministic NetworkService
  integration 12 tests, English/Japanese KotoConfig goldens 2 tests, and
  `python harness/check_project.py`.

- Pico W hardware evidence remains blocked until KOTO-0227 completes its
  product transition owner. Pico 2 W can proceed to LCD and controlled-AP
  validation; credential reboot reuse and coordinated factory reset still need
  their final KOTO-0240 product flows.

### 2026-07-19: Pico 2 W product KotoConfig path

- Replaced the boot-time UART-probe invocation in the `network_service` product
  profile with a concurrent runtime retained by the normal KotoShell loop.
  Driver initialization failure keeps the offline Shell and language page
  usable and withholds the composite `WIFI_CONFIG` capability.
- KotoConfig now uses `new_with_capabilities` with live transport, HAL,
  NetworkService generation, SD presence, and credential-store availability.
  Selecting Wi-Fi opens `KotoConfigWifiUi` on the LCD and returns to the normal
  settings page on exit; UART is diagnostics only.
- The page advances the driver and service without waiting, submits its bounded
  intents, renders only reported damage, supports Tab/Shift-Tab, and stages
  credentials in `WifiSecretStore`. Successful association commits the staged
  profile; failure/cancel/exit zeroizes it; forget resolves the scan result to
  the durable profile id before committing erasure. A retained profile skips
  password entry without copying its secret into page storage; bounded retry
  borrows only the volatile staged provider view.
- Added `tools\build-rp2350a.ps1 -WifiConfig`; the offline build remains the
  default. The enabled release reports `.data` 82,712 bytes, `.bss` 178,248
  bytes, `.text` 555,972 bytes, and `.rodata` 653,068 bytes.

### 2026-07-19: first Pico 2 W product UI confirmation

- User-confirmed on hardware that the normal KotoShell settings path opens the
  LCD Wi-Fi page and that its basic Wi-Fi operation succeeds. This validates
  that the product path is no longer dependent on the UART-only probe.
- Visual-copy follow-up from the device session: credential entry now uses an
  instruction (`Please enter password` / `パスワードを入力してください`) above
  the masked `Password:` field, and the connected page uses `Status` / `状態`
  above the single `Connected` / `接続済み` value instead of repeating the same
  word on both lines.
- The broader controlled-AP fault matrix, reboot persistence, memory margins,
  and Pico W switched-residency soak remain pending; this confirmation does not
  close KOTO-0243 by itself.
