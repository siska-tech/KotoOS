# KOTO-0143: Full-Repaint Reason Codes and Tile Dirty-Cell Coalescing

- Status: in-progress
- Type: feature
- Priority: P1
- Requirements: NFR-RT-2

Source of truth: [GAME2D_RETAINED_RENDER_ARCHITECTURE.md](../../architecture/GAME2D_RETAINED_RENDER_ARCHITECTURE.md) §6.

## Goal

Make "normal gameplay never full-repaints unless explicitly invalidated" a measured,
enforced property. Instrument *why* a full repaint happens, and coalesce tile dirty
cells so a line clear stays incremental.

## Causes of `full=1` today (`app_render::present_app_delta`)

1. `full_screen_base_color` changed (base appears / disappears / recolors).
2. `static_layer.rebuilt` (entering gameplay or a layout change) — one-shot.
3. `dirty_area >= ¾` screen.
4. `dirty_rects > 24`.
5. **Positional command-diff misalignment:** when `current.len != previous.len`, the
   `command[i]`-vs-`command[i]` compare misaligns and every later index becomes a
   spurious dirty rect, escalating into (3)/(4).

(5) is removed by KOTO-0140/0141 emptying the immediate list (sprites/text/tiles diff by
id/cell, not array position). This issue covers the instrumentation and the remaining
line-clear escalation.

## Scope

- **Reason codes:** add a full-repaint reason to the UART metric — `BaseChange`,
  `StaticRebuild`, `AreaExceeded`, `RectsExceeded`, `CommandCountShift` — so any full
  repaint is attributable.
- **Tile dirty-cell coalescing:** merge horizontally/vertically adjacent dirty tile cells
  into bounding band rects (a cleared 160-px row → one `160×16` rect; a 4-line clear →
  ~4 bands, not 40 individual cells that would trip the 24-rect cap).

## Dependencies

The reason-code instrument can land early to guide KOTO-0140. Coalescing is fully
effective once KOTO-0140 has emptied the immediate list.

## Acceptance criteria

- A full KotoBlocks game (spawn → move → rotate → hold → lock → single- and multi-line
  clear → game over) logs **zero `full=1` frames** except the one-shot `StaticRebuild` on
  title→play.
- Every full repaint (any app) carries a reason code.
- A 4-line clear stays incremental (≤ a handful of coalesced band rects, no escalation).
- Immediate draw is excluded from the normal gameplay path (debug/overlay/transition
  only).

## Implementation

Reason codes (`src/koto-pico/src/firmware/`):

- `diag.rs` adds `FullRepaintReason` (`BaseChange`, `StaticRebuild`, `AreaExceeded`,
  `RectsExceeded`, `CommandCountShift`). `PaintMetrics::mark_full_repaint(reason)` latches
  it first-wins alongside `full_repaint`; `log_app_frame_metrics` emits `full_reason=` on
  the `phase=160 app-frame` line (`none` when incremental). Because the only path that sets
  `full=1` now requires a reason, every `full=1` line carries exactly one.
- `app_render.rs` attributes each escalation by a fixed, documented priority: a full-screen
  base change → `BaseChange` and a static-layer rebuild → `StaticRebuild` (both early
  returns, highest priority); otherwise, when the dirty diff escalates, an immediate
  command-list length shift → `CommandCountShift` (the root cause that misaligns the
  positional diff and inflates the counts), else `AreaExceeded` (a genuinely large changed
  surface) outranks `RectsExceeded` (fragmentation / band-buffer overflow). The first paint
  of a session is attributed to `StaticRebuild`. Instrumentation is record-only — it does
  not change which frames repaint.

Tile dirty-cell coalescing:

- `koto-core/src/dirty_tiles.rs` adds `TileBand` + `coalesce_dirty_tiles` (a pure,
  host-tested helper): row-major maximal horizontal runs, vertically merging runs with an
  identical `(col, w)` directly above. A cleared 10-cell row → one `10x1` band (160×16 px);
  a contiguous 4-line collapse → one stacked band; separated changes stay distinct; a
  buffer too small returns `None` so the caller can fall back to a full repaint without
  dropping a band. Nine unit tests cover these cases plus exact coverage / no-overlap on a
  checkerboard.
- `app_render.rs::present_app_delta` coalesces the board's changed cells once (into a
  `BOARD_BAND_CAP`-sized buffer) and uses the bands for both the dirty-size pass and the
  transfer pass, replacing the per-cell `board_cell_rect` loops with `board_band_rect`. A
  band taller than one raster strip is banded via the existing `present_rect_banded`. Each
  band recomposites the full layer stack clipped to its bounding rect, so pixel output is
  identical to per-cell repaints.

Verification: `fmt`, gated `clippy`, `cargo test` (incl. the 9 new coalescing tests),
`build_apps --check`, golden frames, budgets, project harness, and the thumbv6m firmware
build all pass. Device UART verification of zero `full=1` frames in normal KotoBlocks
gameplay (except the title→play `StaticRebuild`) is the remaining step.

---

Device verification found remaining full repaints caused by `CommandCountShift`.

The unexpected frames are not piece blits: the logs show `rect=0 text=5 pixels=0`,
which suggests the remaining positional-diff instability is in immediate text/UI
commands.

Rather than patching KotoBlocks to stabilize immediate text command counts, we will
proceed with KOTO-0141 Retained Text Layer first, then re-run KOTO-0143 acceptance
with normal gameplay expected to have an empty or near-empty immediate list.