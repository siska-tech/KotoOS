# KOTO-0174: present-path (raster/transfer) cost attribution and reduction

- Status: DONE — closed 2026-07-08 with H-P (device ~3× raster/px, quiet fps
  43 → 111; H-A/H-B reverted), then **re-opened and re-closed 2026-07-11 with
  H-A2 kept**: the `phase=178` boot bench proved embassy-rp's SPI write yields
  completely during the data DMA (the original H-A revert was a structural
  bug, not a HAL limit), and the rebuilt zero-RAM pipelined present hides the
  DMA on device (heavy-frame transfer_us ~31 ms → ~14 ms, fps 13–15 → 18–23,
  full repaint 109 → 76 ms). Present is now CPU-bound (vm / raster / convert);
  the fps-8 hitches were [KOTO-0175](KOTO-0175-kotorun-commandcountshift-full-repaints.md)
  (also DONE). Opened 2026-07-07 after KOTO-0173 made the present path the
  largest term.
- Type: performance investigation (render/present)
- Priority: P2
- Requirements: NFR-PERF-1

Source of truth:
[app_render.rs](../../../src/koto-pico/src/firmware/app_render.rs)
(`present_app_delta`: per-dirty-rect full-stack recomposite, then transfer),
[lcd.rs `write_rgb565_rect`](../../../src/koto-pico/src/lcd.rs#L290) (CPU
RGB565→RGB666 conversion + `RAMWR` SPI write per rect), the koto-gfx
compositor (GFX-0004; host-testable), `phase=160`'s
`raster_us`/`transfer_us`/`dirty_px`/`rects` fields.

Relates to: [KOTO-0173](KOTO-0173-two-tile-code-window-cache.md) (removed the
VM-side refill tax; raster is now the largest term),
[KOTO-0169](KOTO-0169-vm-frame-cost-attribution.md) (the attribution-first
method this issue reuses), GFX-0011/GFX-0013 (already minimized *what* gets
repainted; this issue is about the cost of repainting it),
[KOTO-0159](KOTO-0159-kotoblocks-dirty-rect-coalescing.md) (rect count already coalesced),
[KOTO-0172](KOTO-0172-main-task-stack-frame-reduction.md) (the ~67 KiB margin
any buffer spend draws from; `phase=176` guards it).

## Baseline (device, 2026-07-07, KotoRun steady frame)

```
phase=160 ... vm_us=8102 raster_us=9557 transfer_us=3408 dirty_px=4184 rects=2 fps=43 lat_ms=22
```

The ~23 ms frame is now: **raster 9.6 ms > vm 8.1 ms > transfer 3.4 ms**,
executed strictly serially (raster rect → convert → DMA wait → next rect).
Two facts stand out:

- **9,557 µs of raster for 4,184 dirty px is ~2.3 µs/px** — an order of
  magnitude above a plausible clipped-fill cost. Stage 0c (below) resolved
  the split: it is **per-pixel paint** (88%), glyph rasterization first, each
  pixel routed through `Canvas::put_pixel`'s redundant per-pixel clip — not a
  per-command fixed cost (12% ceiling) nor the base clear (16%).
- **`transfer_us` is not transfer**: `write_rgb565_rect` first runs a
  byte-wise CPU RGB565→RGB666 conversion (3 B/px), then awaits the SPI DMA.
  The CPU-vs-DMA split inside 3,408 µs is also unmeasured.

## Stage 0 — attribute, host + device (observe only)

1. **Host microbench (koto-gfx is host-testable):** replay a captured
   KotoRun-shaped frame (command mix: 58 rects + 10 text + 8 static, two
   dirty rects of ~4.2k px) through the compositor and attribute time per
   phase — clear, per-layer walk (static/board/sprites/text/immediate),
   per-command clip-reject vs paint, glyph path. Host absolute times differ
   from device but ratios locate the dominant term.
2. **Device breakdown line (`phase=177`, DIAG-gated under GfxDebug):**
   - inside the present loop: per-rect `raster_us` split into
     `clear/static/board/sprites/text/imm` (coarse per-layer timers around
     `paint_app_commands`);
   - inside `write_rgb565_rect`: `convert_us` vs `dma_us`.
   Sparse cadence like `phase=175`; the canary rule applies (measure cost,
   keep it trivial).

**Gate:** a table attributing ≥90% of `raster_us` and `transfer_us` on a
KotoRun steady frame (plus one KotoShogi event frame as a second shape).

### Stage 0c record — host attribution (2026-07-08, decisive)

[`present_attribution.rs`](../../../src/koto-gfx/tests/present_attribution.rs)
replays the KotoRun command mix (8 static, 58 immediate rects, 10 immediate
text; 2 dirty rects ≈ 4.2k px) through the real koto-gfx compositor. Run:
`cargo test -p koto-gfx --test present_attribution -- --ignored --nocapture`.
20k iterations, ns per frame (2 rects):

| phase | ns/frame | share of FULL |
| --- | --- | --- |
| clear (1 fill pass) | 2,506 | 16% |
| static list (8 cmds) | 4,928 | 32% |
| immediate list (68 cmds) | 7,932 | 52% |
| &nbsp;&nbsp;· 58 rects | 2,247 | — |
| &nbsp;&nbsp;· 10 text | **5,818** | — |
| **FULL recomposite** | **15,334** | 100% (sum accounts 100%) |
| clip-reject floor (76 cmds, nothing painted) | 1,839 | **12%** |
| convert 565→666 | 2,371 | (15% of raster-equiv) |

**Findings (two "obvious" hypotheses falsified, per the KOTO-0169 rule):**

1. **Not the base clear.** The clear is only **16%** of raster. Pipelining or
   SRAM-placing *only* the clear addresses a minority.
2. **Not per-command overhead.** The pure walk of all 76 commands with nothing
   painted (clip-reject floor) is only **12%** — so H-C culling has a ~12%
   ceiling. Non-intersecting commands are already cheap to reject.
3. **It is per-pixel paint: 88% of the frame.** And within it, **glyph
   rasterization dominates** — 10 short text items cost 5,818 ns vs 2,247 ns
   for 58 rect fills (2.6×). Every painted pixel (fill, blit, and *every set
   glyph pixel*) funnels through `Canvas::put_pixel`, which re-runs a
   4-comparison viewport clip + coordinate translation + `to_le_bytes` per
   pixel — redundant for `fill_rect`/`blit_rgb565` (their loops already clip
   to `x0..x1`/`y0..y1`) and for `draw_glyph`.

Host cannot capture the device's XIP-flash amplification (the ~2.3 µs/px is
545× the host's 4.2 ns/px FULL) — that is exactly what `phase=177` +
SRAM-placement (H-B) measure/address. But the *structure* is decisive and the
lever choice is robust to the amplification being uniform or glyph-concentrated:
either way, making each `put_pixel` cheaper (hoist the per-pixel clip; write
row-contiguous runs) and/or SRAM-placing those loops targets the 88%.

### Stage 0a/0b — device instrumentation landed

`PaintMetrics` now records `clear_us` (timed around each `canvas.clear`), and
`phase=177 app-present-cost` emits `raster_us / clear_us / stack_us(=raster−clear)
/ transfer_us / dirty_px / rects / rect / text / pixels / static_cmds` on the
render cadence under `DiagClass::Gfx` (build with `DIAG_PROFILE = Gfx`).
Observe-only; two extra clock reads per rect. This confirms on hardware that
the clear is a minority and the command-stack composite (`stack_us`) is the
bulk — the device counterpart of the host table.

#### Device cross-check (2026-07-08, `DIAG_PROFILE = Gfx`, KotoRun) — gate PASSED

Steady incremental frames (excluding the periodic full repaints, below):

| frame | raster_us | clear_us | stack_us | dirty_px | clear share | clear µs/px |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 30 | 3,439 | 478 | 2,961 | 1,808 | 14% | 0.264 |
| 90 | 12,586 | 2,201 | 10,385 | 8,623 | 18% | 0.255 |
| 150 | 14,270 | 2,202 | 12,068 | 8,484 | 15% | 0.260 |
| 300 | 9,021 | 1,614 | 7,407 | 6,398 | 18% | 0.252 |
| 330 | 8,425 | 1,441 | 6,984 | 5,332 | 17% | 0.270 |

- **clear share ≈ 14–20% (mean ~17%) — matches the host's 16%.** The device
  agrees: the base clear is a minority, `stack_us` (~83%) is the bulk.
- **The base-clear per-pixel cost is stable at ~0.26 µs/px** — the raw
  `put_pixel` fill cost on device. `stack_us` runs ~1.2–1.6 µs/px, i.e. 5–6×
  the clear, because the stack makes *multiple* put_pixel passes over each
  dirty pixel (the static full-screen base rect re-fills the whole dirty area
  a second time after the clear, then the immediate rects and text glyphs
  paint over it). This is exactly the per-pixel-paint domination the host
  bench predicted, now confirmed with device µs/px.
- **Incidental finding:** the static layer's full-screen base rect duplicates
  the `canvas.clear(base)` over each dirty rect (~one clear-pass, ~16%
  redundant). Skipping the base rect when it equals the clear color is a cheap
  secondary win (H-C-adjacent; fold into Stage 1 if convenient).
- **Out-of-scope observation:** frames 1/60/450 are full-surface repaints
  (`dirty_px=102400 rects=20`, `raster ≈ 97–125 ms`). A *recurring* full
  repaint every ~few hundred frames is a dirty-policy issue (KOTO-0159 /
  GFX-0011), not present-cost — flagged for a separate look; a ~100 ms hitch
  is worth its own investigation.

## Stage 1 — levers, picked by the Stage-0 numbers

Stage 0c makes the primary lever unambiguous: **cheapen the per-pixel paint
path** (88% of the frame), glyphs first. Ordered by expected value; each is
independently landable and A/B-able on `phase=160`/`phase=177`:

- **H-P (primary): hoist the per-pixel clip out of the raster inner loops.**
  `Canvas::fill_rect`/`blit_rgb565`/`draw_glyph` currently call `put_pixel`
  per pixel, which re-derives the viewport clip + index every pixel. Compute
  the clipped span once and write a contiguous row run directly into the
  compact viewport buffer (`fill_rect`: memset-style row fill; `blit`: row
  `copy_from_slice`; `draw_glyph`: clip the glyph box once, then set bits).
  Pure koto-gfx, host-testable (existing raster tests + the microbench guard
  the pixels are byte-identical), **zero RAM cost**, benefits every app/frame.
  This is where the 88% lives; do it first and re-measure before anything
  with a RAM cost.
- **H-B: SRAM-place the compositor inner loops** (the KOTO-0169 Stage-2
  pattern, −16% precedent): once H-P shrinks the algorithmic work, a
  `.data`-placed twin of the fill/glyph/convert loops removes the flash-XIP
  amplification (the 545× host→device factor). A few KiB of RAM from the
  ~67 KiB margin; only if `phase=177` after H-P still shows per-pixel time
  far above a store.
- **H-A: overlap raster/convert with DMA (pipelining).** Secondary — the
  Stage-0 device `transfer_us` is 3.4 ms and only part is DMA wait (host
  convert ≈ 15% of raster-equiv). Double-buffer the strip pair to raster
  rect N+1 while rect N's RGB666 is on the DMA. Budget: +10.2 KiB (2nd raster
  strip) + 15.4 KiB (2nd RGB666 scratch) ≈ 25.6 KiB from the ~67 KiB margin.
  Weigh only after H-P/H-B; may be unnecessary if raster drops below the DMA.
- **H-C: per-rect command culling** — **deprioritized.** Stage 0c bounded its
  ceiling at ~12% (the clip-reject floor); not worth the GFX-0013 footprint
  plumbing unless a many-band frame changes the ratio.
- **H-D: cheapen the conversion** — small (convert ≈ 15% of raster-equiv);
  word-at-a-time / small-LUT only if `phase=177` `convert_us` says otherwise.

**Target:** KotoRun quiet-frame fps 43 → **55+** (raster ~9.6 → ~5 ms via H-P,
optionally H-B), no regression on KotoShogi/KotoBlocks event frames or shell
navigation latency.

### H-P landed — host 2.6× on the full recomposite (2026-07-08)

`Canvas::fill_rect`/`blit_rgb565`/`draw_glyph` rewritten to clip once and write
row-contiguous runs (fill: 2-byte pattern per row; blit: `copy_from_slice` per
row; glyph: clip the box once, then direct-index the set bits). `put_pixel` is
unchanged and stays the reference; three new `hp_*_matches_put_pixel_reference`
tests assert byte-identity across negative-origin / straddle / offset-viewport
clip cases, and the koto-sim frame goldens (124 fixtures) pass unchanged.

Microbench before/after (same 2-rect KotoRun shape, ns/frame):

| phase | before | after | speedup |
| --- | ---: | ---: | ---: |
| clear (1 fill pass) | 2,506 | 103 | **24×** |
| static list (8) | 4,928 | 467 | 10.6× |
| immediate — 58 rects | 2,247 | 551 | 4.1× |
| immediate — 10 text | 5,818 | 4,164 | 1.4× |
| **FULL recomposite** | **15,334** | **5,924** | **2.6×** |

- Fills vectorize brilliantly (clear 24×, rects 4.1×). Zero RAM cost.
- **Glyph rasterization is now the residual: text is ~70% of the smaller
  FULL** (4,164 of 5,924 ns). draw_glyph still bit-tests each glyph cell and
  writes pixels individually (glyphs are sparse bitmaps); the clip hoist
  removed the per-pixel viewport branch but not the per-set-pixel write. This
  is the next lever (H-P2: batch glyph row bits / a glyph fast-path, or H-B
  SRAM-placement of the glyph loop).

Host cannot see the device XIP factor, so the device win will differ — the
`phase=160`/`phase=177` A/B on hardware is the gate. Expected: raster drops
sharply on the fill-heavy frames; glyph-heavy frames improve less.

#### H-P device A/B (2026-07-08, `DIAG_PROFILE = Gfx`, KotoRun) — CONFIRMED

Per-pixel cost is the clean cross-run metric (command mix unchanged: static 8,
rect 40-66, text 4-9):

| metric | pre-H-P | post-H-P | device speedup |
| --- | ---: | ---: | ---: |
| clear µs/px | 0.26 | ~0.053 | **~4.9×** |
| raster µs/px | ~1.46 | ~0.48 | **~3.0×** |

- **H-P delivered ~3× device raster/px** (clear ~5×) — the host 2.6× translated
  to hardware, XIP/no-SIMD capping the fill speedup below the host's 24×.
- **Quiet-frame target met and exceeded.** Genuinely idle frames (300–450:
  `dirty_px=0`) now run at **fps 111–112** (VM-bound at `vm_us≈7.5 ms`); the
  original 4,184 px steady frame extrapolates to ~2 ms raster (was 9.6 ms), so
  its fps clears 55+ easily. The 43 fps baseline is gone.

**But the bottleneck moved — the present path is now transfer-bound on
redraw-heavy frames.** With raster cheap, `transfer_us` (RGB666 CPU convert +
SPI DMA) is now the dominant present term:

| frame | dirty_px | raster_us | transfer_us | transfer/raster | fps |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 30 | 34,752 | 16,139 | 28,135 | 1.7× | 19 |
| 90 | 34,010 | 16,740 | 28,046 | 1.7× | 18 |
| 240 | 48,587 | 24,460 | 39,928 | 1.6× | 12 |
| 570 | 15,529 | 9,506 | 12,079 | 1.3× | 34 |

Transfer is a near-constant **~0.80 µs/px** (3 B/px over SPI ≈ 4 MB/s), vs
raster's new ~0.48 µs/px — so transfer is ~62% of present on any heavy frame.
Two out-of-present-cost observations from the same capture:

- **Recurring full repaints are the worst hitches** (frames 1/120/180:
  `full=1 full_reason=StaticRebuild|CommandCountShift`, `dirty_px=102400`,
  ~100 ms frames, **fps 8**). This is a dirty-policy issue (GFX-0011
  `CommandCountShift`), not present-cost, and is the single biggest KotoRun fps
  problem now — worth its own issue (see re-scope below).
- The heavy incremental frames redraw 30–73k px (this capture caught active
  gameplay), so even at 3× raster they cost 14–35 ms; the fps there is set by
  transfer + repaint policy, not raster.

## Non-goals

- No full framebuffer (320×320×2 = 200 KiB does not exist on this part) and
  no retained-surface architecture change — GFX-0011/0013 already minimize
  the repainted area; this issue only cheapens the repaint itself.
- No VM/interpreter work (KOTO-0169 closed its ledger; H2-c stays parked).
- No dirty-rect policy changes (KOTO-0159/GFX-0011 own that).

## Re-scope after H-P (2026-07-08)

H-P closed the raster question: quiet frames are no longer present-bound
(VM-bound at ~7.5 ms, 111 fps), and on redraw-heavy frames raster is now the
*smaller* present term. The remaining present-path work, re-ordered by the A/B:

- **H-A: overlap CPU raster+convert with the SPI DMA.** Pipeline rect N+1's
  raster+convert under rect N's DMA. **SPI-clock analysis revises the ceiling
  down (2026-07-08):** the LCD SPI runs at 62.5 MHz, so the RGB666 DMA floor is
  24 bits/px ÷ 62.5 MHz = **~0.38 µs/px**. Of the measured `transfer_us`
  ~0.80 µs/px, only ~0.38 is the DMA — the other **~0.42 µs/px is the CPU
  RGB565→666 convert**. So present per-px is raster 0.48 + convert 0.42 + DMA
  0.38 ≈ 1.28, CPU-dominated (0.90) not DMA-dominated. A pipeline hides only
  `min(cpu, dma) = dma ≈ 0.38` → **~30% present, not the ~1.6× first claimed.**
  Cost is +15.4 KiB (one extra RGB666 scratch; the RGB565 strip stays single —
  it's freed by convert before the next raster), `phase=176` re-measured.
- **H-D: cheapen/SRAM-place the RGB666 convert (now co-primary with H-A).** The
  convert is ~0.42 µs/px of CPU — nearly the size of raster and *bigger than the
  DMA*, and it sits on the CPU critical path the pipeline can't shrink. If it is
  flash-XIP-slow, SRAM-placing the convert loop (KOTO-0169 Stage-2 pattern, a
  few KiB) cuts the dominant term directly; word-at-a-time / rasterize-to-666
  are the algorithmic variants (rasterize-to-666 touches the shared Canvas —
  own sign-off). H-A + a cheaper convert together approach ~2× present.
- **Step taken (2026-07-08): split the LCD write + measure.** `write_rgb565_rect`
  is refactored into `convert_rgb565_to_rgb666` (CPU) + `transfer_rgb666` (SPI
  DMA) — the primitives the pipeline needs — and the present path times each
  into `PaintMetrics.convert_us`/`dma_us`, emitted on `phase=177`. Behaviour-
  identical, `.bss` unchanged. This split stays in the tree.

#### Device convert/DMA split (2026-07-08) — measured

KotoRun incremental frames, `phase=177` `convert_us`/`dma_us`:

| term | µs/px |
| --- | ---: |
| raster | ~0.48 (CPU) |
| convert | ~0.24 (CPU) |
| DMA | ~0.55 (~1.45× the 62.5 MHz wire floor of 0.38) |

CPU total (raster+convert) 0.72, DMA 0.55. On paper a pipeline hiding the DMA
would give present 1.27 → 0.72 ≈ 1.76×.

#### H-A pipeline — ATTEMPTED and REVERTED (2026-07-08, commits 80266c4 → ef8543d)

The 1-stage pipeline was implemented (split `transfer_rgb666` into
`begin_rgb666` + `write_rgb666_data`, `join` the previous rect's data DMA with
the current rect's `raster_convert_rect`, ping-pong two RGB666 scratch buffers).
**It did not deliver and was reverted:**

- **No measured overlap.** On the H-A build (KotoRun, `phase=160`), present
  `(raster+transfer)/px` came out ~1.05–1.39 (mean ~1.23) vs ~1.24–1.27 before
  — essentially unchanged, and frame-870 was *worse*. present wall ≈ the
  sequential `raster + convert + DMA`, i.e. the CPU `raster_convert_rect` future
  and the SPI data DMA did **not** run concurrently. The assumption that
  embassy-rp's `Spi<Async>::write().await` frees the CPU for a joined synchronous
  future during the transfer did not hold in practice on this path (or the
  per-rect `begin_rgb666` prologue overhead cancelled the gain). Confirming the
  actual embassy SPI-DMA yield behaviour would be a prerequisite to retrying.
- **Broke the Gfx dev-profile boot.** The +15.4 KiB scratch dropped static free
  93,616 → 78,256 B (`phase=176 free_min` 67 KiB → 51 KiB). `DIAG_PROFILE = Gfx`
  compiles a larger main-task poll frame (the extra `phase=177/171/174` locals),
  and the reduced headroom tipped its boot stack over the ceiling — Gfx no longer
  booted while Audio (the shipping profile) did. Reverting restored both.

**Verdict:** transfer overlap is not reachable cheaply here. The DMA is
SPI-clock-bound (0.55 µs/px) and the shipping present is already CPU-bound
(raster+convert 0.72); the remaining lever is **H-B: SRAM-place the compositor
fill/glyph/convert loops** (KOTO-0169 Stage-2 pattern, few KiB, no async, no
DMA), which attacks the dominant CPU term directly. Or accept the present cost:
after H-P, KotoRun's quiet frames are already VM-bound at 111 fps, and its heavy
frames are the KOTO-0175 dirty-policy hitches, not per-pixel present cost.

- **H-P2 (deprioritized): glyph inner loop.** Still the raster residual, but
  raster is no longer the frame bottleneck — revisit only if a glyph-heavy
  app (not KotoRun) shows raster-bound frames.
- **Out of scope but highest fps value: the recurring `CommandCountShift` /
  `StaticRebuild` full repaints** (fps 8 hitches). Belongs to GFX-0011 /
  KOTO-0159 dirty policy — recommend a dedicated issue; it dwarfs the present
  micro-optimizations for KotoRun's worst frames.

### H-B — ATTEMPTED and REVERTED (2026-07-08, commits e0e3dff → 423efcf)

SRAM-placed the koto-gfx raster leaves (`fill_rect`/`blit_rgb565`/`draw_glyph`)
+ `convert_rgb565_to_rgb666` in `.data.koto_gfx_raster` (`inline(never)`,
default-on `sram_raster`, +2 KiB RAM). **No measured raster win, reverted:**

- Device `phase=160 raster_us` over 21 KotoRun incremental frames (dirty > 5k px)
  aggregated to **0.60 µs/px** — *not below* the pre-H-B ~0.48–0.55 µs/px (the
  difference is workload variance, and if anything H-B is marginally higher from
  the per-call long-thunk indirection). Raster did not drop → the "keep only if
  raster drops" gate failed.
- Root cause matches the caveat: the **RP2040 XIP cache (16 KiB) already keeps
  the tight per-pixel loops resident**, and a cache hit runs at SRAM speed. The
  loops were never XIP-thrashed (unlike the VM in KOTO-0169, whose interpreter
  interleaved with the PSRAM CodeWindow), so SRAM placement bought nothing and
  added thunk overhead. Reverted (2 KiB reclaimed, margin back to ~67 KiB).

## Conclusion — KOTO-0174 closed (present path optimized as far as it goes)

Three levers tried, one kept:

- **H-P (KEPT):** hoist the per-pixel clip out of the raster leaves. **~3×
  device raster/px** (2.28 → ~0.48–0.6), quiet-frame fps 43 → 111. Zero RAM,
  byte-identical. The single real win.
- **H-A (reverted):** DMA/CPU pipeline — no overlap on this embassy SPI-DMA
  path; transfer is SPI-clock-bound (~0.55 µs/px @ 62.5 MHz) and un-hideable.
- **H-B (reverted):** SRAM-place the CPU loops — no win; XIP cache already
  covered them.

After H-P, KotoRun's present path is not the frame bottleneck: quiet frames are
VM-bound (~8 ms, 55–86 fps on small-dirty frames), and the one remaining *bad*
frame in every capture is the recurring `CommandCountShift` full repaint
(`dirty_px=102400`, ~100 ms, **fps 8**) — which is [KOTO-0175](KOTO-0175-kotorun-commandcountshift-full-repaints.md)
dirty-policy, not present cost. **The highest-value next work for KotoRun fps is
KOTO-0175, not this issue.** Kept in the tree: H-P (raster.rs), the LCD
convert/transfer split + `phase=176` canary + `phase=177` present-cost line.

## Acceptance criteria

- [x] Stage 0 host attribution table (≥90% accounted — 100% on the KotoRun
      shape); the primary lever is identified (per-pixel paint, glyph-first).
- [x] `phase=177` device cross-check (2026-07-08, KotoRun): clear ≈ 17%,
      `stack_us` ≈ 83%, clear ~0.26 µs/px — matches the host ratios. Gate
      passed on both host and device.
- [x] H-P landed with device A/B: ~3× raster/px, quiet-frame fps 43 → 111+
      (2026-07-08). Present bottleneck re-scoped to transfer (H-A/H-D above).
- [x] H-A pipeline attempted with device A/B and **reverted** (no measured
      transfer overlap on this embassy SPI-DMA path; +15.4 KiB broke the Gfx
      dev-profile boot) — a numbers-backed decision not to keep it. The LCD
      convert/transfer split + `phase=177` `convert_us`/`dma_us` stay.
- [x] H-B SRAM-placement attempted with device A/B and **reverted** (no raster
      drop — the XIP cache already covered the loops). Numbers-backed.
- [x] **Issue closed.** H-P is the kept win; H-A/H-B reverted with data. Present
      is no longer the bottleneck; KotoRun's remaining fps issue is KOTO-0175.

## Re-investigation (a)/(b) — instrumented 2026-07-11, device run pending

KOTO-0175 lever 1 shipped (device-confirmed: zero `CommandCountShift` over 540
frames), so KotoRun's frame cost is now purely the transfer-bound scroll
(fps 13–20 at 30–48k dirty px). That makes the two measurements this issue
closed *past* worth one boot-time capture each — both observe-only, both left
unanswered by the close:

- **(a) The DMA runs ~1.45× the wire floor and the gap was never attributed.**
  Measured `dma_us` ≈ 0.55 µs/px vs the 62.5 MHz RGB666 floor of 0.384; the
  ~0.17 µs/px gap is ~17 ms of every full-surface transfer and ~6 ms of a
  typical 35k-px scroll frame. Is it per-transfer fixed cost (`set_window` +
  `RAMWR` + CS/DC + ~5 executor awaits per strip — recoverable with taller
  strips or chained DMA) or a slow wire (SPI clock config / inter-byte gaps)?
- **(b) H-A's "no overlap" was never root-caused.** The revert note itself
  lists confirming embassy-rp's SPI-DMA yield behaviour as a prerequisite to
  any retry. If `Spi::write().await` does yield for a joined CPU future, H-A
  failed structurally (e.g. the small `begin` awaits serializing ahead of the
  data DMA) and the on-paper ~1.76× present is still live; if it does not,
  H-A is conclusively dead on this HAL.

**Instrument (landed):** `firmware/spi_bench.rs` — a one-shot boot bench under
`DiagClass::Gfx` (no-op in the shipping profile), running right before the
pixel-blit diagnostic on the idle present strips. `lcd.rs` regains the H-A
split primitives (`begin_rgb666`/`write_rgb666_data`/`end_rgb666`) as
diag-only methods; the production path still uses the fused `transfer_rgb666`.

- `phase=179 spi-rate`: mean `transfer_rgb666` time at rows = 1/2/4/8/16
  (960–15,360 B) plus the `begin` prologue alone. Fit `t ≈ a + b·bytes`:
  `b` at the nominal **0.128 µs/B** means the wire is fine and `a` (× ~20
  strips on a full repaint) is the recoverable gap; `b` high means the SPI
  clock itself needs investigation.
- `phase=178 spi-overlap`: with the window already open (the H-A shape), race
  one full-strip data DMA against CPU work calibrated to ~one DMA's duration
  under `embassy_futures::join`. `overlap_us = dma_us + cpu_us − join_us`:
  ≈ `min(dma_us, cpu_us)` → embassy yields, H-A retry viable; ≈ 0 → the
  executor is held for the write's duration, H-A stays dead.

**To run:** set `DIAG_PROFILE = DiagProfile::Gfx` in `firmware/config.rs`,
`cargo build -p koto-pico --bin koto_firmware --target thumbv6m-none-eabi
--release`, flash, and capture the boot UART — the `phase=179` (6 lines) and
`phase=178` (1 line) block appears between `phase=14 catalog-ready` and
`phase=20 first-redraw-start`. No app interaction needed.

### Device capture (2026-07-11) — both questions answered

```
phase=179 spi-rate begin reps=8 mean_us=66
phase=179 spi-rate rows=1  bytes=960   mean_us=220
phase=179 spi-rate rows=2  bytes=1920  mean_us=364
phase=179 spi-rate rows=4  bytes=3840  mean_us=660
phase=179 spi-rate rows=8  bytes=7680  mean_us=1255
phase=179 spi-rate rows=16 bytes=15360 mean_us=2421
phase=178 spi-overlap bytes=15360 reps=8 passes=5 dma_us=2354 cpu_us=1984 join_us=2354 overlap_us=1984
```

**(a) RESOLVED — the 1.45× gap is per-byte pacing, not per-strip overhead.**
The fit is cleanly linear: **b ≈ 0.152 µs/byte** (consecutive-delta estimates
0.150/0.154/0.155/0.152) against the 62.5 MHz nominal of 0.128 — a uniform
**~19% per-byte overhead** — plus a fixed **a ≈ 74 µs/strip** (consistent with
`begin` = 66 µs). Decomposition of the in-app ~0.55 µs/px: 0.384 wire nominal
+ 0.072 pacing (→ 0.456 marginal; the 16-row bench point runs 0.473 µs/px
incl. fixed) + small-rect fixed-cost amortization and in-context wake latency.
Consequences: taller strips recover almost nothing (fixed cost is ~3% at 16
rows — that lever is dead); the pacing overhead is real and byte-granular —
the plausible mechanism is the PL022 TX DREQ/8-bit DMA request granularity,
and the candidate fix is **H-W: 16-bit SPI frames + halfword DMA** (halves the
request rate; RGB666 byte order handled in the convert step; odd-length rects
need a fallback). Note 125 MHz has no integer divisor near 52.6 MHz, so this
is not a misconfigured SCLK.

**(b) RESOLVED — embassy-rp `Spi<Async>::write().await` yields completely
during the data DMA.** `join_us == dma_us` exactly and `overlap_us ==
min(cpu, dma)` exactly: the calibrated 1,984 µs CPU checksum ran entirely
under the in-flight 2,354 µs DMA, with zero measurable join overhead. **H-A's
"no overlap" was therefore a structural bug in the H-A build** (most likely
the join's poll order / the blocking raster future completing before the data
DMA future had been polled and started), not a HAL limitation.

### Verdict — H-A is live again (H-A2), H-W is the fallback

- **H-A2 (pipeline retry) is the primary lever.** Mechanism proven on device.
  Expected: a heavy 35k-px frame's present goes from serial raster+convert
  (~25 ms) + DMA (~16 ms) ≈ 41 ms to ~max(cpu, dma) ≈ 25 ms (**~1.65×**);
  KotoRun active-scroll fps ~15 → ~25–30. Design requirements learned from
  the failure: poll the data-DMA future before running the blocking
  raster+convert (the bench's exact shape: `begin` awaited first, then
  `join(write_rgb666_data, cpu_work)`), and A/B with `phase=160`
  per-px numbers before keeping. RAM: full ping-pong scratch is +15.4 KiB
  (the level that broke the Gfx-profile boot last time — needs either
  headroom work or an 8-row half-strip ping-pong at +7.7 KiB with ~66 µs
  extra fixed cost per extra band).
- **H-W (16-bit frames) only matters if H-A2 fails or stays unmerged** — once
  the DMA is hidden under the CPU term, shaving the DMA's 19% pacing no
  longer moves wall time (present becomes CPU-bound at raster+convert
  ≈ 0.72 µs/px; the next lever after H-A2 would be the convert itself, H-D).

### H-A2 implemented (2026-07-11) — device A/B pending

The app present is now one software-pipelined band stream
(`present_rects_pipelined` in [app_render.rs](../../../src/koto-pico/src/firmware/app_render.rs)),
used by both the incremental delta path (replacing the per-rect loop +
`present_rect_banded`) and the whole-surface full-repaint path. Design differs
from the reverted H-A in exactly the ways the re-investigation dictated:

- **Zero extra SRAM.** The existing 15,360 B RGB666 scratch ping-pongs as two
  7,680 B halves; bands cap at `PIPELINE_BAND_PX` = 2,560 px (8 full-width
  rows) instead of 16. No new buffer, so the +15.4 KiB that broke the
  Gfx-profile boot last time simply doesn't exist. Estimated cost of the finer
  banding: one extra ~74 µs `begin` per 2,560 px, ~3% of the DMA it hides.
- **The proven overlap shape.** Per band: previous band's data drains under
  `join(write_rgb666_data(front), raster_convert_band(next → back))` with the
  DMA future polled first (the exact `phase=178` bench structure); only then
  does `begin_rgb666` open the next window (the SPI bus is shared, so the
  prologue cannot overlap data). First band and final drain are the only
  un-overlapped pieces.
- **A/B toggle in the tree:** `config.rs PRESENT_PIPELINE` (default `true`);
  `false` drains the identical band stream serially — same pixels, same band
  geometry, only the wait structure changes — so the A/B isolates the overlap
  itself.
- **Metrics stay honest under overlap:** `raster_us`/`convert_us`/`clear_us`
  keep true CPU cost (their sum can exceed frame wall now);
  `transfer_us`/`dma_us` count only the *exposed* transfer (window prologue +
  the DMA remainder the CPU work failed to hide) — the exact term the A/B
  watches. `record_transfer`'s fused path remains for the shell and the
  non-base per-command path, both untouched.

Expected on device (from the measured per-px terms): heavy scroll frames
present ≈ raster+convert 0.72 µs/px with the 0.46 µs/px DMA hidden →
**~1.6× present on 30–48k-px frames, KotoRun active fps ~15 → ~25**; full
repaints ~100 ms → ~75 ms. Quiet frames unchanged (VM-bound).

**Device A/B (pending):** flash the default build, capture `phase=160` over a
KotoRun run, and compare `transfer_us`/`fps` on 30–48k-px frames against the
2026-07-11 pre-H-A2 capture (e.g. frame 30: `raster 17,482 / transfer 28,790 /
fps 18`). Expect transfer_us to collapse toward the window-prologue floor
(~74 µs × bands ≈ 1–2 ms) plus any un-hidden DMA. Check `phase=176 free_min`
(should be unchanged — no new buffers) and that the Gfx profile still boots.
Gate: keep if fps improves with no visual artifacts (bands are
geometry-identical to serial, so artifacts would indicate a scratch aliasing
bug, not tearing).

#### H-A2 device A/B (2026-07-11, KotoRun, shipping profile) — CONFIRMED, KEPT

Like-for-like heavy frames (dirty_px-matched against the same-day pre-H-A2
capture):

| dirty_px | pre transfer_us / fps | post transfer_us / fps |
| ---: | ---: | ---: |
| ~40k | 31,908 / 15 | **13,963 / 20** |
| ~38k | ~30,328 / 15 | **13,278 / 23** |
| ~46k | 38,044 / 13 | **13,766 / 18** |
| 102,400 (full) | 71,760 / 9 (109 ms) | **27,787 / 13 (76 ms)** |

- **The DMA is effectively gone from the wall.** Under the pipeline,
  `transfer_us` = convert (CPU, itself overlapped) + window prologues +
  *exposed* DMA. Decomposing frame 510 (41,360 px): convert ≈ 9.9 ms +
  prologue ≈ 2 ms → **exposed DMA ≈ 2 ms**, vs ~22.7 ms of serial DMA
  pre-H-A2. Wall check: vm 11.4 + raster 20.7 + transfer 14.0 ≈ 46 ms ≈
  lat 49 ms — accounts.
- **Heavy-frame fps +33–50%** (13–15 → 18–23); full repaint 109 → 76 ms.
- Small/quiet frames unregressed (44 px → fps 313; 2–8k px → fps 50–80).
- **Known costs, accepted:** (1) full-repaint `raster_us` rose 32 → 43 ms —
  40 bands now each walk the full command stack (vs 20); net still −33 ms of
  wall on the rare transition frames. (2) `phase=160 transfer_us` semantics
  changed: it now carries the (overlapped) convert CPU cost, so cross-firmware
  transfer comparisons must use `phase=177`'s `convert_us`/`dma_us` split.
- Residual verifications both passed (2026-07-11): the Gfx profile boots on
  device, and gameplay shows no visual artifacts.

**Present is now CPU-bound end to end: vm ≈ 7–11 ms, raster ≈ 0.5 µs/px,
convert ≈ 0.24 µs/px, DMA hidden.** The next levers, if KotoRun fps ever needs
another push, are H-D (cheapen/SRAM-place the convert — now the only
transfer-side CPU term), H-P2 (glyph inner loop; frame 690's raster 0.63 µs/px
was a text=10 frame), and the VM. None is urgent: active scroll runs 18–57 fps
with the hitches gone.

### H-D implemented (2026-07-11) — byte-algebra convert, device A/B pending

`convert_rgb565_to_rgb666` rewritten and moved to
[koto-gfx/src/convert.rs](../../../src/koto-gfx/src/convert.rs) (the `DiagProfile`
precedent: koto-pico cannot host-test, so the byte-exactness proof lives in the
host-testable crate; `lcd.rs` re-exports, all call sites unchanged). The u16
reassembly + wide shifts are replaced by single-byte algebra — `R = hi & 0xf8`,
`G = ((hi & 7) << 5) | ((lo >> 5) << 2)`, `B = lo << 3` — plus a two-pixel
unroll. A LUT was considered and rejected: on the M0+ an SRAM table load costs
no less than the AND/shift it would replace. Exhaustive test proves byte
identity with the old loop across all 65,536 RGB565 values (plus odd-count
tails and undersized-buffer rejection).

**Gate (H-B lesson — no "obvious" win is real until measured):** device A/B on
`convert_us`. Direct metric: a `DIAG_PROFILE = Gfx` build's `phase=177
app-present-cost` line, `convert_us` per dirty_px — baseline ~0.24 µs/px.
Proxy in the shipping profile: heavy-frame `phase=160 transfer_us` (≈ convert
+ ~4 ms of prologue/exposed DMA at 40k px — e.g. frame 510's 13,963 µs
baseline). Since the convert sits on the pipelined CPU path, a 1.5× convert is
worth ~3 ms on a 40k-px frame (fps 20 → ~21–22). Keep if `convert_us` drops;
revert the loop (keep the koto-gfx move + tests) if flat.

#### H-D device A/B (2026-07-11) — KEPT; plus a code-layout fragility incident

- **H-D verdict: kept.** Heavy-frame transfer/px improved: a 50,660-px frame
  ran `transfer_us` 14,600 = **0.288 µs/px** vs the pre-H-D 0.297–0.345
  (fps 19 @ 50.7k vs baseline 18 @ 46.3k). The gain is modest and
  frame-shape-dependent (mid-size frames sit in prologue/exposed noise) —
  the expected ~1 fps class win on the heaviest frames.
- **Incident (the real lesson):** the first H-D builds — Gfx, then Perf, both
  *without* `#[inline]` on the moved cross-crate fn — showed a **uniform ~2×
  CPU slowdown** (raster 0.5 → 0.95–1.17 µs/px, vm/transfer up too) **and
  loud BGM crackle**. The identical tree with `#[inline]` added is healthy
  under Audio *and* Perf. Diagnosis: the workspace builds without LTO at
  codegen-units=16, so function layout reshuffles per build/profile; an
  unlucky layout defeats the 16 KiB XIP cache residency the raster hot loops
  depend on (H-B's finding, inverted), and since **the XIP cache is shared by
  both cores**, core0's thrash also starves core1's audio-worker instruction
  fetches — which is the crackle. Mitigation in-tree: `#[inline]` on the
  moved convert. Durable fix to evaluate separately: `[profile.release]
  lto + codegen-units = 1` (deterministic, denser layout) — filed as
  [KOTO-0176](KOTO-0176-release-profile-lto.md); every perf number in this
  file re-baselines when it lands.
- Out-of-scope observation, recorded: KotoRun still crackles briefly on
  SMASH combos (SFX + 12-particle burst + a heavy frame together);
  KotoBlocks audio is clean. KotoRun-specific peak load, not a present-path
  regression.
