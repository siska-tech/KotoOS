# Retained tilemap scrolling sample

This sample keeps a 20x20 retained layer while the arrow keys move its viewport
through the 32x24 world in `maps/world.map`. The `@` glyph supplies the initial
camera marker. Edit the map through the `app.json` Koto Tilemap Editor entry;
its glyph-to-`sprites/tiles.kspr` mapping renders the scrolling world with the
actual tile art. Run `python harness/build_apps.py` to refresh the package. The app uses
`asset_load` and a bounded decoder rather than embedding the world in bytecode.

Scrolling is deliberately cell-step based: one input moves the camera by one
16-pixel tile. `game2d_configure_tilemap()` clears the layer, so the sample calls
it only once and streams changed cell references into the existing layer. Smooth
sub-tile scrolling needs a separate non-clearing camera/origin API and overscan;
that contract is outside this sample.

The four tile images live in `sprites/tiles.kspr` and can be edited with the Koto
Sprite Editor. `build_apps.py` compiles the sheet to KIM1; the app loads that
immutable image before the viewport retains any tile references, keeping device
dirty tracking and simulator rendering in agreement.

The sample intentionally supplies no full-screen background. Initial retained
viewport cells are composed by the host before they become its previous-frame
baseline.
