# QPI transaction PIO/DMA plan

## Stable baseline

The last known-good CPU-polling baseline remains frozen:

- `byte_fallback` works and stays the default public path.
- `polling_burst_diagnostic` is the fastest current CPU-polling read path.
- `word_stream_polling` is the fastest current CPU-polling write path.
- Word-stream read diagnostics stay frozen at `batch_words=4`.
- `rx_fifo_join=true` is unsupported for the current TX-count read stream.
- Public `read_exact` / `write_all` APIs, production defaults, and
  `TimingConfig::PICOCALC_SAFE` remain unchanged.

## Frozen payload-only DMA direction

The payload-only read-DMA experiment is frozen. It showed that
`qpi_read_stream` can run without DMA and that the CPU can manually drain the
RX FIFO, but arming RX DMA before enabling the state machine caused hardware
hangs or boot/log regressions. That suggests the split model, where the CPU
drives command/address/dummy and DMA handles only the payload, is too fragile
for the next milestone.

Do not continue that path as the primary direction unless it is explicitly
reopened.

## New direction: full-transaction PIO

The new direction is a full-transaction QPI PIO program. The transaction is fed
through the TX FIFO as metadata plus command stream:

- output nibble count
- input nibble count
- command
- address
- dummy nibbles
- optional write payload in later milestones

The PIO program owns one whole bus transaction: assert CS, output the outbound
QPI nibbles, switch SIO direction for reads, sample inbound nibbles, restore
SIO output/idle state, and deassert CS. This gives later DMA work a single
coherent transaction model instead of a CPU/DMA phase split.

## rp2040-psram reference

This design is informed by the MIT-licensed polpo/rp2040-psram transaction
model in `docs/References/rp2040-psram-main`. The useful idea is the
transaction-oriented PIO contract: TX FIFO metadata describes how many bits or
nibbles to write and read, and one PIO program performs the write/read sequence
with DMA-ready FIFO behavior.

The implementation here is adapted for PicoCalc QPI PSRAM, the existing Rust
Embassy backend, and the current command/address/dummy timing. It does not copy
unrelated SPI layout, pin setup, or public APIs from the reference project.
See `docs/THIRD_PARTY_NOTICES.md` for attribution.

## Staged milestones

1. CPU-fed transaction PIO diagnostic
   - Read-only first.
   - 16-byte aligned reads only.
   - `repeated_0xad` hardware benchmark coverage only.
   - No DMA and no public API/default changes.

2. TX DMA
   - Feed transaction metadata and command/write stream through TX DMA.
   - Keep RX CPU-drained until TX sequencing is stable.

3. RX DMA
   - Drain read payload through RX DMA after the full transaction PIO program is
     stable under CPU-fed TX.
   - Avoid the frozen payload-only DMA split.

4. Combined TX+RX DMA
   - Drive command/write stream and read payload with paired DMA channels.
   - Promote only after diagnostics show clear completion or clear errors
     without hangs.

# QPI Transaction PIO Throughput Analysis

## Status

- Phase: 4-7L
- Scope: diagnostic analysis
- Target: PicoCalc RP2040 + QPI PSRAM
- Public API impact: none
- Default path impact: none

## Summary

The current transaction PIO read path is now functionally validated across:

- CPU-fed transaction PIO
- TX-DMA transaction PIO
- RX-DMA transaction PIO with u32 staging
- Combined TX/RX DMA transaction PIO
- RX byte-FIFO transaction PIO
- RX byte-FIFO RX-DMA direct-to-u8

The remaining read bandwidth bottleneck is not DMA correctness or staging capacity.
The measured 3.4–3.6 MB/s range matches the theoretical limit of the current
PIO payload loop under `TimingConfig::PICOCALC_SAFE`.

## Current Koto transaction read loop

The current transaction PIO read loop is:

```pio
txpio_in:
    in pins, 4 side 0b10 [1]
    jmp y-- txpio_in side 0b00 [1]
````

This reads one QPI nibble per loop iteration.

Each loop iteration has:

* `in pins, 4` = 1 PIO instruction + `[1]` delay = 2 PIO cycles
* `jmp y--` = 1 PIO instruction + `[1]` delay = 2 PIO cycles

Therefore:

```text
cycles_per_input_nibble = 4
```

## Theoretical throughput

For `TimingConfig::PICOCALC_SAFE`, the current read clock divider is:

```text
read_clkdiv = 4.0
```

Assuming the common RP2040 default system clock:

```text
sysclk = 125 MHz
```

The theoretical payload-only throughput is:

```text
PIO instruction rate = sysclk / read_clkdiv
                     = 125 MHz / 4
                     = 31.25 M instructions/s

QPI nibble rate      = instruction_rate / cycles_per_nibble
                     = 31.25 MHz / 4
                     = 7.8125 M nibbles/s

QPI byte rate        = nibble_rate / 2
                     = 3.90625 MB/s
```

If the bench is running at 133 MHz:

```text
133 MHz / 4 / 4 / 2 = 4.15625 MB/s
```

Measured results around 3.47–3.6 MB/s are therefore plausible.
They are already close to the payload-only ceiling of the current loop.

## Measured results

Representative 16 KiB read results:

```text
CPU-fed transaction PIO, large chunk:
  ~3.6 MB/s

RX byte-FIFO RX-DMA direct-to-u8:
  ~3.47 MB/s

u32 staging RX-DMA, 4096B chunk:
  ~3.0 MB/s
```

The byte-FIFO direct DMA path removes the u32 staging buffer and unpack step,
but it still uses the same PIO payload loop, so the maximum bandwidth remains
limited by the 4-cycle-per-nibble read loop.

## External throughput comparison

Other PicoCalc QPI PSRAM backends are useful as throughput reference points
because they target PicoCalc QPI/QSPI PSRAM.

One high-throughput read loop has no `[1]` delay slots:

```pio
readloop:
    in pins, 4          side 0b00
readloop_mid:
    jmp y--, readloop   side 0b10
```

This is:

```text
cycles_per_input_nibble = 2
```

High-throughput implementations may also use:

* byte FIFO granularity
* `DMA_SIZE_8`
* direct DMA into the caller buffer
* lower clock divider, typically `clkdiv = 1.0f`
* state machine setup that is reused rather than heavily reconfigured per chunk

At 125 MHz and `clkdiv = 1.0`, the theoretical payload-only throughput is:

```text
125 MHz / 1 / 2 / 2 = 31.25 MB/s
```

This explains why aggressively tuned implementations can report much higher
numbers than the current conservative Koto transaction PIO diagnostic path.

## Current bottleneck conclusion

The current bottleneck is the PIO payload loop itself.

DMA is now proven to work, including:

* PAC TX-DMA
* PAC RX-DMA
* combined TX/RX DMA
* RX byte-FIFO DMA direct-to-u8

However, DMA does not increase the PIO-generated payload rate.
The current loop produces one QPI nibble every four PIO cycles.

Therefore, further bandwidth improvement requires a faster PIO program and/or
a faster diagnostic timing profile, not more DMA plumbing.

## Next diagnostic step

Add a diagnostic-only no-delay transaction PIO variant.

Initial goal:

* keep existing working paths unchanged
* keep `TimingConfig::PICOCALC_SAFE` unchanged
* keep RX byte-FIFO direct-to-u8 DMA
* remove only the `[1]` delay slots from the payload read loop
* test with the same read clock divider first

Proposed fast loop:

```pio
txpio_in:
    in pins, 4 side 0b00
    jmp y-- txpio_in side 0b10
```

or an equivalent edge-preserving variant if the current sampling polarity needs
to be maintained.

Expected theoretical throughput at 125 MHz, `read_clkdiv = 4.0`:

```text
125 MHz / 4 / 2 / 2 = 7.8125 MB/s
```

If this works, the next step is a diagnostic clock-divider sweep:

* `read_clkdiv = 4.0`
* `read_clkdiv = 3.0`
* `read_clkdiv = 2.0`
* `read_clkdiv = 1.0`

The fast path must remain diagnostic-only until validated on hardware.
