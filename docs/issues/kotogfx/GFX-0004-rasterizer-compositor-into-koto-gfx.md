# GFX-0004: Rasterizer + compositor into koto-gfx

- Status: done
- Type: refactor
- Priority: P1
- Requirements: NFR-PORT-1, NFR-MEM-3, FR-PM-1

Source of truth: [KOTOGFX_RENDER_MIGRATION_PLAN.md](../../architecture/KOTOGFX_RENDER_MIGRATION_PLAN.md)
§4 Stage 4, [kotogfx-architecture.md](../../architecture/kotogfx-architecture.md) §gfx core層.

Depends on: [GFX-0003](GFX-0003-dirty-derivation-into-koto-gfx.md).

## Goal

Lift the CPU layer compositor into `koto-gfx`, resolving the `Canvas` dependency
inversion. This is the pixel-parity-critical stage: it relocates the existing
rasterizer; it does **not** build the PSRAM-backed surface or a new compositor.

## Acceptance Criteria

- [x] `Canvas` (+ `Rgb565`) moves from `koto-core` into `koto-gfx` (`raster.rs`),
      re-exported from `koto-core` so `shell_render` and other consumers are
      unchanged. The R14 bitmap-font reader (`BitmapFont`/`Glyph`/`FontError`) moved
      with it (`font.rs`) — `Canvas`'s glyph methods depend on it and `koto-gfx` is
      below `koto-core`, so it had to come down too to resolve the inversion;
      re-exported from `koto_core::font` / `koto_core::{BitmapFont, …}` unchanged.
- [x] `paint_board/sprite/text/command_list` + `paint_app_commands` move into
      `koto-gfx` (`paint.rs`) as the chunk compositor operating on a `koto-gfx`
      `Canvas` + the GFX-0002 POD layer slices + the app heap slice. The firmware
      keeps one identically-signed `paint_app_commands` adapter that unpacks
      `DeviceRuntimeHost`/`AppStaticLayer` into those slices (the GFX-0003 adapter
      methodology); it still owns/passes the strip and the surrounding banding. The
      four sub-passes are deleted from firmware (their only caller was the adapter).
      `GAME2D_TILE_BYTES` moved to `koto-gfx` `layer.rs`, re-exported from `config`.
- [x] Fixed z-order (static → board → sprites → text → immediate) lives in
      `koto_gfx::paint_app_commands` verbatim; `clear-to-base` stays in the firmware
      `present_rect_banded` / `present_app_commands` (`canvas.clear(base)` before the
      compose), unchanged.
- [x] **Byte-level golden-frame parity** green: `cargo test -p koto-sim` 13/13
      `fixture_runner` golden frames (KotoBlocks / KotoSnake / Sokoban) + 94 unit,
      plus the budget-observation tests, all pass unchanged.
- [x] `shell_render` (a `Canvas` consumer) and the rest of the `koto-pico` lib build
      green for `thumbv6m-none-eabi`; `koto-gfx` 80 + `koto-core` 132 tests pass;
      `koto-gfx` + default clippy gate clean; firmware-lib clippy adds no new finding
      (`app_render`/`paint` clean; the pre-existing `probe_keyboard` bin lint error is
      untouched by this change).
- [ ] `phase=160` `refills=` / `code_tiles=` CodeWindow check is **hardware-only**
      and unverified on the authoring host (no device) — same posture as prior
      stages. Diff `phase=160` before/after on a device before treating as closed.

## Notes

Highest-risk stage. Ride the existing `present_rect_banded` "clear to base, paint
clipped" path unchanged. A CodeWindow-refill regression is a blocking finding, not
cosmetic — the firmware code layout is sensitive (KOTO-0156/0159).

`cargo test -p koto-pico` cannot run on the host (embassy-rp is ARM-only — the same
limitation noted since GFX-0001); the four `app_render` invariant tests still
compile into the firmware but can only execute on a device. The `thumbv6m` build is
the host-side gate for the firmware path.
