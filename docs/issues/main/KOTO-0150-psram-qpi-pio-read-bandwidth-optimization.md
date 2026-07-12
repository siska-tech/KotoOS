# KOTO-0150: PSRAM QPI/PIO read bandwidth optimization

- Status: todo
- Type: performance / hardware optimization
- Priority: P1
- Related: KOTO-0132, KOTO-0149, KOTO-0146, KOTO-0147

## Background

KotoOS now boots again after KOTO-0149 reduced KWT/audio SRAM pressure.
However, real hardware gameplay remains slow.

Representative KotoBlocks hardware logs show:

```text
phase=160 app-frame app=dev.koto.games.koto-blocks frame=180 vm_us=47520 raster_us=4554 transfer_us=1708 dirty_px=2064 rects=1 fps=17 lat_ms=56
phase=163 cw frame=180 refills=2 code_tiles=2 cw_refill_us=19193 cw_refill_max_us=11651 cw_bytes=27004 cw_effective_mb_s=1.407
````

Normal KotoBlocks frames are no longer dominated by LCD rendering:

* raster: about 4 ms
* LCD transfer: about 1.3–1.7 ms
* VM total: about 47–56 ms
* CodeWindow refill: about 19 ms per sampled frame
* effective PSRAM read bandwidth: about 1.4 MB/s

This indicates that CodeWindow refill from PSRAM is now one of the largest steady-state bottlenecks.

Earlier KOTO-0132 DMA experiments showed that simply moving PSRAM-to-SRAM copies to DMA is not sufficient. The current PSRAM path is still limited by the underlying read protocol and is not using QPI/quad read. DMA only changes who moves the data; it does not improve the PSRAM bus width, command/dummy cycle overhead, or PIO read throughput.

## Problem

The current PSRAM read bandwidth is too low for CodeWindow-backed VM execution.

At about 1.4 MB/s, each 16 KiB CodeWindow refill can take around 11–12 ms, and KotoBlocks can hit multiple refills per frame. This pushes normal gameplay into the 15–17 FPS range even when retained rendering keeps dirty pixels small.

The previous DMA-focused approach does not solve this because:

* PSRAM → SRAM DMA does not widen or accelerate the PSRAM read protocol.
* The path is not QPI/quad read.
* LCD transfer is a separate SRAM → LCD path and is not accelerated by PSRAM DMA.
* Normal KotoBlocks frames are more CodeWindow/VM bound than LCD-transfer bound.

## Goal

Improve PicoCalc PSRAM read bandwidth for CodeWindow refill and reduce VM frame time under real hardware gameplay.

First milestone target:

* Establish a reliable PSRAM read benchmark matrix.
* Identify the current read protocol, clock, chunking, and overhead limits.
* Implement the smallest safe read-path optimization that improves effective MB/s.
* Reduce `cw_refill_us` and KotoBlocks `vm_us` without destabilizing boot, audio, or rendering.

Stretch target:

* Introduce a QPI/quad-read path if the hardware wiring and PSRAM command set support it.
* Reach at least 4 MB/s effective CodeWindow refill bandwidth.
* Keep the implementation feature-gated until hardware logs prove stability.

## Non-goals

* Reopening KOTO-0132 as a DMA-only task
* Moving VM execution to CPU1
* Moving PSRAM CodeWindow semantics to asynchronous prefetch
* Changing bytecode format
* Reducing `CODE_WINDOW_BYTES` below 16 KiB
* LCD SPI/DMA optimization
* Audio worker changes
* SD/FAT or asset loading optimization

## Current Observations

KotoBlocks representative frame:

```text
vm_us=47,520
raster_us=4,554
transfer_us=1,708
dirty_px=2,064
rects=1
refills=2
cw_refill_us=19,193
cw_effective_mb_s=1.407
fps=17
```

KotoShogi first frame full repaint is still dominated by raster/LCD transfer:

```text
raster_us=91,657
transfer_us=75,801
dirty_px=102,400
full=1 full_reason=StaticRebuild
```

This issue focuses on KotoBlocks-style steady gameplay, where CodeWindow refill dominates more than LCD transfer.

## Investigation Plan

### 1. Add a PSRAM read benchmark matrix

Measure effective throughput for:

* 256 B
* 1 KiB
* 4 KiB
* 16 KiB
* 64 KiB

For each mode available:

* current CPU/PIO read path
* existing DMA-backed path
* larger burst/chunk path
* any available fast-read path
* experimental quad/QPI path if implemented

Log:

```text
phase=334 psram-bench mode=<mode> bytes=<n> elapsed_us=<us> mb_s=<rate> chunk=<chunk> clkdiv=<div> sm_hz=<hz> dummy=<cycles> ok=<0|1>
```

### 2. Make the current read mode explicit

Current logs should distinguish:

* CPU bit/PIO read
* PIO FIFO read
* PIO + DMA read
* fast read
* quad read
* QPI read
* fallback

`dma_successes` is not enough. The important field is the actual bus/protocol mode.

Suggested CodeWindow log extension:

```text
phase=163 cw frame=<n> refills=<n> cw_refill_us=<us> cw_bytes=<bytes> cw_effective_mb_s=<rate> read_mode=<mode> chunk=<bytes> sm_hz=<hz>
```

### 3. Reduce per-refill overhead

Investigate whether CodeWindow refill currently performs many small reads.

Compare:

* many small reads
* 1 KiB chunks
* 4 KiB chunks
* one full 16 KiB burst

The goal is to reduce command/setup overhead and maximize continuous read time.

### 4. Tune PIO read frequency and FIFO/DMA behavior

Investigate:

* PIO clock divider
* state machine instruction rate
* FIFO occupancy
* DMA DREQ pacing
* RX FIFO join
* setup/teardown per transaction
* command/dummy cycle overhead

Measure whether the bottleneck is:

* PSRAM protocol speed
* PIO program speed
* DMA setup overhead
* CPU polling overhead
* chunking overhead
* chip select / command sequencing overhead

### 5. Investigate QPI/quad read feasibility

Confirm:

* PicoCalc PSRAM pin mapping
* PSRAM part command set
* whether quad mode enable is required
* whether QPI mode is sticky or must be re-entered
* required dummy cycles
* safe fallback to current read mode
* interaction with existing PIO state machine ownership

If feasible, implement behind a feature gate such as:

```toml
psram_qpi_read = []
```

### 6. Validate against CodeWindow workload

Use real app logs, not only synthetic benchmarks.

Target apps:

* KotoBlocks normal gameplay
* KotoSnake normal gameplay
* KotoShogi first frame / static rebuild as a non-primary reference

Track:

* `vm_us`
* `cw_refill_us`
* `cw_refill_max_us`
* `cw_effective_mb_s`
* `refills`
* `code_tiles`
* `fps`
* `lat_ms`
* audio `drops`, `underruns`, `worker_late`, `worker_max_jitter_us`

## Acceptance Criteria

* [ ] A PSRAM read benchmark matrix is added or documented.
* [ ] Current read mode is logged explicitly.
* [ ] Baseline effective bandwidth is confirmed around the current observed ~1.4 MB/s.
* [ ] At least one optimized read mode shows a measurable bandwidth improvement.
* [ ] CodeWindow refill logs show lower `cw_refill_us` under KotoBlocks.
* [ ] KotoBlocks normal gameplay improves from the current 15–17 FPS range, or the remaining bottleneck is clearly identified.
* [ ] No regression in KOTO-0149 boot stability.
* [ ] No regression in KOTO-0146 CPU1 audio stability:

  * `drops` remains bounded
  * `command_drops` remains bounded
  * `worker_max_jitter_us` remains bounded
* [ ] Optimized path is feature-gated until hardware logs prove stability.
* [ ] Current fallback read path remains available.

## Notes

* DMA alone is not expected to solve the bandwidth issue.
* DMA can reduce CPU involvement, but it cannot improve PSRAM bus width or command/dummy-cycle overhead.
* LCD transfer optimization should be handled separately; normal KotoBlocks frames are currently more CodeWindow/VM-bound than LCD-bound.
* Do not reduce `CODE_WINDOW_BYTES` first. The current 16 KiB window is likely helping reduce miss frequency.
* If QPI/quad read is not feasible on the actual PicoCalc wiring/PSRAM part, document that explicitly and optimize the best available serial/PIO burst path.

## Initial hardware benchmark result

`psram_diag` baseline on hardware shows that increasing the logical chunk from
16B to 256B does not improve throughput.

Observed 64 KiB read bandwidth:

- `prod16_serial_pio_cpu`: 53,035 us, 1.235 MB/s
- `pio_word256_serial_pio_cpu`: 55,152 us, 1.188 MB/s

Observed 16 KiB read bandwidth:

- `prod16_serial_pio_cpu`: 13,265 us, 1.235 MB/s
- `pio_word256_serial_pio_cpu`: 13,789 us, 1.188 MB/s

This weakens the hypothesis that the dominant cost is 16B transaction overhead.

## Hardware rerun after clkdiv register fix (2026-06-25)

The benchmark freeze (about 2.8 s at 256 B) is resolved.
`phase=334` now reports realistic elapsed times again, confirming the root cause
was diagnostic `SMx_CLKDIV` raw register bit placement.

Key observations from the rerun:

- `clkdiv=4`: stable, `ok=1`, about 1.19 to 1.26 MB/s at 64 KiB.
- `clkdiv=3`: stable, `ok=1`, about 1.48 to 1.60 MB/s at 64 KiB.
- `clkdiv=2`: unstable, `ok=0` across sizes/modes despite short elapsed times.
- `sm1_cpu_tx_rx_dma_serial`: still `ok=0` in matrix path and does not provide a
  stable continuous-read baseline.

Interpretation:

- Current serial single-lane production path can be improved from about 1.25 MB/s
  to about 1.60 MB/s by clock tuning (`clkdiv=3`) while preserving correctness.
- Attempting about 2.46+ MB/s equivalent operation (`clkdiv=2`) is not
  correctness-stable on current protocol/program settings.
- Therefore, "serial-only optimization to reliably exceed 2 MB/s" is currently
  not supported by hardware evidence.

Decision impact:

- Keep `clkdiv=3` as the best known stable serial point for further app-level
  evaluation.
- Prioritize physical QPI/quad feasibility investigation as the next major
  throughput path, while retaining serial fallback.
The current bottleneck appears to be the serial PIO data phase, PIO cycles per
bit, CPU FIFO drain/unpack, or SCK frequency.

At `sm_hz=33.25MHz`, the measured 1.235 MB/s corresponds to about 9.9 Mbit/s,
or roughly 3.3 PIO cycles per serial data bit. This is consistent with a PIO
program that needs multiple instructions per bit, so DMA/chunking alone is not
expected to provide a large bandwidth improvement.