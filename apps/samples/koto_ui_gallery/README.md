# KotoUI Gallery

`dev.koto.samples.koto-ui-gallery` is the app-authored counterpart of the
native simulator component Gallery. It uses the standard-library form
`include <sdk/koto_ui.koto>;` and builds
Label, Button, Checkbox, List, TextField, Panel, and Dialog records without any
`draw_*` call. The app owns semantic values; KotoUI owns pixels, focus, damage,
and input routing.

Build and launch the current vertical slice:

```powershell
python harness/build_apps.py --app dev.koto.samples.koto-ui-gallery
cargo run -p koto-sim -- --app dev.koto.samples.koto-ui-gallery
```

The initial focus is **Open dialog**. Activate it to verify the localized modal
body and trapped focus, then activate **Close** or cancel the dialog. Semantic
events update the status Label for activation, checkbox, list, text, capacity,
and dialog outcomes.

Run the deterministic interaction and print the runtime budget:

```powershell
cargo run -p koto-sim -- --app dev.koto.samples.koto-ui-gallery `
  --app-script apps/samples/koto_ui_gallery/scenarios/interaction.txt `
  --inspect --budget
```

The 17-frame baseline yields with `Kotoあ`, peaks at 2,888 VM heap bytes,
58,487/60,000 fuel, and eight host calls in one frame. One call polls lifecycle
intents so F10 exits independently of KotoUI focus state. Low-level draw counts
remain zero because KotoUI owns component drawing. The retained host session is
6,008 bytes and its command image peaks at 70 rectangle/text commands;
`--budget` also reports observational worst-frame microseconds. The app loads
English, Japanese, and 35–50% expanded
`qps-ploc` UTF-8 resources from `locales/*.txt` package assets and applies them
through live `LocaleChanged` packets. Each translator-facing file contains 22
non-empty lines: component text, the initial field value, and semantic status
messages. Unrecognized tags fall back to English. The simulator suite pins
activation pulses, disabled/checked/scrolled states, IME composition, capacity
errors, modal focus, locale changes, and idle zero-redraw behavior. PicoCalc
validation remains before KOTO-0220 can close.

Run the App-specific locale, pixel, focus, response, damage, and idle goldens:

```powershell
cargo test -p koto-sim --test koto_ui_app_gallery
```

To adapt the Gallery, keep business values and locale choice in app-owned
buffers, parse translator assets through `TextResource`, build List blobs with
`UiListRowsBuilder`, change the descriptor calls in `gallery_mount`, and update
state with the typed update builder. Application code should not copy
KUI1/KUP1 field offsets or treat their binary layout as its API;
`sdk/koto_ui.koto` owns packet validation and encoding.
