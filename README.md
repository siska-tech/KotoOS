# KotoOS

KotoOS is a lightweight Japanese PDA environment and small-app runtime for the
ClockworkPi PicoCalc (RP2040), written in Rust.

It ships a portable shell, a bytecode app runtime with its own compiler
toolchain, an SKK-based Japanese IME, a retained-mode graphics pipeline, and a
two-core audio service — all running both on real PicoCalc hardware
(`koto-pico` firmware) and in a desktop simulator (KotoSim) that shares the
same core crates.

## Screenshots

All frames below are captured from KotoSim, which renders the same 320×320
frames as the device.

| Boot splash | Home shell | Memo + SKK IME |
| :---: | :---: | :---: |
| ![Boot splash](docs/images/splash.png) | ![Home shell launcher](docs/images/shell.png) | ![Memo app converting kanji with the SKK IME](docs/images/memo.png) |

| KotoRun | KotoRogue | KotoBlocks | Sokoban |
| :---: | :---: | :---: | :---: |
| ![KotoRun side-scrolling action](docs/images/kotorun.png) | ![KotoRogue dungeon crawl](docs/images/kotorogue.png) | ![KotoBlocks falling-block game](docs/images/koto_blocks.png) | ![Sokoban stage 1](docs/images/sokoban.png) |

## Documentation

The documentation tree is indexed in [docs/README.md](docs/README.md). The
most important source documents are:

- [Requirements](docs/planning/REQUIREMENTS.md)
- [Research](docs/planning/Research.md)
- [Architecture](docs/architecture/ARCHITECTURE.md)
- [HAL API Draft](docs/architecture/HAL_API.md)
- [RP2040 Bring-Up Plan](docs/hardware/RP2040_BRINGUP.md)
- [Implementation Status](docs/planning/IMPLEMENTATION_STATUS.md)
- [KPA Package Format](docs/spec/KPA_FORMAT.md)
- [Bytecode App Development Roadmap](docs/planning/BYTECODE_APP_DEV_ROADMAP.md)
- [Validation Plan](docs/planning/VALIDATION_PLAN.md)
- [Traceability](docs/planning/TRACEABILITY.md)
- [Issues](docs/ISSUES_main.md)

## Current Development Stance

- Target the constrained RP2040 first.
- Use Rust as the primary implementation language.
- Keep core logic portable between KotoSim and PicoCalc.
- Treat PSRAM as block-transfer storage on RP2040, not memory-mapped RAM.
- Prefer measurable harnesses before hardware-specific optimization.
- Keep completed simulator baseline work separate from active embedded bring-up
  and cleanup issues in [Issues](docs/ISSUES_main.md).

## Local Checks

Run the standard local CI checks:

```powershell
python harness\check_all.py
```

This runs Rust formatting, Clippy, tests, and the project harness. It exits non-zero as soon as any check fails.

Run only the dependency-free project harness when iterating on repository metadata:

```powershell
python harness\check_project.py
```

The simulator currently scans package manifests from `sdcard_mock/apps/*.kpa.json`.

## RP2040 Bootstrap Build

The PicoCalc backend is an explicit embedded workspace member, but it is not a
default member. Normal host `cargo test`, Clippy, and KotoSim commands therefore
do not compile embedded dependencies.

Install the RP2040 target and UF2 converter once:

```powershell
rustup target add thumbv6m-none-eabi
cargo install elf2uf2-rs
```

Build the minimal bootstrap probe:

```powershell
cargo build -p koto-pico --bin bootstrap --target thumbv6m-none-eabi
```

To create a UF2 without flashing it automatically:

```powershell
elf2uf2-rs target\thumbv6m-none-eabi\debug\bootstrap `
  target\thumbv6m-none-eabi\debug\bootstrap.uf2
```

Copy `bootstrap.uf2` to the PicoCalc module in BOOTSEL mode. `cargo run` uses
the configured `elf2uf2-rs -d` runner when a mounted RP2040 BOOTSEL volume is
available. The bootstrap only initializes the RP2040 and waits; blink and
USB-CDC logging belong to KOTO-0065.

Build the blink and USB-CDC probe:

```powershell
cargo build -p koto-pico --bin blink_cdc --target thumbv6m-none-eabi
```

After flashing, the standard Pico 1H LED on GP25 should blink once per second.
Open the enumerated USB serial port at any baud rate; the firmware emits a
version banner and a heartbeat every two seconds. Record physical results in
[the hardware bring-up log](docs/hardware/PICO_HARDWARE_LOG.md).

For a Pico W / Pico WH module, use the CYW43-backed variant instead:

```powershell
cargo build -p koto-pico --bin blink_cdc_pico_w --target thumbv6m-none-eabi
```

The Pico W firmware controls the onboard LED through the wireless chip rather
than GP25 directly. Its USB product name is `KotoOS Pico W probe`.

Build the PicoCalc LCD fill probe:

```powershell
cargo build -p koto-pico --bin lcd_fill --target thumbv6m-none-eabi
```

After flashing, open its USB serial port to start the probe. It logs the
selected `ili9488-spi` profile, cycles red/green/blue/black fills, then leaves
corner markers, a centered rectangle, and a cyan scanline band visible for
orientation and address-window inspection. Record the observed panel behavior
in [the hardware bring-up log](docs/hardware/PICO_HARDWARE_LOG.md).

Build the consolidated device probe dashboard:

```powershell
cargo build -p koto-pico --bin device_probe --target thumbv6m-none-eabi
```

The dashboard reports LCD, keyboard I2C, SD-detect, PSRAM readiness, and the
bounded SRAM working set over USB CDC. When LCD initialization succeeds it also
shows color-coded subsystem rows plus live raw/normalized keyboard activity.
PSRAM remains explicitly `pending` until its PIO block-transfer driver lands;
the dashboard never treats it as pointer-addressable memory or allocates a full
framebuffer.

Build the first product-firmware slice:

```powershell
cargo build -p koto-pico --bin koto_firmware --target thumbv6m-none-eabi
```

The firmware initializes the validated LCD, keyboard, and SD paths, runs a
bounded 16 ms frame loop, and feeds PicoCalc key transitions into the portable
`ShellState`. It mounts `APPS/`, reads bounded `.kpa.json` manifests, and shows
the same launcher grid, details pane, status strip, and command bar as KotoSim.
Arrow keys navigate the package list and Z/Enter confirms the selection.
Rendering reuses `ShellState::paint` with the M+ font in 16-line RGB565 strips,
converts them to the panel's RGB666 wire format, and does not allocate a
full-screen framebuffer.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or
  http://opensource.org/licenses/MIT)

at your option.

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms or
conditions.

### Bundled assets

- `assets/fonts/mplus10.kfont` / `mplus12.kfont` are converted from the
  [M+ BITMAP FONTS](https://mplusfonts.github.io/) (Copyright 2002-2005 COZ),
  distributed under their own free license; see
  [assets/fonts/LICENSE_J](assets/fonts/LICENSE_J) and
  [assets/fonts/LICENSE_E](assets/fonts/LICENSE_E).
- `sdcard_mock/dict/skk_koto.skk` is an original SKK-format dictionary written
  from scratch for KotoOS and dedicated to the public domain under
  CC0 1.0 Universal. It is not derived from any SKK-JISYO file.
