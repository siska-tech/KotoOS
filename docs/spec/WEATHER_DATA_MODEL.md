# Koto Weather Data Model (`kwd1`)

KOTO-0247. The packaged Weather application consumes one provider-neutral,
bounded JSON document — the *Koto Weather Data v1* (`kwd1`) contract — through
the public SDK only: the manifest-allowlisted Fetch origin (KOTO-0245) and the
bounded incremental JSON decoder (KOTO-0246). The device never parses a
commercial provider's response shape and no provider API key exists in the
package, query string, log, or fixture. Whatever service answers the app's
`GET` (a self-hosted proxy, a home-automation bridge, a test fixture server)
is the *adapter*: it speaks a provider's protocol on one side and emits `kwd1`
on the other.

## Request

The app issues `GET https://weather.example/kwd1/<location>` where
`<location>` is the user-configured location key. The key is validated before
persistence and use: 1–32 bytes of lowercase ASCII letters, digits, `-`, or
`_`. It is a routing label, not a secret; geolocation is never used. The
origin is pinned by the package manifest exact-origin allowlist; the response
is expected to be small and MUST stay under 2,048 body bytes (the app treats a
larger document as oversized and keeps its previous snapshot).

## Response document

A single JSON object, UTF-8, all values integers or strings. Unknown fields
at any depth are skipped safely (KOTO-0246 depth-1 selection); order is not
significant. Example:

```json
{
  "schema": "kwd1",
  "location": "Tokyo",
  "condition": 5,
  "temperature_dc": 215,
  "temp_min_dc": 180,
  "temp_max_dc": 260,
  "precipitation_pct": 40,
  "observed_at": 1784958000
}
```

| Field | Type | Required | Range / meaning |
| :-- | :-- | :-- | :-- |
| `schema` | string | no | Contract tag; when present and not `"kwd1"` the document is rejected. |
| `location` | string | yes | Display label for the location, 1–48 UTF-8 bytes after decoding. Shown as-is; it need not equal the request key. |
| `condition` | number | yes | Condition code, `0..=8` (below). Out-of-range maps to `0` (unknown). |
| `temperature_dc` | number | yes | Current temperature in deci-degrees Celsius (`215` = 21.5 °C), `-1000..=700`. |
| `temp_min_dc` | number | no | Daily minimum, same unit/range. Absent → shown as unknown. |
| `temp_max_dc` | number | no | Daily maximum, same unit/range. Absent → shown as unknown. |
| `precipitation_pct` | number | no | Precipitation indication `0..=100` (probability or coverage percent). Absent → shown as unknown. |
| `observed_at` | number | no | Observation/update time, Unix UTC seconds, `0..=2^31-1`. Absent → shown as unknown; never invented from local time. |

Duplicate or wrong-typed occurrences of a selected field make the document
invalid. An invalid, truncated, oversized, or non-`200` response never
replaces the app's last valid snapshot (it is reported and the cached
snapshot, if any, stays on screen marked stale).

Integers keep the decoder allocation-free and the VM math exact: the adapter,
not the device, rounds provider floats to deci-degrees and whole percent.

## Condition codes

| Code | Meaning |
| --: | :-- |
| 0 | Unknown |
| 1 | Clear |
| 2 | Partly cloudy |
| 3 | Cloudy |
| 4 | Fog |
| 5 | Rain |
| 6 | Snow |
| 7 | Thunderstorm |
| 8 | Windy |

The set is deliberately closed and small: every code has a translated name in
each packaged locale, and adapters map richer provider vocabularies down to
it.

## Sample adapter: Open-Meteo

One documented mapping, as required by KOTO-0247; any equivalent service
works. A self-hosted proxy calls
`https://api.open-meteo.com/v1/forecast?latitude=..&longitude=..&current_weather=true&daily=temperature_2m_min,temperature_2m_max,precipitation_probability_max&timezone=UTC`
for the coordinates it associates with each `<location>` key, then emits:

| `kwd1` field | Open-Meteo source | Transform |
| :-- | :-- | :-- |
| `location` | proxy configuration | The human label the proxy stores for the key. |
| `condition` | `current_weather.weathercode` (WMO) | 0 → 1 (clear); 1–2 → 2; 3 → 3; 45,48 → 4; 51–67, 80–82 → 5; 71–77, 85–86 → 6; 95–99 → 7; otherwise → 0. |
| `temperature_dc` | `current_weather.temperature` (°C float) | `round(value * 10)`. |
| `temp_min_dc` / `temp_max_dc` | `daily.temperature_2m_min[0]` / `..._max[0]` | `round(value * 10)`. |
| `precipitation_pct` | `daily.precipitation_probability_max[0]` | Clamp to `0..=100`; omit when the provider returns null. |
| `observed_at` | `current_weather.time` (ISO 8601 UTC) | Convert to Unix seconds. |

The proxy owns any provider API key and TLS session to the provider; the
device holds only the `kwd1` origin in its manifest. Weather data and the
advisory SNTP clock (KOTO-0244) stay non-safety-critical display data.
