# Diagnostics

Diagnostics are kept separate from the stable driver path. They are intended for
hardware bring-up, timing sweeps, and transfer-path experiments on RP2040
PicoCalc-style QPI PSRAM.

## Feature Flags

- `diag`: host-side diagnostic helpers.
- `rp2040-embassy`: hardware backend and examples.
- `rp2040-embassy-u8-direct-probe`: direct `u8` RX DMA probe.
- `psram_fast_read_clkdiv2`: opt-in fast read diagnostic validation.

## Examples

Stable example:

- `rp2040_embassy_smoke`: initialization and basic hardware smoke test.
- `rp2040_embassy_payload_path_bench`: the stable payload-path benchmark
  (`byte_fallback`, `polling_burst_diagnostic`, and `word_stream_polling`).

Diagnostic harnesses (exploratory; not part of the stable path):

- `rp2040_embassy_transaction_pio_diag`: transaction-PIO diagnostics — CPU-fed,
  TX DMA, RX DMA, TX/RX DMA, RX byte-FIFO probe, fast clkdiv sweep, and
  falling-alignment diagnostics.
- `rp2040_embassy_fast_clkdiv2_validation`: opt-in `FastFallingClkdiv2` practical
  validation (`basic`, `size_sweep`, `boundary_sweep`, `stress`, `write_read`),
  gated behind `psram_fast_read_clkdiv2`.
- `rp2040_embassy_compare`: compare-oriented read/write checks.
- `rp2040_embassy_clkdiv_sweep`: clock-divider sweep.
- `rp2040_embassy_chunk_bench`: chunk-size benchmark.
- `rp2040_embassy_dma_memcpy_probe` and `rp2040_embassy_dma_pac_memcpy_probe`:
  DMA probes.
- `rp2040_uart_hello`: UART logging sanity check.

## Build Commands

```sh
cargo check --target thumbv6m-none-eabi --features rp2040-embassy --example rp2040_embassy_smoke
cargo check --target thumbv6m-none-eabi --features rp2040-embassy --example rp2040_embassy_payload_path_bench
cargo check --target thumbv6m-none-eabi --features rp2040-embassy --example rp2040_embassy_transaction_pio_diag
cargo check --target thumbv6m-none-eabi --features "rp2040-embassy psram_fast_read_clkdiv2" --example rp2040_embassy_fast_clkdiv2_validation
```

## Hardware Diagnostic Commands

Baseline payload path benchmark:

```sh
cargo run --release --target thumbv6m-none-eabi --features rp2040-embassy --example rp2040_embassy_payload_path_bench
```

Transaction-PIO exploratory diagnostics:

```sh
cargo run --release --target thumbv6m-none-eabi --features rp2040-embassy --example rp2040_embassy_transaction_pio_diag
```

`FastFallingClkdiv2` practical validation (opt-in):

```sh
cargo run --release --target thumbv6m-none-eabi --features "rp2040-embassy psram_fast_read_clkdiv2" --example rp2040_embassy_fast_clkdiv2_validation
```

Expected fast-path success marker:

```text
fast_falling_clkdiv2_practical validation status=passed
```

The `FastFallingClkdiv2` validation is compiled only when
`psram_fast_read_clkdiv2` is enabled; without it that example is an empty stub.

## Diagnostic Boundaries

Diagnostic paths may change timing, PIO programs, FIFO mode, DMA use, or
alignment strategy. The stable driver path remains `TimingConfig::DEFAULT` and
does not depend on the fast read feature.
