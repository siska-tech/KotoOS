# koto-pico

PicoCalc RP2040 / RP2350A backend for KotoOS: the official product firmware
plus a small set of single-peripheral hardware probes.

## Binaries

RP2040 remains the default Cargo feature and builds for `thumbv6m-none-eabi`.
RP2350A/Pico 2 W builds for `thumbv8m.main-none-eabihf` with an explicit MCU
feature. Firmware does not initialize the wireless radio.

### Official firmware

```sh
cargo build -p koto-pico --bin koto_firmware --target thumbv6m-none-eabi --release
```

For Pico 2 W, install the Armv8-M target and Raspberry Pi's official
`picotool` 2.x, then use the board-specific build helper:

```powershell
rustup target add thumbv8m.main-none-eabihf
$env:PICOTOOL = "C:\path\to\picotool.exe"
tools\build-rp2350a.ps1
```

The helper runs the equivalent Cargo command:

```powershell
cargo build -p koto-pico --bin koto_firmware --release `
  --target thumbv8m.main-none-eabihf --no-default-features `
  --features board-picocalc-pico2w,ram_interpreter,ram_audio_mixer
```

The `board-picocalc-pico2w` profile selects RP2350A and includes the fast
CodeWindow QPI/RX-DMA path. Its
divider-2 profile runs the PIO state machine at 75 MHz on the 150 MHz RP2350A
clock and automatically re-reads through the safe path if a fast refill fails.

It emits
`target/thumbv8m.main-none-eabihf/release/koto_firmware-picocalc-pico2w-rp2350a.uf2`
with the RP2350 Arm-secure UF2 family and the RP2350-E10 absolute block. Do not
use `elf2uf2-rs` for this artifact: it emits the RP2040 family ID. Pass
`-AllBins` to release-build the product firmware and every retained probe.
Pass `-ValidationBundle` to also convert all six probes to board-named UF2
files and generate a product image with `force_psram_fallback` enabled. The
latter is diagnostic-only and proves the SRAM fallback; do not distribute it
as the normal firmware.

Enable the KOTO-0243 product NetworkService and the native KotoConfig Wi-Fi
page with:

```powershell
tools\build-rp2350a.ps1 -WifiConfig
```

The board-named output is
`target/thumbv8m.main-none-eabihf/release/koto_firmware-picocalc-pico2w-rp2350a-wifi-config.uf2`.
The legacy `-NetworkServiceProbe` switch remains as a compatibility alias for
older validation notes; new product validation uses `-WifiConfig`.

### Pico W (RP2040) wifi-config profile

The KOTO-0251 Pico W product image adds `network_service` to the Pico W board
profile. Unlike Pico 2 W there is no boot-time network runtime: the switchable
KOTO-0227 residency installs the arena-owned CYW43 lifecycle on demand when the
radio is enabled from the KotoConfig Wi-Fi page and tears it down again on
radio-off, capability loss, or fault, so all network runtime state lives inside
the borrowed 36 KiB rich-audio arena and the offline boot path stays identical
to the offline artifact.

```powershell
cargo build -p koto-pico --bin koto_firmware --release `
  --target thumbv6m-none-eabi --no-default-features `
  --features board-picocalc-picow,ram_interpreter,ram_audio_mixer,network_service
elf2uf2-rs target\thumbv6m-none-eabi\release\koto_firmware `
  target\thumbv6m-none-eabi\release\koto_firmware-picocalc-picow-rp2040-wifi-config.uf2
```

The default offline Pico W artifact
(`--features board-picocalc-picow,ram_interpreter,ram_audio_mixer`, board-named
`koto_firmware-picocalc-picow-rp2040-offline.uf2`) keeps its accepted behavior
and links no network stack, sockets, or DHCP/DNS state.

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

Cross-check both MCU profiles without generating UF2 files:

```powershell
python harness\check_embedded.py
```

## Archived bring-up experiments

Obsolete or superseded bring-up binaries live under
[`bringup/archive/`](bringup/archive). They are intentionally kept out of
`src/bin/` so Cargo does not build them, and they have no `[[bin]]` entry. See
`bringup/archive/README.md` for what each one was and why it was retired.
