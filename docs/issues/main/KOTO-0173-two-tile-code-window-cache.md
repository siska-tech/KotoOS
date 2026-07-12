# KOTO-0173: re-land the two-tile CodeWindow cache (KOTO-0134 retry)

- Status: DONE 2026-07-07 — hardware-confirmed. KotoRun's fixed per-frame
  refill tax is **gone** (`refills=0 cw_refill_us=0` for the whole session;
  steady `vm_us` ≈ 8.1 ms, fps 43); all apps launch (incl. the KOTO-0131
  hang case KotoBlocks); `free_min=67,000 B` — the exact predicted margin.
  kotorogue still walks a 3-tile working set (see the record below).
- Type: firmware performance enablement (VM code-fetch)
- Priority: P2
- Requirements: NFR-PERF-1

Source of truth:
[psram.rs](../../../src/koto-core/src/psram.rs) (`PsramCodeWindow::new_two_tile`,
the two-slot MRU/LRU lookup/refill, and the host tests),
[config.rs](../../../src/koto-pico/src/firmware/config.rs)
(`CODE_WINDOW_TILES` / `CODE_WINDOW_TOTAL_BYTES` and the sizing rationale),
[app_runtime.rs](../../../src/koto-pico/src/firmware/app_runtime.rs) (the launch
path constructing the two-tile shape).

Relates to: KOTO-0134 (the first attempt's launch hang + the refill
diagnostics this issue's hardware run reads),
[KOTO-0172](KOTO-0172-main-task-stack-frame-reduction.md) (funded the budget
and explained the historical hang),
[KOTO-0170](KOTO-0170-ram-interpreter-default-on.md) (the phase=176 canary
guarding the spend), KOTO-0156 (per-app code-layout opts — the other half of
the refill story; both remain useful).

## Why retry now

The KOTO-0131/0134 2-tile attempt was reverted after a KotoBlocks launch
hang, with the cause unknown. Two things changed:

1. **The hang has a retrospective explanation.** The cache doubled the window
   static (+8 KiB `.bss`) in the era when the invisible main-stack ceiling
   sat within ~2.5 KiB of `.bss` — the same failure KOTO-0131 hit growing the
   window alone. KOTO-0170/0172 measured the real peak (26,616 B) and widened
   the margin to ~81 KiB with a permanent `phase=176` tripwire, so the
   +16 KiB this needs is a measured, guarded spend (margin stays ≈ 67 KiB).
2. **The workload changed.** KOTO-0134's own data showed KotoBlocks' per-frame
   working set was the whole code segment (8 × 8 KiB tiles), which no small
   cache fixes — but since then the tile grew to 16 KiB, KOTO-0156's layout
   opts front-load steady loops into one tile, and `psram_fast_code_window`
   (default since KOTO-0171) cut the per-refill cost ~7×. What remains on
   KOTO-0169's ledger is the *fixed per-frame refill tax* of apps whose
   steady loop straddles one boundary (KotoRun ~3.6 ms/frame) and large
   tiling apps (kotorogue ~73 KiB) — exactly the two-region ping-pong shape
   a second resident tile eliminates.

## Design

`PsramCodeWindow` now has two constructors sharing one implementation:

- `new` — one tile spanning the whole buffer (historical shape; host/sim/
  tests and the dma-diag header compare keep it).
- `new_two_tile` — the buffer split into two resident tiles, MRU/LRU
  replacement. The fetch hot path checks the MRU slot first, so straight-line
  execution pays the same compares as before; a hit in the LRU slot flips
  MRU; a miss refills the LRU slot only (a failed transfer invalidates just
  the victim — the other tile stays servable). Slot 1 is permanently empty in
  the single-tile shape, so there is no shape branch on the hot path.

Firmware: `CODE_WINDOW_BYTES` stays the **tile** size (16 KiB — tile indices
in `phase=160 refills=/code_tiles=` stay comparable across firmwares); the
static buffer is `CODE_WINDOW_TOTAL_BYTES` = 2 tiles = 32 KiB, and the launch
path wraps it with `new_two_tile`. Side effect: the no-PSRAM fallback (plain
slice over the buffer) now covers apps up to 32 KiB.

Diagnostics semantics are unchanged (`refills`, `code_tiles`, tile histogram,
transitions, `cw_refill_us`) — a two-tile hit simply doesn't refill.
`current_window_bytes`/`debug_state` report the MRU tile. The sim's
`TileRecorder` keeps the single-tile model as a conservative upper bound
(device refills ≤ model; the KotoBlocks 0-refill lock-in is exact either way).

## Budget (deliberate, per the sizing rule)

| Metric | Before | After |
| --- | --- | --- |
| `CODE_WINDOW` static | 16,384 B | 32,768 B |
| Static free RAM above `.bss` | 110,000 B | 93,616 B |
| Margin over measured 26,616 B stack peak | ~81 KiB | **~67 KiB** |

Two slots, not more: the observed pathology is a *two*-region ping-pong
(`main` high tile ↔ helpers tile 0); more slots pay lookup cost on the fetch
hot path for a pattern no current app exhibits. The `phase=176` canary guards
the spend.

## Hardware validation (2026-07-07, all passed)

- [x] Launch soak: every app launches and plays, including KotoBlocks (the
      KOTO-0131 hang case). `phase=176` peak unchanged at 26,616 B and
      `free_min=67,000 B` across the whole session — **exactly** the
      predicted margin (93,616 − 26,616), i.e. the cache added zero stack.
- [x] KotoRun: `refills=0 code_tiles=0 cw_refill_us=0` for the *entire*
      session — the two-region straddle is fully resident. Steady frame:

      ```
      phase=160 ... frame=690 vm_us=8102 raster_us=9557 transfer_us=3408 ... refills=0 code_tiles=0 fps=43 lat_ms=22
      phase=175 ... vm_us=8102 ops=5715 host_us=406 cw_refill_us=0 refills=0
      ```

      The KOTO-0169 ledger's fixed ~3.6 ms/frame refill tax is gone:
      `vm_us` 13.5 → **8.1 ms** (8,102 µs / 5,715 ops ≈ 1.42 µs/op — pure
      interpretation at the KOTO-0169 Stage-3 ns/op, no refill term), and
      quiet-frame fps moved 24 → **43**.
- [x] kotorogue (~73 KiB): stable `refills=3 code_tiles=3` per sampled frame,
      `cw_refill_us=5407` (~1.8 ms per 16 KiB fast-path refill), `vm_us`
      12,075, fps 74, lat 13 ms. Its steady working set spans **3 tiles**, so
      two slots still reload once per tile per frame — a known limit, not a
      regression (each refill is fast-path). If it ever matters: KOTO-0156
      per-app layout opts to compact the loop, or a 3rd slot (another
      deliberate +16 KiB, weigh against the ~67 KiB margin).
- [x] KotoSnake/KotoShogi/KotoMemo: no regression (single-tile-resident apps
      behave identically).
