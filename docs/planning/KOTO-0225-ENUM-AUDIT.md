# KOTO-0225 checked-in Koto integer-domain audit

This audit classifies every checked-in `apps/**/*.koto` family considered for
the first enum migration. It is intentionally semantic rather than a prefix-only
mechanical rewrite.

## Accepted domains

- KotoUI Gallery: locale resource selection, mount-record identities, widget
  identities, localized status identities, and semantic `UiResponse` kinds.
  Migrated to `GalleryLocale`, `GalleryMountRecord`, `GalleryWidget`,
  `GalleryStatus`, and the compiler-backed `UiResponse` domain. Repeated
  packet/resource capacities use app-local constants. SDK-wide sentinels such
  as `UI_PARENT_ROOT` remain flat because each is a distinguished value rather
  than an integer domain.
- KotoRun: title/play/game-over state machine. Migrated to `RunState`; this is
  the required non-UI proving app.
- Memo: interaction and dialog modes are valid follow-up domains.
- KotoBlocks, KotoMines, KotoSnake, KotoRogue, and Sokoban: game states,
  directions, hazard/message kinds, and tile/cell kinds are valid follow-ups.
  KotoRogue's `ST_*` values are buffer offsets, not a state enum.
- Retained sprite/text slot IDs are eligible only when used solely as stable
  identities. They remain deferred because several apps intentionally rely on
  ordering and arithmetic adjacency for draw-command stability.

## Rejected families

- `C_*` RGB565 colors, screen/geometry dimensions, capacities, ABI versions,
  byte sizes, buffer-layout offsets, counters, and raw array indices.
- `UI_FLAG_*`, `INTENT_*`, and other composable bit masks.
- Tile byte offsets and sprite-sheet image indices used arithmetically (for
  example `IMG_MON0 + type * 2 + frame`).
- Isolated numeric constants without a mutually exclusive domain.

The deferred accepted groups should be migrated app-by-app with scenario,
golden-frame, opcode/rodata, and budget comparisons so enum readability changes
do not conceal layout or performance regressions.
