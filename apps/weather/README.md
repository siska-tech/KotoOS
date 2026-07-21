# Koto Weather (KOTO-0247)

A small bilingual Weather application that proves a packaged Koto app can
retrieve, decode, cache, and present bounded Internet data through the public
SDK only, and stay useful and honest when Wi-Fi, time, the provider, or the
cache is unavailable.

## What it exercises

- **Bounded Fetch** (KOTO-0245): one manifest-allowlisted origin
  (`https://weather.example`), no native network escape hatch. The same package
  runs in KotoSim and on device.
- **Bounded JSON decoding** (KOTO-0246): the response is decoded chunk by chunk
  (at most one 128-byte chunk per frame) with the depth-1 named-field selection
  idiom. Unknown fields and subtrees are skipped; missing, duplicate, and
  wrong-type fields stay distinguishable. The document is the provider-neutral
  `kwd1` contract in [`WEATHER_DATA_MODEL.md`](../../docs/spec/WEATHER_DATA_MODEL.md).
- **Advisory time** (`time_query`, Host ABI minor 21): synchronized UTC is used
  only for update labels and cache age, and for the bounded refresh interval.
  Unknown time is shown explicitly (`?`) and never disables refresh or makes
  downloaded data trusted.
- **KotoUI** retained form (KOTO-0217+): a location `TextField`, a Refresh
  button (which reads *Cancel* while a request is in flight), a unit toggle,
  and status/condition/temperature/range/precipitation/updated labels.
- **Localization**: `ja-JP` and `en-US` with deterministic English fallback and
  a `qps-ploc` pseudolocale for layout/clipping coverage.

## Behavior

- Enter a location key (`[a-z0-9-_]{1,32}`) and submit, or press Refresh. Only
  non-secret settings (unit and location key) are validated and persisted in the
  app sandbox; no provider API key is embedded anywhere.
- Fetch happens only while the app is active. Loading/refresh/cancel states are
  explicit, duplicate requests are prevented, and a bounded refresh interval
  paces requests. Key input, redraw, exit, and audio stay responsive.
- The last valid snapshot is stored atomically in a two-slot sandbox cache
  (newest valid slot wins; a torn write can only damage the slot being
  replaced). After restart or any failure the cached snapshot is shown with an
  explicit stale/offline indicator, and an invalid, partial, oversized, or
  non-200 download never replaces it.

Weather data and the advisory SNTP clock are non-safety-critical display data.

## Adapter

Whatever server answers the `GET` is the adapter: it speaks a provider's
protocol and emits `kwd1`. A documented Open-Meteo mapping is in the data-model
spec; the adapter owns any provider key and TLS session, so the device only
ever holds the `kwd1` origin in its manifest.

KotoSim drives every scenario deterministically (scripted fetch responses and a
scripted advisory clock, never the host network or wall clock); see
`src/koto-sim/tests/koto_weather_service.rs`.
