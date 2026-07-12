# KOTO-0140: Retained Sprite/Stamp Layer (Cell Stamps)

- Status: done
- Type: feature
- Priority: P1
- Requirements: NFR-RT-2

Source of truth: [GAME2D_RETAINED_RENDER_ARCHITECTURE.md](../../architecture/GAME2D_RETAINED_RENDER_ARCHITECTURE.md) §2–§3.

## Goal

Add a generic, app-agnostic retained sprite layer so apps update *placed instances*
instead of re-emitting per-frame blit commands. Removes KotoBlocks' loopless `blit_piece`
(KOTO-0138) and its 5 per-frame piece blits from the VM, and removes the per-frame
immediate-list churn that drives positional-diff full repaints (see KOTO-0143).

## Terminology (v1 decision)

- **Stamp** = a reusable, position-independent cell *pattern definition* (the "what").
- **Sprite** = a retained on-screen *instance* of a stamp at a position with a tile (the
  "where").
- **v1 is cell-stamp-only.** Pixel stamps (a `w×h` RGB565 blit with a transparent color
  key) are **deferred** — do not add pixel-stamp complexity yet.

## ABI (host-call IDs `0x19`–`0x1C`, reserved by GAME2D_ABI.md)

| ID | Name | Stack args | Effect |
| :-- | :-- | :-- | :-- |
| `0x19` | `game2d_stamp_define` | `stamp_id cells_off count format` | Register a stamp: `count` cells at heap byte offset `cells_off` (`format 0` = packed `(dcol,drow)` nibble pairs, the KOTO-0138 layout). Host stores only a descriptor. |
| `0x1A` | `game2d_sprite_set` | `inst_id stamp_id x y tile_ref` | Create/update retained sprite `inst_id`: draw `stamp_id`'s cells at `(x + dcol·16, y + drow·16)`, each blitting the 16×16 tile at heap offset `tile_ref`. |
| `0x1B` | `game2d_sprite_hide` | `inst_id` | Hide sprite (footprint becomes a dirty erase next present). |
| `0x1C` | `game2d_sprite_clear_all` | — | Hide every sprite. |

`game2d_present` (`0x16`) is generalized to composite all layers in **fixed z-order**
(static → tile → sprite → text → immediate). This replaces the KOTO-0135 `Board` stream
marker; KotoBlocks migrates by dropping the marker. Immediate draw is debug/overlay/
transition/fallback only.

## Host data structures and budget

```rust
struct StampDef { cells_off: u32, count: u8, format: u8 }                          // 8 B
struct Sprite   { stamp_id: u8, x: i16, y: i16, tile_ref: i32, visible: bool }     // ~12 B
const MAX_STAMPS:  usize = 16;   // 128 B (session-stable, not diffed)
const MAX_SPRITES: usize = 32;   // 384 B, ×2 for diff snapshot = 768 B
```

Total new SRAM ≈ **0.9 KiB**; funded by the §9 / KOTO-0144 command-cap reduction. Stamp
data lives in the app heap by byte offset (and naturally in the KOTO-0139 const heap
image) — no host tile cache.

## Dirty tracking

Per present, per `inst_id`: if `(stamp, x, y, tile_ref, visible)` changed, the dirty
region is the **union of old and new footprints** (small sets of 16×16 cells). Sprites
diff by *stable instance id*, not array position, so a moving piece yields one small
stable dirty band. Compositing a sprite dirty box recomposites static→tiles→sprites→text
clipped to the box (reuse the `present_rect_banded` compose-clipped-to-rect path).

## Migration (KotoBlocks)

`stamp_define` each orientation once (reuse the existing 28-entry CELLS table); per frame
`sprite_set` the active piece, ghost, NEXT×3, and HOLD only when they move. No VM loop, no
unrolling, no per-frame blit cost.

## Dependencies

Cleaner after KOTO-0139 (stamp data as const). Enables KOTO-0143 (empty immediate list).

## Acceptance criteria

- KotoBlocks' active piece / ghost / NEXT / HOLD render as sprites with **0 per-frame
  blit commands** in the immediate list.
- Pixel-parity with the KOTO-0138 loopless-blit renderer.
- Sprite table ≤ 1 KiB SRAM.
- A falling piece produces one small, stable dirty band (no positional-diff balloon).

## Implementation (done)

- **ABI** ([runtime.rs, since moved into koto-vm](../../../src/koto-vm/src/lib.rs)): host calls `0x19`
  `game2d_stamp_define`, `0x1A` `game2d_sprite_set`, `0x1B` `game2d_sprite_hide`,
  `0x1C` `game2d_sprite_clear_all`, wired through the VM dispatch, verifier
  stack-effects, `known_host_call`, and the `name()` table. `HOST_ABI_MINOR` → 14.
  Mnemonics in [kbc-asm](../../../tools/kbc-asm/src/lib.rs); intrinsic wrappers in
  [codegen.rs](../../../tools/koto-compiler/src/codegen.rs). `VmHost` trait methods
  default to `UNSUPPORTED`.
- **Stamp format** (`0`): each cell is one nibble at `cells_off + i/2` (low nibble
  even `i`, high odd), `nibble = drow*4 + dcol` — the KOTO-0138 CELLS layout, so a
  tetromino stamp is `(CELLS + (t*4+r)*2, count 4)`. Only `format 0` is accepted;
  the descriptor stores `(cells_off, count)` (the cell bytes stay in the app heap).
- **Sim** ([host.rs](../../../src/koto-sim/src/runtime/host.rs)): `stamps`/`sprites`
  tables on `SimRuntimeHost`; `game2d_present` re-emits each visible sprite's cells
  into `draw_pixels` over the board tilemap and beneath text (the existing fixed
  paint order), reading cell offsets and tile bytes from the heap.
- **Device** ([app_host.rs](../../../src/koto-pico/src/firmware/app_host.rs),
  [app_render.rs](../../../src/koto-pico/src/firmware/app_render.rs)): `stamps`/`sprites`
  arrays in `DeviceRuntimeHost` (retained across frames like `board`, diffed via the
  current-vs-previous two-list delta). The present path composites in fixed z-order
  `static → tile → sprite → immediate`, replacing the KOTO-0135 `Board` stream marker
  (`game2d_present` is now a no-op ack). A changed sprite contributes one dirty rect
  = the union of its old and new footprints (bounding box of the stamp's cells from
  the heap), feeding the existing area/rect escalation and banded-transfer path. New
  SRAM ≈ 0.9 KiB across both lists (`MAX_STAMPS 32`, `MAX_SPRITES 16`, in
  [config.rs](../../../src/koto-pico/src/firmware/config.rs)).
- **KotoBlocks** ([main.koto](../../../apps/koto_blocks/src/main.koto)): the 28 piece
  orientations are `stamp_define`d once at title→play (reusing the const CELLS
  table), plus a baked ghost outline tile (1px `C_GHOST` border over a `C_WELL`
  interior — pixel-identical to the old per-cell outline because the well shows
  through a cell interior either way). The active piece, ghost, NEXT×3, and HOLD are
  `game2d_sprite_set`/`hide`; the immediate list carries zero piece blits in
  gameplay. `game2d_present` moved to the end of the render block so the simulator
  re-emits the final sprite state.
- **Parity:** falling-piece+ghost+previews, hold+locked-board+hard-drops, and pause
  frames are byte-for-byte identical between the KOTO-0138 baseline and the sprite
  build (sim BMP capture). Full `check_all` (tests, `build_apps --check`, golden
  frames, budgets) and the thumbv6m firmware build pass.

---

Device verification completed.

Shell is stable after reducing `MAX_APP_DRAW_COMMANDS` to 96.

KotoBlocks device run:
- code_size: 26896
- heap_request: 5015 / heap_ceiling 16384
- draw command cap: 96
- peak draw usage: 71/96
- ovf: 0
- normal falling-piece frames produce a single small dirty rect/band
  with dirty_px typically around 1.2K–2.2K
- frame=1 startup remains clean with code_tiles=2 / refills=2
- gameplay steady state uses code_tiles=4 / refills=4
- app exits cleanly and returns to Shell

This confirms the retained sprite/stamp layer works on hardware and the immediate
draw buffer reduction recovers enough SRAM headroom for stable Shell startup.