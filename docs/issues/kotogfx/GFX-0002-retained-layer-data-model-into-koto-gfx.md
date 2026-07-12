# GFX-0002: Retained layer data model into koto-gfx

- Status: done
- Type: refactor
- Priority: P1
- Requirements: NFR-PORT-1, FR-PM-1, NFR-MEM-3

Source of truth: [KOTOGFX_RENDER_MIGRATION_PLAN.md](../../architecture/KOTOGFX_RENDER_MIGRATION_PLAN.md)
§4 Stage 2 (split R5), [GAME2D_RETAINED_RENDER_ARCHITECTURE.md](../../architecture/GAME2D_RETAINED_RENDER_ARCHITECTURE.md).

Depends on: [GFX-0001](GFX-0001-surface-geometry-into-koto-gfx.md).

## Goal

Split `DeviceRuntimeHost`'s two commingled concerns: move the **retained layer
data model** (the POD layout) into `koto-gfx`, while the **VM hostcall landing
pad** (dispatch + field lifetime) stays in firmware `app_host.rs`. This is the
structural step that stops the firmware from owning the graphics state *layout*.

## Acceptance Criteria

- [x] `koto-gfx` owns the POD types: `AppDrawCommand`, `Game2dSprite`,
      `Game2dText`, `Game2dStampDef`, the board cell array shape (`Game2dBoard`),
      `AppStaticLayer` — in `koto-gfx/src/layer.rs`, with the capacity constants
      they need (`MAX_APP_TEXT_BYTES`, `GAME2D_TEXT_BYTES`, `GAME2D_STATIC_CMD_CAP`,
      board dims).
- [x] `app_host.rs` re-exports them and keeps the hostcall methods and the
      *instances* (`DeviceRuntimeHost` + its diff double-buffer stay
      firmware-owned). `config.rs` re-exports the moved constants (single source).
- [x] Hostcall IDs, dispatch, and the field bytes are byte-for-byte unchanged.
      `AppStaticLayer::push` (returned a koto-core `HostCallOutcome`) split into a
      pure `try_push -> Result<(), LayerFull>` in koto-gfx, mapped to `NO_MEMORY`
      at the one dispatch site — same behaviour.
- [x] `size_of` / `align_of` assertions in `layer.rs` pin the layout (stamp 8,
      sprite 12, text 40, command 76, board 800 B; static layer relative to its
      parts) — matching the pre-move SRAM rationale in `config.rs`.
- [x] `cargo build -p koto-pico --target thumbv6m-none-eabi --bins` builds; all
      consumers compile via the re-export. koto-gfx clippy-clean (default gate).
- [x] Golden frames unchanged (`cargo test -p koto-sim` 13/13 + 94 unit); SRAM
      footprint unchanged (types moved, not instances; `DeviceRuntimeHost` fields
      identical).

## Notes

Per the SRAM-doubling constraint, this stage must not add fields to the host
(every field doubles across the current/previous diff buffers). It only relocates
struct definitions. Keep single-instance retained state (static layer, stamp
defs) out of the doubled host, as today.
