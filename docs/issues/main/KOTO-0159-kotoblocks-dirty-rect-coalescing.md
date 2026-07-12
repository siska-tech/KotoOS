# KOTO-0159: KotoBlocks event-frame dirty-rect coalescing

- Status: in-progress (implemented; hardware UART confirmation pending)
- Type: performance
- Priority: P1
- Requirements: NFR-RT-2

Source of truth: [GAME2D_RETAINED_RENDER_ARCHITECTURE.md](../../architecture/GAME2D_RETAINED_RENDER_ARCHITECTURE.md) §6,
following [KOTO-0143](KOTO-0143-full-repaint-instrumentation-coalescing.md) (tile-cell
band coalescing) and the KOTO-0157/0158 app-side effect optimizations.

## Symptom

KotoBlocks **steady** CodeWindow refills are now 0, but **event** frames still show
huge raster times over a tiny dirty area. From a hardware UART sample:

```
frame=1440: vm_us=34029, raster_us=36110, transfer_us=3375, dirty_px=256, rects=16, refills=0
```

`dirty_px=256` (one 16x16 tile's worth) yet `raster_us=36110` across `rects=16` —
the cost tracks the dirty-rect **count**, not the dirty **area**.

## Root cause: raster is per-pass, not per-pixel

`present_app_delta` recomposites the whole retained layer stack — static chrome →
board tilemap → sprites → text → immediate commands — **once per dirty rect**,
clipped to that rect's viewport. Two facts make each pass expensive regardless of
how small the rect is:

1. `paint_board_layer` iterates all `GAME2D_BOARD_CELLS` (200) cells every pass.
2. `Canvas::blit_rgb565` loops the full `w*h` of each tile (256 px for a 16x16),
   relying on `put_pixel`'s per-pixel bounds check to clip — so a tile that lands
   entirely outside the viewport still costs 256 bounds checks.

So each pass is ≈ `200 cells * 256 px` ≈ 51 k `put_pixel` calls (plus sprites/text/
commands), **independent of the rect's size**. With 16 scattered rects that is ~16×
the whole-scene work — `raster_us ≈ passes × full_scene`, which is exactly the
observed `36110 / 16 ≈ 2257 µs` per pass.

The fragmentation comes from transient effects (line clear, hard drop, game over)
dirtying many small, scattered board cells / sprites / text rows in one frame. Each
fragment was its own recomposite + transfer pass.

## Fix

### 1. Diagnostics (gated, throttled) — confirm the hypothesis on hardware

`present_app_delta` now records a `DirtyRectGeometry` snapshot, logged on the same
throttled cadence as `phase=160` (and only when the incremental delta path ran):

```
phase=164 dirty-rects app=… frame=… rects_pre=… rects_post=… area_pre=… bbox=… max_area=… min_area=… sample=x,y,w,h;…
```

A high `rects_pre` over a tiny `area_pre`, with `bbox` far larger than `area_pre`
(scattered, not one block), is the per-rect-overhead signature. `rects_post` shows
how far coalescing collapsed the passes. Gated behind
`not(psram_qpi_code_window_prod_profile)` and throttled, so it never floods UART.

### 2. Conservative dirty-rect coalescing — cut the pass count

- `koto-core` gains a pure, host-tested `coalesce_rects(rects, len, max_waste)`
  (in `dirty_tiles.rs`): it merges two dirty rects into their bounding box when the
  box wastes ≤ `max_waste` pixels beyond the area the pair already covers
  (`bbox_area − area_a − area_b`). Nested/edge-adjacent rects merge even at
  `max_waste = 0`; far-apart rects never merge. Merging only ever **grows** a rect,
  and the merged set's union always covers every input rect, so recompositing the
  scene clipped to the merged rects reproduces identical pixels — no dirty region is
  dropped. (Same correctness argument as KOTO-0143 bands and existing union rects.)

- `present_app_delta` is restructured to **collect** every dirty rect (immediate
  command diff, board bands, sprite footprints, text rows) into one bounded working
  set, then coalesce it with `DIRTY_COALESCE_MAX_WASTE` (~four tiles, 1024 px), then
  run a single unified transfer loop. This replaces the previous four separate
  per-source transfer loops, so a frame with board + sprite + text fragments now
  composites them in a handful of merged passes instead of one pass per fragment.

The escalation policy is **unchanged**: the working set is capped at
`FULL_REPAINT_RECTS + 1`; overflowing it, exceeding the area threshold, or a board
band-buffer overflow still escalates to a full repaint with the same
`FullRepaintReason` priority (`CommandCountShift` > `AreaExceeded` > `RectsExceeded`).
Escalation is decided on the **pre-coalesce** counts, so coalescing never introduces
a full-screen redraw — it only reduces passes on frames that already stayed
incremental (the `rects=16 < 24` slow case).

## What it does NOT change

VM semantics, opcode values, bytecode ABI, hostcall IDs, `RuntimeLimits`, the PSRAM
backend, and CodeWindow policy are all untouched. Steady `refills=0` is unaffected
(coalescing is firmware-side, post-VM). Visible output is preserved (coalesced rects
only grow the recomposited region; pixels are identical).

## Acceptance

- `cargo test -p koto-vm` — ok.
- `cargo test -p koto-core -p koto-sim` — ok (incl. 8 new `coalesce_rects` tests).
- `cargo build -p koto-pico --target thumbv6m-none-eabi --bins` — ok.
- KotoBlocks still runs on hardware.
- Hardware `phase=164` logs confirm whether slow event frames are dominated by
  dirty-rect fragmentation, and `rects_post` < `rects_pre` with `raster_us` falling
  roughly in proportion on frame=1440-like cases, without `transfer_us` ballooning.

## Remaining work

- Device UART run: capture `phase=160` + `phase=164` across a full game (spawn →
  move → lock → single/multi-line clear → hard drop → game over) to confirm the
  fragmentation signature and the post-coalesce `raster_us` reduction, and to tune
  `DIRTY_COALESCE_MAX_WASTE` against real geometry.
- Follow-up opportunity (not in this change): clip `Canvas::blit_rgb565`'s loop
  bounds to the viewport so a fully-clipped tile costs O(1) instead of 256
  `put_pixel` calls — an output-preserving per-pass speedup orthogonal to pass-count
  coalescing.
