# KOTO-0224: KotoConfig optional Wi-Fi extension design

- Status: done
- Type: research
- Priority: P2
- Requirements: FR-CONFIG-3, NFR-MEM-2, NFR-PORT-3, NFR-PORT-4, NFR-PORT-6, NFR-REL-1, NFR-REL-5
- Related: KOTO-0204, KOTO-0205, KOTO-0206, KOTO-0223, KOTO-0227, KOTO-0239, KOTO-0240, KOTO-0241, KOTO-0242, KOTO-0243
- Roadmap: [KotoConfig Roadmap](../../planning/KOTOCONFIG_ROADMAP.md)

## Goal

Freeze a safe, bounded extension contract for configuring Wi-Fi from KotoConfig
on future radio-enabled firmware without coupling the settings UI to CYW43,
board identity, credentials, or a particular network stack.

## Acceptance Criteria

- [x] Compare viable embedded network stacks/drivers for the supported W boards
  and record firmware, SRAM, radio-firmware, executor, and licensing costs.
- [x] Define the composite `WIFI_CONFIG` capability: board radio transport,
  initialized Wi-Fi HAL, compiled NetworkService, and credential provider are
  all required; a W-suffixed board name alone is insufficient.
- [x] Specify a bounded asynchronous NetworkService API for radio state, scan,
  connect, disconnect, forget, status/error snapshots, cancellation, timeout,
  and retry ownership without blocking KotoUI frames.
- [x] Fix maximum SSID bytes, scan result count, security modes, credential
  length, status records, queues, and total RP2040/RP2350 SRAM budgets.
- [x] Produce a credential-storage threat model covering at-rest limitations,
  redaction, zeroization, logs/dumps, corruption, factory reset, and the absence
  of hardware-backed confidentiality on applicable boards.
- [x] Define the `network.wifi` page states and keyboard flow for disabled,
  scanning, results, credential entry, connecting, connected, failed, forget,
  and radio-unavailable conditions in `en-US` and `ja-JP`.
- [x] Specify failure isolation: unavailable firmware/radio/network service
  hides or disables only the Wi-Fi page and never prevents KotoOS boot, language
  settings, Shell use, or offline app launch.
- [x] Add deterministic fake-NetworkService fixtures for later simulator tests;
  no real network access is required for this design issue.
- [x] Split subsequent implementation into independently verifiable HAL/service,
  secret persistence, KotoConfig page, simulator, and PicoCalc validation issues.
- [x] Update board capability, configuration, security, and validation docs and
  pass `python harness/check_project.py`.

## Notes

This is a future-readiness gate, not authorization to enable networking in the
current MVP. KotoConfig remains usable when this issue is unimplemented.

## Outcome

Completed on 2026-07-18. The normative design is the
[KotoConfig Wi-Fi extension contract](../../architecture/KOTOCONFIG_WIFI_EXTENSION.md).
It selects the existing CYW43/Embassy family subject to release-ELF gates,
freezes the composite capability, API, capacities, SRAM ceilings, credentials
threat model, bilingual page flow, and failure isolation, and provides the
deterministic
[`network_service_v1.json`](../../../harness/fixtures/network_service/network_service_v1.json)
fixture. No product networking, credential persistence, or Wi-Fi UI was enabled.

Follow-up implementation is tracked by
[KOTO-0239](KOTO-0239-bounded-network-service-embassy-net.md),
[KOTO-0240](KOTO-0240-wifi-secret-credential-provider.md),
[KOTO-0241](KOTO-0241-kotoconfig-wifi-page.md),
[KOTO-0242](KOTO-0242-kotosim-fake-network-service.md), and the integrated
[KOTO-0243](KOTO-0243-picocalc-wifi-config-validation.md) hardware gate.
