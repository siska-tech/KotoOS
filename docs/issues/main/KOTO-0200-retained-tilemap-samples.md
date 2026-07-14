# KOTO-0200: Retained tilemap display and scrolling samples

- Status: done
- Type: feature
- Priority: P2
- Requirements: NFR-RT-2
- Related: KOTO-0135, KOTO-0143, KOTO-0198, KOTO-0199

## Goal

Add two SDK samples that connect ASCII maps authored through an `app.json`
`maps` declaration to the generic retained tilemap runtime: one that displays a
complete 20x20 map and one that scrolls a 20x20 viewport through a larger map.

## Acceptance criteria

- [x] Add complete `apps/samples/retained_tilemap` and
  `apps/samples/retained_tilemap_scroll` apps, including `app.json`, Koto source,
  icon, map data, and sample documentation.
- [x] Both map files are declared in their app's `app.json` `maps` block, pass
  map validation, and can be opened and edited with the Koto Tilemap Editor.
- [x] Both apps declare a four-tile `sprites/tiles.kspr` image source, compile it
  to package-local KIM1 through the `app.json` `images` block, and load the
  immutable sheet before assigning retained tile references.
- [x] The static sample uses a 20-column by 20-row authored map, configures
  retained layer 0 as 20x20 at pixel origin `(0, 0)`, maps every allowed glyph
  deterministically to a 16x16 RGB565 tile, and displays the entire map.
- [x] The static sample stages tile creation and map upload if required to stay
  within VM fuel and host-call budgets. Once initialized, an idle frame issues
  no `game2d_set_tile` calls.
- [x] The scrolling sample uses an authored logical world larger than 20x20 and
  keeps the active runtime layer at 20x20.
- [x] Arrow input moves the scrolling sample's camera by one 16-pixel tile per
  action and clamps the camera at every world boundary.
- [x] The scrolling sample configures the retained layer once during
  initialization rather than reconfiguring it for each movement or frame.
- [x] A camera move updates the visible viewport from the authored world. A
  shadow of the visible tile references avoids `game2d_set_tile` calls for
  unchanged cells, and an idle frame issues no tile updates.
- [x] Each sample contains exactly one `@` required by the current map
  validator and documents its meaning; the scrolling sample uses it as the
  initial camera or player marker.
- [x] Both samples call `game2d_present` and use the retained tilemap for normal
  map rendering instead of redrawing the map with immediate rectangles or
  pixel blits every frame.
- [x] Simulator integration tests prove that both samples configure, populate,
  and present the retained layer without trapping; scrolling input changes the
  visible tile positions and boundary movement remains clamped.
- [x] The SDK sample launch sweep includes both apps and recognizes retained
  pixel output instead of requiring immediate-mode rectangle output.
- [x] Sample documentation explains the static and tile-step scrolling
  patterns, how to edit the source maps, and why smooth sub-tile scrolling is
  outside this issue.
- [x] `python harness/build_apps.py --check` and
  `python harness/check_project.py` pass.

## Design notes

- Use package names `sample_retained_tilemap` and
  `sample_retained_tilemap_scroll`.
- The larger authored world is application data, not a larger retained layer;
  the scrolling sample streams its visible 20x20 subset into the bounded
  runtime layer.
- Tile pixels remain fixed at 16x16 RGB565 as defined by KOTO-0199.
- The glyph palette is sprite-authored rather than procedurally baked: `.` maps
  to grass, `#` to stone, `~` to water, and `@` to the marker tile in the KIM1
  strip. This keeps the sample art editable with the Koto Sprite Editor.
- `game2d_configure_tilemap` clears the layer, so it must not be used as a
  per-frame camera operation. Smooth pixel scrolling would require a separate
  non-clearing origin/camera contract and likely an overscan design; track that
  separately if needed.
- The current generic map build validation requires exactly one `@`. This issue
  uses that marker intentionally rather than broadening the validator contract.
