# KOTO-0097: Game2D ABI design (tile/sprite rendering boundary)

- Status: done
- Type: research
- Priority: P2
- Requirements: FR-PM-1

## Goal

Decide which 2D-game rendering responsibilities (tile cache, sprite blit, tilemap/board
repaint) a shared host "Game2D" ABI should own, versus what stays inside a game app
like KotoBlocks — now that `draw_pixels_rgb565` has landed and KotoBlocks does all of
it in the VM.

## Acceptance Criteria

- [x] A design document records the host/app responsibility split.
- [x] It proposes a concrete host ABI surface (reserved call IDs + signatures) for the
  tile cache, `draw_tile`, host tilemap, and sprite list.
- [x] It states what KotoBlocks keeps in-app for now and a phasing plan for
  implementation as future issues.

## Resolution

[docs/GAME2D_ABI.md](../../spec/GAME2D_ABI.md). Summary: the VM names tiles, positions, and
cells; the host turns those into pixels. Proposed Game2D calls reserved in the draw
block `0x14`–`0x1F` (`tile_define`, `tile_palette`, `draw_tile`, `tilemap_set`,
`tilemap_blit`, `sprite_set`, `sprite_flush`). No code this round:
`draw_pixels_rgb565` stays the rendering path and KotoBlocks keeps its in-app tile
cache + per-cell blit. Phasing: implement the tile cache + `draw_tile` first (mirroring
the `draw_pixels_rgb565` wiring), then the host tilemap, then the sprite list — each a
separate issue when picked up.
