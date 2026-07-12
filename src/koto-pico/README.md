# koto-pico

PicoCalc (RP2040) backend for KotoOS: the official product firmware plus a small
set of single-peripheral hardware probes.

## Binaries

All targets build for `thumbv6m-none-eabi`. Flash a `.uf2` over BOOTSEL, or use
the configured `elf2uf2-rs` runner (`cargo run --bin <name> --target thumbv6m-none-eabi --release`).

### Official firmware

```sh
cargo build -p koto-pico --bin koto_firmware --target thumbv6m-none-eabi --release
```

`koto_firmware` is the product entry point (thin `main` over the
`koto_pico::firmware` module tree).

### Retained hardware probes

Each probe validates one peripheral in isolation. They are **not** part of
normal development — flash one manually only to re-validate that peripheral on
new hardware. Build/observation details are recorded in
[`docs/PICO_HARDWARE_LOG.md`](../../docs/hardware/PICO_HARDWARE_LOG.md).

| Probe | Validates | Issue |
| :--- | :--- | :--- |
| `probe_lcd` | ILI9488 SPI LCD fills, orientation, DMA band | KOTO-0066 |
| `probe_keyboard` | STM32 keyboard FIFO over I2C, latency, mappings | KOTO-0067 |
| `probe_sd` | SD mount + sequential read via `embedded-sdmmc` | KOTO-0068 |
| `probe_psram` | PSRAM round-trip (the KOTO-0127 streaming foundation) | KOTO-0069 |
| `probe_power` | STM32 battery/version bridge over I2C | KOTO-0115 |
| `probe_audio` | DMA-paced PWM audio output | KOTO-0114 |

```sh
# e.g. re-validate the LCD
cargo build -p koto-pico --bin probe_lcd --target thumbv6m-none-eabi --release
```

## Archived bring-up experiments

Obsolete or superseded bring-up binaries live under
[`bringup/archive/`](bringup/archive). They are intentionally kept out of
`src/bin/` so Cargo does not build them, and they have no `[[bin]]` entry. See
`bringup/archive/README.md` for what each one was and why it was retired.
