# KotoConfig Wi-Fi extension contract

- Status: design frozen by KOTO-0224
- Scope: optional OS-owned configuration for supported radio boards
- Fixture: [`network_service_v1.json`](../../harness/fixtures/network_service/network_service_v1.json)

## Decision

KotoConfig talks only to a bounded `NetworkService`. It never owns CYW43,
Embassy tasks, an IP stack, sockets, credentials, or board identity. The first
implementation uses the existing Rust `cyw43` 0.7 transport and adds
`embassy-net` from the same Embassy generation after a target ELF measurement
passes the budgets below. This design does not enable networking in an MVP
build and does not expose networking to ordinary applications.

### Stack and driver comparison

| Candidate | Firmware/code cost | Internal SRAM and executor cost | Radio firmware | License and integration | Decision |
| :-- | :-- | :-- | :-- | :-- | :-- |
| `cyw43` 0.7 + local `cyw43-pio` 0.10 + `embassy-net` | Rust code stays in flash/XIP. Exact incremental `.text`/`.rodata` is an implementation gate because `embassy-net` is not yet linked. | Existing Pico W ELF measures 13,296 B for CYW43 state, handles, and 512 B runner poll scratch. Runner future, IP stack, sockets, and service must fit the remaining 23,568 B RP2040 arena. Uses the existing Embassy executor/time model. | 43439 A0 Wi-Fi blob is 231,077 B plus CLM/NVRAM, all in flash. | Embassy/CYW43 Rust crates are MIT or Apache-2.0; the redistributed Infineon/Cypress blobs retain their separate vendor terms and require a release notice/license audit. One async model and the existing driver boundary. | Selected, conditional on measured target budgets. |
| `cyw43` + direct `smoltcp` integration | Similar protocol code in flash, but KotoOS must write and retain the device, timer, polling, and socket glue already supplied by Embassy. | Can be bounded, but duplicates executor wake/timer integration and has no demonstrated SRAM saving in this repository. | Same blob and vendor obligations. | `smoltcp` is 0BSD. More KotoOS-owned unsafe/lifetime-sensitive integration surface. | Rejected unless ELF evidence shows a material SRAM benefit. |
| Pico SDK/CYW43 + lwIP through C FFI | Pulls the Pico SDK, lwIP, C runtime glue, and bindings into the firmware image. | Separate callback/threading model, C pools, and synchronization must coexist with Embassy; teardown from the switchable RP2040 arena is harder to prove. | Same class of vendor blob and terms. | lwIP is BSD-style; Pico SDK is BSD-3-Clause, with separate blob terms. Adds a second toolchain/runtime boundary. | Rejected for the first implementation. |

The implementation issue records release ELF deltas for `.text`, `.rodata`,
`.data`, `.bss`, each named network allocation, executor task storage, and blob
bytes on both MCU targets. Estimates cannot promote `WIFI_CONFIG`.

## Composite capability

`WIFI_CONFIG` is true only when all four inputs are true at runtime:

1. the selected board profile declares a supported radio transport;
2. the Wi-Fi HAL initialized and can be quiesced by its lifecycle owner;
3. a compiled `NetworkService` is alive for the current lifecycle generation;
4. a credential provider initialized its bounded secret namespace.

`WIFI`, `AUDIO_WIFI_CONCURRENT`, a W-suffixed artifact name, or an initialized
IP stack alone is insufficient. A build may contain all code while reporting
`WIFI_CONFIG=false`. KotoConfig constructs its fixed page registry from the
composite value. It normally omits `network.wifi`; if capability is lost while
the page is open, the controller enters `RadioUnavailable` and permits exit.

## Bounded data model

All byte limits are octets, not displayed characters. No network-owned state
uses the general heap.

| Item | Limit | Rule |
| :-- | --: | :-- |
| SSID | 32 B | Arbitrary 802.11 bytes plus explicit length; display invalid UTF-8 escaped, never as a C string. |
| Scan results | 16 | Keep strongest unique BSSID entries; deterministic sort by RSSI descending, then BSSID ascending. |
| BSSID | 6 B | Stored for result identity; never treated as a credential. |
| Security modes | 2 | `Open` and `Wpa2PersonalAes`. WEP, enterprise/EAP, WPA1, WPA3/SAE, WPS, and captive-portal setup are unsupported in v1. |
| Credential | 63 B | WPA2 passphrase is 8..63 printable ASCII bytes. Open requires zero bytes. Raw 64-hex PSKs are deferred. |
| Retained profiles | 4 | Credential provider owns fixed slots; NetworkService receives only one operation-scoped secret view. |
| Status history | 8 | Public redacted records; oldest is overwritten. No SSID bytes are copied into an error message. |
| Command queue | 4 | Fixed FIFO; overflow returns `Busy` immediately. |
| Event queue | 8 | Fixed FIFO carrying request ID and state/error enum only. |
| Active operation | 1 | Scan, connect, disconnect, forget, and radio transitions are serialized. |

`NetworkSnapshot` contains generation, monotonically wrapping nonzero request
ID, radio state, operation state, connected profile/result identity, result
count, retry count, deadline ticks, last redacted error, and queue depths.
KotoConfig copies at most one snapshot per frame.

The regulatory country/region is a signed release/product policy selected for
the destination market and passed to the radio HAL before enable. It is not a
KotoConfig field, credential attribute, SSID-derived value, or user override.
An absent/invalid region prevents radio initialization and therefore keeps
`WIFI_CONFIG` false. Device validation records the effective region and channel
set without logging credentials or scan payloads.

## NetworkService contract

The portable interface is poll-driven and nonblocking. Methods validate and
enqueue work, returning `Accepted(request_id)`, `Busy`, `InvalidInput`,
`Unavailable`, or `StaleGeneration` without waiting for radio I/O:

```text
snapshot() -> NetworkSnapshot
set_radio(enabled) -> SubmitResult
scan() -> SubmitResult
connect(result_id, security, credential_view) -> SubmitResult
disconnect() -> SubmitResult
forget(profile_id) -> SubmitResult
cancel(request_id) -> SubmitResult
service(now_ticks, work_budget) -> ServiceProgress
```

`service` is called outside painting with a bounded poll/work budget. It never
loops until completion. The service owns driver polling, deadlines, connection
lifetime, and retries. KotoConfig owns no timers and never sleeps.

| Operation | Deadline | Retry ownership |
| :-- | --: | :-- |
| Radio enable/disable | 10 s | No automatic retry. |
| Scan | 15 s | No automatic retry; user may request another scan. |
| Connect | 30 s | Service may retry association twice at 1 s and 2 s only for transient radio/link errors. Authentication and invalid credentials never auto-retry. |
| Disconnect | 5 s | No retry; timeout forces the lifecycle owner toward offline. |
| Forget | 5 s | No retry; success requires credential-provider commit acknowledgement. |

Cancellation is idempotent for the current request. It stops retries, asks the
driver to cancel, zeroizes operation credential staging, and emits one terminal
`Cancelled` snapshot. A late driver completion with an old generation/request
ID is discarded. Lifecycle shutdown cancels the active request, drains queues,
closes sockets, joins the runner, and only then releases arena storage.

Errors are fixed enums: `Busy`, `InvalidInput`, `UnsupportedSecurity`,
`RadioUnavailable`, `FirmwareUnavailable`, `CredentialStoreUnavailable`,
`AuthenticationFailed`, `NetworkNotFound`, `LinkLost`, `Timeout`,
`Cancelled`, `StorageCorrupt`, and `Internal`. They contain no free-form driver
strings, SSIDs, credentials, keys, packet bytes, or memory addresses.

## SRAM ceilings

These are hard internal-SRAM ceilings, including padding. PSRAM may hold
pointer-free download bodies or caches later, but never driver channels,
futures, socket windows, credentials, or active service state.

| Owner | RP2040 Pico W | RP2350 W boards |
| :-- | --: | --: |
| CYW43 state/handles/poll scratch | 13,296 B measured | 16,384 B ceiling, remeasure |
| Runner future and executor storage | 4,096 B | 8,192 B |
| IP stack, packet metadata, DHCP/DNS, and bounded sockets | 12,288 B | 24,576 B |
| NetworkService, scan/results, queues, and redacted status | 4,096 B | 4,096 B |
| Alignment, guards, and measured headroom | 3,088 B | 12,288 B |
| **Total** | **36,864 B (36 KiB)** | **65,536 B (64 KiB)** |

RP2040 gets one DHCP/DNS control socket and at most two application-internal
TCP/UDP sockets in the reserved stack allocation; no app socket API exists.
The final socket/window sizes are compile-time constants chosen by the HAL
implementation and reported by ELF. Exceeding any row fails the build/report;
borrowing audio, UI, stack-canary, or general heap margin is forbidden. The
RP2040 product must additionally remain above the KOTO-0170 measured stop-ship
floor under the KOTO-0227 Wi-Fi-plus-stream soak.

## Credential threat model

Assets are WPA2 passphrases, SSIDs associated with them, and profile metadata.
Attackers include a reader of the SD card/flash, diagnostic output, crash dump,
stale SRAM, or a discarded device. Radio protocol attacks and compromised APs
remain network-stack concerns, not storage confidentiality claims.

- Credentials use a versioned, checksummed, fixed-size secret namespace
  separate from `KCF1`, KUC1, apps, and ordinary file enumeration. Two-slot
  commit detects torn/corrupt writes; corruption disables Wi-Fi profiles and
  preserves offline boot rather than guessing or exposing bytes.
- RP2040/Pico W and the listed RP2350 W profiles provide no KotoOS-managed
  hardware-backed confidentiality. At-rest obfuscation or a device-unique ID
  is not encryption. Documentation must state that physical storage access can
  recover credentials. Optional authenticated encryption is permitted only
  when a separately provisioned non-exportable key is actually available.
- Secret buffers use fixed lengths, are zero-initialized, and are volatile-
  zeroized after connect submission, cancellation, failure, forget, page exit,
  and arena teardown. Copies are forbidden outside provider and operation
  staging. Zeroization reduces accidental retention; it is not a proof against
  compiler, DMA, wear-leveling, or physical acquisition.
- Logs, status, telemetry, panic/crash dumps, screenshots, fake fixtures, and
  support exports contain only profile/result IDs, security enum, lengths, and
  redacted errors. Password fields render bullets and never reveal/replay text.
- Factory reset erases both secret slots, verifies absence/invalidity, clears
  RAM staging and profiles, then resets public configuration independently.
  Forget commits erasure before reporting success. Flash/SD wear remanence means
  secure physical erasure is not guaranteed.
- Credential-provider initialization, read, validation, or commit failure sets
  its capability input false or reports a bounded storage error. It never
  blocks boot and never falls back to the public settings record.

## `network.wifi` page

The page uses these states: `Disabled`, `Scanning`, `Results`,
`CredentialEntry`, `Connecting`, `Connected`, `Failed`, `ForgetConfirm`, and
`RadioUnavailable`. At most 16 result rows exist; no retained history grows.

| State | Primary text `en-US` / `ja-JP` | Keyboard flow |
| :-- | :-- | :-- |
| Disabled | `Wi-Fi is off` / `Wi-Fi はオフです` | Enter enables; Esc returns. |
| Scanning | `Scanning...` / `検索中...` | Esc cancels; completion opens Results. |
| Results | `Networks` / `ネットワーク` | Up/Down selects, Enter chooses, `R` rescans, Esc disables/returns according to prior state. |
| CredentialEntry | `Password` / `パスワード` | Text input edits a masked field; Backspace deletes; Enter connects only at valid length; Esc zeroizes and returns. |
| Connecting | `Connecting...` / `接続中...` | Esc cancels and returns to Results. |
| Connected | `Connected` / `接続済み` | Enter opens disconnect action; `F` opens ForgetConfirm; Esc returns. |
| Failed | `Connection failed` / `接続に失敗しました` | Enter retries by submitting a new request; Esc returns to Results. Authentication failure returns through CredentialEntry with an empty field. |
| ForgetConfirm | `Forget this network?` / `このネットワークを削除しますか?` | Left/Right selects cancel/forget, Enter confirms, Esc cancels. |
| RadioUnavailable | `Wi-Fi unavailable` / `Wi-Fi を利用できません` | Enter retries capability initialization only when lifecycle owner permits; Esc always exits. |

Tab/Shift-Tab follow the same focus order as directional navigation. Labels
come from complete `en-US` and `ja-JP` resource tables; raw driver text is never
displayed. Loss of capability while editing zeroizes the field before entering
`RadioUnavailable`.

## Failure isolation and fake service

Radio firmware, transport, HAL, stack, service, and credential provider each
fail closed. Before KotoConfig registry construction, failure omits the page.
After entry, failure changes only this page to `RadioUnavailable`. KotoOS boot,
`system.language`, KotoShell, SD/PSRAM, and offline app launch continue. No
network task may be a boot dependency or hold the UI executor hostage.

The v1 fake fixture is deterministic: input actions occur at integer ticks,
outputs are ordered snapshots, request IDs are fixed, and no wall clock, RNG,
DNS, radio, or host network is used. It covers capability absence, successful
scan/connect/disconnect, authentication failure, cancellation, radio loss, and
forget. Later simulator tests must reject unconsumed actions and snapshot
differences.

## Follow-up issues

Implementation remains split into independently verifiable work:

1. **[KOTO-0239](../issues/main/KOTO-0239-bounded-network-service-embassy-net.md), HAL/service:** selected CYW43 + Embassy network integration, fixed socket
   capacities, lifecycle cancellation/join, ELF accounting, and host unit tests.
2. **[KOTO-0240](../issues/main/KOTO-0240-wifi-secret-credential-provider.md), secret persistence:** two-slot provider, corruption/torn-write behavior,
   zeroization instrumentation, forget, and factory reset.
3. **[KOTO-0241](../issues/main/KOTO-0241-kotoconfig-wifi-page.md), KotoConfig page:** fixed controller/resources and keyboard/accessibility
   tests against fake snapshots only.
4. **[KOTO-0242](../issues/main/KOTO-0242-kotosim-fake-network-service.md), simulator:** fake service parser/runner, deterministic scenarios, and
   golden state/keyboard tests with host networking prohibited.
5. **[KOTO-0243](../issues/main/KOTO-0243-picocalc-wifi-config-validation.md), PicoCalc validation:** Pico W switched-residency and Pico 2 W concurrent
   builds, scans/connects against a controlled AP, timeout/cancel/fault cases,
   SRAM/stack reports, 100 transitions, and the KOTO-0227 soak.

No follow-up may combine credential persistence with general app storage or
make Wi-Fi a prerequisite for an offline release.