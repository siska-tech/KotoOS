# Public Release Checklist

- [ ] `cargo fmt --check`
- [ ] `cargo check`
- [ ] `cargo test --target x86_64-pc-windows-msvc --features diag`
- [ ] `cargo check --target thumbv6m-none-eabi`
- [ ] `cargo check --target thumbv6m-none-eabi --features rp2040-embassy --example rp2040_embassy_smoke`
- [ ] `cargo check --target thumbv6m-none-eabi --features rp2040-embassy --example rp2040_embassy_payload_path_bench`
- [ ] `cargo check --target thumbv6m-none-eabi --features rp2040-embassy --example rp2040_embassy_transaction_pio_diag`
- [ ] `cargo check --target thumbv6m-none-eabi --features "rp2040-embassy psram_fast_read_clkdiv2" --example rp2040_embassy_fast_clkdiv2_validation`
- [ ] Hardware diagnostic:
  `cargo run --release --target thumbv6m-none-eabi --features "rp2040-embassy psram_fast_read_clkdiv2" --example rp2040_embassy_fast_clkdiv2_validation`
- [ ] UART log includes:
  `fast_falling_clkdiv2_practical validation status=passed`
- [ ] No external-project-derived identifiers in code, public APIs, feature
  flags, comments, or docs.
- [ ] README feature flags match `Cargo.toml`.
- [ ] License files are present and README references them.
