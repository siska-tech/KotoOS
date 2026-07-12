# KOTO-0138: KotoBlocks Loopless `blit_piece` Cell Table

- Status: done
- Type: bug
- Priority: P2
- Requirements: NFR-RT-2

## Goal

[KOTO-0137](KOTO-0137-koto-blocks-shape-table.md) removed the main KOTO-0136 code-
window regression (shape `if`-chain → heap table) and stays closed. A **smaller
residual** tile 2 ↔ tile 3 ping-pong remained, visible only late in a run:

```
frame=810/840/870/900: refills=36 code_tiles=4 cw_hist=0:1,1:1,2:17,3:17
                       cw_trans=2>3:17,3>2:16 vm_us≈158-175ms pixels=20 text=4
```

This is a separate issue from KOTO-0137 and is tracked on its own.

## Diagnosis

- It appears **only after HOLD is used**: `pixels=20` is five piece blits (active +
  3 NEXT + **HOLD**); the baseline `pixels=16` had the HOLD slot empty.
- The bytecode layout map (post-KOTO-0137, `code_size=26704`, 4 tiles) put the tile
  2/3 boundary (word 6144, byte 24576) inside the **HOLD-preview `blit_piece` inlined
  at [main.koto](../../../apps/koto_blocks/src/main.koto) L748** (`if hashold != 0 {
  blit_piece(hold, 0, 246, 163); }`).
- `blit_piece` was a `while i < 16` scan of the 16-bit occupancy mask. The loop body
  straddled word 6144, and **the loop-back branch re-crossed the boundary every
  iteration** → ~16 crossings each way = `2>3:17 / 3>2:16`, `refills` 4 → 36. The
  other four blits sat within a tile, so before HOLD was used the render path ran
  sequentially (`refills=4`).

So the residual was the HOLD-preview `blit_piece` 16-iteration draw loop straddling
the tile 2/3 boundary; the *loop-back branch* was the ping-pong mechanism.

## Fix (Variant B: loopless `blit_piece`)

Two variants were measured. **A** (keep a 4-iteration loop over a cell table) only
shrank the crossing count (refills ~12) — the loop could still straddle and re-cross —
and pushed local slots to 44/45. **B was chosen:** remove the loop entirely. Every
tetromino is exactly four cells, so a loopless body crosses a tile boundary **at most
once** (no re-entry) and *cannot* produce repeated `17/16` transitions, regardless of
where the inlined copy lands. That is a structural guarantee, not a layout accident.

- **`const CELLS = 4046`** — heap offset of a new `buf cells[56]` (declared right
  after `shapes`, so `CELLS == SHAPE_TBL + 56`). Each of the 28 u16 entries packs the
  four occupied cell indices (`row*4 + col`, 0–15) of one orientation as four nibbles.
- `main` bakes the table once (28 `heap_set_u16`) right after the KOTO-0137 shape
  table. The packed values are derived from the shape masks and were verified to
  round-trip back to each mask exactly, so rendering is unchanged.
- `blit_piece` reads the entry and draws the four cells **straight-line, no `while`**.
  It no longer calls `shape()`; the shape mask table (KOTO-0137) stays for the
  collision / lock / ghost loops, which are unaffected.

## Results

| metric | KOTO-0137 | KOTO-0138 (B) |
| :--- | ---: | ---: |
| code segment | 26,704 B | **30,272 B** (+3,568) |
| PSRAM code tiles (8 KiB) | 4 | **4** |
| user local slots | 43/45 | **43/45** (the unrolled body reuses one temp) |
| heap | 4,447 B | 4,503 B (+56) |
| `.kbc` file | 37,008 B | 41,304 B |

A whole-program back-edge analysis confirms **no hot gameplay inner loop crosses a
tile boundary**: the only boundary-crossing back-edges are the static-layer grid
build (runs once on title→play), the title-screen `continue` (state 0 only), and the
outer per-frame `loop` wrap (the one crossing already in baseline `refills=4`). The
HOLD/NEXT/active `blit_piece` expansions are loopless.

**Rendering parity:** a mid-gameplay capture with HOLD in use is **pixel-for-pixel
identical** to the original if-chain renderer — `ImageChops.difference(...).getbbox()`
is `None`. ABI, `CODE_WINDOW_BYTES`, `MAX_APP_DRAW_COMMANDS=160`, the KOTO-0137 shape
table, and the KOTO-0136 static layer are all unchanged. The app build/drift gate
(`harness/build_apps.py --check`) passes.

## Expected on-device effect (to confirm on hardware)

- Normal gameplay unchanged: `refills=4`, `code_tiles=4`, no catastrophic ping-pong.
- HOLD-used (frame 810-style) case: the loop-back ping-pong is gone, so
  `cw_trans=2>3:17,3>2:16` should disappear and `refills` should fall from 36 toward
  the ~4–5 baseline (≤8), with `vm_us` dropping back toward the normal 42–60 ms range.

Compare `phase=160/163` for a pre-HOLD frame and a post-HOLD frame and confirm the
`2>3` / `3>2` transition counts no longer appear.

## Notes

The fix also slightly speeds every frame: each of the five piece blits now runs four
draw iterations instead of sixteen mask-scan iterations. The ~+3.6 KiB code cost
keeps KotoBlocks at 4 PSRAM tiles, well under `DEVICE_CODE_CEILING`. A larger future
option — representing *all* the piece loops (spawn/move/rotate/gravity/lock/ghost) as
4-cell walks and dropping the mask table — would net-shrink code and cut `vm_us`
further, but it touches ~9 loops and is out of scope here (it is not required to
remove the residual ping-pong, which this loopless-blit change does structurally).
