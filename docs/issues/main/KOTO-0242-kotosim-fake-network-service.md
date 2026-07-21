# KOTO-0242: KotoSim deterministic fake NetworkService

- Status: done
- Type: test infrastructure
- Priority: P1
- Requirements: FR-CONFIG-3, NFR-PORT-1, NFR-REL-1, NFR-REL-5, NFR-DEV-5
- Related: KOTO-0223, KOTO-0224, KOTO-0239, KOTO-0240, KOTO-0241, KOTO-0243
- Roadmap: [KotoConfig Roadmap](../../planning/KOTOCONFIG_ROADMAP.md)

## Goal

Turn the checked-in KOTO-0224 fixture into an executable deterministic
NetworkService test double for portable service and KotoConfig development.
No simulator test or fake-service path may access the host network.

## Acceptance Criteria

- [x] Parse and strictly validate `koto.fake-network-service.v1`, all declared
  limits, network identities, capability inputs, ordered actions/snapshots,
  integer ticks, generations, request IDs, and fixed error/state enums.
- [x] Reject unknown schema versions, fields/operations/enums, duplicate names
  or IDs, invalid SSID/BSSID/security/credential data, decreasing ticks, stale
  expectations, over-capacity lists, and unconsumed actions/snapshots.
- [x] Implement a fake NetworkService runner driven only by explicit integer
  ticks and submitted commands; wall clock, RNG, DNS, radio, sockets, and host
  network APIs are absent from the execution path.
- [x] Replay exact ordered snapshots for capability absence,
  scan/connect/disconnect, authentication failure, scan cancellation, radio
  loss with stale completion, and forget commit from the v1 fixture.
- [x] Add scenarios/tests for command/event queue overflow, timeout boundaries,
  transient retry schedule, invalid input, empty/16-result scans, duplicate
  BSSID ordering, secret-store corruption, and generation/request wrap.
- [x] Integrate the fake with KotoSim's native KotoConfig launch path behind an
  explicit development/test option; default simulator startup remains offline
  and does not advertise `WIFI_CONFIG` accidentally.
- [x] Add semantic keyboard/state assertions for KOTO-0241 and deterministic
  framebuffer/golden coverage where visual output changes.
- [x] Add a repository check that parses every fake-network fixture and fails
  if host-network dependencies or nondeterministic fixture fields are added.
- [x] Document the exact focused command for replaying all fake service and
  KotoConfig Wi-Fi scenarios in CI and local development.

## Non-goals

- Simulating 802.11 frames, TCP/IP, signal propagation, or real credentials
- Treating fake success as PicoCalc hardware or network-stack validation

## Outcome

Completed on 2026-07-18. KotoSim now strictly parses and executes the checked-in
v1 fixture against the real portable `NetworkService` through deterministic
radio and credential doubles. The replay exposes only redacted public snapshots
and retained scan results, which are exercised directly by the KOTO-0241 Wi-Fi
page controller keyboard/state integration test.

`--fake-network PATH` provides a focused headless replay and, when explicitly
combined with `--window`, attaches the validated fixture to the native
KotoConfig development launch path. Default simulator startup remains offline
and does not enable `WIFI_CONFIG`. The project harness checks every committed
fake fixture for fixed integer fields and rejects host-network dependencies or
wall-clock/RNG/network APIs. The portable `net::tests` suite supplies the queue,
timeout, retry, scan-boundary, corruption, and wrap coverage shared by the fake.

Validation passed for the focused replay, portable service boundaries, existing
KotoConfig English/Japanese frame goldens, warnings-as-errors KotoSim Clippy,
the complete default workspace tests except for the pre-existing unrelated
`app_gallery_skk_candidate_replaces_reading_and_uses_standard_commit_cancel_keys`
failure, and `python harness/check_project.py`.
