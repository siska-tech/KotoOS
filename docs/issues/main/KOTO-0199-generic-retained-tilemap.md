# KOTO-0199: Generic 20x20 retained tilemap geometry

- Status: done
- Type: feature
- Priority: P1
- Requirements: NFR-RT-2
- Related: KOTO-0135, KOTO-0143, KOTO-0144, KOTO-0198

## Goal

Replace the KotoBlocks-specific 10x20 retained board with an allocation-free
tilemap that has a 20x20 maximum capacity and app-configurable active dimensions
and pixel origin, while preserving existing KBC behavior and device/simulator
rendering parity.

## Acceptance criteria

- [x] KotoGFX owns a `Game2dTilemap` with fixed `[i32; 400]` storage, active
  `columns`/`rows`, and signed `origin_x`/`origin_y` metadata.
- [x] Active dimensions accept every size from 1x1 through 20x20 and reject zero
  or dimensions greater than 20 with `BAD_ARGUMENT`.
- [x] Origins accept the full signed 16-bit pixel range and reject values outside
  it; drawing remains clipped to the app surface.
- [x] `game2d_configure_tilemap(layer, columns, rows, origin_x, origin_y)` is
  exposed as host call `0x22`, has a verified five-argument stack effect, clears
  the layer on success, and currently accepts only layer 0.
- [x] Unconfigured legacy KBCs retain the previous 10x20 geometry at origin
  `(8, 0)`.
- [x] Cell storage uses the fixed maximum-width stride, while bounds checks,
  painting, and dirty-band coalescing use the configured active dimensions.
- [x] A geometry/origin change damages both the previous and current clipped
  tilemap bounds so moved or shrunk layers cannot leave stale pixels.
- [x] The VM, assembler, compiler intrinsic, KotoGame2D semantic API, simulator,
  KotoGFX compositor, and Pico firmware all implement the same contract.
- [x] KotoBlocks explicitly configures its existing 10x20 `(8, 0)` geometry and
  its retained-layer fixture still reaches `set_tile` and `present` without a
  trap.
- [x] The 20x20 storage increase is documented as 1.6 KiB per retained frame and
  3.2 KiB across the Pico current/previous pair.
- [x] `cargo check -p koto-pico --target thumbv6m-none-eabi` passes.
- [x] Unit tests cover maximum dimensions, arbitrary/negative origins, invalid
  configuration, VM argument order, simulator placement, and legacy defaults.
- [x] `python harness/build_apps.py --check` and
  `python harness/check_project.py` pass.

## Design notes

- Tile pixels remain fixed at 16x16 little-endian RGB565 (512 bytes). Variable
  tile pixel formats/sizes would also change sprite stamps and asset validation
  and are outside this issue.
- The maximum capacity is compile-time fixed because the RP2040 firmware is
  `no_std` and retains current/previous layer snapshots without heap allocation.
- Only tilemap layer 0 is implemented. The `layer` argument remains in the ABI so
  a future bounded multi-layer design does not require changing existing calls.
- A separate sample issue should connect an authored `app.json` `maps` source to
  this runtime rendering API; KOTO-0199 generalizes the renderer itself.
