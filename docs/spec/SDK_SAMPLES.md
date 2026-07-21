# KotoSDK Samples

These source-authored apps are small regression fixtures for the Koto compiler,
bytecode runtime, SDK prelude, app build loop, and KotoSim launch path. They live
under `apps/samples/`, build through `harness/build_apps.py`, and are registered
in `apps/apps.json`.

Run any sample directly:

```powershell
python harness\build_apps.py
cargo run -p koto-sim -- --app dev.koto.samples.hello-text
```

Use F10 or an app script containing `exit` to leave the sample.

The compiler-focused source fixture `sdk/examples/static_record.koto`
demonstrates KOTO-0228 structured App state: one initialized heap record,
typed field mutation, a struct-reference parameter, and receiver methods. It is
not a dynamically constructed object; the `static` instance persists for the
whole App lifetime, and every method call is inlined at its call site.

| Sample | App ID | Demonstrates | SDK calls exercised |
| :----- | :----- | :----------- | :------------------ |
| Hello Text | `dev.koto.samples.hello-text` | Static source-authored app, full-screen clear, text rendering, frame yield. | `text_intent`, `draw_rect`, `draw_text`, `yield_frame`, `exit` |
| Input Echo | `dev.koto.samples.input-echo` | Typed input from KotoSim into app heap-backed display text. | `text_input`, `text_intent`, `draw_rect`, `draw_text`, `yield_frame`, `exit` |
| Counter Loop | `dev.koto.samples.counter-loop` | A `CounterState` static record and inline `increment` method preserve cooperative loop state across frames. | `text_intent`, `draw_rect`, `draw_text`, `yield_frame`, `exit` |
| Fetch Weather | `dev.koto.samples.fetch-weather` | Manifest-v2 exact-origin permission, nonblocking polling, bounded incremental reads, and deterministic offline-safe data retrieval. | `fetch_start`, `fetch_poll_state`, `fetch_poll_metadata`, `fetch_read`, `fetch_cancel`, `draw_rect`, `draw_text`, `yield_frame`, `exit` |
| JSON Weather | `dev.koto.samples.json-weather` | Streams the fetched response through the bounded incremental JSON decoder (KOTO-0246): named-field selection at depth 1, unknown-subtree skip by nesting depth, missing/duplicate/wrong-type distinguishability, one bounded chunk per frame. | `json_reset`, `json_next`, `json_finish`, `json_token`, `json_consumed`, `json_depth`, `json_error_code`, `json_error_offset`, `fetch_start`, `fetch_poll_state`, `fetch_read`, `fetch_cancel`, `draw_rect`, `draw_text`, `yield_frame`, `exit` |
| Vault Fetch | `dev.koto.samples.vault-fetch` | Uses an OS-owned network credential without seeing the secret (KOTO-0248): resolves an opaque handle for a granted TLS origin, starts an authenticated GET whose credential the OS injects, and shows that an ungranted origin resolves to no handle (default-denied). | `vault_handle`, `fetch_start_authenticated`, `fetch_poll_state`, `fetch_read`, `text_intent`, `draw_rect`, `draw_text`, `yield_frame`, `exit` |
| File Note | `dev.koto.samples.file-note` | Sandboxed save-data round trip behind an interactive KotoUI form (KOTO-0221 pilot): bounded note field, Save/Reload buttons, localized status. | `file_open`, `file_write`, `file_read`, `file_close`, `asset_load`, `ui_capabilities`, `ui_mount`, `ui_update`, `ui_present`, `ui_poll_event`, `ui_reset`, `yield_frame`, `exit` |
| IME Playground | `dev.koto.samples.ime-playground` | Typed characters and convert/commit/cancel intents routed through the host IME. | `text_input`, `text_intent`, `ime_feed_key`, `ime_convert`, `ime_display`, `draw_rect`, `draw_text`, `yield_frame`, `exit` |
| Dirty Rects | `dev.koto.samples.dirty-rects` | A `MotionState` record owns the moving rectangle position and its wraparound transition. | `text_intent`, `draw_rect`, `draw_text`, `yield_frame`, `exit` |
| Actor Array | `dev.koto.samples.actor-array` | Heap-backed actor state for a player plus NPC actors without growing user-local slots. | `actor_array_new`, `actor_set_pos`, `actor_set_vel`, `actor_x`, `actor_y`, `draw_rect`, `yield_frame`, `exit` |
| Audio Codecs | `dev.koto.samples.audio-codecs` | A `RegressionState` record drives the bounded PCM16/SLD4 regression soak. | `play_sfx_asset`, `text_input`, `text_intent`, `draw_rect`, `draw_text`, `yield_frame`, `exit` |
| Full Color Image Gallery | `dev.koto.samples.full-color-tile-image` | A `GalleryState` record drives four streamed RGB565 scenes and their dissolve/wipe phases. | `asset_load_range`, `draw_pixels_persistent`, `play_bgm_asset`, `yield_frame`, `exit` |
| Retained Tilemap | `dev.koto.samples.retained-tilemap` | A `TilemapState` record tracks map parsing and bounded multi-frame upload progress. | `asset_load`, `game2d_configure_tilemap`, `game2d_set_tile`, `game2d_present`, `yield_frame`, `exit` |
| Retained Tilemap Scroll | `dev.koto.samples.retained-tilemap-scroll` | A `ScrollState` record owns the camera and restartable viewport upload state. | `asset_load`, `game2d_configure_tilemap`, `game2d_set_tile`, `game2d_present`, `yield_frame`, `exit` |
| KotoUI Gallery | `dev.koto.samples.koto-ui-gallery` | App-authored retained scene containing every KotoUI v1 component; a `GalleryState` record owns selection, status, and locale resource state. | `ui_capabilities`, `ui_mount`, `ui_update`, `ui_present`, `ui_poll_event`, `ui_reset`, `yield_frame`, `exit` |
| Weather | `dev.koto.weather` | Bilingual Internet-weather reference app (KOTO-0247): retrieves, decodes, caches, and presents the bounded provider-neutral `kwd1` document through Fetch + JSON + advisory time only; explicit loading/refresh/cancel/stale/offline states, a two-slot atomic snapshot cache, a bounded refresh interval, and non-secret settings persistence. See [Weather data model](WEATHER_DATA_MODEL.md). | `fetch_start`, `fetch_poll_state`, `fetch_poll_metadata`, `fetch_read`, `fetch_cancel`, `json_reset`, `json_next`, `json_finish`, `json_token`, `json_consumed`, `json_depth`, `time_query`, `file_open`, `file_read`, `file_write`, `file_close`, `asset_load`, `ui_capabilities`, `ui_mount`, `ui_update`, `ui_present`, `ui_poll_event`, `ui_reset`, `yield_frame`, `exit` |

File Note (KOTO-0221) is the first application-facing KotoUI adoption: an
existing sample migrated off per-frame immediate drawing while preserving its
app ID, sandbox behavior, and exit route. The remaining immediate-drawing
samples are Hello Text, Input Echo, Counter Loop, IME Playground, Dirty Rects,
Actor Array, and Full Color Image Gallery (plus the retained-tilemap pair,
which uses the retained Game2D layers rather than KotoUI). Games and the
host-owned Memo editor intentionally keep their current drawing paths; a Memo
migration remains a separate later decision.

The harness check:

```powershell
python harness\build_apps.py --check
```

recompiles each sample to temporary bytecode, verifies it through the normal
compiler path, and fails if the committed `sdcard_mock/bytecode/*.kbc` fixtures
drift from their source.
