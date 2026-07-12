# KotoSnake length-aware render budget

The retained static/text migration
([KOTO_KOTOSNAKE_RETAINED_RENDER.md](KOTO_KOTOSNAKE_RETAINED_RENDER.md)) moved
KotoSnake's fixed chrome and HUD numbers off the per-frame immediate path and roughly
doubled the snake-length headroom. But the snake body itself stayed immediate (it is
per-frame animated — flowing rainbow + smooth sub-cell motion), so its cost still grew
~2 rects per segment. On a *very* long snake that re-overran the device's 96-command
immediate draw budget (`MAX_APP_DRAW_COMMANDS`):

```
frame=1560: used=96/96 rect=93 overflow full_reason=AreaExceeded
frame=1590: rect=84 full_reason=RectsExceeded
```

Once the immediate list hits the cap the device drops commands and the delta presenter
escalates to a full-screen repaint — exactly the slowness the migration was undoing.

## The fix: a constant-bounded snake body

The snake is rendered in two zones, by distance from the head:

- **Near-head zone — full identity, unchanged.** The head plus the next `RICH_N-1`
  body cells (`RICH_N = 12`) render exactly as before: a flowing-rainbow 14×14 block,
  the glossy white glint, the head's chrome outline + heading eyes, and the smooth
  sub-cell slide between cells. This is where the eye tracks the snake, so it keeps the
  full look.
- **Tail zone — coalesced runs.** A snake's body is a path of adjacent cells, so a
  stretch heading the same way is a straight *run* that draws as **one** rect instead of
  one-per-cell. Walking head→tail, contiguous collinear cells are merged into a bounding
  run; a turn (or a board-edge wrap) flushes the run as a single rainbow rect and starts
  the next. An adjacency check (the new cell must be exactly one step past a run end)
  keeps a wrap — same row/column but on the opposite edge — from merging a whole row.

A per-frame run-rect budget, `TAIL_BUDGET = 16`, caps the tail. So the snake's whole
immediate cost is a **constant**, independent of length:

```
head 8 + (RICH_N-1)*2 + TAIL_BUDGET  =  8 + 22 + 16  =  46 rects
```

Adding the other immediate elements — the logo (1), the apple (≤4), the 24-spark
particle pool, and the transient eat-flash/popup/banner overlays (≤~13) — the
worst-case immediate frame is ≈ 89 commands, comfortably under 96. The structural cap
is what guarantees no overflow: however long or coiled the snake, the body cannot
exceed 46 immediate rects.

## Tuning the constants

`RICH_N` and `TAIL_BUDGET` (top of `apps/kotosnake/src/main.koto`) trade visual
fidelity against headroom. Raising `RICH_N` keeps more of the snake fully smooth/glinted
but costs 2 rects per added segment; raising `TAIL_BUDGET` lets a more sharply
zig-zagging tail draw every run before clipping. Their sum bounds the body, so keep
`8 + (RICH_N-1)*2 + TAIL_BUDGET` under ~50 to leave room for particles + overlays under
the 96 cap. (Note: KotoSnake's `main` is at the 45 user-local-slot ceiling, so the tail
pass reuses dead render scratch — `px`/`py`/`intent` — for the run bounding box and adds
only two locals, `ry1` and `rbud`.)

## Visible result

- **len ≤ 12:** identical to before — the whole snake is the rich near-head zone.
- **len > 12:** the near-head 12 cells still flow smoothly with the glint and eyes; the
  older tail is a continuous rainbow ribbon (one hue per straight run, cell-aligned, no
  sub-cell slide). The transition sits ~12 cells back from the head, away from the
  action.
- **Remaining case (documented):** if a pathologically zig-zagging tail produces more
  than `TAIL_BUDGET` runs, the oldest (last-walked) runs are dropped — the snake reads a
  touch shorter for those frames — rather than overflowing APP_DRAW. A diagonal
  staircase is the only shape that reaches this, and it is hard to sustain in real play;
  a normally coiled/straight long snake stays well within the budget.

## Verification

`cargo test -p koto-sim --test fixture_runner kotosnake_long_snake_stays_under_app_draw_budget`
plays KotoSnake with a built-in greedy apple-seeker (length and apple position are
app-local, so a long snake is only reachable by real play). It reads the head from the
`occ`-grid heap delta and the apple from the rendered `C_FOOD` rect, grows the snake
past `RICH_N`, and asserts that (1) multi-cell coalesced run rects were actually drawn
(the tail path ran) and (2) **no** frame's immediate draw-command count reached 96. The
draw count masks the retained static-layer capture window, mirroring how the device
routes that chrome to the static layer rather than APP_DRAW.
