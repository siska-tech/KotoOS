# KOTO-0239: Bounded NetworkService and Embassy network integration

- Status: done
- Type: firmware feature
- Priority: P1
- Requirements: FR-CONFIG-3, NFR-MEM-2, NFR-PORT-3, NFR-PORT-4, NFR-PORT-6, NFR-REL-1, NFR-REL-3, NFR-REL-5
- Related: KOTO-0204, KOTO-0205, KOTO-0224, KOTO-0227, KOTO-0240, KOTO-0242, KOTO-0243
- Roadmap: [KotoConfig Roadmap](../../planning/KOTOCONFIG_ROADMAP.md)

## Goal

Implement the portable bounded NetworkService contract over the selected
CYW43 and Embassy network stack without exposing sockets or radio ownership to
KotoConfig or ordinary applications. Fit the complete RP2040 service into the
36 KiB switchable Wi-Fi arena established by KOTO-0227 and retain independent
bounded storage on RP2350 W profiles.

## Acceptance Criteria

- [x] Add fixed-capacity, `no_std` NetworkService types for radio, operation,
  scan result, status/error snapshot, request ID, generation, submission, and
  service progress using the exact KOTO-0224 limits and no general heap.
- [x] Implement nonblocking `set_radio`, `scan`, `connect`, `disconnect`,
  `forget`, `cancel`, `snapshot`, and budgeted `service` behavior; no call made
  by a KotoUI frame waits for radio, timer, storage, DHCP, DNS, or socket I/O.
- [x] Integrate `cyw43` 0.7, local `cyw43-pio`, and a compatible `embassy-net`
  with fixed packet/socket capacities, one active operation, four commands,
  eight events/status records, and at most 16 deterministic scan results.
  *(Hardware-validated scan on Pico 2 W; connect is still pending.)*
- [x] Enforce the specified 10/15/30/5/5-second deadlines and service-owned
  connect retry policy; authentication/input failures do not auto-retry.
- [x] Cancel and join driver/network futures before arena release, reject stale
  generations and late completions, close sockets, and land timeout/fault
  teardown in the KOTO-0227 safe offline state. *(Hardware-validated on Pico 2 W:
  teardown -> shutdown-ok -> offline-ok, then boot continues cleanly.)* *(Partial: service-level stale
  generation / late-completion rejection, cancellation, and timeout/fault
  handling land; driver/socket join and arena release are firmware.)*
- [x] Compute `WIFI_CONFIG` only from supported transport, initialized HAL,
  live NetworkService generation, and initialized credential provider; board
  names and individual board capability bits cannot promote it alone.
- [x] Keep regulatory region a release/product policy passed to the HAL before
  radio enable and reject missing/invalid policy without blocking offline boot.
- [x] Add host tests with fake HAL/credential providers covering capacities,
  sorting/deduplication, overflow, timeout, retry, cancellation, stale events,
  capability loss, and redacted fixed-enum errors.
- [x] Record release ELF `.text`, `.rodata`, `.data`, `.bss`, task/future,
  driver, stack/socket, service, guard, and blob costs for RP2040 and RP2350.
  RP2040 totals at most 36,864 B and each contract row remains within ceiling.
  *(`check_network_service_budget.py` records the IP-stack and service rows;
  `check_wifi_residency_layout.py` records the driver/reserve and the exact
  36 KiB arena.)*
- [x] Product builds with networking disabled retain current offline behavior
  and do not allocate NetworkService runtime state. *(Offline RP2040 product ELF
  contains zero embassy-net/smoltcp/cyw43/NetworkService symbols.)*

## Implementation Progress

### 2026-07-18: Portable bounded NetworkService core

Landed the portable, `no_std`, heap-free `NetworkService` in
[`src/koto-core/src/net.rs`](../../../src/koto-core/src/net.rs), the layer the
credential provider (KOTO-0240), KotoConfig page (KOTO-0241), and KotoSim fake
service (KOTO-0242) build on. This is the hardware-independent foundation; the
CYW43/Embassy transport binding and ELF accounting ride on the KOTO-0227
residency mechanism, whose radio bring-up is the current hardware frontier.

- Fixed-capacity types with no general heap: `Security`, `RadioState`,
  `OperationState`, `NetworkError` (13-variant redacted enum), `RequestId`
  (monotonic wrapping nonzero), `Generation`, `Ssid` (32 B + length),
  `RawScanResult`/`ScanResult`, `CredentialView`, `NetworkEvent`,
  `NetworkSnapshot`, `SubmitResult`, and `ServiceProgress`. Capacities are the
  exact KOTO-0224 limits: SSID 32 B, 16 scan results, 63 B credential (WPA2
  8..=63 printable ASCII; Open 0), 4 retained profiles, 8 status records, 4
  command queue, 8 event queue, 1 active operation.
- Non-blocking submissions (`set_radio`, `scan`, `connect`, `disconnect`,
  `forget`, `cancel`) that validate and enqueue only, returning
  `Accepted/Busy/InvalidInput/Unavailable/StaleGeneration` without radio I/O. A
  test proves submission never touches the HAL; the transport is kicked and the
  deadline armed only on the first `service` advance.
- Budgeted `service(now_ms, work_budget, hal, creds)` that owns driver polling,
  the 10/15/30/5/5-second deadlines, and the connect retry policy (two
  association retries at 1 s and 2 s for transient radio/link faults only;
  authentication and not-found never auto-retry). Scan ingestion dedups by
  BSSID keeping the strongest RSSI, sorts RSSI-descending then BSSID-ascending,
  and caps at 16.
- Cancellation (idempotent for the active request, queued-command removal),
  radio/capability loss that advances the generation and lands in
  `RadioUnavailable`, stale/late-completion discard, and volatile credential
  staging zeroization at every terminal boundary.
- Two portable driver seams the firmware and fake both implement: the
  `WifiHal` radio trait (begin/poll per operation, `HalFault` taxonomy) and the
  `CredentialProvider` trait (availability + bounded forget commit).
- 22 host tests with fake HAL/credential providers cover capacities, sort/dedup,
  16-result cap, credential validation, connect success/auth-failure/transient
  retry/retry-exhaustion/timeout, disconnect, forget commit and redacted store
  errors, command-queue overflow, cancellation of active and queued requests,
  radio-loss generation bump + zeroization, late-completion discard, bounded
  event queue, and reinitialize. `koto-core` `cargo fmt --check`, `clippy
  --all-targets -D warnings`, and all 241 tests pass.

### 2026-07-18: embassy-net linked and bounded IP-stack budget measured

Cleared the KOTO-0224 stack-selection gate: `embassy-net` links against this
tree's Embassy generation and the concrete cyw43 driver, and its bounded
storage cost is measured on both W boards.

- Added `embassy-net = "0.7"` as an optional dependency behind a new
  `network_service` feature. It resolves to `embassy-net 0.7.1` and dedupes onto
  the existing `embassy-time 0.5.1` and `embassy-net-driver 0.2.0`, pulling
  `smoltcp 0.12.0`. No version conflict; the whole Embassy generation unifies.
- Added `firmware::wifi_residency::net_stack` fixing the KOTO-0224 socket
  capacities (`StackResources<4>`: one DHCP control, one DNS, two bounded
  application sockets) and a `NetworkStackStorage` = resources + two
  1536 B RX / 1536 B TX application windows. A type-checked `build_network_stack`
  calls `embassy_net::new` against the concrete cyw43 `NetDriver` to prove the
  integration compiles/links.
- Added the `probe_network_service` compile/link + size probe. Cross-builds:
  - Pico 2 W (RP2350A, `thumbv8m.main-none-eabihf`): links.
  - Pico W (RP2040, `thumbv6m-none-eabi`): links.
  - Both report identical bounded IP-stack sizes:
    `StackResources<4>` = 3,120 B, full `NetworkStackStorage` = 9,264 B,
    `embassy-net` `Runner` value = 36 B (its poll frame is captured at runtime
    like the cyw43 runner). 9,264 B is within the KOTO-0224 RP2040 IP-stack row
    ceiling of 12,288 B.
- Confirmed the default offline feature set pulls no `embassy-net` or `smoltcp`
  (dependency-level check), so networking-disabled builds link no IP stack.

### 2026-07-18: cyw43-backed WifiHal and arena command loop

Wired the portable `NetworkService` to the real cyw43 radio through a
single-core mailbox so the async radio work stays quarantined in the residency
arena.

- Refined `koto_core::net::WifiHal::begin_connect` to pass the `Ssid` (cyw43
  `Control::join` joins by SSID string, not BSSID); updated the service call
  site, the fake HAL, and the host tests. All 22 `net` host tests plus koto-core
  `clippy`/`fmt` pass.
- Added `firmware::network` (gated on `network_service` + a W board): a bounded
  `critical_section`-guarded static mailbox, a zero-sized `Cyw43WifiHal`
  implementing the sync `WifiHal` over it, a `StubCredentialProvider`
  (placeholder until KOTO-0240), a `service_network` entry point, and the async
  `radio_command_loop` that owns cyw43 `Control` and drives
  `init`/`set_power_management`/`scan`/`join`/`leave`. Scan maps `BssInfo` to
  bounded `RawScanResult` (privacy bit -> `Wpa2PersonalAes`/`Open`, i16 RSSI
  clamped), and a cancel aborts an in-progress scan.
- Added `wifi_residency::cyw43_network_future`, the product lifecycle future that
  brings CYW43 up in the arena and joins the cyw43 runner with the command loop
  (stock ~20 ms WL_ON pre-delay, not the KOTO-0227 diagnostic one-second one).
- Both boards cross-compile the full binding: Pico 2 W (RP2350A) and Pico W
  (RP2040). Because CPU0 runs both `NetworkService::service` and the command loop
  cooperatively (CPU1 owns audio), the mailbox needs no cross-core sync.

### 2026-07-18: embassy-net Stack in the arena and Pico 2 W boot probe

- `cyw43_network_future` now builds the bounded embassy-net `Stack` from the
  cyw43 `NetDriver` and a `NetworkStackStorage` local (living in the
  generation-owned arena future frame) and joins the cyw43 runner, the
  embassy-net runner, and the command loop. DHCP runs via the net runner; the
  `Stack` handle is retained for the bounded application sockets a later step
  will create. Added a fixed `NETWORK_RANDOM_SEED` to be replaced by a hardware
  entropy source before release.
- Added the Pico 2 W `run_network_service_probe` boot path (gated on
  `network_service`, mutually exclusive with `wifi_residency_probe`). It installs
  `cyw43_network_future` in a dedicated 36 KiB arena, constructs a
  `NetworkService`, and drives one `set_radio` + `scan`, logging only redacted
  results (`ssid_len`, RSSI, security flag) under a `phase=239` marker. Added the
  `-NetworkServiceProbe` switch to `tools/build-rp2350a.ps1`.
- Cross-build matrix passes: koto_firmware for Pico 2 W with `network_service`
  (monomorphizes `cyw43_network_future`), Pico 2 W offline, the retained
  `wifi_residency_probe`, and the default RP2040 product. On-hardware scan on
  Pico 2 W is the next validation step.

### 2026-07-18: first hardware scan on Pico 2 W

The `phase=239` boot probe on Pico 2 W reached `driver-ready`, `radio-enabled`,
and `scan-ok count=7`, then `shutdown-ok`. This is the first end-to-end hardware
proof that the bounded `NetworkService` drives a real cyw43 scan through the
CPU0 mailbox and arena command loop.

- The seven results are RSSI-descending (`-73, -74, -74, -82, -82, -88, -91`),
  confirming the deterministic sort and the 16-cap / BSSID-dedup path on real
  radio data. Visible (`ssid_len=14`) and hidden (`ssid_len=0`) APs both appear,
  proving the packed `BssInfo` SSID/length copy is correct. Only redacted fields
  (`ssid_len`, RSSI, security flag) are logged.
- The first run timed out because the arena future was polled every 5 ms, so the
  ~231 KB CYW43 firmware upload plus CLM overran the 10 s radio-enable deadline.
  Fixed by bursting 64 arena polls with `yield_now` and waiting for `DriverReady`
  before starting the service clock (so the deadline covers only CLM init).
- `future-bytes=15048` (cyw43 driver future plus the 9,264 B embassy-net stack
  storage), within the 23,568 B arena reserve.

### 2026-07-18: interactive `network.wifi` page connect probe (KOTO-0241)

To validate connect without compiling any credential into the image, the
`phase=239` scan probe was replaced by a `phase=241` interactive page probe
driven by the portable `network.wifi` controller (KOTO-0241). It renders the
redacted page to UART and takes PicoCalc keyboard input, so the user scans,
selects an AP, types the password, and connects on real hardware.

- Added the portable `WifiPageController` in `koto-core::net_ui`: a
  fixed-capacity, heap-free state machine over the nine KOTO-0224 page states
  that turns `NetworkSnapshot`s and keyboard keys into bounded `WifiIntent`s,
  with a masked 63-byte credential zeroized on submit/cancel/exit/capability
  loss and auth-failure return to `CredentialEntry`. 10 host tests;
  koto-core clippy/fmt/251 tests pass.
- `run_network_service_probe` now maps PicoCalc keycodes to `WifiKey`, drives the
  controller each frame, submits the resulting intent to the `NetworkService`
  (`set_radio`/`scan`/`connect`/`disconnect`/`forget`/`cancel`), and renders the
  page on change. Credential bytes are never logged. Build with
  `tools\build-rp2350a.ps1 -NetworkServiceProbe`.
- Cross-build matrix still passes (Pico 2 W network_service, offline, and default
  RP2040). On-hardware connect against a controlled AP is the next validation.

### 2026-07-18: first hardware connect via the page probe on Pico 2 W

The `phase=241` page probe completed a full interactive connect on Pico 2 W:
`driver-ready -> page=Disabled -> Scanning -> Results (8 APs) -> select ->
CredentialEntry (8-byte password) -> connect-submitted -> Connecting ->
Connected`. This is the first end-to-end hardware proof of association with a
real WPA2 AP through the bounded `NetworkService` and the `network.wifi`
controller, with no credential compiled into the image.

- The eight results are RSSI-descending (`-28, -67, -67, -67, -88, -88, -90,
  -91`); the strongest was a secured `ssid_len=8` network. Only `ssid_len`,
  RSSI, and the security flag were logged; the password rendered only as an
  increasing length.
- `NetworkService` `Connected` means associated (cyw43 `Control::join` returned
  Ok). DHCP runs in the background via the embassy-net runner but the probe does
  not yet surface IP config-up.

### 2026-07-18: embassy-net DHCP status in the page probe

- `cyw43_network_future` now joins a fourth future that polls the embassy-net
  `Stack` (`is_link_up` / `is_config_up` / `config_v4`) every 200 ms and
  publishes link/config/IPv4 octets into the network mailbox;
  `firmware::network` exposes `publish_dhcp_status` / `dhcp_status`.
- The `phase=241` probe prints `dhcp-up ip=A.B.C.D link=1` once after
  `Connected`, so association can be confirmed as IP-ready. Cross-build matrix
  still passes.
- Hardware confirmed on Pico 2 W: after `Connected`, the probe reported
  `dhcp-up ip=10.240.1.45 link=1` — a real DHCP-assigned address with link up.
  This proves the bounded embassy-net stack acquires an IP through the residency
  arena, completing scan -> associate -> DHCP end to end.

### 2026-07-18: graceful lifecycle teardown into the safe offline state

- Added `NetworkService::quiesce_offline`: cancels the active request, drains the
  command and event queues, clears results, zeroizes credential staging, and
  advances the generation so any outstanding handle is rejected as stale, landing
  in `RadioUnavailable`. Host test covers state clearing, generation advance,
  staging zeroization, and stale-handle rejection (koto-core 33 net tests pass).
- The `phase=241` probe now tears down gracefully on exit: it issues
  `disconnect` and pumps until the service leaves the connected states (5 s cap),
  calls `quiesce_offline`, clears the credential, then `WifiRuntime::shutdown`
  cancels/joins the cyw43 and embassy-net futures (closing the DHCP/DNS sockets
  with the dropped `Stack`) and powers the radio down before returning the arena.
  It emits `teardown`, `shutdown-ok`, and `offline-ok`/`offline-failed` (the
  latter checks the KOTO-0227 `Offline` lifecycle phase = GP23 low).
- Hardware confirmed on Pico 2 W: after `Connected`, exit produced
  `teardown -> shutdown-ok -> offline-ok`, and boot then continued cleanly into
  PSRAM discovery and the audio diagnostic, proving the arena was returned intact.
  The full lifecycle scan -> connect -> DHCP -> graceful teardown -> safe offline
  is now validated end to end.

### 2026-07-18: composite `WIFI_CONFIG` runtime computation

- Added `koto_core::WifiConfigInputs` (in `config`): the four KOTO-0224 runtime
  inputs (supported transport, initialized HAL, live NetworkService generation
  matching the lifecycle owner, initialized credential provider). `capability()`
  maps each live input to its bit and `wifi_config()` is true only when all four
  hold. A stale NetworkService generation, a single bit, or a board declaration
  alone never promotes the composite. 5 host tests (all-four, each-missing,
  single-bit/board, stale generation, page-registry feed); koto-core clippy/fmt
  pass.
- The `phase=241` probe logs `wifi-config=1` after `DriverReady`, exercising the
  composite with the real live inputs (board transport, HAL ready, live service
  generation, stub provider) on Pico 2 W.

### 2026-07-18: regulatory-region release policy gate

- Added `koto_core::RegulatoryRegion` (a signed release/product policy, never a
  KotoConfig field or user override) with `RegionError`, `resolve`, and
  `permits_radio_enable`. Absent (`None`) and unsupported codes are rejected; the
  supported v1 set is `XX` (worldwide), `JP`, `US`. 2 host tests.
- The firmware carries `PRODUCT_REGION` (default `XX`, matching cyw43 0.7's fixed
  `WORLD_WIDE_XX` domain). The command loop validates it before `Control::init`
  on `SetRadio(true)`: an absent/invalid policy reports `Firmware` and refuses to
  initialize the radio, keeping `WIFI_CONFIG` false without blocking boot. The
  probe logs `region=XX wifi-config=1`, and the composite's `hal_initialized`
  input is now gated on the resolved region.

### 2026-07-18: release ELF budget accounting and offline-clean verification

- Extended `probe_network_service` with `NETWORK_SERVICE_SIZE` and
  `NETWORK_PAGE_CONTROLLER_SIZE` symbols and added
  `harness/check_network_service_budget.py` (parser self-test registered in
  `check_all.py`). It maps the KOTO-0224 RP2040 rows this issue owns to their
  ceilings and emits `koto.network-service-budget.v1`.
- Measured on the RP2040 (`thumbv6m`) probe ELF: IP-stack storage **9,264 B**
  (StackResources 3,120 + two 1536/1536 socket windows 6,144) within the
  12,288 B ceiling; NetworkService **992 B** + page controller **778 B** =
  **1,770 B** within the 4,096 B ceiling; embassy-net runner value 36 B. The
  CYW43 driver storage (13,296 B) and the exact 36 KiB switchable arena stay
  gated by `check_wifi_residency_layout.py`; the network residency lives inside
  that arena. Sizes are identical on RP2350 (both 32-bit ARM).
- Criterion 10: the offline RP2040 product ELF
  (`--features board-picocalc-pico`) contains **zero**
  embassy-net/smoltcp/cyw43/NetworkService symbols, confirming networking-disabled
  builds allocate no network runtime state.

All ten acceptance criteria are now met. Remaining before closure is the
KOTO-0227-scoped hardware soak (100 residency transitions and the five-minute
Wi-Fi-plus-stream audio soak) on Pico W, tracked with that issue.
- Compute runtime `WIFI_CONFIG` from supported transport, initialized HAL, live
  NetworkService generation, and initialized credential provider.
- Enforce the regulatory-region release policy passed to the HAL before enable.
- Record the full release ELF `.text`/`.rodata`/`.data`/`.bss` and per-row
  network allocation costs for RP2040 (<= 36,864 B total) and RP2350 once the
  live stack and service occupy the arena.

## Non-goals

- Credential persistence or password UI
- A public Koto app socket/network ABI
- HTTP, captive portal, WPA3/SAE, enterprise/EAP, WPS, or raw 64-hex PSKs
- Replacing KOTO-0227 residency-transition and audio-stream soak ownership
