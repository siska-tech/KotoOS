# PicoMings Scanline Sprite Model

This document sketches the first PicoMings rendering model. The goal is a small
2D engine that composes a tile background and many moving actors one scanline at
a time, without keeping a full 320x200 RGB565 framebuffer in SRAM.

## Fixture

The first source-side level fixture is
[small_level.json](../../harness/fixtures/picomings/small_level.json). It is not a
runtime binary format yet; it captures the data shapes a packer or later engine
prototype should accept.

```json
{
  "format": "picomings-level",
  "version": 0,
  "viewport": { "width": 320, "height": 200 },
  "tile_size": { "width": 16, "height": 16 },
  "map_size": { "columns": 40, "rows": 13 }
}
```

## Tile Data

Tiles are fixed 16x16 indexed-color images. A level map stores tile IDs in
row-major order. The first renderer should treat the map as the collision source
and keep visual tile attributes simple.

| Field     | Type                     | Notes                                                       |
| :-------- | :----------------------- | :---------------------------------------------------------- |
| `tile_id` | `u16`                    | Index into the level tile table.                            |
| `pixels`  | 16x16 `u8`               | Palette index for each pixel.                               |
| `flags`   | `u8`                     | Bit 0 solid, bit 1 slope, bit 2 hazard, remaining reserved. |
| `palette` | 16 or 256 RGB565 entries | Shared per level for the first prototype.                   |

The visible map is wider than the viewport so horizontal scrolling can be
tested. A 40x13 map of 16x16 tiles covers 640x208 pixels. The renderer clips the
bottom 8 pixels to fit the KotoDOS-style 320x200 game region.

## Sprite Data

Sprites are small indexed-color images with a transparent index. The engine
keeps sprite metadata in SRAM and stores frame pixels in package assets or
PSRAM-backed blocks.

| Field               | Type               | Notes                                                 |
| :------------------ | :----------------- | :---------------------------------------------------- |
| `x`, `y`            | signed fixed-point | 8.8 fixed-point is enough for subpixel movement.      |
| `frame_id`          | `u16`              | Selects an animation frame.                           |
| `width`, `height`   | `u8`               | Initial frames should stay at or below 16x16.         |
| `transparent_index` | `u8`               | Pixels with this value do not overwrite the scanline. |
| `z`                 | `u8`               | Draw order bucket. Lower values draw first.           |
| `state`             | `u8`               | Actor state owned by game logic.                      |

For the first prototype, active sprites should be sorted by `y` each frame or
inserted into per-scanline buckets. A fixed maximum of 64 active actors is a
reasonable starting point for memory and CPU measurement.

## Scanline Composition

The renderer owns one RGB565 scanline buffer for the 320-pixel game region:

```text
for each visible y:
  fill scanline from tile map
  find active sprites intersecting y
  draw non-transparent sprite pixels in z order
  submit scanline to the render/HAL path
```

Tile fill is deterministic and cheap:

1. Convert screen `y` plus camera `scroll_y` to `tile_row` and `tile_y`.
2. Walk visible tile columns from `scroll_x / 16`.
3. Copy or palette-expand the requested tile row into the scanline buffer.
4. Clip the left and right edge tiles when the camera is not tile-aligned.

Sprite overlay works only on the current scanline. The renderer computes
`sprite_y = screen_y - sprite.y`, reads the corresponding sprite row, skips the
transparent index, and writes RGB565 pixels over the tile result. Dirty
rectangle tracking can mark the game region or actor bounds, but the actual
engine path should remain scanline-first.

## Memory Estimate

This estimate targets a small 640x208 level with 64 active actors, 128 visual
tiles, 16-color indexed art, and one 320-pixel RGB565 scanline.

| Item                                           |              Size |
| :--------------------------------------------- | ----------------: |
| Tile map, 40x13 `u16` IDs                      |       1,040 bytes |
| Tile flags, 128 bytes                          |         128 bytes |
| Collision bits, 640x208 at 1 bit per pixel     |      16,640 bytes |
| Tile art, 128 tiles x 16x16 x 4 bits           |      16,384 bytes |
| Shared 16-color RGB565 palette                 |          32 bytes |
| Sprite metadata, 64 actors x 16 bytes          |       1,024 bytes |
| Sprite frame cache, 16 frames x 16x16 x 4 bits |       2,048 bytes |
| Per-scanline RGB565 buffer, 320 pixels         |         640 bytes |
| Per-scanline sprite index buckets              |   about 512 bytes |
| Camera, counters, and scratch state            | about 1,024 bytes |
| **Approximate SRAM working set**               |      **39-40 KB** |

Tile art and sprite frames can move to PSRAM or package streams later. If only
the map, collision data, metadata, frame cache, and scanline buffer stay in
SRAM, the renderer stays comfortably below the 150-180 KB application budget.

## Input Constraints

FR-PM-2 limits the default controls to direction keys plus Enter and Space. That
means the first UI should avoid chord-heavy shortcuts and use a small cursor
state machine:

| Input      | Action                                                       |
| :--------- | :----------------------------------------------------------- |
| Left/Right | Move the selection cursor or scroll the viewport near edges. |
| Up/Down    | Cycle the selected command or target row.                    |
| Enter      | Confirm the selected command or target.                      |
| Space      | Pause, cancel, or step back from targeting.                  |

The engine should poll input once per frame through the normal `InputState`.
Text-entry keys are not required for gameplay. Real PicoCalc hardware still
needs the KOTO-0025 matrix validation results before these become final default
bindings, especially for direction plus Enter or direction plus Space cases.

## VM And Host Boundary

The VM should own high-level game rules: actor state, command selection, timers,
and win or loss conditions. The host engine should own scanline rendering,
palette expansion, package asset streaming, collision queries, and input
normalization. This split keeps the VM heap small and lets the renderer use the
same line-oriented paths as KotoDOS.

## Open Questions

- Whether collision data should be authored as bits, nibble material IDs, or
  derived from tile flags.
- Whether sprite frame rows need a small RLE codec before the first playable
  prototype.
- Whether the KotoDOS 320x200 region is enough, or whether PicoMings should
  eventually support a 160x160 performance mode.
