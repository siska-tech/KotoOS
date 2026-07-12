# KotoOS Implementation Status

This is the compact “what exists today?” map. Detailed requirements remain in
[REQUIREMENTS.md](REQUIREMENTS.md); this page points to issue records instead
of repeating their design text.

## Status Labels

| Label | Meaning |
| :-- | :-- |
| `core-implemented` | Portable behavior exists in `koto-core` and has host tests. |
| `simulated` | The behavior is integrated and exercised through KotoSim. |
| `hardware-validated` | A physical PicoCalc/Pico probe result has been recorded. |
| `hardware-pending` | Core/simulator foundations may exist, but physical peripheral validation is still required. |
| `planned` | Accepted work exists only as an issue or design document. |

## Current Baseline

| Area | Status | Evidence |
| :-- | :-- | :-- |
| Shell, package scan, save data, failure recovery | `simulated` | KOTO-0002, KOTO-0010, KOTO-0055–KOTO-0058 |
| Bytecode verifier, VM, compiler, SDK loop | `core-implemented`, `simulated` | KOTO-0033–KOTO-0053 |
| Memo editor and Japanese IME | `core-implemented`, `simulated` | KOTO-0037–KOTO-0042, KOTO-0070–KOTO-0100 |
| Runtime budgets and heap-backed ActorArray state | `core-implemented`, `simulated` | KOTO-0101–KOTO-0105 |
| Pico HAL/toolchain bootstrap | `core-implemented` | KOTO-0064 |
| UF2 boot, visible blink, USB CDC logging | `hardware-validated` | KOTO-0065 and [PICO_HARDWARE_LOG.md](../hardware/PICO_HARDWARE_LOG.md) |
| LCD init, RGB666 drawing, and bounded DMA updates | `hardware-validated` | KOTO-0066 and [PICO_HARDWARE_LOG.md](../hardware/PICO_HARDWARE_LOG.md) |
| Pico firmware frame loop and portable shell input | `core-implemented` | KOTO-0117 |
| Pico firmware SD manifest discovery and launcher population | `core-implemented` | KOTO-0118 |
| Pico firmware portable shell raster and RGB565-to-RGB666 transport | `hardware-pending` | KOTO-0119–KOTO-0120 |

## Hardware Bring-Up

| Peripheral | Status | Issue |
| :-- | :-- | :-- |
| LCD init, fill, orientation, partial update | `hardware-validated` | KOTO-0066 |
| Keyboard I2C and chord matrix | `hardware-validated` | KOTO-0067, KOTO-0025 |
| SD FAT mount and sequential manifest read | `hardware-validated` | KOTO-0068 |
| PSRAM explicit block-transfer round trip | `hardware-validated` | KOTO-0069, KOTO-0022 |
| PWM audio output | `hardware-validated` | KOTO-0114 |
| Battery/power reporting | `hardware-validated` | KOTO-0115 |

The peripheral probes are complete. KOTO-0117 starts product-firmware
integration by running the portable shell input/state path on hardware.
KOTO-0118 populates that shell from SD manifests. KOTO-0119 uses the same
portable KotoShell painter and M+ font as KotoSim through bounded scanline
strips, but physical interaction performance remains below the accepted gate.
KOTO-0120–KOTO-0126 track dirty rendering, catalog reintegration, metadata and
icons, shell actions/status, runtime launch, and final KotoSim parity.
