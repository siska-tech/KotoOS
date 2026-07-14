# Retained tilemap display sample

This sample displays the complete 20x20 map in `maps/world.map` through retained
Game2D layer 0. Open the map from `app.json` with the Koto Tilemap Editor; the
descriptor maps its glyphs to `sprites/tiles.kspr`, so the editor shows the
actual tile art while continuing to edit ASCII glyphs. Then run
`python harness/build_apps.py` to validate and package it as a read-only data
asset. The app loads it with `asset_load`, validates its
LF/CRLF row stride, and indexes cells directly from the bounded raw buffer.

Glyphs map to four 16x16 tiles authored in `sprites/tiles.kspr`: `.` is grass,
`#` is stone, `~` is water, and `@` is the single marker required by map
validation. Open the sheet with the Koto Sprite Editor; `build_apps.py` compiles
it to `sprites/static_tiles.kim`, which the app loads before retaining its tile
references. Once upload completes, the app calls `game2d_present()` without
rewriting unchanged cells.

The sample intentionally supplies no full-screen background. The host derives
the first retained damage from its empty app surface and composites each staged
slice, so applications do not need a dummy base command for correct device
output.
