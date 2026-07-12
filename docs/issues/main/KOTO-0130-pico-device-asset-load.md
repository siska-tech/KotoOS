# KOTO-0130: Pico Device `asset_load` Support

- Status: done
- Type: feature
- Priority: P1
- Requirements: FR-SDK-1

## Goal

Make package image asset loading work on the physical PicoCalc device so that
games that call `asset_load` (e.g. KotoRogue) display their sprites rather than
rendering nothing.

## Context

Follow-up to KOTO-0129, which implemented `draw_pixels_rgb565` on the device.
After KOTO-0129 shipped, KotoRogue was re-tested on hardware. The VM ran without
errors and `draw_pixels_rgb565` was wired up, but the game displayed nothing.
Root cause: KotoRogue calls `asset_load("sprites/kotorogue_tiles.kim", ...)` at
startup to populate its `sheet[9736]` sprite buffer, but `DeviceHost` did not
override `VmHost::asset_load`. The default implementation returns
`HostErrorCode::UNSUPPORTED`, leaving the buffer zeroed. Every `draw_pixels`
call then blitted a 16×16 block of zero pixels (black) onto the dark background
colour `C_BG = 2114` — invisible.

`KotoSim` passes because its host reads `.kim` files from the virtual SD card at
build time. The device host had no equivalent path until this issue.

## Acceptance Criteria

- [x] `DeviceHost::asset_load` is implemented in
  `src/koto-pico/src/firmware/app_host.rs`.
- [x] Paths of the form `"dir/file"` (one subdirectory level) are supported,
  covering `"sprites/kotorogue_tiles.kim"` and all current package assets.
- [x] Long filenames (e.g. `kotorogue_tiles.kim`) are resolved to their FAT 8.3
  short names via `LfnBuffer` + `iterate_dir_lfn`, mirroring the
  KOTO-0121 storage pattern.
- [x] At most `dst.len()` bytes are read; the return value is
  `HostCallOutcome::Ok1(bytes_read as i32)` on success and
  `HostCallOutcome::Err(HostErrorCode::IO_ERROR)` on any failure.
- [x] Bounded UART diagnostics are emitted via a deferred `LineBuffer` field on
  `DeviceHost`, drained by the `app_runtime` frame loop after each
  `step_frame_with`:
  - `phase=158 asset-load-ok bytes=N path=...`
  - `phase=258 asset-load-fail reason=<not-found|dir|volume|root|open|read|no-dir> path=...`
- [x] `cargo build -p koto-pico --bin koto_firmware --target thumbv6m-none-eabi --release` passes.
- [x] KotoRogue displays sprites on physical hardware (confirmed by user).

## Hardware Validation Log

```
phase=150 launch-request app=dev.koto.games.kotorogue
phase=156 app-staged backing=psram code_size=72688
phase=152 app-started
phase=158 asset-load-ok bytes=9736 path=sprites/kotorogue_tiles.kim
phase=155 app-draw frame=1 used=9/384 rect=3 text=4 pixels=2
phase=154 app-heartbeat frame=60 pc=4021 fuel=226
phase=155 app-draw frame=60 used=9/384 rect=3 text=4 pixels=2
```

`bytes=9736` matches `KIM_HDR(8) + 19 tiles × 512 bytes` exactly.
Title screen: 9 draw commands, 226 fuel/frame (idle loop), PC stable at 4021.
Delta mode skips LCD writes on unchanged title screen frames.

## Notes

**Supported paths:** `"dir/file"` only (one level). Root-level paths (`"file"`)
and nested paths (`"a/b/file"`) return `IO_ERROR`. All current package assets
use one-level paths, so this is sufficient.

**SD card layout required:**

```
/sprites/kotorogue_tiles.kim    ← loaded by asset_load("sprites/kotorogue_tiles.kim", ...)
```

The `apps.json` build pipeline already places the compiled `.kim` at this path;
no SD card layout change is needed.

**Stack budget:** `asset_load` allocates `[0u8; MANIFEST_LFN_BYTES]` (192 bytes)
on the stack for the LFN scratch buffer — the same bound used by `storage.rs`.

**Audio assets** (`.kmml`): `asset_load` for audio paths would succeed (the file
read works), but `play_bgm_asset` / `play_sfx_asset` are not implemented on the
device, so audio would remain silent.
