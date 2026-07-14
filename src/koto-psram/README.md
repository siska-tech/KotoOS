# koto-psram

`koto-psram` is a `no_std` Rust PSRAM driver foundation for RP2040 boards that
connect to PicoCalc-style QPI PSRAM. It provides a stable byte-slice blocking
driver surface, an Embassy RP2040 backend behind a feature flag, and separate
diagnostic paths for hardware bring-up and throughput experiments.

## Target Hardware

The current hardware target is RP2040 with PicoCalc PSRAM wiring:

- SIO0: GP2
- SIO1: GP3
- SIO2: GP4
- SIO3: GP5
- CS: GP20
- SCK: GP21

UART examples log through the PicoCalc UART-USB bridge on UART0 TX / GP0.

## Current Status

The safe default path is the release baseline. `TimingConfig::DEFAULT` is
`TimingConfig::PICOCALC_SAFE`, using conservative QPI timing, chunk splitting,
and byte-slice reads and writes. Diagnostics and experimental transfer paths are
kept separate from the default driver behavior.

An optional fast read diagnostic path has passed practical hardware validation
with:

- `read_clkdiv = 2.0`
- falling-edge sampling
- extra dummy byte alignment
- RX byte-FIFO reads
- RX DMA direct-to-`u8`

That path remains opt-in and is not used by default.

## Feature Flags

- `default`: empty. Builds the core `no_std` crate with the stable API only.
- `diag`: enables host-side diagnostic helpers.
- `dma`: enables DMA support modules used by lower-level experiments.
- `rp2040-embassy`: enables the concrete Embassy RP2040 backend and hardware
  examples.
- `rp2040-embassy-u8-direct-probe`: enables the direct `u8` RX DMA probe.
- `psram_fast_read_clkdiv2`: enables the clkdiv2 fast read validation example
  (`rp2040_embassy_fast_clkdiv2_validation`).

## Building

Host checks:

```sh
cargo fmt --check
cargo check
cargo test --target x86_64-pc-windows-msvc --features diag
```

RP2040 checks:

```sh
cargo check --target thumbv6m-none-eabi --features rp2040-embassy
cargo check --target thumbv6m-none-eabi --features rp2040-embassy --example rp2040_embassy_smoke
cargo check --target thumbv6m-none-eabi --features rp2040-embassy --example rp2040_embassy_payload_path_bench
cargo check --target thumbv6m-none-eabi --features rp2040-embassy --example rp2040_embassy_transaction_pio_diag
cargo check --target thumbv6m-none-eabi --features "rp2040-embassy psram_fast_read_clkdiv2" --example rp2040_embassy_fast_clkdiv2_validation
```

The repository includes `.cargo/config.toml` for `thumbv6m-none-eabi` and an
`elf2uf2-rs -d` runner.

## Hardware Diagnostics

The hardware examples are split by purpose:

- `rp2040_embassy_payload_path_bench`: the stable payload-path benchmark
  (`byte_fallback`, `polling_burst_diagnostic`, and `word_stream_polling`).
- `rp2040_embassy_transaction_pio_diag`: the exploratory transaction-PIO
  diagnostics (CPU-fed, TX/RX/TX+RX DMA, RX byte-FIFO probe, fast clkdiv sweep,
  and falling-alignment diagnostics).
- `rp2040_embassy_fast_clkdiv2_validation`: the opt-in `FastFallingClkdiv2`
  practical validation, gated behind `psram_fast_read_clkdiv2`.

The baseline benchmark runs with:

```sh
cargo run --release --target thumbv6m-none-eabi --features rp2040-embassy --example rp2040_embassy_payload_path_bench
```

The `FastFallingClkdiv2` validation is opt-in and requires the feature:

```sh
cargo run --release --target thumbv6m-none-eabi --features "rp2040-embassy psram_fast_read_clkdiv2" --example rp2040_embassy_fast_clkdiv2_validation
```

On success its UART log includes:

```text
fast_falling_clkdiv2_practical validation status=passed
```

See [docs/diagnostics.md](docs/diagnostics.md) and
[docs/fast-read-clkdiv2.md](docs/fast-read-clkdiv2.md) for details.

## Known Limitations

- The RP2040 backend is blocking and Embassy-based.
- The safe path prioritizes correctness over peak throughput.
- Fast clkdiv2 reads are diagnostic-only, feature-gated, and not the default.
- `FastFallingClkdiv2` is not yet applied to CodeWindow Refill or any production
  refill path.
- Hardware validation is currently focused on PicoCalc-style RP2040 QPI PSRAM.

## Release Checklist

See [docs/release-checklist.md](docs/release-checklist.md).

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option. Third-party notices are collected in
[docs/THIRD_PARTY_NOTICES.md](docs/THIRD_PARTY_NOTICES.md).
