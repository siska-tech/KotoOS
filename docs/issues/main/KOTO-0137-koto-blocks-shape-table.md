# KOTO-0137: KotoBlocks Shape Table and Bytecode Locality Fix

- Status: done
- Type: bug
- Priority: P1
- Requirements: NFR-RT-2

## Goal

[KOTO-0136](KOTO-0136-game2d-static-layer.md) cut KotoBlocks' per-frame draw work
but slowed the VM: hardware `phase=163` showed the PSRAM code-window refills rising
~8 → ~39 per gameplay frame and `vm_us` ~75 ms → ~245-263 ms, with the thrash — not
the draw workload — now dominating the frame. This issue removes that regression by
fixing the bytecode that the code window has to fetch.

## What the diagnostics actually showed

The KOTO-0136 follow-up hypothesis was that the hot loop "calls `shape`/`pmid`/
`blit_piece` in tile 0, refilling on each call/return" and that the cure was to
**reorder functions** so a loop and its callees share a tile. The histogram +
transition diagnostics (`cw_hist`, `cw_trans`) disproved both halves of that:

- **The compiler inlines every function** (`koto-compiler` codegen emits one linear
  `main:` stream; `emit_inline` expands each call site, there are no `call`/`ret`
  opcodes). So helper functions **do not exist as standalone bytecode blocks**, and
  **reordering `fn` definitions is a no-op for locality** — verified empirically:
  moving `fn shape` to the end of the file produced byte-identical bytecode (same
  total words, same boundary owners at the same source lines).
- The thrash was therefore **not** call/return across tiles. It was **hot
  `while`-loop bodies landing across an 8 KiB PSRAM code-window boundary**. The
  measured `cw_trans` lines were a concentrated adjacent-tile ping-pong:
  - frame 180: tile 5 ↔ tile 6 (`5>6:17 / 6>5:16`) — the ghost-drop scan at the
    word-12288 boundary.
  - frame 210: tile 4 ↔ tile 5 (`4>5:221 / 5>4:220`) — the lock "mark full rows"
    double loop (`while tr>=0` × `while i<10` ≈ 200 iterations) at the word-10240
    boundary, fired on the frame a piece locks.

  KOTO-0136 moved ~65 chrome draw calls out of the per-frame render block, shifting
  all later word offsets so those two loops came to sit exactly on tile boundaries
  they had previously fit inside.

## Root cause: `shape()` inline bloat

`shape(k)` was a 28-branch constant-return `if`-chain. Inlined ~20 times (8 direct
in `main`, 12 via `blit_piece`), it dominated the code segment:

| metric | value |
| :--- | ---: |
| `shape` total bytes | 40,880 B |
| share of code | 61.9% |
| copies | ~20 |

That bloat forced everything to high word offsets and made the layout fragile: any
edit relocated the hot loops onto a different boundary. Shrinking it is both the
size fix and the stability fix.

## Fix

Replace the branch chain with a compact heap lookup table, baked once at startup:

- `const SHAPE_TBL = 3990` — the heap offset of `main`'s new `buf shapes[56]` (after
  `tiles+board+shown+num` = 3584+200+200+6), kept equal to that buffer's offset.
- `fn shape(k) -> int { return heap_get_u16(SHAPE_TBL + k * 2); }` — signature
  unchanged, so **no call site changed** and the local-slot budget did not move.
- `main` writes the 28 u16 masks once with `heap_set_u16` after seeding the RNG.

`k = piece*4 + rotation` is always 0–27, so the table fully covers the input domain
(the former `return 0` default was dead code).

## Results

| metric | before | after |
| :--- | ---: | ---: |
| KotoBlocks bytecode | 66,064 B | **26,960 B** |
| PSRAM code tiles (8 KiB) | 9 | **4** |
| `shape` bytes | 40,880 B | 3,924 B |
| user local slots | 43/45 | **43/45** |
| heap | 4,391 B | 4,447 B (+56) |

ABI, `CODE_WINDOW_BYTES`, `MAX_APP_DRAW_COMMANDS=160`, the KOTO-0136 static layer,
and rendering behavior are all unchanged. After the fix every `shape` inline copy is
a few words, and both formerly-straddling hot loops sit wholly inside one tile.

**KotoSim visual parity:** a mid-gameplay capture (active piece, NEXT O/Z/T, HOLD L,
ghost, locked cells, all panels/labels) renders correctly; the table reproduces the
former mask values exactly.

### Hardware validation (after shape table)

`code_size=26704`. Normal gameplay frames:

```
refills=4 code_tiles=4 cw_hist=0:1,1:1,2:1,3:1 cw_trans=0>1:1,1>2:1,2>3:1 vm_us≈42-60ms
```

The catastrophic ping-pongs are gone (frame 180 was `5>6:17 / 6>5:16`; frame 210 was
`4>5:221 / 5>4:220`). This confirms the shape-table fix solved the main KOTO-0136
performance regression.

## Lesson (compiler/runtime design note)

For PSRAM-backed bytecode on RP2040, **aggressive whole-program inline expansion can
be harmful even though it removes call overhead**, because code *size* and
code-window *locality* dominate the cost, not call/return. A constant-return helper
or a large branch chain, inlined N times, multiplies into the code segment and
pushes hot loops across 8 KiB tile boundaries. Such helpers should be represented as
**tables or data** read from the heap, not duplicated inline. (See also the
`CODE_WINDOW_BYTES` note in `src/koto-pico/src/firmware/config.rs` and the
`PsramCodeWindow` diagnostics in `src/koto-core/src/psram.rs`.)

## Residual (follow-up)

A much smaller residual tile 2 ↔ tile 3 ping-pong remains once HOLD is in use
(frame 810+: `refills=36`, `cw_hist=…2:17,3:17`, `cw_trans=2>3:17 / 3>2:16`,
`vm_us≈158-175ms`). It is the HOLD-preview `blit_piece` 16-iteration draw loop
straddling the word-6144 boundary. See the residual diagnosis in this issue's
follow-up notes / KOTO-0138.
