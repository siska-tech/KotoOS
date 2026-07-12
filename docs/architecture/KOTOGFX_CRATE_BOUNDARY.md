# KotoGFX crate boundary (v0)

`koto-gfx` is the first extraction of KotoOS's retained-rendering concepts into a
dedicated, hardware-independent foundation crate. **This is v0: a
behaviour-preserving move of pure data structures and policy only.** It is *not*
the final KotoGFX — there is no compositor, no PSRAM-backed surface, and no
retained Tile/Sprite/Text/Effect layer here yet. The live firmware rendering
path (`koto-pico`'s `present_app_delta`, `paint_app_commands`, the Game2D layer
composition, LCD/PSRAM/CodeWindow code) is unchanged and still owns all of that.

Everything moved into `koto-gfx` was lifted verbatim from its previous home and
is re-exported from there, so no rendering behaviour, VM ABI, hostcall ID,
bytecode, PSRAM, LCD, CodeWindow, or Game2D semantics changed.

## koto-gfx owns (v0)

- `Rect` — the axis-aligned rectangle type (was `koto_core::hal::Rect`) plus its
  pure helpers `Rect::area` / `Rect::bbox` (were the private `rect_area` /
  `rect_bbox` in `koto_core::dirty_tiles`).
- Dirty coalescing — `TileBand`, `coalesce_dirty_tiles`, `coalesce_rects` (were
  `koto_core::dirty_tiles`).
- Full-repaint policy — `FullRepaintReason` (was `koto_pico::firmware::diag`),
  the thresholds `FULL_REPAINT_AREA` / `FULL_REPAINT_RECTS` and the
  escalation/attribution branch (were inline constants + logic in
  `app_render::present_app_delta`), exposed as
  `FullRepaintPolicy::decide(DeltaInputs) -> DeltaDecision`.
- Dirty-rect diagnostics model — `DirtyRectGeometry`, `DIRTY_SAMPLE_QUADS`, and
  the `DirtyRectGeometry::from_rects` summarizer (were
  `koto_pico::firmware::diag` + the private `dirty_geometry` in `app_render`).

## KotoOS / koto-pico still owns

- `present_app_delta`, `present_app_commands`, `present_rect_banded` and the
  per-frame layer composition (static → board → sprites → text → commands).
- `PaintMetrics` (it times raster/transfer with `embassy_time::Instant`, so it
  is hardware-coupled and stays in `diag`; it now holds a
  `koto_gfx::DirtyRectGeometry`).
- LCD/PSRAM/CodeWindow drivers, Game2D host state, hostcall implementations.

  > **Stage 1 update (GFX-0001).** App-surface clipping has moved into
  > `koto-gfx` as surface-parameterised `Rect::clip(x, y, w, h, surf_w, surf_h)`
  > and `Rect::union_clipped(a, b, surf_w, surf_h)`. Firmware `clip_app_rect` /
  > `union_rect` are now one-line delegations passing `320, 320`; their
  > signatures and call sites are unchanged, so no pixels or timing change.
  >
  > **Stage 2 update (GFX-0002).** The retained-layer *data model* (the POD
  > layout) has moved into `koto-gfx` (`layer.rs`): `AppDrawCommand`,
  > `Game2dSprite`, `Game2dStampDef`, `Game2dText`, `Game2dBoard`, `AppStaticLayer`,
  > plus the capacity constants (`MAX_APP_TEXT_BYTES`, `GAME2D_TEXT_BYTES`,
  > `GAME2D_STATIC_CMD_CAP`, board dims). The firmware still owns the *instances*
  > (`DeviceRuntimeHost` + its diff double-buffer) and the VM hostcall methods;
  > `app_host.rs` / `config.rs` re-export the moved items so field bytes, hostcall
  > IDs, and call sites are unchanged. `AppStaticLayer::push` split into a pure
  > `try_push -> Result<(), LayerFull>` (koto-gfx) with the `NO_MEMORY` mapping
  > kept at the firmware dispatch site.
  >
  > **Stage 3 update (GFX-0003).** Dirty-region *derivation* has moved into
  > `koto-gfx` (`derive.rs`): the per-command/sprite/text footprints and dirty
  > unions, `board_band_rect`, `stamp_cell`, and `push_dirty` — pure geometry over
  > the POD types + app heap, surface-parameterised like `Rect::clip`. The
  > board-placement constants (`GAME2D_ORIGIN_X/Y`, `GAME2D_TILE_PX`) moved too.
  > `present_app_delta` is unchanged: its collect loops call identically-signed
  > firmware adapters that delegate to koto-gfx. `FullRepaintPolicy` (the
  > *decision*) was already koto-gfx and is untouched; only the *collection* moved.
  >
  > **Stage 5 update (GFX-0005).** The firmware present is now routed through a
  > **display-service seam** (`firmware/display_service.rs`,
  > `DisplayService::present`) instead of the frame loop inlining the compose/flush.
  > The frame loop builds a `PresentRequest` and calls the service; it keeps only the
  > present *trigger* (`*host.draw != *previous_draw || static_rebuilt` + the
  > `previous_draw` copy-back). The service runs the resource-ownership §4 flow —
  > receive request → collect → **no-op overlay/status-bar hook** → decide
  > (`FullRepaintPolicy`) → coalesce → compose (koto-gfx) → flush (HAL) — but the
  > collect→…→flush body is **unchanged**: it still lives in `present_app_delta` /
  > `present_app_commands`, which the service calls, so this is byte-equivalent (no
  > pixel or timing change). `PaintMetrics` and the `phase=160/164/161` logs stay in
  > firmware and are threaded through unchanged. A **real** overlay / status bar /
  > display takeover / capture / async present queue / surface registry, and any
  > CPU0/CPU1 re-homing, are explicitly **future work** — only the seam (and its empty
  > overlay hook) is created here, so present is no longer a straight shot to the LCD.

## Re-exports preserving existing paths

- `koto_core::Rect`, `koto_core::hal::Rect` → `koto_gfx::Rect`.
- `koto_core::{coalesce_dirty_tiles, coalesce_rects, TileBand}` and
  `koto_core::dirty_tiles::*` → `koto_gfx::*`.
- `koto_pico::firmware::diag::{FullRepaintReason, DirtyRectGeometry,
  DIRTY_SAMPLE_QUADS}` → `koto_gfx::*`.

## Compatibility rules

- Keep `koto-gfx` `no_std`-compatible (std only under `cfg(test)`) and
  heap-free; it must build for `thumbv6m-none-eabi`.
- Keep `koto-gfx` dependency-free. It is the lowest layer:
  `koto-gfx` ← `koto-core` ← `koto-pico` / `koto-sim`.
- Do not change rendering behaviour, the VM ABI, hostcall IDs, `.kbc` format,
  PSRAM/LCD/CodeWindow code, or Game2D semantics in KotoGFX-extraction work.
- Any future move that would change pixels or timing is out of scope for v0:
  stop and report it instead of implementing it.

## Not in v0 (future KotoGFX direction)

- A retained surface / compositor (PSRAM-backed).
- Retained Tile / Sprite / Text / Effect layers owned by the crate.
- Owning the present path and the per-frame layer composition.
