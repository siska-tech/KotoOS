# GFX-0003: Dirty derivation into koto-gfx

- Status: done
- Type: refactor
- Priority: P1
- Requirements: NFR-PORT-1, NFR-DRAW-1

Source of truth: [KOTOGFX_RENDER_MIGRATION_PLAN.md](../../architecture/KOTOGFX_RENDER_MIGRATION_PLAN.md)
§4 Stage 3, following the dirty-rect coalescing work
([KOTO-0159](../main/KOTO-0159-kotoblocks-dirty-rect-coalescing.md)).

Depends on: [GFX-0002](GFX-0002-retained-layer-data-model-into-koto-gfx.md).

## Goal

Move the dirty-region derivation — footprints, old/new unions, board-band rects —
onto the GFX-0002 POD types, so `koto-gfx` owns the dirty tracker that already
holds `FullRepaintPolicy`, `coalesce_rects`, and `DirtyRectGeometry`.

## Acceptance Criteria

- [x] `command_dirty_rect`, `sprite_footprint_rect`/`sprite_dirty_rect`,
      `text_footprint_rect`/`text_dirty_rect`, `board_band_rect`, `stamp_cell`, and
      `push_dirty` live in `koto-gfx` (`derive.rs`), surface-parameterised
      (`surf_w`/`surf_h`) and operating on the GFX-0002 POD types + the app heap
      slice. The board-placement constants they need (`GAME2D_ORIGIN_X/Y`,
      `GAME2D_TILE_PX`) moved too.
- [x] `present_app_delta`'s collect loops are unchanged: the host-typed entry
      points (`command_dirty_rect`, `board_band_rect`, `sprite_dirty_rect`,
      `text_dirty_rect`) are now identically-signed firmware adapters that unpack
      `DeviceRuntimeHost` into POD slices and call koto-gfx (the Stage 1
      methodology); `push_dirty`/`stamp_cell` are re-exports. The
      `FullRepaintPolicy` call is untouched.
- [x] No timing or transfer code is touched (pure geometry move only).
- [x] Property tests in `derive.rs`: `command_rect`/text-band swept over a grid
      against a hardcoded-320 reference re-implementation of the firmware
      originals; sprite/text/board/`push_dirty` behaviour asserted (move union,
      appear/disappear, off-screen, hidden, tighter non-320 surface). The four
      `app_render` invariants reference the wrappers via unchanged signatures.
- [x] Golden frames unchanged (`cargo test -p koto-sim` 13/13 + 94 unit);
      `thumbv6m` build green; koto-gfx + default clippy gate clean.

## Notes

Pure geometry over POD. Escalation thresholds and reason-code priority stay in
`koto_gfx::FullRepaintPolicy` — this issue only relocates the *collection* of
dirty rects, not the decision.
