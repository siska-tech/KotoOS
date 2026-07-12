# LCD Init Profiles

This document defines the panel-variation boundary for the PicoCalc LCD. It is
the research output of [KOTO-0026](../issues/main/KOTO-0026-lcd-init-profiles.md) and
extends the LCD bring-up probe in [RP2040_BRINGUP.md](RP2040_BRINGUP.md).

KotoOS core rendering must not branch on LCD controller names. The embedded
device HAL selects one LCD profile, initializes the controller from that
profile, and exposes only the existing `VideoHal` update paths to core code.

Status: profile contract accepted. Final command byte sequences remain pending
hardware validation because PicoCalc units may ship with ILI9488,
ST7365P-compatible, or clone controllers.

## Known Controller Variants

| Variant | Expected role | Compatibility notes | HAL handling |
| :------ | :------------ | :------------------ | :----------- |
| `ili9488-spi` | Original / common PicoCalc-compatible 320x320 IPS panel controller | DBI Type-C SPI command set. Treat RGB565 write support as a probe result, not an assumption; some reports indicate 18-bit RGB666 transfers are the reliable path. | Preferred first profile. Try ID read when MISO is usable, then run conservative init and pixel-format probe. |
| `st7365p-compatible` | Newer or alternate PicoCalc panel controller reported as ILI9488-compatible | Expected to share the core address-window, memory-write, MADCTL, sleep, and display-on commands, but may need different power/gamma/inversion defaults or delays. | Separate profile with its own init table and timing values. Do not patch the ILI9488 table in place. |
| `unknown-9488-compatible` | Clone, relabeled, or unreadable panel | May not return a stable ID over SPI, especially if MISO is unavailable or not driven by the module. | Select by board config or fallback order; require color/orientation/partial-update probes before marking supported. |

The profile names above are stable identifiers for logs and validation records.
They are not user-facing product names.

## HAL Profile Data

The embedded HAL should represent the LCD profile as data so controller
differences stay out of core rendering code and out of ad hoc probe binaries.
A profile needs:

| Field | Purpose |
| :---- | :------ |
| `name` | Stable log identifier such as `ili9488-spi`. |
| `logical_width`, `logical_height` | KotoOS surface size. PicoCalc is currently `320x320`. |
| `native_width`, `native_height` | Controller addressable size when larger than the visible square region. |
| `x_offset`, `y_offset` | Visible-window offset for `CASET` / `PASET` address windows. |
| `reset_low_us`, `reset_high_us` | GPIO reset timing before command writes. |
| `sleep_out_delay_ms`, `display_on_delay_ms` | Required waits after `SLPOUT` and `DISPON`. |
| `init_commands` | Ordered command table: command byte, parameter bytes, and optional per-command delay. |
| `madctl_base` | Orientation and scan-order bits for the selected mounting. |
| `color_order` | RGB vs BGR selection, usually encoded through MADCTL. |
| `write_pixel_formats` | Supported write formats and command parameters for `COLMOD`, e.g. RGB565 or RGB666. |
| `preferred_wire_format` | Format used for normal DMA transfers after probing. |
| `requires_565_to_666_expand` | Whether the HAL must expand `PixelFormat::Rgb565` surfaces to 18-bit wire data. |
| `supports_read_id` | Whether the backend should attempt ID/status reads on MISO. |
| `supports_partial_mode` | Whether `PTLON` / `PTLAR` style partial mode is safe, separate from normal address-window updates. |
| `max_spi_hz` | Initial conservative SPI clock and validated upper bound for the profile. |
| `dma_chunk_bytes` | Transfer chunk size that keeps DMA descriptors and line buffers bounded. |

The command table should be small enough to keep in flash. Runtime-selected
state, such as the measured pixel format or SPI clock, belongs in the HAL
instance, not in the static profile.

## Required Command Capabilities

Every supported profile must define or explicitly reject these operations:

| Operation | Typical command role | Used by |
| :-------- | :------------------- | :------ |
| Hardware reset | RESET pin plus post-reset delay | `VideoHal::init` |
| Exit sleep | `SLPOUT` with delay | `VideoHal::init` |
| Pixel format | `COLMOD` or compatible | `VideoHal::init` |
| Memory access control | `MADCTL` or compatible | Orientation probe and normal init |
| Address window | Column and page address set | `update_rect`, `update_scanlines` |
| Memory write | Start pixel stream | `update_rect`, `update_scanlines` |
| Display on/off | `DISPON` / optional `DISPOFF` | Init and future suspend |
| Inversion | Inversion on/off when required by the panel | Init profile |
| Partial mode | Partial area plus partial on/off, if reliable | Optional optimization only |

Normal dirty-rectangle updates do not require controller partial mode. They only
require setting an address window and streaming the changed pixels. Controller
partial mode is treated as a later power/bandwidth optimization and must be
proven separately.

## Bring-Up Probes

Run these probes from the first LCD probe binary before the profile is accepted.
Each result should log profile name, board label, firmware or build revision,
SPI clock, wire pixel format, and pass/fail status over USB-CDC.

### 1. Identity and Fallback

1. Toggle RESET using the profile timing.
2. If MISO is available, attempt read-ID and read-status commands at a slow SPI
   clock.
3. Log returned bytes even when they are all zeroes or all `0xff`.
4. If ID read is unavailable, select the configured profile and mark detection
   as `manual`.

Pass: the selected profile is recorded unambiguously. A missing ID is not a
failure if the board wiring or controller does not support reads.

### 2. Orientation

1. Draw a labeled corner pattern: red top-left, green top-right, blue
   bottom-left, white bottom-right.
2. Sweep the candidate `MADCTL` values for normal, rotated, mirrored, and BGR
   combinations.
3. Pick the value where the origin, axes, and text orientation match KotoOS
   coordinates: `(0, 0)` is top-left, `x` grows right, `y` grows down.

Pass: the chosen `madctl_base`, color order bit, and any visible-window offsets
are recorded in the profile notes.

### 3. Color Format

1. Initialize with the profile's preferred `COLMOD`.
2. Draw RGB primary bars, grayscale ramps, black, white, and alternating pixels.
3. Repeat using RGB565 wire data and RGB666 wire data when the profile lists
   both as candidates.
4. Verify whether byte order is high-byte-first for RGB565 and whether RGB666
   requires 3 bytes per pixel on the SPI wire.

Pass: colors are stable, not swapped, and the HAL records whether it can stream
`PixelFormat::Rgb565` directly or must expand to RGB666.

### 4. Address Windows and Dirty Rectangles

1. Fill the full screen black.
2. Draw small rectangles at all four corners and at center.
3. Draw one-pixel-wide vertical and horizontal lines at the edges.
4. Repeat at several SPI clocks up to the candidate profile limit.

Pass: only the requested rectangle changes, with no off-by-one edge pixels and
no wraparound into another row or column.

### 5. Scanline DMA

1. Transfer 1, 2, 8, and 16 scanline bands from SRAM line buffers.
2. Alternate two line buffers while DMA is active if the backend supports
   overlap.
3. Log transfer time and whether the CPU blocks the frame loop.

Pass: the DMA path updates the requested scanlines and returns control without
violating the non-blocking intent of NFR-DRAW-2.

### 6. Partial Mode

1. Use normal address-window updates as the baseline.
2. Enter controller partial mode only if the profile declares support.
3. Set a partial area and update inside and outside that area.
4. Exit partial mode and confirm full-screen updates still work.

Pass: partial mode is either marked supported with exact behavior, or disabled
for that profile. A failed partial-mode probe must not block dirty rectangles.

## Acceptance Record Template

Copy this table into hardware validation notes when a profile is proven:

| Field | Value |
| :---- | :---- |
| Date | pending |
| Board / panel label | pending |
| Selected profile | pending |
| Detection method | read-id / manual / fallback |
| Read-ID bytes | pending |
| SPI clock tested | pending |
| `MADCTL` value | pending |
| `COLMOD` value | pending |
| Wire pixel format | pending |
| RGB565 direct | yes / no / pending |
| Address-window result | pending |
| Scanline DMA result | pending |
| Partial mode | supported / disabled / pending |
| Evidence log | pending |

