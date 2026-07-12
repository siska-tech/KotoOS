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

| Sample | App ID | Demonstrates | SDK calls exercised |
| :----- | :----- | :----------- | :------------------ |
| Hello Text | `dev.koto.samples.hello-text` | Static source-authored app, full-screen clear, text rendering, frame yield. | `text_intent`, `draw_rect`, `draw_text`, `yield_frame`, `exit` |
| Input Echo | `dev.koto.samples.input-echo` | Typed input from KotoSim into app heap-backed display text. | `text_input`, `text_intent`, `draw_rect`, `draw_text`, `yield_frame`, `exit` |
| Counter Loop | `dev.koto.samples.counter-loop` | Cooperative loop state preserved across frames. | `text_intent`, `draw_rect`, `draw_text`, `yield_frame`, `exit` |
| File Note | `dev.koto.samples.file-note` | Sandboxed save-data write/read round trip. | `file_open`, `file_write`, `file_read`, `file_close`, `draw_rect`, `draw_text`, `yield_frame`, `exit` |
| IME Playground | `dev.koto.samples.ime-playground` | Typed characters and convert/commit/cancel intents routed through the host IME. | `text_input`, `text_intent`, `ime_feed_key`, `ime_convert`, `ime_display`, `draw_rect`, `draw_text`, `yield_frame`, `exit` |
| Dirty Rects | `dev.koto.samples.dirty-rects` | A narrow redraw band plus moving rectangle for dirty-rectangle inspection. | `text_intent`, `draw_rect`, `draw_text`, `yield_frame`, `exit` |
| Actor Array | `dev.koto.samples.actor-array` | Heap-backed actor state for a player plus NPC actors without growing user-local slots. | `actor_array_new`, `actor_set_pos`, `actor_set_vel`, `actor_x`, `actor_y`, `draw_rect`, `yield_frame`, `exit` |

The harness check:

```powershell
python harness\build_apps.py --check
```

recompiles each sample to temporary bytecode, verifies it through the normal
compiler path, and fails if the committed `sdcard_mock/bytecode/*.kbc` fixtures
drift from their source.
