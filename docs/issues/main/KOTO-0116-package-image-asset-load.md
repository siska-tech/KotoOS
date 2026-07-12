# KOTO-0116: Package image assets — `asset_load` host call and `.kim` sprite pipeline

- Status: done
- Type: feature
- Priority: P1
- Requirements: FR-PM-1, FR-PM-2

## Goal

Let bytecode apps ship pixel-art tiles/sprites as real files inside the `.kpa`
and draw them, instead of composing every sprite from `draw_rect` primitives.
This adds a one-shot `asset_load` host call that copies a manifest-declared,
read-only package asset into the app heap, plus an authoring pipeline that
compiles a reviewable `.kspr` ASCII sprite sheet into a committed `KIM1` RGB565
image asset that the app blits with `draw_pixels`.

## Acceptance Criteria

- [x] `asset_load(path, len, buf, max)` host call (ABI `0x44`, repurposing the
  previously reserved `asset_open`) is dispatched by the VM, copies a
  manifest-declared package asset into the app heap, and returns the byte count.
- [x] `koto-sim` implements `asset_load`: only manifest-declared asset paths are
  readable, the asset is read-only and never touches the save sandbox, and the
  copy is bounded by the destination length.
- [x] `asset_load` SDK prelude wrapper (compiler) and `kbc-asm` name mapping.
- [x] `build_apps.py` `images` block compiles `.kspr` → `KIM1` (`harness`
  converter); `--check` fails on a stale committed `.kim`.
- [x] KotoRogue authors `apps/kotorogue/sprites/tiles.kspr` (19 tiles: 5 static +
  player/6 monsters as 2 animation frames each), ships `sprites/kotorogue_tiles.kim`,
  loads it once at startup, and blits floor/wall/stairs/gold/potion/player/monster
  tiles via `draw_pixels` within budget. The animation frame is selected from the
  app's `ST_ANIM` counter so entities stay lively (replacing the per-frame
  `draw_rect` sprite animation that the static-tile first cut had dropped).
- [x] Tests: koto-core VM round-trip (path read + heap write surfaced through
  `draw_pixels`), koto-sim declared-vs-undeclared asset access, compiler codegen.
- [x] Docs: `KOTO_SDK.md` (Package Assets), `RUNTIME_BYTECODE_ABI.md` (`0x44`),
  `ASSET_PIPELINE.md` (`.kspr`/`KIM1`). `python harness/build_apps.py --check`
  and golden frames remain clean.

## Notes

- One-shot `asset_load` (vs an open/read/close handle trio) was chosen for app
  ergonomics: a tile sheet loads in a single call into a fixed heap buffer. The
  reserved `asset_open` slot (which had no implementation, format, or converter)
  is repurposed rather than left dangling.
- `no_std` runtime constraint: the dispatch copies the path out of the heap into
  a fixed stack buffer (`MAX_ASSET_PATH_LEN`) before taking the mutable
  destination borrow, so the path read and the heap write do not overlap and no
  allocator is required.
- `KIM1` = `"KIM1"` magic, `u16` width/height (little-endian), then row-major
  little-endian RGB565. `draw_pixels` is opaque, so entity/item tiles bake the
  floor colour as their background and are only drawn on lit floor cells; the
  fog-of-war dim tier stays a flat `draw_rect` fill (cheap over large explored
  areas). See [[koto-tilemap-build-pipeline]].
- Reused KotoRogue's existing `draw_rect` sprite geometry as the source of truth
  for the `.kspr` art, so the image tiles match the earlier procedural look.
  Effects (impact spark, hurt shake/vignette, torch flicker) stay `draw_rect`
  overlays composited over the blitted tiles.
