# Architecture

`koto-psram` separates the portable PSRAM model from RP2040-specific PIO and DMA
code. The stable API is intentionally small: configure the bus, force a known
device mode, and read or write byte slices through bounded QPI transactions.

## Layers

- `src/addr.rs`, `src/region.rs`, `src/protocol.rs`, and `src/state.rs` define
  addressing, transaction shape, and driver state.
- `src/config.rs` defines `Pins` and `TimingConfig`. `TimingConfig::DEFAULT`
  is the safe PicoCalc profile.
- `src/pio/blocking.rs` owns the blocking driver wrapper and chunk splitting.
- `src/rp2040_embassy` contains the feature-gated Embassy RP2040 backend.
- `src/diag` contains optional host-side diagnostic helpers behind `diag`.

## Stable Path

The stable/default path uses `BlockingDriver` with `TimingConfig::PICOCALC_SAFE`:

- `read_clkdiv = 4.0`
- `write_clkdiv = 2.0`
- `fallback_read_clkdiv = 8.0`
- QPI read dummy cycles set to 6
- maximum chunk length of 256 bytes

This path is the default crate behavior and does not require experimental fast
read features.

## RP2040 Embassy Backend

The `rp2040-embassy` feature exposes the concrete backend for hardware builds.
Board code passes owned PIO, state-machine, GPIO, and DMA resources into
`EmbassyRpQpiBackend`, then uses the same blocking driver surface as the core
crate.

Diagnostic setters are available only on the concrete backend and are named with
`for_diagnostics` where they expose non-default transfer paths. They are not
part of the safe default path.

## Experimental Fast Reads

`FastFallingClkdiv2` is an opt-in diagnostic profile enabled by
`psram_fast_read_clkdiv2`. It uses the neutral diagnostic path
`transaction_pio_fast_rx_byte_fifo_rx_dma_diagnostic` and is documented in
[fast-read-clkdiv2.md](fast-read-clkdiv2.md).

The fast path has passed practical validation, but it is not the default and has
not been applied to CodeWindow Refill.
