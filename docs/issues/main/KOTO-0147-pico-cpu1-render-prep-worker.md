# KOTO-0147: Pico CPU1 render-prep worker after audio service

- Status: todo
- Type: feature / experiment
- Priority: P2
- Related: KOTO-0146, KOTO-0143, KOTO-0131, KOTO-0120

## Background

KOTO-0146 reserves CPU1 for stable PCM service because audio pacing is the only
currently observed hard real-time gap. A review of the Pico frame loop found one
additional CPU1 candidate that does not need to own LCD SPI, SD, or PSRAM:
render preparation in SRAM.

The current app loop on CPU0 does all work serially:

1. read keyboard events
2. service audio opportunistically
3. run one VM frame
4. diff retained draw state
5. raster dirty rectangles into an RGB565 strip
6. convert RGB565 to RGB666 inside `PicoCalcLcd::write_rgb565_rect`
7. transfer the window over LCD SPI/DMA

KOTO-0131 and KOTO-0143 reduced normal KotoBlocks rendering to small dirty
regions, so this is not the first CPU1 target. However, render prep remains a
bounded, deterministic workload: it touches only SRAM snapshots, app heap bytes,
font data, and strip buffers. That makes it a better CPU1 candidate than PSRAM
CodeWindow prefetch or SD/FAT reads, which would contend with PIO/DMA/SPI state
and have synchronous VM-visible semantics.

## Goal

Prototype a low-priority CPU1 render-prep worker that runs only around the CPU1
audio service loop and prepares one dirty window at a time for CPU0 to transfer.

First milestone goal:

- preserve KOTO-0146 audio timing as the highest priority on CPU1
- keep LCD SPI/DMA ownership on CPU0
- move only SRAM-local prep work to CPU1
- measure whether CPU0 frame time or audio drops improve under KotoBlocks stress
- fall back to the existing CPU0-only render path on overload or mismatch

## Candidate Work

Start with the smallest useful offload:

1. CPU0 computes the dirty plan and enqueues one render-prep job containing a
   stable snapshot reference or copied metadata for one rect/band.
2. CPU1 composites that rect into a worker-owned RGB565 strip, in small chunks
   between audio sample-service deadlines.
3. Optionally, CPU1 converts the prepared RGB565 strip to RGB666 so CPU0 can
   call a raw transfer path without doing conversion.
4. CPU1 marks the job ready; CPU0 transfers it through the existing LCD path.
5. If the worker is busy, late, or reports mismatch, CPU0 renders the rect itself.

## Non-goals

- moving LCD SPI/DMA transactions to CPU1
- sharing live `PicoCalcLcd` ownership across cores
- moving VM execution to CPU1
- moving PSRAM `PsramCodeWindow` refill or PSRAM DMA reads to CPU1
- moving SD/FAT reads or `asset_load` to CPU1 in this milestone
- weakening KOTO-0146 audio timing or fixed-rate PCM service

## Investigation Notes

Reviewed candidates:

- **Audio service:** already tracked by KOTO-0146 and remains the primary CPU1
  workload.
- **Render prep:** viable as a secondary, cooperative workload because the heavy
  parts are SRAM-local (`present_app_delta`, `present_rect_banded`,
  `paint_app_commands`, RGB565->RGB666 conversion). CPU0 can retain LCD SPI/DMA
  ownership and fall back safely.
- **LCD transfer:** not a good first CPU1 target. `PicoCalcLcd` owns SPI1, CS/DC,
  reset pins, and async DMA writes; cross-core ownership would add more risk than
  it removes.
- **PSRAM CodeWindow prefetch:** not a good first CPU1 target. The VM currently
  refills synchronously on instruction miss, and KOTO-0132 DMA/PIO diagnostics are
  sensitive to state-machine/channel ownership. A CPU1 prefetcher would need a
  separate prediction/cache design before it can help.
- **SD/FAT and asset/SKK loads:** lower priority. They are synchronous launch or
  hostcall latency paths rather than the steady per-frame bottleneck, and moving
  `VolumeManager`/directory state across cores would be invasive.

## Plan

1. Land KOTO-0146 first so CPU1 startup, stack ownership, panic behavior, and the
   audio queue contract are known.
2. Add render-prep timing fields to `phase=160` or a concise adjacent diagnostic:
   queued, completed, fallback, worker_us, cpu0_wait_us.
3. Define a bounded render job protocol for one rect/band and one worker-owned
   strip buffer.
4. Start with RGB565->RGB666 conversion offload only, then try full rect
   compositing if timing headroom remains.
5. Chunk CPU1 work so the audio service deadline is checked before and after each
   chunk.
6. Keep a compile-time feature gate until hardware logs prove no audio regression.

## Acceptance criteria

- [ ] CPU1 audio worker from KOTO-0146 remains stable with render-prep enabled.
- [ ] Render-prep jobs never run longer than the audio service budget without
      checking audio service.
- [ ] CPU0 keeps exclusive ownership of `PicoCalcLcd` and all LCD SPI/DMA calls.
- [ ] KotoBlocks representative gameplay shows no increase in `drops` or
      `underruns` versus KOTO-0146 alone.
- [ ] `phase=160`/adjacent diagnostics show completed jobs, fallbacks, worker time,
      and CPU0 wait time.
- [ ] Pixel output matches the CPU0-only path for dirty rects, line clears, sprite
      moves, text changes, and the one-shot `StaticRebuild` full repaint.
- [ ] If CPU1 is busy or disabled, the existing CPU0-only render path remains the
      default fallback.
- [ ] Feature can be disabled at compile time and runtime without changing output.
- [ ] If CPU0 would have to wait for CPU1 longer than the CPU0 local render estimate, CPU0 falls back immediately.
- [ ] RGB565/RGB666 output is byte-identical or pixel-equivalent to the CPU0 path in simulator tests.
- [ ] The experiment is allowed to conclude "no measurable win" and remain disabled by default.

## Notes

- Treat this as a post-audio experiment. The second core should not become a
  general background executor until audio has a proven fixed-rate schedule.
- Avoid large static buffer growth. Any worker strip or RGB666 buffer must be
  budgeted against the stack/SRAM headroom constraints recorded in `config.rs`.
- Prefer one-rect jobs first; multi-rect batching can wait until the worker timing
  is visible in hardware logs.