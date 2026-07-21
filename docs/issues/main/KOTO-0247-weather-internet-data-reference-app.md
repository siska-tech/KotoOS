# KOTO-0247: Weather Internet-data reference app

- Status: in progress
- Type: feature
- Priority: P2
- Requirements: FR-SDK-4, FR-SDK-5, FR-SDK-9, FR-RT-4, FR-FS-2, FR-PKG-1, FR-PKG-3, NFR-MEM-2, NFR-PORT-3, NFR-REL-1, NFR-I18N-1, NFR-I18N-2, NFR-DEV-3, NFR-DEV-4
- Related: KOTO-0036, KOTO-0047, KOTO-0052, KOTO-0230, KOTO-0244, KOTO-0245, KOTO-0246

## Goal

Ship a small bilingual Weather application that proves a packaged Koto app can
retrieve, decode, cache, and present bounded Internet data through the public
SDK. It must remain useful and understandable when Wi-Fi, time synchronization,
the provider, or previously cached data is unavailable.

## Acceptance Criteria

- [x] Package the Weather app through the normal `.kpa` toolchain with only its
  declared Fetch origins and sandbox-file access. The app uses no native-only
  network escape hatch and the same package runs in KotoSim and on device.
  (`apps/weather`, `dev.koto.weather`; manifest declares `fs: sandbox` and the
  single `https://weather.example` origin; built by `build_apps.py` like every
  other sample. Device-run half of this AC folds into the hardware AC below.)
- [x] Define a provider-neutral bounded Weather model and one documented sample
  adapter covering location label, condition code, temperature, daily range,
  precipitation indication, and observation/update time. Provider fields are
  decoded through KOTO-0246 and unknown fields are ignored safely.
  ([`docs/spec/WEATHER_DATA_MODEL.md`](../../spec/WEATHER_DATA_MODEL.md): the
  `kwd1` contract plus an Open-Meteo adapter mapping. Decoding uses depth-1
  `json_*` selection; the `station` subtree and other unknown fields are
  skipped, and missing/duplicate/wrong-type fields are distinguished.)
- [x] Allow manual location configuration without device geolocation. Validate
  and persist only non-secret settings in the app sandbox; no provider API key
  is embedded in the package, query string, log, or screenshot fixture.
  (Location `TextField`, `[a-z0-9-_]{1,32}` key validation, `config` sandbox
  file `KWF1` with unit + key only, Fletcher-16 guarded. No key anywhere.)
- [x] Fetch only while the app is active, expose loading/refresh/cancel states,
  prevent duplicate requests, and apply an app-level bounded refresh interval.
  Key input, redraw, exit, and audio remain responsive during requests.
  (One live request at a time; Refresh reads *Cancel* while in flight; 15 s
  monotonic-clock cooldown; at most one 128-byte chunk decoded per frame.)
- [x] Store the last valid bounded snapshot atomically in the app sandbox and
  show it with an explicit stale/offline indicator after restart or failure.
  Invalid or partial downloads never replace the last valid snapshot.
  (Two-slot `cache_a`/`cache_b` `KWC1` images, newest-valid-generation wins,
  Fletcher-16 guarded; a torn write can only damage the slot being replaced.)
- [x] Use synchronized time only for advisory update labels and cache age.
  Unknown time is displayed explicitly and never disables refresh or causes
  downloaded data to be trusted. (New advisory `time_query` host call, Host ABI
  minor 21: UTC seconds / offset minutes / monotonic ms. Unknown UTC shows
  `?`; refresh works without synchronized time.)
- [x] Provide `ja-JP` and `en-US` strings with deterministic English fallback,
  temperature-unit presentation, clipping, and pseudolocale layout coverage.
  (`locales/{en-US,ja-JP,qps-ploc}.txt`; `ui_locale_match` fallback to en-US;
  °C/°F toggle; ellipsis-clipped labels.)
- [x] Add deterministic KotoSim scenarios for first success, partial reads,
  refresh, malformed/oversized JSON, provider error, timeout, cancellation,
  offline start with/without cache, unknown time, and locale changes. Tests use
  fixtures rather than the host network or wall clock.
  (`src/koto-sim/tests/koto_weather_service.rs`, 13 tests green; the fetch
  backend and advisory clock are scripted through new session setters.)
- [ ] Validate on hardware against a controlled authenticated endpoint and
  record package size, peak app memory, response limits, render responsiveness,
  and network failure recovery. (Pending user device run — Pico 2 W / Pico W.)

## Implementation progress

Host ABI minor bumped 20 → 21 for the advisory `time_query` call (`0x56`),
implemented across `koto-vm` (dispatch, verifier stack effect, known-call
table), `kbc-asm`, `koto-compiler` (`time_query` intrinsic + `TIME_*`
constants), `koto-sim` (frame-clock-driven, scripted UTC), and `koto-pico`
(`sntp_utc_seconds` + config offset, `network_service`-gated). Selector codes
live in `koto_core::time::app_time_query`. Specs updated: `KOTO_SDK.md`,
`RUNTIME_BYTECODE_ABI.md`, new `WEATHER_DATA_MODEL.md`, `SDK_SAMPLES.md`.

App: `apps/weather` (single `WApp` record + `helpers.koto`), boot peak
`local 47/48`, `heap 3866/24576`; `fuel` bounded per frame except the one-time
Japanese-resource re-parse on a locale change, which is a bounded operation
that may span two simulator frames under the sim per-frame guard and resumes
transparently (fetch/input/exit stay on their own bounded frames). All KBC
rebuilt (header minor 20 → 21); golden trace regenerated (22 → 23 packages);
`check_budgets`/`check_golden_frames`/`check_embedded` green; the pre-existing
firmware-rustfmt and SKK-dictionary drift is unrelated to this change.

## Non-goals

- GPS/geolocation, weather alerts, radar maps, or background notifications
- Coupling the SDK to a commercial Weather provider
- Treating Weather data or advisory SNTP time as safety-critical information
