# Board profiles

KotoOS separates a physical board selection from its MCU implementation.
Firmware build commands select exactly one public `board-*` feature. That
feature selects the internal `mcu-*` feature needed by Embassy and the PAC.

## Boundaries

- `src/koto-pico/src/board/` owns board identity, module memory capacities,
  validated clocks, board-sized memory policy such as CodeWindow slots,
  carrier GPIO mapping, and semantic peripheral roles.
- MCU features own chip/PAC differences such as DMA register layout, linker
  memory maps, and the Embassy chip feature. They are not build entry points.
- Product firmware consumes `board::split_peripherals()` and names such as
  `lcd_spi`, `sd_detect`, and `psram_sio0`; it must not add `PIN_n` mappings.
- Portable crates (`koto-core`, `koto-gfx`, `koto-vm`, and app code) must not
  depend on board features, GPIO numbers, or `embassy-rp` peripheral types.

The PicoCalc carrier mapping shared by Pico and Pico 2 W lives in
`board/picocalc.rs`. Module-specific clocks and capacities live in
`board/picocalc_pico.rs` and `board/picocalc_pico2w.rs`.

## Wi-Fi capability composition

`BoardCapabilities::WIFI` means only that the profile provides a supported
radio transport. It does not expose KotoConfig's `WIFI_CONFIG` page by itself.
That composite capability additionally requires an initialized Wi-Fi HAL, a
compiled and live NetworkService, and an initialized credential provider. A
W suffix, RP2040/RP2350 MCU selection, or concurrent Audio/Wi-Fi residency is
never used as a substitute. See the
[KotoConfig Wi-Fi extension contract](KOTOCONFIG_WIFI_EXTENSION.md).

## Adding a board

1. Add a public `board-<name>` Cargo feature that selects one internal MCU
   feature and the board's deliberate optional capabilities.
2. Add a profile module under `src/board/` with a stable artifact/banner ID,
   capacities, validated clocks, and peripheral-role adapter.
3. Add the feature to the exactly-one-board checks in `lib.rs` and the build
   script's artifact identity selection.
4. Add its target/linker configuration and one `harness/check_embedded.py`
   cross-build row.
5. Generate a board-named UF2 and complete the applicable hardware validation
   matrix before marking the profile supported.

Pico Plus 2(W) should follow this sequence. Its RP2350B/QMI PSRAM support is a
board capability; it must not be inferred merely from the RP2350 MCU feature.
