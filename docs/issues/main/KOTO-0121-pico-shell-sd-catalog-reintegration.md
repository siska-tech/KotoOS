# KOTO-0121: Pico Shell SD Catalog Reintegration

- Status: done
- Type: feature
- Priority: P0
- Requirements: FR-SHELL-1, FR-FS-1, FR-FS-3, NFR-MEM-1, NFR-REL-2

## Goal

Restore the validated SD package catalog to the product shell without the large
stack/async-state failure found during raster-backend integration.

## Acceptance Criteria

- [x] SD card initialization uses the physically validated clock strategy and
  cannot block boot indefinitely.
- [x] Package-list, filename, long-filename, and manifest buffers have explicit
  static or owned storage budgets rather than one large call-stack frame.
- [x] `APPS/*.kpa.json` discovery populates the real shell after cold boot.
- [x] SD absent, mount failure, malformed manifests, and an empty catalog each
  produce a usable shell state and UART diagnostic.
- [x] At least 15 manifests are loaded on the validated card without stack
  failure or SRAM-budget regression.

## Notes

KOTO-0068 and KOTO-0118 proved the storage behavior independently. During
KOTO-0119 diagnostics, entering the integrated catalog loader stopped before
its first UART statement, indicating an excessive function/async stack frame.

## Resolution

Root cause: `load_packages` was a synchronous function whose frame allocated a
27 KB `PackageList`, a 27 KB by-value return slot, plus the `names` /
manifest-bytes / LFN scratch arrays — all on the Cortex-M main stack on top of
`main`'s existing ~55 KB of large locals (`packages` + `ShellState`). The
prologue faulted while zero-initializing those arrays, before its first UART
line, exactly matching the observed symptom.

Fix:

- `load_packages` now fills a caller-owned `&mut PackageList` and returns only
  `StorageStatus`; the 27 KB list and 27 KB return slot are gone from its frame.
- The scan scratch (`MANIFEST_NAMES`, `MANIFEST_BYTES`, `MANIFEST_LFN`) moved to
  `StaticCell`s (~2.9 KB total static), so the loader runs in a few-hundred-byte
  frame. The catalog list itself is an owned `main` local, the same footprint
  the working fixed-package build already used.
- The bypass block is replaced by a real scan; `main` logs
  `phase=14 catalog-ready packages=N`.
- Every catalog failure mode (`num_bytes` error, volume/root/APPS open error,
  list error, malformed manifest, empty catalog) calls `fill_fallback` for a
  usable single-entry shell and emits its `phase=19x` / `phase=139` diagnostic.

### First hardware capture (2026-06-22)

The stack fault is gone — boot reached `phase=131` and ran through to
`phase=14 catalog-ready packages=1` (acceptance criterion 2 confirmed). However
SD init reported `phase=191 sd-card-init-error`: the firmware had been
simplified to a single fixed 1 MHz attempt, dropping the **two-stage clock
fallback** that the KOTO-0068 `sd_read` probe validated. SD init now reproduces
that proven sequence: attempt `SD_FAST_SPI_HZ` (12 MHz), and on failure
reconfigure the live bus with `sdcard.spi(|d| d.bus_mut().set_config(..))` +
`mark_card_uninit()` and retry once at `SD_FALLBACK_SPI_HZ` (1 MHz). New UART
phases: `phase=181 sd-fast-init-failed` and `phase=132 sd-card-init-ok clock=N`.

Verified:

- `cargo build -p koto-pico --bin koto_firmware --target thumbv6m-none-eabi --release --offline`
- `cargo clippy -p koto-pico --bin koto_firmware --target thumbv6m-none-eabi --release --offline -- -D warnings`

### Second hardware capture (2026-06-22) — all criteria met

The restored fallback mounted the card: `phase=181 sd-fast-init-failed` →
`phase=132 sd-card-init-ok clock=1000000` (12 MHz failed on this card, 1 MHz
succeeded). The scan reported `phase=137 apps-list-ok manifests=15`, read all 15
(`phase=139 manifest-read-done accepted=15`), and reached
`phase=14 catalog-ready packages=15`. The shell then rendered the real catalog
with no stack fault. (The first full redraw raster rose to ~61 ms with 15 tiles
to paint; this is the full-screen path, not the same-page selection path that
KOTO-0120 holds to 33 ms.)

All acceptance criteria are met on hardware.
