# KOTO-0208: KotoUI allocation-free foundation

- Status: done
- Type: feature
- Priority: P1
- Requirements: FR-SDK-1, NFR-DRAW-1, NFR-MEM-2, NFR-PORT-1
- Related: KOTO-0005, GFX-0001, GFX-0003
- Roadmap: [KotoUI GUI Component Roadmap](../../planning/KOTOUI_ROADMAP.md)

## Goal

Create the `no_std`, allocation-free `koto-ui` crate and its geometry, identity,
theme, painter, response, and dirty-region contracts so components can remain
independent of KotoCore, KotoGFX backends, fonts, and hardware.

## Acceptance Criteria

- [x] Add `src/koto-ui` to the workspace as a `no_std` crate with no default
  allocator or hardware dependency.
- [x] Define stable component identity, signed/clipped rectangles, interaction
  state, semantic response, and theme/style tokens needed by the roadmap.
- [x] Define a painter trait for clipped fills, borders, text/glyph runs, and
  focus marks without exposing framebuffer or LCD ownership.
- [x] Implement a fixed-capacity damage set that clips to a supplied surface and
  falls back to one documented region when capacity is exceeded.
- [x] Visual state changes report damage for the union of old and new bounds;
  unchanged state reports no damage.
- [x] Public types document ownership, coordinate, clipping, and overflow
  behavior and do not depend on `koto-core`.
- [x] Unit tests cover rectangle overflow, clipping, empty intersections,
  damage deduplication/coalescing, capacity fallback, and unchanged frames.
- [x] Record `size_of` measurements for the core context, theme, and damage set
  and document the selected fixed capacities.
- [x] Workspace tests, `cargo check -p koto-ui`, and
  `python harness/check_project.py` pass.

## Notes

Absolute rectangles and a flat caller-owned collection are deliberate. Layout
engines, recursive owned trees, input routing, and concrete components belong to
later issues.

## Validation Notes

- `cargo test`, `cargo test -p koto-ui`, `cargo check -p koto-ui`,
  `cargo clippy -p koto-ui --all-targets -- -D warnings`, and
  `cargo check -p koto-ui --target thumbv6m-none-eabi` pass on 2026-07-15.
- Requirement/link/issue-index checks pass.
- `python harness/check_all.py` passes after synchronizing the committed KPA
  packages on 2026-07-15.
