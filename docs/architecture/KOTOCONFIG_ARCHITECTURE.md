# KotoConfig Architecture

- Status: planned
- Requirements: FR-CONFIG-1, FR-CONFIG-2, FR-CONFIG-3, FR-SHELL-6,
  FR-SDK-9, NFR-I18N-1, NFR-MEM-2, NFR-PORT-1, NFR-PORT-3, NFR-REL-1,
  NFR-REL-5

## Decision

KotoConfig is an OS-owned native KotoUI application. It is not a sandboxed KPA:
changing global settings and, eventually, network credentials requires authority
that ordinary applications must not receive. A shared `ConfigService` owns
validation, bounded persistence, change generations, and read-only snapshots.
KotoConfig owns user interaction; KotoShell and KotoRuntime are consumers.

```text
KotoConfig ---- validated mutations ----> ConfigService ---- public snapshot ----> KotoShell
                                               |                    |
                                               |                    +-------------> KUC1 / apps
                                               |
                                               +---- secret provider ----> future NetworkService
```

## Settings and storage boundary

- Public settings use stable namespaced keys and fixed maximum encoded lengths.
  v1 begins with `system.locale`; unknown keys survive compatible reads but are
  never applied without a registered validator.
- The persisted format is versioned, bounded, checksummed, and committed through
  the platform storage adapter. Missing, incomplete, oversized, or invalid data
  loads safe defaults; a fresh system defaults to `en-US`.
- Shell-only favorites, category, sort, and pane state remain in Shell
  preferences. Global locale moves to ConfigService and is not duplicated there.
- Every public mutation increments a nonzero wrapping configuration generation.
  Locale changes also increment the locale generation published in `KUC1` and
  enqueue the defined `LocaleChanged` event for a mounted app.
- Runtime apps may read explicitly allowlisted derived values such as locale.
  They cannot enumerate storage, mutate global settings, or read secret values.
- Secret settings use a separate provider and persistence namespace. Wi-Fi
  credentials never enter the public record, `KUC1`, crash/diagnostic dumps, or
  normal logs. This separation reduces exposure but does not claim hardware-backed
  confidentiality on boards without secure storage.

## Page registry and capabilities

KotoConfig uses a compile-time, fixed-capacity page registry. Each descriptor
contains a stable page ID, localized title key, required capability bits, order,
and a native controller/render entry. Unsupported pages are omitted rather than
shown as broken controls. Registry construction performs no heap allocation.

`system.language` requires the always-present `LOCALE_CONFIG` capability. A
future `network.wifi` page requires the composite `WIFI_CONFIG` capability,
which is advertised only when all of these are true:

1. the selected board profile has a supported radio transport;
2. the firmware includes an initialized Wi-Fi HAL/driver;
3. a NetworkService implementation is compiled and available;
4. a credential provider is available.

A `W` suffix in the board name is not sufficient by itself. Wi-Fi is optional
and failure to initialize it must never prevent KotoOS or KotoConfig from booting.

## Language page

- Choices are displayed as self-identifying `English` and `日本語`, independent
  of the current locale, so users can recover from an accidental selection.
- Selecting a language validates and persists through ConfigService, then
  immediately rebuilds KotoConfig labels and publishes the new locale generation.
- Unknown/corrupt values fall back to `en-US`. `qps-ploc` remains a simulator
  and test-only injection, not a normal device choice.
- KotoShell does not provide a second locale selector; it opens KotoConfig.

## Future Wi-Fi page contract

KotoConfig will be a client of NetworkService rather than owning a radio driver.
The page may request radio enable/disable, asynchronous scan, select an SSID,
submit security mode/credential, connect, disconnect, and forget. Scan/connect
progress and errors are bounded state-machine snapshots; UI frames never block
on radio work. NetworkService owns retry policy and connection lifetime.

The first Wi-Fi design issue must settle supported security modes, credential
storage guarantees, scan/result capacities, timeout/cancellation, regulatory
region ownership, and hardware memory/firmware budgets before implementation.

KOTO-0224 freezes those details in the
[KotoConfig Wi-Fi extension contract](KOTOCONFIG_WIFI_EXTENSION.md). The
composite capability, bounded service API, v1 page states, secret threat model,
and SRAM ceilings in that document are normative for subsequent implementation.

## Non-goals

- Enabling networking merely because a Pico W/Pico 2 W board profile builds.
- Giving ordinary Koto apps unrestricted system-setting or credential access.
- Dynamic plugin loading, unbounded page registries, cloud account settings,
  captive-portal support, or remote configuration in the first version.
