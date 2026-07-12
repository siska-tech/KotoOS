# KOTO-0144: Game2D API Cleanup and Retained-Rendering Author Docs

- Status: todo
- Type: task
- Priority: P2
- Requirements: NFR-RT-2

Source of truth: [GAME2D_RETAINED_RENDER_ARCHITECTURE.md](../../architecture/GAME2D_RETAINED_RENDER_ARCHITECTURE.md) (whole document).

## Goal

Consolidate the Game2D ABI after the retained layers land (KOTO-0139–0143) and document
the retained-rendering model so app authors write retained-state updates — never code-
window boundaries, unrolling, inline bloat, command-list caps, or per-frame redraw.
Architectural cleanup; do last.

## Scope

- **ABI consolidation:** reconcile `RUNTIME_BYTECODE_ABI.md` and `GAME2D_ABI.md` into the
  final retained surface — fixed z-order (static → tile → sprite → text → immediate),
  the `0x14`–`0x1F` Game2D family, and `game2d_present` as the single composite call.
  Retire the KOTO-0135 `Board` stream marker from the docs.
- **Author documentation:** describe the four-layer model and the
  `tile_set` / `stamp_define` / `sprite_set` / `text_set` / `present` workflow; state
  that immediate draw (`draw_rect` / `draw_text` / `draw_pixels_rgb565`) is
  **fallback / debug / overlay / transition only**, not a normal gameplay layer.
- **SDK prelude:** expose the new primitives (`stamp_define`, `sprite_set`, `sprite_hide`,
  `sprite_clear_all`, `text_set`, `text_hide`, `text_clear`) with the existing wrapper
  conventions in `KOTO_SDK.md`.
- **Command-cap reduction:** now that content has migrated off the immediate list, lower
  `MAX_APP_DRAW_COMMANDS` (160 → ~64) to fund the retained sprite/text SRAM — net SRAM-
  positive (`(160−64) × ~80 B × 2 ≈ 15 KiB` reclaimed).
- **Mapping examples:** show KotoBlocks, plus a sketch for KotoRogue / Snake / Mines /
  Shogi / Shell, on the generic primitives (no app-specific host API).

## Non-goals

- New rendering features beyond consolidation (pixel stamps, text v2/cell-grid, call/ret
  remain in their own deferred issues).

## Acceptance criteria

- The retained model is documented for app authors; immediate draw is documented as
  fallback/debug/overlay only.
- The SDK prelude exposes the new primitives.
- `MAX_APP_DRAW_COMMANDS` reduced with the SRAM rationale recorded in `config.rs`.
- `GAME2D_ABI.md` and `RUNTIME_BYTECODE_ABI.md` reflect the final surface with no stale
  `Board`-marker or "immediate is normal" guidance.
