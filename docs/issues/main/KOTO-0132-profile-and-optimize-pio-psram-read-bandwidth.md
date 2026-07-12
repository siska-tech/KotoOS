# KOTO-0132: Profile and optimize PIO PSRAM read bandwidth

- Status: done
- Type: research
- Priority: P1
- Requirements: NFR-RT-2

## Summary

KotoOS now executes larger app bytecode from PSRAM via `PsramCodeWindow`.
The code window is a single-tile SRAM cache. On a tile miss,
`PsramCodeWindow::refill()` synchronously reads one tile from PSRAM into SRAM.

This issue investigated whether KotoBlocks gameplay `vm_us` was dominated by
PSRAM code-window refill cost, and evaluated a safe path toward faster PSRAM
reads without changing the production PSRAM path.

The main conclusion is:

- The current production 16-byte PSRAM read path is correct and stable.
- Its effective read bandwidth is about 1.4 MB/s.
- KotoBlocks gameplay still spends about 19 ms/frame in code-window refills.
- PSRAM refill cost is therefore a major contributor to gameplay `vm_us`.
- Prior word-oriented DMA assumptions are retired.
- Future optimization should follow a byte-oriented PIO FIFO / DMA direction.
- A feature-gated CodeWindow DMA read experiment passed hardware smoke testing.

Production behavior remains unchanged by default.

## Background

Recent retained-rendering work significantly reduced normal KotoBlocks rendering
overhead. During steady gameplay, rendering counters are often near zero:

```text
rect=0 text=0 pixels=0
full=0 full_reason=none
ovf=0
````

After increasing `CODE_WINDOW_BYTES` from 8 KiB to 16 KiB, the KotoBlocks title
screen improved dramatically because the title hot path fits inside the code
window and can run with:

```text
refills=0
cw_refill_us=0
```

However, normal gameplay still often shows high `vm_us` with remaining code
window refills. This suggested that the next major bottleneck was either:

1. PSRAM code-window refill cost, or
2. VM execution cost after rendering overhead had been reduced.

This issue focused on measuring and isolating PSRAM refill cost.

## Current PSRAM backend

The current PicoCalc PSRAM backend is PIO-backed, but it is not a high-speed
DMA/QPI implementation.

Current production characteristics:

* 1-bit serial PIO protocol
* `clock_divider = 4`
* Around 16.6 MHz serial bit clock at 133 MHz sysclk
* CPU busy-wait push/pull on PIO TX/RX FIFOs
* Read chunks are 16 bytes
* No DMA in the production path
* No QPI / 4-bit read mode

The current production `read()` path loops over `dst.chunks_mut(16)` and issues
one PSRAM fast-read transaction for each 16-byte chunk.

For a 16 KiB code-window refill, this means:

```text
16 KiB / 16 B = 1024 PSRAM read transactions
```

## Goals

* Measure PSRAM code-window refill cost.
* Determine whether KotoBlocks `vm_us` is dominated by PSRAM refill time or VM
  execution time.
* Add PSRAM correctness diagnostics independent of app launch.
* Identify a safe path toward higher PSRAM read bandwidth.
* Avoid papering over the issue only by increasing `CODE_WINDOW_BYTES`.

## Non-goals

This issue intentionally did not include:

* Changing the default production `PsramHal` path.
* Changing `PsramBlocks`.
* Changing `PsramCodeWindow`.
* Increasing the production read chunk size beyond 16 bytes.
* Implementing `dma24`.
* Implementing `dma32`.
* Implementing QPI / 4-bit PSRAM reads.
* Reworking retained rendering.
* Hand-optimizing KotoBlocks source code as the first response.
* Continuing word-sized DMA debugging as the active implementation track.
* Increasing `CODE_WINDOW_BYTES` again as the main fix.

## Verified findings

The following findings are established as the baseline for this issue.

### Production path correctness

The production 16-byte read path is correct and remains the only default
production read path.

`psram_diag` pattern verification passed on hardware for known-good production
reads.

### Production path bandwidth

Effective PSRAM read bandwidth is approximately:

```text
1.4 MB/s
```

Representative CodeWindow measurements during KotoBlocks gameplay:

```text
cw_bytes=27004
cw_refill_us≈19040–19070
cw_effective_mb_s≈1.416–1.418
```

This means gameplay can spend about 19 ms/frame in code-window refills.

### KotoBlocks behavior

With `CODE_WINDOW_BYTES=16KiB`:

* The title path can run with `refills=0`.
* Gameplay often still shows `refills=2` and `code_tiles=2`.
* Refill cost is a large share of `vm_us`.

Representative gameplay observation:

```text
refills=2
code_tiles=2
cw_bytes=27004
cw_refill_us≈19060
```

### Diagnostic read variants

The following diagnostic paths passed and matched `prod16`:

* `pio_word16`
* `pio_word256`

These are retained as diagnostics, but they are not the preferred direction for
future optimization.

## Direction correction

Earlier investigation included word-oriented DMA experiments. Those experiments
are now considered historical context only.

RP2040 PSRAM prior art indicates that the relevant DMA/FIFO path should be
byte-oriented:

* PIO FIFO access should be treated as byte-oriented.
* DMA transfer width should be 8-bit.
* RX DMA transfer count should be the requested byte count.
* DREQ should use the PIO RX channel for the active state machine.

For the current configuration, that means using the active PIO/SM RX DREQ, such
as:

```text
PIO1_RX1
```

for a dedicated SM1 diagnostic path.

## Retired assumptions

The following assumptions are retired and should not guide future
implementation:

* 16-byte reads should be modeled as word-sized DMA transfers.
* DMA destination should be word buffers followed by byte repacking.
* Word-DMA experiments are the primary path to production optimization.
* The long phase 3b/3d word-DMA debug trail should remain active.

The word-DMA trail is retained only as historical context.

## Diagnostic flow

The default `psram_diag` flow is:

```text
1. prod16
2. pio_word16
3. pio_word256
4. prior_art_fifo8_dedicated_sm
```

The following legacy diagnostics are retained only behind an explicit debug flag:

* `dma16_v2`
* `dma16_dual`

They are not executed by default.

## Dedicated-SM prior-art diagnostic

The prior-art diagnostic path is diagnostic-only and does not modify the
production launch path.

It uses a dedicated PIO state machine instead of reusing the production SM0.

### State machine

Use:

```text
PIO1 SM1
```

Do not copy already-loaded PIO instructions to a new offset.

The existing loaded program offset and wrap configuration should be reused where
possible. PIO jump targets may be tied to loaded instruction addresses.

### DMA configuration

Configure byte DMA against the dedicated SM1 FIFOs:

```text
TX DMA: command bytes -> TXF1
RX DMA: RXF1 -> [u8; 16]
```

DMA settings:

```text
TX data size: DMA_SIZE_8
RX data size: DMA_SIZE_8
TX DREQ/TREQ: PIO1_TX1
RX DREQ/TREQ: PIO1_RX1
TX count: command byte count
RX count: requested byte count
```

For the single 16-byte read diagnostic:

```text
RX count: 16
```

### SM0 isolation

During the dedicated-SM1 diagnostic run:

1. Save the current SM0 enable state.
2. Temporarily disable SM0.
3. Run the SM1 diagnostic transaction.
4. Restore the previous SM0 enable state.

This prevents SM0 from driving PSRAM pins concurrently with SM1.

## Validation matrix

For each test pattern:

1. Write the pattern.
2. Read with `prod16`.
3. Read with `pio_word16`.
4. Read with `pio_word256`.
5. Read with `prior_art_fifo8_dedicated_sm`.
6. Compare all relevant outputs.

Required comparisons:

```text
prod16     vs expected
pio_word16 vs prod16
pio_word256 vs prod16
prior_art  vs expected
prior_art  vs prod16
prior_art  vs pio_word16
```

## Expected logs

### Success log

Expected success log shape:

```text
dma16_prior_art pio_variant=prior_art_fifo8_dedicated_sm pio=1 sm=1 prog_off=... tx_ch=... rx_ch=... data_size=8 tx_count=7 rx_count=16 read_us=... verify=pass prior_vs_prod=pass prior_vs_pio=pass
```

### Failure log

Expected failure log fields:

```text
dma16_prior_art error=PriorArtFailure pio_variant=prior_art_fifo8_dedicated_sm pio=1 sm=1 prog_off=... read_us=... timeout_stage=...
tx_remaining=... rx_remaining=... tx_dreq=... rx_dreq=...
fifo_levels=... fifo_stat=...
```

## Guardrails

The following production behavior must remain unchanged unless a later issue
explicitly promotes an experimental path:

* `PsramHal`
* `PsramBlocks`
* `PsramCodeWindow`
* `CODE_WINDOW_BYTES`
* App launch behavior
* Production 16-byte read behavior

The following work is explicitly out of scope for this issue:

* `dma24`
* `dma32`
* QPI implementation
* Retained rendering rework
* KotoBlocks source-level hand optimization

## Acceptance criteria

This issue is accepted when:

* Default `psram_diag` executes:

  * `prod16`
  * `pio_word16`
  * `pio_word256`
  * `prior_art_fifo8_dedicated_sm`
* `dma16_v2` and `dma16_dual` are not executed by default.
* The dedicated prior-art path uses:

  * SM1
  * `TXF1`
  * `RXF1`
  * `PIO1_TX1`
  * `PIO1_RX1`
  * 8-bit DMA transfers
* SM0 does not drive PSRAM pins during the dedicated-SM1 diagnostic run.
* SM0's prior enable state is restored afterward.
* Production launch behavior remains unchanged.
* KotoBlocks launch/gameplay behavior remains unchanged outside diagnostic
  builds.

## Final smoke test

Commit:

```text
f56661c
```

passed hardware smoke testing.

Result:

```text
KotoBlocks staged from PSRAM successfully.
App launch result: started
Gameplay result: running
Final result: exited code=0
KotoBlocks gameplay result: pass
BadBytecodeSize did not reproduce.
```

CodeWindow refill remained stable:

```text
cw_bytes=27004
cw_refill_us≈19041–19071
cw_effective_mb_s≈1.416–1.418
dma_fallbacks=0
```

The feature-gated CodeWindow DMA read experiment is stable enough to keep as the
KOTO-0132 result.

## Follow-up

One minor cleanup remains:

```text
dma_successes currently reports 0 even during the experiment summary.
```

This should be handled in a cleanup-only follow-up issue by checking or renaming
the counter semantics.

Possible clearer counter names:

```text
cw_dma_attempts
cw_dma_successes
cw_dma_fallbacks
```

or, if the counter is refill-specific:

```text
cw_dma_refill_attempts
cw_dma_refill_successes
cw_dma_refill_fallbacks
```

## Outcome

KOTO-0132 established that the current PSRAM production path is correct but slow.
KotoBlocks gameplay is significantly affected by code-window refill cost.

The issue also corrected the optimization direction:

```text
Retired: word-oriented DMA path
Adopted: byte-oriented PIO FIFO / DMA diagnostic path
```

The production path remains safe and unchanged, while the feature-gated
CodeWindow DMA read experiment has passed hardware smoke testing and can be
kept as the result of this research issue.

