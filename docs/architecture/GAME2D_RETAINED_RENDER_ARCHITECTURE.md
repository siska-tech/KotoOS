# Game2D Retained Rendering Architecture (post-KOTO-0138)

> **Status: design proposal.** This document steps back from the KotoBlocks-specific
> optimizations of KOTO-0135…0138 and proposes a general retained-rendering model for
> KotoOS, plus the issue roadmap (KOTO-0139…0144) to reach it. It supersedes the
> sprite/tile-cache sketch in [GAME2D_ABI.md](../spec/GAME2D_ABI.md) §"Proposed host ABI
> surface" and builds on the implemented retained board tilemap (KOTO-0135) and static
> layer (KOTO-0136).

## 1. Why this redesign

KOTO-0135→0138 made KotoBlocks fast, but every fix was a piece of *app-author
craftsmanship* the runtime should have owned:

| Fix | What the author had to do by hand |
| :-- | :-- |
| KOTO-0137 | table-lower a constant `if`-chain (`shape()`) so inline expansion didn't bloat code |
| KOTO-0138 | nibble-pack cell patterns and **unroll** the blit loop so no hot loop crosses an 8 KiB code-window boundary |
| KOTO-0136 | route chrome into a retained static layer with begin/end |
| general | bake const tables with `heap_set_u16` at startup, avoid `MAX_APP_DRAW_COMMANDS` overflow, reason about PSRAM tile locality |

These exist because the **immediate `draw_*` path is the normal frame path**: the VM
recomputes and re-emits the whole frame every tick, so per-frame VM cost and command-
list churn dominate, and the author is forced to micro-optimize VM code that should not
run per frame at all.

**Design goal:** Koto apps update *retained state*; the host renders. Authors name
tiles, sprites, and text; the host composites pixels, tracks dirty regions, and never
makes them think about code-window boundaries, unrolling, inline bloat, command-list
overflow, or per-frame redraw cost. Immediate draw becomes a fallback for debug,
overlays, transitions, and rare custom rendering.

## 2. The retained model

Four retained layers, composited by `game2d_present` in fixed back-to-front z-order,
with the immediate list on top:

```
base clear
  → static/BG layer     (KOTO-0136, retained command list; chrome/grid/fixed labels)
  → tile layer(s)       (KOTO-0135 board generalized; grid of 16×16 cells)
  → sprite layer        (NEW, KOTO-0140; placed instances of stamps)
  → text layer          (NEW, KOTO-0141; retained strings, dirty-on-change)
  → immediate list      (debug / overlay / transition / fallback only)
```

This is the classic 2D model. KotoBlocks, KotoRogue, Snake, Mines, Shogi, KotoMemo,
and the Shell all map onto it without any Tetris- or app-specific host API:

| App element | Layer | Primitive |
| :-- | :-- | :-- |
| background / well / grid / panel frame / fixed labels | static | `game2d_static_begin/end` (existing) |
| board / map / floor / locked cells | tile | `tile_set` / `tile_fill` / `tile_clear` |
| active piece / ghost / cursor / actor / icon / highlight / effect | sprite | `sprite_set` / `sprite_hide` |
| score / level / lines / menu / status / memo status | text | `text_set` / `text_hide` |
| transitions, debug overlays, one-off custom pixels | immediate | `draw_rect` / `draw_text` / `draw_pixels_rgb565` |

The KotoBlocks-specific surface the original ABI sketch hinted at (`draw_piece`,
`test_piece`, `lock_piece`, `draw_preview`) is **explicitly rejected**. A tetromino is
just a 4-cell stamp; locking is `tile_set`×4; the ghost and previews are sprites.

### Terminology decision (endorsed)

- **Stamp** = a reusable, position-independent *pattern definition* (a small set of
  cells, or a pixel bitmap). Defined once. This is the "what".
- **Sprite** = a *retained on-screen instance* of a stamp at a position with a color/
  tint. This is the "where". `sprite_hide` removes an instance; the stamp persists.

So the ABI defines stamps and manages sprites. There is no `stamp_set` — that conflated
the two. This matches the stated preference and minimizes API surface.

## 3. Generic Sprite/Stamp primitive (KOTO-0140)

A **cell stamp** is `N` cells, each `(dcol, drow)` relative to the sprite origin; the
sprite supplies the *tile* drawn in every cell (so one stamp + different tiles =
different colored pieces — exactly KotoBlocks' model, with no Tetris in the host). A
later **pixel stamp** format (a `w×h` RGB565 blit with a transparent color key) covers
free-form icons/actors; cell stamps come first because they reuse the existing tile
path and cover tetrominoes, cursors, highlights, and small actors immediately.

Stamp cell data lives in the **app heap** (and, after KOTO-0139, naturally in the const
heap image) — referenced by byte offset, exactly like tiles today (KOTO-0135). No host
tile cache, no new large host buffer.

### ABI (host-call IDs `0x19`–`0x1F`, reserved by GAME2D_ABI.md)

| ID | Name | Stack args | Effect |
| :-- | :-- | :-- | :-- |
| `0x19` | `game2d_stamp_define` | `stamp_id cells_off count format` | Register a stamp: `count` cells at heap byte offset `cells_off` (`format 0` = packed `(dcol,drow)` nibble pairs, the KOTO-0138 layout). Host stores only a descriptor. |
| `0x1A` | `game2d_sprite_set` | `inst_id stamp_id x y tile_ref` | Create/update retained sprite `inst_id`: draw `stamp_id`'s cells at `(x + dcol·16, y + drow·16)`, each cell blitting the 16×16 tile at heap offset `tile_ref`. |
| `0x1B` | `game2d_sprite_hide` | `inst_id` | Mark sprite hidden (footprint becomes a dirty erase next present). |
| `0x1C` | `game2d_sprite_clear_all` | — | Hide every sprite. |
| `0x1D` | `game2d_text_set` | `id x y ptr len rgb565` | Retained text item (see §4). |
| `0x1E` | `game2d_text_hide` | `id` | Hide a retained text item. |
| `0x1F` | `game2d_text_clear` | — | Hide all retained text. |

`game2d_present` (`0x16`) already exists and is generalized to composite all four
layers in fixed z-order (the KOTO-0135 `Board` stream marker is replaced by fixed layer
ordering — simpler and app-agnostic; KotoBlocks migrates by dropping the marker).

### Host data structures and budget

```rust
struct StampDef   { cells_off: u32, count: u8, format: u8 }      // 8 B
struct Sprite     { stamp_id: u8, x: i16, y: i16, tile_ref: i32, visible: bool } // ~12 B
const MAX_STAMPS:  usize = 16;   // 128 B
const MAX_SPRITES: usize = 32;   // 384 B
```

Stamp defs are session-stable (defined at start, never diffed). The sprite table is
diffed old-vs-new each present, so it needs a previous snapshot: 2 × 384 B = **768 B**.
Total new SRAM ≈ **0.9 KiB** — an order of magnitude less than the board (1.6 KiB) and
trivially funded by §8's command-cap reduction.

### Dirty tracking and compositing

Per present, for each `inst_id`: if `(stamp, x, y, tile_ref, visible)` changed, the
dirty region is the **union of the old footprint and the new footprint** (each a small
set of 16×16 cells). Sprites diff by *stable instance id*, not array position, so a
piece moving down produces exactly one small, stable dirty band — never the positional-
diff balloon that drives full repaints today (§6). Compositing a sprite dirty box
recomposites static→tiles→sprites→text clipped to the box (the present path already does
"clear to base, paint layers clipped to rect" in `present_rect_banded`).

### How this replaces `blit_piece`

KotoBlocks' loopless `blit_piece` (KOTO-0138) and its 5 per-frame piece blits disappear
from the VM entirely:

```
# once at start:
stamp_define(orient, CELLS + orient*?, 4, 0)   # reuse the existing 28-entry CELLS table
# per frame, only when something moves:
sprite_set(ACTIVE, orient, px, py, tile_ref(color))
sprite_set(GHOST,  orient, px, gy, ghost_tile)
sprite_set(NEXT0..2 / HOLD, ...)   # set once when the queue changes, else untouched
```

No VM loop, no unrolling, no code-window concern, no per-frame blit cost. The author
never thinks about tile boundaries because there is no per-frame draw code to straddle
one.

## 4. Retained text layer (KOTO-0141)

Today every app re-emits score/level/status as `draw_text` commands each frame. The
`Text` command inlines 64 bytes (`MAX_APP_TEXT_BYTES`), which is *why* a command costs
~80 B and the 160-cap list costs ~24 KiB across both buffers — text is the heaviest
churn on both the VM and the command list.

**Model:** `text_set(id, x, y, ptr, len, rgb565)` retains a string; the host repaints it
only when the bytes/position/color for that `id` change. `text_hide(id)` / `text_clear`.

```rust
struct TextItem { x: i16, y: i16, rgb565: u16, len: u8, bytes: [u8; 48], visible: bool } // ~56 B
const MAX_TEXT_ITEMS: usize = 16;   // 16 × 56 = 896 B, ×2 for diff = ~1.8 KiB
```

Dirty region per changed `id` = the text's bounding box. **Minimal v1** keeps the
current row-height band (`h = cell_h`, full width) so font-metric/CJK width math stays
out of scope, and simply removes the per-frame VM + command-list churn (the big win).
**v2** computes a tight horizontal box from the layout advance (mixed half/full-width for
CJK) so two adjacent text items don't repaint each other's row. **Pixel text** (the
existing `draw_text` font path) is v1; a **cell-grid text mode** for Shell/Memo dense
text is a later optimization, not in the first cut.

Scope by app: games set a handful of items on change (`text_set` only when the score
actually changes); Shell sets app labels once per page; Memo's dynamic status (`Ln/Col`,
IME mode) becomes a few items instead of per-frame text commands. Static labels stay in
the static layer (KOTO-0136) — the text layer is for values that *change*.

## 5. Const data / initial heap image (KOTO-0139)

**Problem.** KOTO-0137/0138 proved tables are the right representation, but they are
baked at runtime with `heap_set_u16` (the `store16` opcode). That bake is VM code that
runs at startup and produced KOTO-0138's frame=1 stall + a startup-only tile 0↔1 code-
window ping-pong.

**Key finding:** the `KbcHeader` *already has* `rodata_offset`/`rodata_size`, and the
verifier already range-checks them — they are simply **unused for heap initialization**
today. Reuse them as the **initial heap image**.

- **KBC model:** `rodata` becomes the initial image of `heap[0..rodata_size]`. The
  compiler places const-initialized buffers at the bottom of the heap (offsets known at
  compile time, which they already are) and emits their bytes into `rodata`.
- **Compiler output:** a `const`/`data` buffer initializer (or auto-promotion of a `buf`
  initialized from a literal array) emits bytes into `rodata` instead of `heap_set_u16`
  sequences. Mutable buffers sit above the const region.
- **Runtime load:** at app start, before `entry_word`, the host does **one `memcpy`** of
  `rodata` into `heap[0..rodata_size]` and leaves the rest zeroed. Microseconds, no VM
  execution, no code-window activity.
- **KPA / KBC format:** *no KPA change needed* — `rodata` is inside the `.kbc` asset
  already. Only the runtime loader and compiler change. (This is simpler than the "new
  package data segment" the brief floated.)
- **Validation:** add `rodata_size <= max_heap_bytes` to the verifier (it already checks
  `rodata` lies within `bytecode_size`); const buffer offsets must lie within
  `rodata_size`.
- **Migration:** KotoBlocks' shape table (KOTO-0137) and CELLS table (KOTO-0138) become
  const data; the ~56 `heap_set_u16` calls in `main` are deleted. This *structurally*
  removes the frame=1 bake stall and the startup ping-pong — they cannot recur because
  no table-bake code exists. Stamp definitions (§3) live here too.

Do this **first**: it is low-risk, removes a class of startup stalls, and makes stamp/
tile/palette data live in const memory cleanly for KOTO-0140/0141.

## 6. Full-repaint elimination (KOTO-0143)

**Causes of `full=1` today** (from `app_render::present_app_delta`):

1. `full_screen_base_color` changed (base appears / disappears / recolors).
2. `static_layer.rebuilt` (entering gameplay or a layout change) — one-shot.
3. `dirty_area >= ¾` screen.
4. `dirty_rects > 24`.
5. **Positional command-diff misalignment:** when `current.len != previous.len`, the
   `command[i]`-vs-`command[i]` compare misaligns and every later index becomes a
   spurious dirty rect, escalating into (3)/(4). This is the instability the brief calls
   out, and it is *caused by* using the immediate list for moving content.

**How the retained model removes it.** Once the piece → sprites, text → text layer, and
board → tiles, the immediate list is **near-empty in normal gameplay**, so its length
never shifts and (5) cannot fire. Sprites diff by stable id, text by id, tiles per cell —
all id-keyed, producing small, stable dirty regions instead of array-shift balloons.

**Remaining risk: a line clear** changes up to ~40 board cells at once → >24 rects →
escalates to full. Fix with **tile dirty-rect coalescing**: merge horizontally/
vertically adjacent dirty cells into bounding bands (a cleared 160-px row → one
`160×16` rect; a 4-line clear → ~4 bands, not 40). Cheap, and keeps line clears
incremental.

**Instrumentation:** add a full-repaint *reason code* to the UART metric (`BaseChange`,
`StaticRebuild`, `AreaExceeded`, `RectsExceeded`, `CommandCountShift`) so any future
full repaint is attributable rather than mysterious.

**Acceptance:** across a full KotoBlocks game (spawn → move → rotate → hold → lock →
single- and multi-line clear → game over), **zero `full=1` frames** except the one-shot
`StaticRebuild` on title→play. Immediate draw is excluded from the normal gameplay path
(debug/overlay/transition only).

## 7. Performance targets (PicoCalc / RP2040, 320×320)

24 fps = **41.7 ms/frame** total (VM + raster + transfer). Current post-0138 state:
title ~13 ms VM; gameplay ~42–60 ms VM; small-dirty raster+transfer ~8–12 ms; full
repaint >170 ms.

| Stage | Target | VM | Render (raster) | Transfer | Dirty behavior |
| :-- | :-- | :-- | :-- | :-- | :-- |
| **Baseline (playable)** | stable 15–20 fps, no multi-second stalls | < 40 ms typical | ≤ 12 ms | ≤ 12 ms | no full repaints in steady play; occasional VM spikes tolerated |
| **Good UX** | most normal frames ≤ 41.7 ms (≈24 fps) | ≤ 25 ms | ≤ 8 ms | ≤ 10 ms | small, stable, id-keyed dirty regions; **no ordinary full repaint** |
| **Stretch** | 30 fps (≤ 33.3 ms) on simple apps/games | ≤ 15 ms | ≤ 8 ms | ≤ 8 ms | dirty regions a few hundred px |

What must be true: no per-frame draw loops in the VM (sprites/tiles/text are host-side);
const data via the heap image (no per-frame or first-frame bake); app code shrinks
(deleting `blit_piece` + the table-bakes drops code tiles and refills), pushing gameplay
VM from ~42–60 ms toward the ~13 ms title regime; dirty regions bounded to sprite
footprints + coalesced tile bands + changed text boxes.

## 8. Issue roadmap

| Order | Issue | Scope | Required for comfortable play? | Risk |
| :-- | :-- | :-- | :-- | :-- |
| 1 | **KOTO-0139** ✅ Const data / initial heap image | reuse `rodata` as heap image; compiler const placement; one-`memcpy` load; migrate KotoBlocks tables | **Yes** — kills frame=1 bake stall + startup ping-pong | Low |
| 2 | **KOTO-0140** ✅ Retained Sprite/Stamp layer | `stamp_define` / `sprite_set` / `sprite_hide` / `sprite_clear_all`; id-keyed dirty diff; migrate KotoBlocks piece/ghost/preview/hold | **Yes** — removes `blit_piece` + the immediate-list churn behind full repaints | Medium |
| 3 | **KOTO-0143** Full-repaint instrumentation + tile coalescing | reason codes; coalesce adjacent dirty cells into bands; line-clear stays incremental | **Yes** — line clears must not full-repaint | Low–Med |
| 4 | **KOTO-0141** ✅ Retained text layer | `text_set` / `text_hide` / `text_clear_all`; dirty-on-change; v1 row-band; migrate KotoBlocks status text | No (needed for **Good**) | Medium |
| 5 | **KOTO-0142** Compiler inline diagnostics (short-term only) | inlined-expansion size report; code-layout map; loop-back-edge-crosses-tile warning; table-lowerable-chain hint | No (cleanup / regression guard) | Low |
| 6 | **KOTO-0144** Game2D API cleanup + retained-model docs | consolidate ABI; document the layer model for app authors; deprecate immediate-as-default | No (architectural cleanup) | Low |

**Dependencies:** 0140/0141 are cleaner after 0139 (stamp/text data as const). 0143's
coalescing depends on 0140 emptying the immediate list to be fully effective, but its
reason-code instrument can land early to guide 0140. **Required before KotoBlocks is
comfortably playable: 0139, 0140, 0143.** Architectural cleanup: 0142 (short-term),
0144. Can wait: 0141 (good-UX, not playable-blocking).

### Compiler roadmap split (KOTO-0142 and beyond)

The VM has **no `call`/`ret` and a single shared 16-slot local file**; the compiler
inlines every function and the verifier is a single linear non-CFG pass. Real
out-of-lining is therefore a deep change. Split:

- **Short-term (KOTO-0142, do now):** *diagnostics only, no ABI change.* Per-function
  inlined-expansion byte total + share of code (the report that would have flagged
  `shape` at 40,880 B / 61.9%); a code-layout map (function/source-line → word range);
  a warning when a loop back-edge body straddles an 8 KiB code-window boundary
  (automates the KOTO-0137/0138 hand analysis); a hint when a constant-return branch
  chain is table-lowerable.
- **Medium-term (deferred):** `#[noinline]`/`cold` annotations + real `call`/`ret`.
  Requires a VM calling convention with per-frame local windows **and** a control-flow-
  aware verifier — a substantial rewrite. **Not required:** the retained-render work
  (0139/0140/0141) removes the hot VM code (blit loops, table bakes) that motivated
  out-of-lining, so this buys little until apps are far larger.
- **Long-term:** optimizer auto-table-lowering of constant chains; hot/cold code layout
  that packs hot loops away from tile boundaries; basic-block alignment.

## 9. Risks, tradeoffs, and SRAM budget

- **SRAM (the binding constraint).** ~84 KiB free; the 8 KiB code window cannot grow
  (a 16 KiB window HardFaulted the boot stack, KOTO-0131), and **every field added to
  `DeviceRuntimeHost` doubles** (current + previous diff buffers). New retained state:
  sprites ~0.9 KiB, text ~1.8 KiB. **Funded by shrinking `MAX_APP_DRAW_COMMANDS`:** as
  content migrates off the immediate list (which becomes near-empty), drop the cap from
  160 → ~64. That reclaims `(160−64) × ~80 B × 2 ≈ 15 KiB` — more than the new layers
  cost — so the change is SRAM-*positive*. Keep stamp defs and text-static data out of
  the doubled host where possible (single retained instance, like `AppStaticLayer`).
- **Const heap image (0139)** is low risk: `rodata` is already verified; the loader adds
  one bounds check + one `memcpy`.
- **Sprite/text diff correctness** (0140/0141) is the main complexity: the old/new
  footprint union and the clipped recomposite must be exact, or stale pixels linger.
  Mitigated by reusing the proven `present_rect_banded` compose-clipped-to-rect path and
  golden-frame parity tests (the KOTO-0137/0138 `ImageChops` pixel-parity method).
- **Tradeoff:** fixed z-order (static→tiles→sprites→text→immediate) replaces the
  KOTO-0135 `Board` stream marker. It is simpler and app-agnostic but slightly less
  flexible (an app can no longer interleave immediate draws *under* sprites); the
  immediate list is debug/overlay-only, so this is acceptable.
- **Author experience (the point):** after this, an app author writes retained-state
  updates — `tile_set`, `sprite_set`, `text_set`, `present` — and never touches code-
  window boundaries, unrolling, inline bloat, command-list caps, or per-frame redraw.
  The efficient patterns are encoded in the host and SDK, not in each app.

## Acceptance criteria by phase

- **KOTO-0139:** KotoBlocks loads with tables as const data, zero `heap_set_u16` bakes;
  frame=1 VM time and startup code-window refills drop to the steady-state range;
  verifier rejects `rodata_size > max_heap_bytes`; pixel-parity with the baked version.
- **KOTO-0140:** KotoBlocks' active piece/ghost/NEXT/HOLD render as sprites with `0`
  per-frame blit commands in the immediate list; pixel-parity with the loopless-blit
  renderer; sprite table ≤ 1 KiB; a falling piece produces one small stable dirty band.
- **KOTO-0141:** score/level/lines/status render from the text layer; per-frame `Text`
  commands drop to `0` in steady play; a changed value repaints only its box.
- **KOTO-0143:** a full KotoBlocks game logs zero `full=1` frames except the title→play
  `StaticRebuild`; every full repaint (in any app) carries a reason code; a 4-line clear
  stays incremental (≤ a handful of coalesced band rects).
- **KOTO-0142:** the build emits the inlined-expansion report and the loop-straddles-
  tile-boundary warning; both fire on a deliberately-bloated test app.
- **KOTO-0144:** the retained model is documented for app authors; immediate draw is
  documented as fallback/debug/overlay only; the SDK prelude exposes the new primitives.
