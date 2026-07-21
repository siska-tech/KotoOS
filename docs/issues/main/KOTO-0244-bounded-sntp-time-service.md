# KOTO-0244: Bounded SNTP time service and Shell clock

- Status: done — Pico 2 W device-confirmed 2026-07-19
- Type: feature
- Priority: P1
- Requirements: FR-SHELL-5, FR-CONFIG-3, NFR-MEM-2, NFR-PORT-1, NFR-PORT-3, NFR-PORT-4, NFR-PORT-6, NFR-REL-3, NFR-DEV-5
- Related: KOTO-0084, KOTO-0124, KOTO-0223, KOTO-0227, KOTO-0239, KOTO-0243
- Roadmap: [KotoConfig Roadmap](../../planning/KOTOCONFIG_ROADMAP.md)

## Goal

After Wi-Fi reaches DHCP config-up, obtain UTC from an SNTP server through the
bounded NetworkService stack and publish a monotonic-backed wall clock to
KotoShell and filesystem timestamps. Keep synchronization optional,
nonblocking, allocation-free on device, explicit about its unauthenticated
trust level, and harmless to boot, audio, Shell, and offline applications when
DNS, UDP, the server, or the radio is unavailable.

## Acceptance Criteria

- [x] Add a portable `no_std`, fixed-capacity time-service model separating
  monotonic time from synchronized UTC. Its snapshot exposes validity, source,
  synchronization generation, age, and fixed-enum failure state without heap
  allocation or server-provided diagnostic text.
- [x] Implement one bounded SNTP client inside the firmware-owned
  NetworkService/embassy-net lifecycle using a fixed 48-byte request and
  bounded response buffer. No socket, DNS handle, or network-stack ownership is
  exposed to KotoConfig, KotoShell, or ordinary applications.
- [x] Validate response length, server mode, supported version, leap alarm,
  stratum, echoed originate timestamp/request identity, nonzero transmit time,
  and a supported calendar range before publishing a new generation. Reject
  malformed, stale, replayed, mismatched, or implausible replies.
- [x] Start synchronization only after DHCP config-up, keep at most one request
  active, enforce a bounded timeout, use capped retry backoff, and refresh no
  more frequently than a documented interval. Submissions and UI frames never
  wait for DNS, UDP, or wall-clock completion.
- [x] Store UTC internally and add a bounded public KotoConfig fixed UTC-offset
  setting (`-12:00..+14:00`, minute precision with validated increments).
  English/Japanese labels describe that daylight-saving transitions are not
  automatic; locale alone never guesses a timezone.
- [x] Drive `ShellState::set_clock` from synchronized UTC plus the configured
  offset, advancing from the monotonic clock between synchronizations. Repaint
  only the clock region when its displayed minute changes and preserve the
  existing unknown placeholder before the first valid synchronization.
- [x] Supply synchronized timestamps to the firmware filesystem `TimeSource`.
  Before synchronization or after invalid state, use an explicitly documented
  safe fallback rather than inventing current time or blocking SD access.
- [x] Treat SNTP as unauthenticated advisory display/filesystem time. It must
  not authorize updates, validate certificates, order security records, expire
  credentials, or become a trusted audit timestamp; logs contain only fixed
  status, generation, offset, and bounded timing metadata.
- [x] Capability loss, disconnect, DNS failure, timeout, malformed response,
  large time jump, and service teardown never prevent boot, language settings,
  KotoShell, audio, SD access, or offline app launch. Volatile socket/request
  state is cancelled and zeroized or reset on network-generation loss.
- [x] Add deterministic unit and KotoSim tests for epoch/calendar conversion,
  leap years, offset boundaries, minute rollover, first sync, resync, timeout,
  retry/backoff, malformed/replayed replies, capability loss, and clock-region
  damage without using the host network or host wall clock.
- [x] Validate the primary Pico 2 W profile on hardware through DHCP, SNTP
  synchronization, and Shell time display. The user accepted the Pico 2 W
  device result as the closure gate on 2026-07-19; Pico W-specific switchable
  Audio/Wi-Fi residency remains tracked by KOTO-0227.

## Validation

- Pico 2 W: device-confirmed by the user on 2026-07-19.
- Confirmed outcome: the network-backed system time is visible on the physical
  Shell. Pico W-specific residency work is not part of this closure decision.

## Dependencies

KOTO-0239 supplies the bounded embassy-net stack. Pico 2 W integration follows
KOTO-0243's product Wi-Fi path. Pico W hardware and switchable Audio/Wi-Fi
residency validation remains in KOTO-0227 and does not block this Pico 2 W
closure. This issue is a follow-on feature and does not block completion of
KOTO-0243.

## Non-goals

- Authenticated NTS, TLS certificate validation, or security/audit time
- Automatic daylight-saving or geographic timezone databases
- Exposing raw UDP, DNS, NTP packets, or server selection to applications
- Requiring network connectivity for boot, filesystem access, or Shell use
