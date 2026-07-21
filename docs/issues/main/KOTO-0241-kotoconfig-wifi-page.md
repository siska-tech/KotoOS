# KOTO-0241: KotoConfig Wi-Fi page and bilingual interaction

- Status: done
- Type: feature
- Priority: P1
- Requirements: FR-CONFIG-3, NFR-I18N-1, NFR-I18N-2, NFR-MEM-2, NFR-PORT-1, NFR-REL-1, NFR-REL-5
- Related: KOTO-0209, KOTO-0211, KOTO-0212, KOTO-0223, KOTO-0224, KOTO-0239, KOTO-0240, KOTO-0242, KOTO-0243
- Roadmap: [KotoConfig Roadmap](../../planning/KOTOCONFIG_ROADMAP.md)

## Goal

Add the capability-gated native `network.wifi` page to KotoConfig using only
NetworkService snapshots and credential-provider commands. Preserve bounded
KotoUI frame work, complete English/Japanese operation, and safe recovery when
the radio or any composite capability input disappears.

## Acceptance Criteria

- [x] Register `network.wifi` in the fixed page registry only when composite
  `WIFI_CONFIG` is true; unsupported startup consumes no page runtime state.
- [x] Implement `Disabled`, `Scanning`, `Results`, `CredentialEntry`,
  `Connecting`, `Connected`, `Failed`, `ForgetConfirm`, and
  `RadioUnavailable` as a fixed-capacity controller with no heap allocation.
- [x] Render at most 16 service-owned results, escaped invalid UTF-8 SSIDs,
  security/RSSI metadata, bounded progress, and fixed redacted errors without
  copying credentials or free-form driver text into labels/status history.
- [x] Implement the KOTO-0224 Enter/Esc/directional/Tab/Shift-Tab, rescan,
  retry, disconnect, and forget-confirm flows with stable focus and no keyboard
  trap. Every state remains escapable when service progress stops.
- [x] Use a masked fixed 63-byte credential field; enforce Open/WPA2 lengths,
  clear it after submission and authentication failure, and zeroize it on
  cancel, page exit, capability loss, and controller drop/reset.
- [x] Supply complete `en-US` and `ja-JP` resources for every title, state,
  action, validation message, and error. Locale changes rebuild the active page
  without losing safe selection/state or exposing credential text.
- [x] Poll at most one bounded snapshot per frame and submit commands without
  waiting. Idle frames produce no damage; changed rows/status damage only the
  required regions within the existing KotoConfig surface budget.
- [x] If capability disappears while open, cancel current UI intent, zeroize
  entry storage, enter `RadioUnavailable`, and permit exit; failure never
  blocks language settings, KotoShell, or offline app launch.
- [x] Unit/golden tests against fake snapshots cover all states, keyboard flow,
  empty/full scan lists, invalid SSID bytes, locale switch, failure/retry,
  cancellation, forget, capability loss, and clean return to Shell.
- [x] Record KotoConfig UI/controller SRAM, command count, maximum dirty area,
  credential buffer size, and frame-service work against RP2040 ceilings.

## Non-goals

- Calling CYW43, Embassy, sockets, filesystem adapters, or host networking
- Captive portal, advanced network details, region selection, or app networking

## Outcome

- Added the capability-gated native Wi-Fi page, fixed-capacity controller
  integration, English/Japanese rendering, bounded simulator driver, and the
  RP2040 LCD rendering adapter. The normal offline configuration path retains
  only the language page when the composite capability is absent.
- Host measurements: `WifiPageController` 778 bytes,
  `KotoConfigWifiUi` 1,008 bytes, 63-byte credential capacity, 16 result rows,
  at most 25 render commands, and a maximum full-panel dirty area of 92,416
  pixels. Runtime integration consumes at most one snapshot boundary and one
  key per frame; idle frames produce no damage.
- Validation: `cargo test -p koto-core` (304 tests), KotoSim network integration
  (12 tests), English/Japanese golden frames (2 tests), core/simulator Clippy
  with warnings denied, KotoSim window check, RP2040 `koto_firmware` build, and
  `python harness/check_project.py` all pass.
