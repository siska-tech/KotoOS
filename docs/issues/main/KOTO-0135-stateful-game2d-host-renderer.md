# KOTO-0135: Stateful Game2D Tile/Sprite Host Renderer

- Status: done
- Type: design + feature
- Priority: P1
- Requirements: NFR-DRAW-2

> **Closed (board-only tilemap shipped).** Phase 1 — the retained board tilemap
> of §3/§6 — is implemented and hardware-validated (see below). The follow-up
> *static/background layer* that removes the remaining per-frame static-UI host
> calls shipped as **[KOTO-0136](KOTO-0136-game2d-static-layer.md)**; the sprite
> table / host tile cache / second layer remain deferred there. This issue is
> closed at its first cut; new Game2D work tracks under KOTO-0136 and successors.

## Implementation status (first cut)

**Phase 1 hardware validation: successful.** On the physical PicoCalc with
`MAX_APP_DRAW_COMMANDS = 160`, a heavily filled KotoBlocks board stays well under
the cap and no longer flickers:

- `phase=155 used=103/160`, `phase=160 peak=119 ovf=0`.
- `pixels=16` — the board blit term no longer grows with occupancy (was up to
  ~200 per frame on the immediate renderer).
- `dirty_px=1664 full=0` — no full repaints on lock/clear; no right-side flicker.
- `fps=10-13`.

This confirms the retained tilemap removed the fixed-board immediate-blit term, so
160 holds and the ~90 KiB stack-headroom build is preserved (no return to the 384
stopgap). The remaining per-frame cost is now VM execution (`vm_us ~75-78 ms`,
fuel ~23k, host calls ~107), i.e. PSRAM code-window thrashing (KOTO-0134), not the
draw path — see "Next" below.

The **board-only tilemap** of §3/§6 is implemented end to end as host ABI minor
`13`:

- **ABI** (`koto-core/src/runtime.rs`): host calls `game2d_set_tile` (`0x14`),
  `game2d_clear_layer` (`0x15`), `game2d_present` (`0x16`); `VmHost` trait methods
  (default `UNSUPPORTED`), VM dispatch, `known_host_call` + `host_call_stack_effect`.
- **Simulator** (`koto-sim/src/runtime/host.rs`): a retained `tilemap[200]`;
  `present` re-emits non-empty cells into the `draw_pixels` list, so the existing
  paint pipeline renders them. Verified visually (locked board, ghost, piece, and
  panels render identically) and through the budget scenario (game-over → retry).
- **Device** (`koto-pico` `app_host.rs` / `app_render.rs` / `config.rs`): the
  tilemap is a `board[200]` field of `DeviceRuntimeHost`, so the existing two-list
  delta diffs it cell-by-cell — **no separate dirty bitset**. `game2d_present`
  pushes an `AppDrawCommand::Board` marker; `paint_app_commands` composites the
  layer **at the marker's position**, under the piece/panels and over the
  well/grid — the z-order fix for the compositing-order risk. SRAM cost is
  `200*4*2 = 1.6 KiB` (stack headroom ~59 → ~57 KiB; left `MAX_APP_DRAW_COMMANDS`
  at 384 — lowering it is the SRAM-reclaim follow-up).
- **App** (`apps/koto_blocks/src/main.koto`): a `shown[200]` shadow drives
  `set_tile` only on per-cell colour change; the line-clear flash and game-over
  flat-rect board stay immediate; `game2d_present()` once per frame.

**Deferred (follow-ups):** the sprite table (`set_sprite`) for the active piece /
ghost / previews, the host tile cache (`tile_define`, IDs `0x17`–`0x1F`), a second
layer, and dropping the immediate `APP_DRAW` lists to reclaim SRAM.

### Next bottleneck (post-Phase-1)

With the draw path now idle, the frame is VM-bound: `vm_us ~75-78 ms` against a
`fuel ~23k` frame at `fps 10-13`, with the render path near-zero. That is ~3
us/instruction — the KOTO-0134 PSRAM code-window thrashing signature (8 KiB window,
KotoBlocks code spans 8 tiles, hot loop ping-pongs `main` ↔ helpers, refilling a
tile per call/return). The recommended next step is **KOTO-0134 VM/code-window
fetch diagnostics** (instrument refills/frame and the thrashing PCs) before any
window or layout change — `CODE_WINDOW_BYTES` stays at 8 KiB for now. Game2D
Phase 2 (sprites) is lower priority here: command count is already `103/160` with
`ovf=0` and `full=0`, so offloading the piece/ghost/previews would trim host calls
but not the dominant `vm_us`.

## Goal

Move KotoBlocks (and tile games after it) off the immediate-mode draw-command
model, where the app rebuilds the *entire* game screen as a fresh command list
every frame. Replace it with a **stateful host-side renderer**: the host holds the
tilemap, a sprite table, and a per-tile dirty bitset, and the app only sends the
*changes* — locked cells when they lock, the active piece when it moves, UI cells
when a value changes. `game2d_present()` then repaints only dirty tiles.

This is the structural fix the symptom-level work has been pointing at:

- KOTO-0131 tamed the positional-diff blowup (banding + full-repaint thresholds +
  metrics) but noted *"the positional diff is the root limit"* and named a stateful
  host API as "the real fix for tile games" (there called KOTO-0132).
- KOTO-0134 right-sized `MAX_APP_DRAW_COMMANDS` to 160, which **hardware showed
  flickers at ~1/3 board fill** (160/160 hit, tail dropped, full repaint
  `dirty_px=102400 full=1 fps=3`). It has been reverted to 384 as a stopgap
  (`config.rs`), at the cost of ~34 KiB `.bss` (stack headroom 92→59 KiB).
- The immediate ABI sketch in [GAME2D_ABI.md](../../spec/GAME2D_ABI.md) / KOTO-0097
  reserved `draw_tile` / `tilemap_blit` calls but kept compositing per-frame; this
  issue goes further to a **retained** model with dirty tracking.

**Scope:** design only. KotoMemo, Shell, and text apps stay out of scope (they
want a text-cell renderer, not this tile model). Do not reintroduce the KOTO-0131
2-tile PSRAM code cache.

---

## 1. Current immediate-mode draw pattern

Every frame, `koto_blocks/src/main.koto` `main()`'s render block (lines ~526–770)
re-emits the whole screen as `draw_rect` / `draw_text_color` / `draw_pixels` host
calls. Each call pushes one `AppDrawCommand` into `DeviceRuntimeHost.commands`
(`app_host.rs`); the firmware presents the list via `present_app_delta` /
`present_app_commands` (`app_render.rs`). The five emitters:

| Region | How it's emitted today | Commands/frame |
| :----- | :--------------------- | -------------: |
| **Page + well + grid** | full-screen `draw_rect` clear, well frame+fill, then a `draw_rect` per grid line (11 vertical `cx 0..=10`, 21 horizontal `cy 0..=20`) | ~35 (constant) |
| **Fixed board cells** | nested `cy 0..20` × `cx 0..10` walk; one `draw_pixels` per occupied cell (or flat `draw_rect` in game-over) | **0–200 (grows with fill)** |
| **Active piece** | `blit_piece` → up to 4 `draw_pixels`; optional `fxhi` action-flash = 4 cells × 4 outline rects | up to ~20 |
| **Ghost piece** | landing-row outline, 4 cells × 4 `draw_rect` | up to 16 |
| **Right-side UI** | title bar, NEXT (3 previews ×4 blits), HOLD, SCORE/LEVEL/LINES (re-formatted every frame), controls box — all redrawn unconditionally | ~50 (constant) |
| **Overlays** | hard-drop trail, pause / game-over panel + board sweep, "4 LINE!" / "LEVEL UP!" banners, score popup | 0–~40 |

The list is then either delta-diffed against the previous frame or fully composed.

## 2. Why the command count grows with board fill (the bottleneck)

Two compounding problems, both rooted in *re-emitting everything every frame*:

1. **Linear growth.** The fixed-board emitter issues one command per *occupied*
   cell. An empty board adds ~0; a near-full 10×20 adds ~200. Add the ~35 static +
   ~50 UI + ~40 overlay/piece commands and a busy late-game / game-over frame
   peaks at **~310–340 commands**. At 1/3 fill (~67 cells) the list is already
   ~150–160 — exactly where the (now-reverted) 160 cap clipped the tail, dropping
   the right-side panel text and triggering a full repaint.

2. **The positional delta collapses on any count change** (KOTO-0131 §Diagnosis).
   `present_app_delta` compares `command[i]` old-vs-new by index. The instant the
   board cell count shifts (lock +cells, line clear −cells, ghost ±16), every later
   index is misaligned — a panel command is diffed against a board tile, producing
   huge spurious union rects that escalate to banded or full repaints. So even when
   the cap is *not* hit, a growing board makes the per-frame work super-linear.

Raising `MAX_APP_DRAW_COMMANDS` only buys headroom against (1); it does nothing for
(2) and costs SRAM. The real fix is to stop re-emitting unchanged pixels at all.

## 3. Proposed stateful host-side Game2D model

The host retains the frame between presents; the VM mutates retained state and
calls `present`. Three host-owned structures (sketched as Rust; final home is a
`koto-core` trait + a firmware `StaticCell`, mirroring how `RASTER_STRIP` /
`APP_DRAW` are owned):

```text
Game2D host state
├─ tilemap layers   [TileCell; COLS*ROWS] × LAYERS   (cell = tile_ref, 0 = empty)
├─ sprite table     [Sprite; MAX_SPRITES]            (x, y, tile_ref, flags, live)
├─ dirty bitset     [u32; ceil(COLS*ROWS/32)]        (1 bit per tile cell)
└─ tile art         either a host tile cache, OR (cut 1) referenced by app-heap
                    offset, reusing the existing AppDrawCommand::Pixels mechanism
```

- **Tilemap layer** — a fixed grid of tile ids. KotoBlocks' board is 10×20; sizing
  the grid to the full surface is 320/16 = **20×20 = 400 cells**. A back layer holds
  the board; a front layer (or the sprite table) holds the active/ghost piece.
- **Sprite table** — a small fixed array (e.g. 16) of `{x, y, tile_ref, flags}` drawn
  *over* the tilemap at pixel (not grid) positions, with the active piece, ghost,
  and previews as sprites. `flags` carries z / flip / transparency.
- **Dirty tile bitset** — `set_tile` / `set_sprite` mark the covered cells dirty
  (a sprite move marks both the *old* and *new* covered cells). `present` walks the
  bitset, and for each dirty cell composites background tile + overlapping sprites
  into one 16×16 strip and ships one transfer, then clears the bit. This *replaces*
  the positional diff entirely — there is no previous-frame command list to misalign.

### Minimal host calls (the four the issue asks for)

| Proposed ID | Name | Stack args | Effect |
| :---------- | :--- | :--------- | :----- |
| `0x14` | `game2d_clear_layer(layer)` | `layer` | Set every cell of `layer` to empty; mark all its cells dirty. |
| `0x15` | `game2d_set_tile(layer, x, y, tile_ref)` | `layer x y tile_ref` | Write one cell; mark it dirty if changed. |
| `0x16` | `game2d_set_sprite(index, x, y, tile_ref, flags)` | `index x y tile_ref flags` | Update one sprite; mark old+new covered cells dirty. `tile_ref < 0`/`flags` clears the slot. |
| `0x17` | `game2d_present()` | — | Composite + transfer only dirty cells, then clear the bitset. |

**Companion call (required, not in the four above):** tiles must reach the host
before they can be named. Reuse the reserved `tile_define(tile_ref, ptr, len, fmt)`
from [GAME2D_ABI.md](../../spec/GAME2D_ABI.md) `0x18`, **or** for the smallest first cut,
keep tile art in the app heap and let `tile_ref` carry the heap offset — the present
path already re-reads RGB565 from the app heap by `(off,len)` for
`AppDrawCommand::Pixels`, so no host tile cache is needed on day one.

These slot into the draw block next to `draw_rect 0x10`…`draw_text_color 0x13`;
IDs are a proposal for review and overlap the GAME2D_ABI.md sketch — the two should
be reconciled into one reserved block when implemented.

### How KotoBlocks would use it (target behaviour)

- **Fixed board cells** — `game2d_set_tile(BOARD, cx, cy, tile)` only when a cell
  *locks* or *clears* (the lock loop and the row-collapse loop), not every frame.
  On a clear, the shifted rows re-`set_tile`. ~4 set_tile on a lock, up to ~40 on a
  4-line clear+collapse — vs 200 blits/frame today.
- **Active piece** — 4 sprites (or one composite sprite). Moving down is
  `set_sprite` ×4, which dirties only the ~8 cells the piece left and entered.
- **Ghost piece** — sprites with an outline tile / flag; updated only when the
  active piece moves.
- **Right-side UI** — `set_tile` / text only when SCORE/LEVEL/LINES/NEXT/HOLD/state
  *change*. A still frame redraws nothing.
- **Overlays** (banners, sweep) stay immediate via the existing `draw_rect` path
  layered after `present`, or become a sprite/scratch layer — out of scope for the
  first cut.

## 4. Expected command / work reduction

| Frame type | Immediate-mode now | Stateful target |
| :--------- | -----------------: | --------------: |
| Free-fall (piece drifting down, mid-board) | ~150–200 commands, delta usually holds | ~4–8 dirty cells (`set_sprite` ×4 + present) |
| Piece locks | ~200+ commands, **delta misaligns → banded/full repaint** | ~4 `set_tile` + present (~4–8 dirty cells) |
| Line clear (collapse) | full repaint (`dirty_px≈102400`, fps 3) | dirty = cleared rows + shifted rows only |
| Idle / paused | full list re-emitted every frame | **0 dirty cells, present is a no-op** |

The headline: per-frame transferred pixels become **proportional to what changed**,
not to board fill. The ~310–340-command worst case and the positional-diff
escalation both disappear; the 384 cap stops being load-bearing.

## 5. Expected SRAM cost

The win is that this can be **net SRAM-negative**, because it removes the need for
the two 384-entry `AppDrawCommand` lists (current + previous) in `APP_DRAW` — the
previous-frame list exists only to feed the positional delta, which dirty tracking
replaces.

| Item | Bytes |
| :--- | ----: |
| Tilemap: 20×20 `u8` × 2 layers | 800 |
| Dirty bitset: 400 bits × 2 layers | 100 |
| Sprite table: 16 × ~12 B | ~192 |
| **Stateful state, tile art kept in app heap (cut 1)** | **~1.1 KiB** |
| Optional host tile cache: 32 × 16×16 RGB565 (512 B) | +16,384 |
| **Stateful state with host tile cache** | **~17.5 KiB** |
| — for comparison — | |
| `APP_DRAW` today at 384 (two lists) | ~58.4 KiB |
| `APP_DRAW` at 160 | ~24.3 KiB |

So the first cut (tilemap referencing app-heap tile art) is **~1.1 KiB** and lets
us shrink or drop the immediate `APP_DRAW` lists — a large net reclaim of the ~59
KiB stack margin KOTO-0134/0131 have been fighting over. Even the fuller version
with a 16 KiB host tile cache is smaller than today's 384 lists. (If both the
immediate path and Game2D must coexist during migration, the peak is the sum until
KotoBlocks is fully ported.)

## 6. Smallest first implementation step

Migrate **only the locked board** to one tilemap layer; leave everything else
(active piece, ghost, UI, overlays) on the existing immediate `draw_*` path, and
keep tile art in the app heap (no host tile cache, no sprite table yet):

1. Add a single host tilemap layer (20×20 `u8`) + dirty bitset to the runtime
   `VmHost` trait, the simulator host, and the firmware `DeviceHost`, with
   `game2d_set_tile` / `game2d_clear_layer` / `game2d_present` wired through VM
   dispatch and SDK wrappers (mirror exactly how `draw_pixels_rgb565` was added).
   `tile_ref` is the app-heap offset of a pre-baked 16×16 tile (reuse the
   `AppDrawCommand::Pixels` heap re-read).
2. `present` composites dirty board cells over the existing well/grid background and
   transfers them; non-board regions keep flowing through `present_app_delta`.
3. In KotoBlocks, replace the per-frame `cy×cx` board-blit loop with `set_tile` on
   lock/clear only, and call `game2d_present` once per frame.
4. Measure on hardware against the kept `phase=160 peak=/ovf=` diagnostics: the
   board term should drop out of the command count, and full repaints on lock/clear
   should stop.

This proves the dirty-tile present path and kills the *"grows with board fill"*
bottleneck — the single reported failure — before committing to the sprite table,
host tile cache, or a second layer. Sprites (active/ghost/previews) and a host tile
cache become follow-up steps once the tilemap path is validated.

## Open questions for review

- **Tile art ownership:** app-heap-referenced (cut 1, 0 extra SRAM) vs a host tile
  cache (`tile_define`, frees the app's ~3.5 KiB tile buffer but costs ~16 KiB host
  SRAM). Recommend starting app-heap-referenced.
- **Layer count:** is one board layer + a sprite table enough, or is a second
  tilemap layer warranted for the UI region? (Leaning: sprite table for moving
  pieces, immediate path for static UI, first.)
- **ABI reconciliation:** unify these IDs with the GAME2D_ABI.md / KOTO-0097
  reservations into one draw-block range before implementing.
- **Overlay compositing order:** banners/sweep are `draw_rect` over the tilemap;
  confirm the present order keeps them above board tiles without a full repaint.
