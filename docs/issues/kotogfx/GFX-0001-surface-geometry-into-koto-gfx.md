# GFX-0001: App-surface geometry into koto-gfx (first safe move)

- Status: done
- Type: refactor
- Priority: P1
- Requirements: NFR-PORT-1, NFR-DRAW-1

Source of truth: [KOTOGFX_RENDER_MIGRATION_PLAN.md](../../architecture/KOTOGFX_RENDER_MIGRATION_PLAN.md)
§4 Stage 1 + §5 (first safe code-moving step), [KOTOGFX_CRATE_BOUNDARY.md](../../architecture/KOTOGFX_CRATE_BOUNDARY.md).

## Goal

Move the pure app-surface geometry (rect clipping and union-clip) out of the
firmware `app_render.rs` and into `koto-gfx` as surface-dimension-parameterised
`Rect` helpers, so the panel size is no longer hardcoded in the compositor path.
This is the first, lowest-risk step of the rendering realignment and unblocks
GFX-0002…0004.

## Acceptance Criteria

- [x] `koto-gfx` gains `Rect::clip(x, y, w, h, surf_w, surf_h) -> Option<Rect>`
      and `Rect::union_clipped(a, b, surf_w, surf_h) -> Option<Rect>`, with unit
      tests (off-screen, partial overlap, zero area, saturating bounds, surface
      dims ≠ 320).
- [x] Firmware `clip_app_rect` / `union_rect` keep **identical signatures** and
      become one-line delegations passing `320, 320` — no call site changes.
- [x] `cargo test -p koto-gfx` passes (55 tests). `cargo test -p koto-pico` cannot
      compile on the host (embassy-rp cortex-m asm), so the four `app_render`
      invariants are exercised through `cargo test -p koto-sim` (13/13, incl.
      golden frames + budget) and the thumbv6m firmware build.
- [x] Hardware-confirmed: release firmware (`--release --features
      psram_fast_code_window`) flashed to the PicoCalc and runs correctly.
- [x] `cargo build -p koto-pico --target thumbv6m-none-eabi --bins` builds.
- [x] Golden-frame parity (KotoBlocks / KotoSnake / Sokoban) unchanged
      (`cargo test -p koto-sim` fixture_runner).

## Notes

Pure integer geometry: no `Canvas`, no host state, no `embassy_time`, no LCD — it
cannot change a pixel or a microsecond. `koto-gfx` stays `no_std`, heap-free,
dependency-free (only `Rect`).

Hard constraints (apply to every GFX issue): no change to VM semantics, opcode
values, bytecode ABI, hostcall IDs, PSRAM/LCD/CodeWindow/audio behaviour, or
KotoBlocks/KotoSnake/Sokoban behaviour; no app bytecode rebuild.
