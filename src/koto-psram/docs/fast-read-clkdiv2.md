# Fast Read Clkdiv2

`FastFallingClkdiv2` is an opt-in diagnostic read profile for RP2040
PicoCalc-style QPI PSRAM. It is enabled only with `psram_fast_read_clkdiv2`.

## Problem

The current no-delay `clkdiv = 2.0` read-loop candidate did not align payload
bytes correctly with the existing sampling phase. In practice, the mismatch
looked like an alignment problem rather than a simple throughput limit.

## Candidate

The hardware-passing candidate uses:

- `read_clkdiv = 2.0`
- falling-edge sampling
- one extra dummy byte for alignment
- RX byte-FIFO reads
- RX DMA direct-to-`u8`
- diagnostic transfer path:
  `transaction_pio_fast_rx_byte_fifo_rx_dma_diagnostic`

## Validation

The practical validation in `rp2040_embassy_fast_clkdiv2_validation` covers:

- basic
- size sweep
- boundary sweep
- stress
- write-read

The UART success marker is:

```text
fast_falling_clkdiv2_practical validation status=passed
```

## Result

Practical hardware validation passed. 16 KB reads were observed around
10 MB/s on the diagnostic path.

## Status

- Feature-gated by `psram_fast_read_clkdiv2`.
- Not default.
- Kept separate from the safe `TimingConfig::PICOCALC_SAFE` path.
- Not yet applied to CodeWindow Refill.
- Intended to remain diagnostic until the production integration is designed and
  validated separately.
