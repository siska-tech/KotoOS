# KOTO-0094: KotoBlocks tetromino game and sprite/tile primitive

- Status: done
- Type: feature
- Priority: P1
- Requirements: FR-PM-1, FR-PM-2

## Goal

Ship the first KotoOS game and the first pixel-level sprite/tile-graphics sample:
"KotoBlocks" (`dev.koto.games.koto-blocks`), a Tetris-style block puzzle that renders
its board and falling piece as palette-indexed 16x16 RGB565 tiles per the
[PicoMings sprite/tile model](../../spec/PICOMINGS_SPRITE_MODEL.md), wiring up the previously
reserved `draw_pixels_rgb565` host call end-to-end.

## Acceptance Criteria

- [x] `draw_pixels_rgb565` is dispatched by the VM, implemented by the simulator host
  (recorded + blitted in window mode), exposed as the `draw_pixels` SDK wrapper, and
  covered by runtime and compiler tests.
- [x] `Canvas::blit_rgb565` blits little-endian RGB565 pixel blocks with clipping.
- [x] KotoBlocks compiles, verifies, and runs in KotoSim: 7 tetrominoes, rotation,
  gravity/soft/hard drop, line clears, score/level/lines, NEXT x3, HOLD, pause,
  game over, and retry — within the local-slot, heap, and frame-fuel budgets.
- [x] App registered in `apps/apps.json` with manifest, icon, and a smoke scenario.
- [x] `python harness\check_all.py` passes (golden frames updated for the new package).

## Notes

- The 7 tetromino tiles are baked once into the app heap on the title screen (a few
  rows per frame to stay inside the per-frame fuel budget), then blitted each frame;
  the falling piece is a sprite composed over the board (empty cells are not blitted).
- Required raising the simulator profile (see [KOTO-0060](KOTO-0060-sim-runtime-profile-cleanup.md)):
  heap 2 KB -> 4 KB to cache the tiles, frame fuel 10000 -> 60000 for the full-screen
  board repaint.
- All board/tile heap reads and writes live in `main` because Koto buffers are
  function-scoped; pure helpers compute piece shapes/colours and issue blits.
- Audio (BGM/SFX) is intentionally out of scope here; see
  [KOTO-0095](KOTO-0095-app-audio-host-call.md).
- Visual effects (follow-up): a deferred **line-clear flash** (full rows are marked
  with a sentinel and blink white for a few frames before the row-shift removal and
  scoring) and a **hard-drop trail** (a faint streak in the piece's columns from the
  drop's start row to the landing). Room for the effect state was freed by inlining
  `grav_interval` (relying on KOTO-0092 slot reuse keeps the program under 45 locals).
- Rendering responsibilities are analysed in [Game2D ABI](../../spec/GAME2D_ABI.md)
  ([KOTO-0097](KOTO-0097-game2d-abi-design.md)); KotoBlocks keeps its in-app tile
  cache + per-cell blit until a host tile/sprite API lands.
