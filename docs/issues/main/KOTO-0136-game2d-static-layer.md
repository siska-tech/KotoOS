# KOTO-0136: Game2D Retained Static/Background Layer

- Status: done
- Type: feature
- Priority: P1
- Requirements: NFR-DRAW-2

## Goal

[KOTO-0135](KOTO-0135-stateful-game2d-host-renderer.md) moved KotoBlocks' locked
board to a host-retained tilemap, removing the "grows with board fill" immediate
draw term. Hardware then showed the frame was **VM-bound**, not draw-bound:
`vm_us ~75-85 ms`, `fuel ~18k-28k`, `hostcalls ~107`, `rect=72 text=14 pixels=16`.
The cause is that KotoBlocks still rebuilds its *entire static screen* — full-page
background, the well frame/fill, the cell grid, the right-side panel boxes, and the
fixed labels — as fresh `draw_rect`/`draw_text` host calls **every frame**, even
though none of it changes during play.

This issue adds a reusable **retained static/background layer** to the Game2D host
ABI so an app builds that chrome **once** and the host composites it every frame,
dropping the per-frame static-UI host calls (and the VM work behind them).

## Chosen design: static *command* layer (not a tile/region layer)

Two options were considered:

- **A. Static command layer** — record a bounded list of draw commands once, replay
  it from retained host state as the stable compositing base.
- **B. Static tile/background layer** — represent the static background as tile cells
  or coarse dirty regions through the existing tilemap mechanism.

**A was chosen.** KotoBlocks' chrome is a heterogeneous mix that does not map onto
16x16 tile cells: a full-screen fill, 32 one-pixel grid lines, navy panel frames,
and CJK text labels. B would need its own rect/text representation anyway, whereas A
reuses the existing `AppDrawCommand` machinery and the same painter verbatim — the
smaller, safer cut the issue asked for. The decisive property: KotoBlocks' chrome
(page/well/grid + right-side panels + labels) and its gameplay (board/piece/ghost/
values/overlays) occupy **disjoint screen regions**, so a "static beneath immediate"
layer reproduces byte-identical output (verified, see below).

### Compositing order

Each frame is composed in this order, in both the device strip/delta painter and the
simulator:

1. **retained static/background layer** (page, well, grid, panel frames, fixed labels)
2. **retained board tilemap** (KOTO-0135) — composited at the `game2d_present` marker
   *within the immediate stream*, so it lands after the static background and before
   the piece/overlays, exactly as before
3. **immediate** commands — active piece, ghost, dynamic score/level/lines values, the
   run-state badge, NEXT/HOLD contents, and all overlays

The app's full-screen page clear now lives in the static layer, so it also supplies
the delta's retained base colour (`full_screen_base_color` checks the static layer
first, then the immediate list).

## Host ABI (minor bump)

Two no-arg calls in the Game2D sub-block, next to the KOTO-0135 tilemap calls:

| ID | Name | Effect |
| :-- | :--- | :----- |
| `0x17` | `game2d_static_begin` | Clear the retained static layer; route subsequent `draw_rect`/`draw_text`/`draw_text_color`/`draw_pixels` into it. |
| `0x18` | `game2d_static_end` | Route draw calls back to the per-frame immediate list. |

`game2d_present`, `game2d_set_tile`, and `game2d_clear_layer` are **not** redirected
by a capture — they always target the board tilemap / immediate stream. IDs
`0x19`-`0x1F` stay reserved for the deferred sprite table and `tile_define` host tile
cache (GAME2D_ABI.md). Wired through the `VmHost` trait (default `UNSUPPORTED`), VM
dispatch, `known_host_call`, `host_call_stack_effect` (`(0, 1)` each), the
koto-compiler intrinsics, and the kbc-asm name table.

## How KotoBlocks uses it

The static layer is built **once**, inline on the title->play transition (using the
existing `cx`/`cy` loop locals — no new VM local slot, so the 43/45 user-slot budget
is unchanged). It is retained across game-over -> retry. The per-frame render block
emits only the board tilemap, piece, ghost, dynamic values, badge, HOLD content, and
overlays. The 56 chrome rects + 9 fixed labels (65 commands) no longer cost a host
call per frame.

## Results

### Simulator (KotoBlocks budget scenario, 81 frames)

| Metric | Before | After |
| :----- | -----: | ----: |
| `host_calls_peak` | (chrome re-emitted every frame) | **109** (the one build frame) |
| immediate `draw_rects_peak` | 119 incl. chrome | **63** |
| `heap_peak` | 4391 | 4391 (no new buffer) |
| user local slots | 43/45 | 43/45 (no new local) |

**Identical-output proof:** the old committed bytecode (which draws all chrome
immediately) rendered on the new simulator is **pixel-for-pixel identical** to the
new bytecode (which uses the static layer) — `ImageChops.difference(...).getbbox()`
is `None` on a mid-gameplay frame. The golden-frame gate (hello-text) is unaffected.

### Device SRAM — and a boot regression that forced a layout change

**First cut (reverted):** the static layer lived inside `DeviceRuntimeHost`, so it
rode the double-buffered `APP_DRAW` pair (current + previous). That **duplicated**
it — ~152 B/slot (≈76 B `AppDrawCommand` × 2 hosts) — for `.bss` +12,184 B and
headroom ~90.4 → ~78.5 KiB. On hardware this **hung boot**: the blue screen never
reached the shell, stopping after `phase=146 battery` (the same boot region as the
KOTO-0134 headroom regression), with manifest/FAT scanning otherwise smooth — i.e.
SRAM/layout, not SD.

**Fix:** the static layer is retained app-session state, not a positional-diff
target, so it needs no previous copy. It now lives in its own single `APP_STATIC`
StaticCell (`AppStaticLayer`), out of the `APP_DRAW` pair. A rebuild is signalled by
an explicit `rebuilt` flag (`game2d_static_begin` sets it; the presenter takes one
full repaint on it; `clear_frame` resets it each frame) instead of a previous-vs-
current diff.

Measured (release, `thumbv6m-none-eabi`, llvm-size / llvm-nm):

| Symbol / section | First cut (doubled) | Single instance (shipped) |
| :--- | --: | --: |
| `APP_DRAW` | 38,108 B | **25,932 B** (= pre-KOTO-0136) |
| `APP_STATIC` | — (inside APP_DRAW) | **6,092 B** (cap 80) |
| `.bss` | 189,936 B | **183,896 B** |
| stack headroom (264 KiB − .bss) | ~78.5 KiB | **~84.4 KiB** |

So the static layer costs a single ~6.0 KiB `APP_STATIC` (vs ~12 KiB doubled), and
`APP_DRAW` returns to its pre-KOTO-0136 size. `GAME2D_STATIC_CMD_CAP` stays 80 (the
cap is not the fix; the layout is). `CODE_WINDOW_BYTES`, the PSRAM code cache policy,
and `MAX_APP_DRAW_COMMANDS=160` are all unchanged; the KOTO-0131 2-tile code cache is
not reintroduced.

**Boot triage markers:** post-`phase=146 battery`, the binary now logs
`phase=21 shell-render-start`, `phase=22 shell-render-ok shell-present-ok` (around
the first `paint_shell`, which fuses compose+transfer per strip), and
`phase=23 shell-loop-enter` (before the main loop), so any remaining post-battery
hang can be localized over UART to the shell paint vs the loop entry.

### Expected on-device effect (to confirm on hardware)

Static UI no longer reaches the VM each frame, so the per-frame immediate host calls
should drop sharply from ~107, with `rect` falling from 72 toward the ghost/overlay
remainder, `text` from 14 to the dynamic values + badge, `pixels` unchanged at 16,
and `fuel`/`vm_us` falling with the removed host-call and formatting work. New
`phase=160` fields aid the read: `static_cmds=` (retained layer size, built once then
free) and `static_rebuilt=` (the one full-repaint frame). No draw overflow and no
right-side flicker are expected — the static layer composites atomically beneath the
delta, which still escalates to a clean full repaint only on a real change.

## Code-window refill regression (follow-up, in triage)

After the layout fix booted, hardware showed KOTO-0136 **cut draw work but slowed the
VM**:

| gameplay frame | after KOTO-0135 | after KOTO-0136 |
| :-- | --: | --: |
| `hostcalls` | ≈107 | ≈42 |
| `rect` / `text` / `pixels` | 72 / 14 / 16 | 16 / 5 / 16 |
| `static_cmds` / `static_rebuilt` | — | 65 / 0 |
| `refills` | ≈8 | **≈39** |
| `vm_us` | ≈70–85 ms | **≈245–263 ms** |

So the immediate draw calls dropped as intended, but the PSRAM code-window refills
per frame rose ~8 → ~39, and that thrash — not the draw workload — now dominates
`vm_us`. This is the KOTO-0134 `CODE_WINDOW_BYTES`=8 KiB tiling cost (KotoBlocks' code
spans several 8 KiB tiles; `main`'s hot loop calls the `shape`/`pmid`/`blit_piece`
helpers in tile 0, refilling on each call/return). Removing the ~65 static draw calls
from `main`'s per-frame render block changed `main`'s code size/offsets and which
tiles the per-frame hot path spans, so the leading hypothesis is that the hot loop
now straddles a tile boundary it previously fit inside — a **code-layout** regression,
not a workload one.

**Diagnostics added (this commit) before any layout/window change:** the
`PsramCodeWindow` now tracks, per frame, a refill **histogram by tile index** and a
bounded table of the top tile→tile **transitions**, exposed via new `CodeSource`
methods (`tile_refills`, `tile_transitions`) and emitted on a new throttled UART line
alongside `phase=160`:

```
phase=163 cw frame=60 refills=39 code_tiles=8 cw_hist=0:18,3:18,4:3 cw_trans=0>3:18,3>0:17
```

(`cw_hist=tile:count,…`; `cw_trans=from>to:count,…`.) This classifies the thrash:

- **Concentrated** on 2–3 buckets with a dominant `cw_trans` pair ⇒ a hot-path tile
  ping-pong ⇒ fix by **bytecode/function layout** first (order KotoBlocks' hot
  gameplay/render code and its helpers so the loop and its callees share a tile),
  target `refills` back near ≤8.
- **Spread** evenly across many buckets ⇒ a many-tile walk ⇒ reduce per-frame VM work
  further or consider a *safe* victim cache.

Acceptance for the diagnostics: compare a title-ish frame (`refills≈4`) against a
gameplay frame (`refills≈39`) and report `cw_hist`/`cw_trans` for both. Per the
constraints, `CODE_WINDOW_BYTES` is unchanged, the 2-tile cache is not reintroduced,
and no layout change is made until the histogram data is in hand.

## What remains immediate after this step

The active piece, ghost, NEXT/HOLD piece previews, the run-state badge, the dynamic
SCORE/LEVEL/LINES values, the game-over flat board + sweep, and the pause / "4 LINE!"
/ "LEVEL UP!" / score-popup overlays. Deferred to follow-ups (KOTO-0135 §Deferred):
the sprite table for piece/ghost/previews, the host tile cache (`tile_define`), a
second tilemap layer, and shrinking the immediate `APP_DRAW` lists to reclaim SRAM.
