# Game2D ABI: Tile and Sprite Rendering Boundary

> **Status update (KOTO-0135, first cut implemented).** The retained **tilemap**
> half of this design is now implemented as host ABI minor `13`, with IDs
> reconciled into a single Game2D sub-block:
>
> | ID | Name | Stack args | Effect |
> | :-- | :--- | :--------- | :----- |
> | `0x14` | `game2d_set_tile` | `layer x y tile_ref` | Write one 16x16 cell; `tile_ref` is the app-heap byte offset of a 16x16 RGB565 tile, `< 0` clears the cell. |
> | `0x15` | `game2d_clear_layer` | `layer` | Clear every cell of `layer`. |
> | `0x16` | `game2d_present` | — | Composite the retained tilemap for this frame. |
>
> Tile art stays in the **app heap** (referenced by `tile_ref` offset, reusing the
> `draw_pixels_rgb565` heap re-read) rather than a host tile cache — the smallest
> first cut. The host owns the tilemap + per-cell change tracking; the device
> composites the layer beneath the immediate command list and transfers only
> changed cells, the simulator re-emits set cells as pixel blits. KotoBlocks'
> locked board is migrated to this path; the active piece, ghost, previews, UI, and
> overlays remain on the immediate `draw_*` path for now. See
> [KOTO-0135](../issues/main/KOTO-0135-stateful-game2d-host-renderer.md).
>
> **Status update (KOTO-0136, retained static layer).** A second retained layer
> captures an app's *static* chrome (page/well/grid/panel/label UI) once so it no
> longer costs a host call and an immediate command per frame:
>
> | ID | Name | Stack args | Effect |
> | :-- | :--- | :--------- | :----- |
> | `0x17` | `game2d_static_begin` | — | Clear the static layer; route subsequent draw calls into it. |
> | `0x18` | `game2d_static_end` | — | Route draw calls back to the per-frame immediate list. |
>
> The presenter composites **static layer → board tilemap → immediate commands**.
> `game2d_present`/`set_tile`/`clear_layer` are not redirected by a capture. See
> [KOTO-0136](../issues/main/KOTO-0136-game2d-static-layer.md).
>
> **Status update (KOTO-0140, retained sprite/stamp layer).** The reserved sprite
> IDs are now implemented as cell stamps (v1 — no pixel stamps yet):
>
> | ID | Name | Stack args | Effect |
> | :-- | :--- | :--------- | :----- |
> | `0x19` | `game2d_stamp_define` | `stamp_id cells_off count format` | Register a stamp: `count` cells at app-heap byte offset `cells_off`. `format 0` packs each cell as a `(dcol,drow)` nibble (`drow*4 + dcol`, the KOTO-0138 CELLS layout). Descriptor only — cell bytes stay in the heap. |
> | `0x1A` | `game2d_sprite_set` | `inst_id stamp_id x y tile_ref` | Create/update retained sprite `inst_id`: draw `stamp_id`'s cells at `(x + dcol·16, y + drow·16)`, each blitting the 16×16 tile at heap offset `tile_ref`. |
> | `0x1B` | `game2d_sprite_hide` | `inst_id` | Hide a sprite (footprint becomes a dirty erase next present). |
> | `0x1C` | `game2d_sprite_clear_all` | — | Hide every sprite. |
>
> The presenter now composites in **fixed z-order** `static → tile → sprite →
> immediate`, replacing the KOTO-0135 `game2d_present` stream marker (the board no
> longer composites at the call site; `present` is a frame acknowledgement).
> A *stamp* is a reusable cell pattern; a *sprite* is a retained placed instance,
> diffed by stable `inst_id` so a moving piece is one small dirty band. IDs
> `0x1D`–`0x1F` stay **reserved** for the retained text layer (KOTO-0141). See
> [KOTO-0140](../issues/main/KOTO-0140-retained-sprite-stamp-layer.md).
>
> **Status update (KOTO-0141, retained text layer).** The reserved text IDs are now
> implemented:
>
> | ID | Name | Stack args | Effect |
> | :-- | :--- | :--------- | :----- |
> | `0x1D` | `game2d_text_set` | `id x y ptr len rgb565` | Create/update retained text item `id`: the UTF-8 string at `ptr`/`len` (an app-heap byte range, decoded like `draw_text`) pinned at `(x, y)` in colour `rgb565`. |
> | `0x1E` | `game2d_text_hide` | `id` | Hide a text item (its footprint becomes a dirty erase next present). |
> | `0x1F` | `game2d_text_clear_all` | — | Hide every text item. |
>
> A text item is diffed by stable `id`, so a value that does not change costs
> nothing and a change repaints only its own row band — removing the per-frame
> `draw_text` churn that shifted the immediate command count and forced
> positional-diff full repaints (KOTO-0143 `CommandCountShift`). The presenter
> composites the text layer in fixed z-order `static → tile → sprite → text →
> immediate`. v1 keeps the existing pixel-font row-height band as the footprint (no
> tight CJK width metrics). `HOST_ABI_MINOR` → 15. The full `0x14`–`0x1F` Game2D
> block is now implemented; immediate `draw_*` is debug/overlay/transition/fallback
> only. See [KOTO-0141](../issues/main/KOTO-0141-retained-text-layer.md).
>
> **Status update (KOTO-0199, generic tilemap geometry).** Host ABI minor `16`
> adds an explicit configuration call outside the full `0x14`–`0x1F` block:
>
> | ID | Name | Stack args | Effect |
> | :-- | :--- | :--------- | :----- |
> | `0x22` | `game2d_configure_tilemap` | `layer columns rows origin_x origin_y` | Clear/configure layer 0 with a 1..20 by 1..20 active grid and signed pixel origin. |
>
> Storage is a fixed 20x20 array with no runtime allocation; the active shape
> controls bounds, painting, and dirty coalescing. Tiles remain 16x16 RGB565.
> Geometry changes dirty both old and new clipped bounds. Legacy apps that never
> call configure retain KotoBlocks' 10x20 origin `(8, 0)` behavior.

The remainder of this document is the original **design** rationale, retained for
the responsibility split and the not-yet-implemented sprite/tile-cache surface.

This is a **design document**, not an implemented ABI. It decides which 2D-game
rendering responsibilities a future shared "Game2D" host ABI should own, versus what
stays inside an app such as [KotoBlocks](../issues/main/KOTO-0094-koto-blocks-game.md). It
builds on the [PicoMings scanline sprite model](PICOMINGS_SPRITE_MODEL.md) — *the host
owns scanline rendering and asset streaming; the VM owns game rules* — and on the one
pixel primitive that exists today, `draw_pixels_rgb565` (host-call `0x12`, see
[RUNTIME_BYTECODE_ABI.md](RUNTIME_BYTECODE_ABI.md)).

## Why now

`draw_pixels_rgb565` shipped with KotoBlocks, which currently does everything in the
app: it bakes seven 16×16 RGB565 tiles into its own heap on the title screen and, each
frame, walks the 10×20 board and issues a `draw_pixels` blit per occupied cell, plus
blits for the active piece, ghost, and previews. That works, but it pushes three
distinct jobs — **tile cache**, **sprite blit**, **board (tilemap) repaint** — into VM
bytecode, where they cost app heap (the tile cache is ~3.5 KB of the app's 4 KB),
fuel (a full-board repaint is most of a frame), and duplicated code in every game.
The question this doc answers: how much of that should the host own?

## Responsibility split

| Concern | Owner | Rationale |
| :------ | :---- | :-------- |
| Game rules: piece/board state, scoring, RNG, win/loss, timers | **App (VM)** | This is the game; it must stay in bytecode (PicoMings "VM owns rules"). |
| Input handling and intent → action mapping | **App (VM)** | Per-game; already served by `input_snapshot` / `text_input`. |
| *Which* tile/sprite goes *where* this frame | **App (VM)** | The app decides placement; it should not also composite pixels. |
| Tile **art** storage (pixel data, palettes) | **Host** | Static art belongs in package assets / a host cache, not re-baked into app heap each run. |
| Tile/sprite **compositing** (blit, transparency, z-order, clipping) | **Host** | Pixel-level work is the host's scanline job; identical for every game. |
| Tilemap/board **repaint** (fill a grid region from cell→tile) | **Host** | A grid blit is generic; the app sets cells, the host paints. |
| Scrolling / camera | **Host** | Scanline offset is a host concern (PicoMings camera). |

The guiding line: **the VM names tiles, positions, and cells; the host turns those
into pixels.** `draw_pixels_rgb565` stays as the low-level escape hatch for apps that
want to push raw pixels (and as the fallback KotoBlocks uses until the calls below
exist).

## Proposed host ABI surface (to reserve, not yet implement)

A "Game2D" family in the draw block (`0x14`–`0x1F`, next to `draw_rect 0x10` …
`draw_text_color 0x13`). Names/IDs are a proposal for review:

| ID | Name | Stack args | Returns | Notes |
| :-- | :--- | :--------- | :------ | :---- |
| `0x14` | `tile_define` | `tile_id ptr len fmt` | status | Upload tile art once into a host tile cache (`fmt`: RGB565 or indexed+palette). `len` covers a fixed tile size (e.g. 16×16). |
| `0x15` | `tile_palette` | `pal_id ptr len` | status | Define/replace a shared palette for indexed tiles. |
| `0x16` | `draw_tile` | `tile_id x y` | status | Composite one cached tile at a pixel position (transparent index honoured). |
| `0x17` | `tilemap_set` | `map_id col row tile_id` | status | Set one cell of a host-held tilemap (the board). |
| `0x18` | `tilemap_blit` | `map_id x y cols rows` | status | Repaint a tilemap region in one call (host walks cells → tiles). |
| `0x19` | `sprite_set` | `index tile_id x y flags` | status | Define/update one entry in a host sprite list (z/flip in `flags`). |
| `0x1A` | `sprite_flush` | `count` | status | Composite the active sprite list (sorted by z) for this frame. |

With these, KotoBlocks would: `tile_define` its 7 tiles once (host cache, not app
heap), keep the board as a host tilemap (`tilemap_set` on lock/clear, `tilemap_blit`
once per frame), and draw the active piece/ghost/previews as a handful of sprites or
`draw_tile` calls — turning a ~200-blit, ~3.5 KB-heap frame into a few host calls.

## Out of scope for the first cut

- Rotation/scaling of sprites (fixed orientation first; the app pre-bakes rotations,
  as KotoBlocks does with its 4 mask rotations).
- Animation timelines (the app drives frame selection).
- More than one tilemap layer and parallax (single board layer first).

## Phasing

1. **(this doc)** Decide the split and reserve IDs. *No code.*
2. Implement the tile cache + `tile_define` / `draw_tile` end-to-end (runtime trait +
   VM dispatch + simulator host + SDK wrappers + tests), mirroring how
   `draw_pixels_rgb565` was wired. Migrate KotoBlocks' tile cache off the app heap.
3. Add the host tilemap (`tilemap_set` / `tilemap_blit`) and move the board repaint to
   the host; measure the heap/fuel reclaimed.
4. Add the sprite list (`sprite_set` / `sprite_flush`) for the active piece, ghost,
   and previews.

Each of steps 2–4 is a separate issue when picked up. Until then, `draw_pixels_rgb565`
remains the rendering path and KotoBlocks keeps its in-app tile cache and per-cell
blit (unchanged by this document).
