# KOTO-0156: KotoBlocks hot-path layout & cold-block outlining analysis

Where does the ~20 KiB steady-gameplay code extent actually come from, and which of
it is *hot every frame* versus one-time or rare? This is a **diagnosis** document that
follows on from [KOTO_CODEWINDOW_KOTOBLOCKS_REFILL.md](KOTO_CODEWINDOW_KOTOBLOCKS_REFILL.md)
(KOTO-0155, which established `refills=2 / code_tiles=2` is the structural floor of a
>1-tile hot loop) and [KOTO_VM_PROFILE_KOTOBLOCKS.md](KOTO_VM_PROFILE_KOTOBLOCKS.md)
(KOTO-0005, the instruction-mix profile).

It changes **no** VM semantics, opcode values, the bytecode ABI, hostcall IDs,
`RuntimeLimits`, the verifier, the PSRAM backend, the CodeWindow refill policy, or any
graphics/audio/input/app behavior. It **implements nothing**: it adds no compiler pass
and no app-source change. The only artifact is this report. The word→source-line
attribution below is read straight from the compiled `koto_blocks.kbc` debug section
(`KDBG`), host-side, with no instrumentation added to any crate.

## TL;DR

The 20 KiB steady extent is **mostly not hot code**. Attributing every code word to its
source line via the debug map shows the per-frame-resident *hot* path is only
**~1,093 words (~4.3 KiB)** — it would fit one 16 KiB tile with ~11 KiB to spare. The
extent reaches 20 KiB only because **~12.8 KiB of one-time setup and ~7.4 KiB of rare
per-event branches physically sit *between* the two hot endpoints** (the loop-top
dispatch at word ~1391 and the render/`yield` tail at word ~6526). They are branched
over every steady frame but still inflate `max_word − min_word` across the word-4096
tile boundary.

The lowest-risk-*looking* lever was **relocating the one-time, pre-loop string-init
preamble (~5.1 KiB) to the code tail.** It is purely mechanical and behavior-preserving,
and it shifts the steady hot extent toward word 0 as predicted (loop top 1391 → 189).
**But implementing it (KOTO-0156, below) showed it is *not* a safe standalone win**: on
its own it slides the 16 KiB tile boundary out of the *cold, branched-over*
horizontal-move block and into the **hot 200-cell board-diff loop**, exploding steady
refills from 2 to **442** — a refill ping-pong. Relocation only pays off **combined with
outlining the title-screen block (~7.4 KiB, branched over in play)**: that shrinks the
per-frame loop body under one tile so the hot loop collapses to ~3,100 contiguous words
wholly inside one tile → 0 steady refills, with nothing hot straddling the boundary.
**Both transforms are now implemented and enabled together for KotoBlocks** (per-app opt-in
via `apps.json`), taking steady gameplay from 2 refills to **0**. See
[#1 result](#implementation-result-koto-0156-1-preamble-relocation) and
[#2 result + the winning pair](#implementation-result-koto-0156-2-cold-block-outlining--the-winning-pair).

## Method (host-side, zero instrumentation)

The compiler emits the program as one stream: `main:` → a string-init preamble →
**main's body inlined in source order** → `exit`. All helpers (`pmid`/`shape`/
`blit_piece`/`sfx_*`) are inlined at their call sites — confirmed in
[codegen.rs](../../tools/koto-compiler/src/codegen.rs) (`run()` emits `store_str` for each
string then `emit_block(main_body)`; there is no out-of-line function section and no
basic-block reordering). **Therefore physical code-word order tracks main's source
order**, and the KBC debug section gives an exact word→line map.

The `.kbc` carries a `KDBG` debug section (built in
[kbc-asm/src/lib.rs](../../tools/kbc-asm/src/lib.rs) `build_debug_section`): a list of
`(pc, line)` entries, one per source-position change, where `pc` is the **code-word
index**. Parsing `sdcard_mock/bytecode/koto_blocks.kbc` directly yields 698 entries,
the first at `(pc=1203, line=153)` — so words 0–1202 are the lineless string preamble,
and word 1203 is the first body statement (`let state = 0`). Each subsequent
`[pc_i, pc_{i+1})` span is attributed to `line_i`. Inlined helper lines (98–141) appear
wherever called and are counted where they physically sit.

Reproduce the extraction (pure read of the committed fixture):

```sh
python - <<'PY'
import struct
d=open('sdcard_mock/bytecode/koto_blocks.kbc','rb').read()
i=d.find(b'KDBG'); _,_,nf,ne=struct.unpack_from('<HHHH',d,i+4); o=i+16
for _ in range(nf):
    n=struct.unpack_from('<H',d,o)[0]; o+=2+n
e=[struct.unpack_from('<IHHHH',d,o+k*14)[:1]+struct.unpack_from('<IHHHH',d,o+k*14)[3:4] for k in range(ne)]
print(e[0], e[-1], 'entries', ne)
PY
```

## Physical layout of the code segment (from the debug map)

Code segment is words 0–6526 (~25.5 KiB). The `R` column marks residency:
**O** = runs once (launch / title→play transition), **E** = rare per-event branch,
**H** = executed every steady-play frame.

| R | Block | Source lines | Words | Count |
| --- | --- | --- | --- | --- |
| O | string-init preamble (`store_str`×435 B, 3 words/byte) | — | 0..1202 | 1203 |
| O | persistent state + RNG seed init | 153–201 | 1203..1390 | 188 |
| H | loop top: `text_intent` + state dispatch | 202–205 | 1391..1398 | 8 |
| O | title: tile-cache bake (8 rows/frame) | 206–244 | 1399..1771 | 236 |
| O | title: title-screen immediate draw | 246–284 | 1772..2885 | 1114 |
| O | title→play: static-UI layer bake (once) | 285–335 | 2886..3224 | 339 |
| O | title→play: ghost-tile + 28 stamp defs (once) | 336–368 | 3225..3429 | 205 |
| H | exit check + play guard | 369–377 | 3430..3487 | 58 |
| E | line-clear collapse (frame a flash ends) | 378–415 | 3488..3744 | 257 |
| E | spawn / topout test (per piece) | 417–440 | 3745..3933 | 173 |
| E | hold (on H) | 442–457 | 3934..4015 | 82 |
| E | horizontal move (on ←/→) | 459–484 | 4016..4194 | 167 |
| E | rotate (on ↑) | 486–509 | 4201..4358 | 146 |
| H/E | vertical: gravity tick (always) + soft/hard step (event) | 511–545 | 4365..4575 | 199 |
| E | lock + row-full mark + score (per lock) | 547–619 | 4576..4892 | 289 |
| E | pause / game-over state update (states 2/3) | 621–644 | 4893..5164 | 272 |
| E | render: game-over board as dim rects (state 3) | 645–678 | 5165..5242 | 78 |
| H | render: play board diff (`game2d_set_tile`) | 679–706 | 5243..5411 | 169 |
| E | hard-drop trail (frame of a hard drop) | 708–722 | 5412..5497 | 74 |
| H | ghost piece (drop-scan + sprite) | 724–757 | 5498..5699 | 190 |
| H | active piece + (E) action-flash halo | 759–789 | 5700..5904 | 193 |
| H | run-state badge | 791–797 | 5905..5974 | 70 |
| H | NEXT/HOLD sprites + hold hint | 799–817 | 5975..6042 | 68 |
| H | stats number formatting (3× divmod) | 819–831 | 6043..6177 | 135 |
| E | pause / game-over overlays (states 2/3) | 833–847 | 6178..6295 | 118 |
| E | "4 LINE!" / score-popup banner (during flash) | 849–875 | 6296..6484 | 189 |
| E | "LEVEL UP!" banner (on level rise) | 877–883 | 6485..6523 | 39 |
| H | `game2d_present` + `yield_frame` | 885–891 | 6524..6526 | 3 |

The **16 KiB tile boundary (word 4096)** falls inside the *horizontal-move* block
(words 4016–4194, source ~line 468) — a branch that is **not taken** on a no-input
steady frame. The hot code straddling the boundary is incidental, not structural.

## Residency summary

| Class | Words | Size | Share of extent |
| --- | --- | --- | --- |
| **One-time** (preamble + init + title) | 3,285 | 12.8 KiB | ~64% |
| **Rare per-event** (input/lock/clear/overlay/banner) | 1,884 | 7.4 KiB | ~37% |
| **Every-frame hot** | 1,093 | 4.3 KiB | ~21% |

(Shares exceed 100% only because the vertical-step block splits across H/E.) The
decisive read: **only ~4.3 KiB is genuinely resident per steady frame.** If the hot
blocks were contiguous they would occupy roughly one quarter of a single tile.

## Hot bytecode ranges (executed every steady-play frame)

- **1391–1398** loop-top `text_intent` + dispatch
- **3430–3487** exit check + play guard
- **~4365–4420** gravity-tick counter (the always-run head of the vertical block; the
  `while steps>0` step body runs only on a gravity tick or input)
- **5243–5411** play board diff (the 200-cell shadow-diffed `game2d_set_tile` walk)
- **5498–5699** ghost piece (landing drop-scan + `game2d_sprite_set`)
- **5700–5860** active-piece ease + `game2d_sprite_set` (excludes the `fxhi` halo)
- **5905–6177** badge + NEXT/HOLD sprites + stats formatting
- **6524–6526** `game2d_present` + `yield_frame`

Min hot word ≈ 1391, max hot word ≈ 6526 — the endpoints are ~5,135 words apart, but
~4,040 words of that gap is one-time/rare code, not hot work.

## Cold / conditional ranges embedded in the hot extent

Two cohorts, both physically *between* the hot endpoints:

1. **One-time setup that the steady loop branches over (words 1399–3429, ~7.4 KiB).**
   The entire `if state == 0 { … continue; }` title block: tile-cache bake, title-screen
   draw, and the once-per-transition static-UI / ghost-tile / stamp bakes. Proven cold
   in play by `koto_blocks_play_uses_retained_game2d_layers` (it asserts the retained
   Game2D path) and by KOTO-0155's title-vs-play tile profile (title hot words top out
   at 3428, never reaching the play render). Plus the genuinely pre-loop preamble +
   state init (words 0–1390, ~5.4 KiB), which run before the loop is ever entered.

2. **Rare per-event branches interleaved through the update+render region
   (~7.4 KiB).** line-clear collapse, spawn, hold, horizontal move, rotate, the
   soft/hard-drop step, lock+score, pause/game-over update and render, hard-drop trail,
   the action-flash halo, pause/game-over overlays, and the three banner/popup blocks.
   None executes on a quiet steady frame; each sits across or near the word-4096 line.

## Candidate transforms (ranked by risk), with estimated savings

All are **ABI-preserving layout changes** — they move *where* existing instructions
land, never their opcodes, operands, hostcall IDs, or count. None touches the VM,
verifier, PSRAM, or CodeWindow policy. Savings are reductions in the steady
`max_word − min_word` extent (the quantity that decides tile crossings).

| # | Transform | Layer | Risk | Extent removed | Reaches 0 steady refills? |
| --- | --- | --- | --- | --- | --- |
| 1 | **Relocate the pre-loop one-time code** (string-init preamble, words 0–1202) to the code tail; enter via one forward branch, return via one back branch. | koto-compiler `run()` emission order | **low to emit, but regresses alone** | ~1,202 words / ~4.7 KiB off the front | **No — alone it moves the boundary into the hot board-diff loop: refills 2 → 442. Safe only with #2.** |
| 2 | **Outline the title block** (words 1399–3429, the `if state==0 { … continue }` body) to the code tail: branch to it on title frames, branch back to `yield`. | koto-compiler (new cold-block outliner) **or** app-source restructure | medium | ~1,894 words / ~7.4 KiB out of the loop span | **Yes, combined with #1** |
| 3 | **Cluster the rare per-event branches** (lock, spawn, move, rotate, collapse, overlays, banners) out of the contiguous loop span, leaving only the ~1.1 KiB hot path inline. | koto-compiler (general cold-block outliner) | medium–high | up to ~1,884 words / ~7.4 KiB | Yes, even without #2 |
| 4 | Continue the KOTO-0154 peephole / constant-folding shrink of the hot path itself. | koto-compiler | low | small per pass | Only over many passes |

### Why #1 + #2 is the clean target

With the current preamble in place the loop body is pinned to start at word ~1391, so
even after outlining the title block the body would span ~1391→~4495 — still crossing
word 4096. Conversely, relocating only the preamble leaves the title block inside the
loop, so the body spans ~0→~5135 — still crossing 4096. **Doing both** front-loads the
steady loop: state/RNG init (words 0–187), loop top, then update+render with the title
gap removed, ending near word ~3,100 — **wholly inside tile 0, 0 steady refills**, the
title-screen regime KOTO-0155 measured. This matches that doc's stated target of moving
~4 KiB / ~1040 words out of the per-frame-resident span (here ~5.4 KiB + ~7.4 KiB are
moved, comfortably past the threshold and past tile alignment).

### Feasibility notes

- **#1 is genuinely low-risk**: the preamble and state init are strictly pre-loop, run
  exactly once, and have no fallthrough dependency on their physical position. The
  change is local to `run()` in [codegen.rs](../../tools/koto-compiler/src/codegen.rs)
  (emit the body first, place the `store_str` sequence + a label after it, and bracket
  with two branches). It adds two one-time branch words and **zero** hot-path
  instructions. The assembler already supports labels and `br`.
- **#2/#3 need real outlining**: the compiler inlines every function and does no
  basic-block reordering, so a cold block cannot be relocated by moving it into a Koto
  `fn` (it would inline straight back). Reaching them requires either a new,
  opt-in/heuristic cold-block outliner in codegen (cold blocks emit a `br` to a
  tail-placed body that `br`s back — a call/return-free relocation, so it does **not**
  reintroduce the KOTO-0134 ping-pong on the hot path) or an app-source refactor. These
  are deferred per the task's "no risky transforms yet" constraint.

## Implementation result (KOTO-0156 #1): preamble relocation

The preamble relocation was implemented in the koto-compiler codegen
([codegen.rs](../../tools/koto-compiler/src/codegen.rs) `run()`): at entry, branch once to
a tail-placed `store_str` preamble that initializes the strings and branches back to the
body start. It is **layout-only and behavior-preserving** — proven by three new
koto-compiler tests:

- `preamble_relocation_preserves_behavior` — compiles the same string-using program with
  relocation on and off, runs both in the VM, and asserts the two **differ in bytecode
  but produce identical results and captured output**.
- `preamble_relocation_moves_store_str_past_the_body` — asserts the relocated layout
  (entry `br`, `store_str` after the body's terminator, a `br` back to the body) and that
  the disabled layout is the original inline preamble.
- `stringless_program_is_byte_identical_either_way` — a no-string program is byte-for-byte
  identical with and without relocation (no spurious labels/branches).

**Alone, the transform regresses KotoBlocks.** Driving the relocated build through
`koto_blocks_code_window_tile_profile` confirmed the predicted downward shift **and** the
regression: the 16 KiB tile boundary (absolute word 4096) moves from the *cold,
branched-over* horizontal-move block into the **hot 200-cell board-diff loop** (source
~line 687, the per-frame `game2d_set_tile` walk), and with the device's single-tile
window a hot loop straddling the boundary ping-pongs once per crossing — ~442
refills/frame versus the 2-refill floor. So #1 is not a safe *standalone* optimization;
whether it helps depends entirely on what code the shifted boundary lands in.

## Implementation result (KOTO-0156 #2): cold-block outlining + the winning pair

Candidate #2 was then implemented as a **gated, call-free cold-block outliner** in the
same codegen. At an `if` whose then-block is large and **does not fall through** (ends in
`continue`/`break`/`return` — a coldness proxy that matches the title-state block), the
then-body is emitted at the code tail: the inline site becomes `br <cold>` and the tail
holds `<cold>: <then-body> br <merge>`. It uses only `br` (no CALL/RET, so no hot-path
ping-pong), and correctness does not rely on the coldness heuristic — the relocated copy
always ends in the branch back to the merge point, so even a mis-classified fall-through
block rejoins correctly. Because operand depth is 0 between statements, the relocated
block (and the tail) sit at depth 0 and the linear verifier accepts them. Four more
koto-compiler tests prove behavior preservation (outline-only and #1+#2 run identically
to baseline), the tail relocation, and the size threshold.

Both transforms are **off by default and opt-in per app** (apps.json `codegen` block →
build_apps.py → `--relocate-preamble` / `--outline-cold-blocks` CLI flags), because they
only help apps whose per-frame loop crosses a tile boundary and would needlessly relayout
(and could regress) other large apps (kotorun/kotorogue/kotoshogi). Measuring all three
layouts on KotoBlocks via the tile profile:

| Layout | Steady `hot_words` | Steady refills | Steady tiles |
| --- | --- | --- | --- |
| Baseline (shipped elsewhere) | 1391..6528 (~20 KiB) | 2 | 2 |
| #1 preamble-relocation only | 189..5326 | **442** | 2 |
| #2 outline-only | 1391..4510 (~12 KiB) | **14** | 2 |
| **#1 + #2 (shipped for KotoBlocks)** | **189..3308 (~12 KiB)** | **0** | **1** |

Only the **pair** wins: #1 alone moves the boundary into the board-diff loop (442); #2
alone shrinks the span but its hot tail still straddles 4096 (14, *worse* than the
baseline 2); together they front-load the steady loop — preamble at the tail pulls the
start to word 189, title outlining removes the ~1,894-word gap — so the whole hot loop
(words 189..3308, ~12 KiB) sits inside tile 0: **0 steady refills, 1 tile.** KotoBlocks
now enables both (`apps.json` `codegen`); `koto_blocks_code_window_tile_profile` asserts
the `(refills, tiles) == (0, 1)` result as a regression guard.

## What was changed

- **koto-compiler**: two off-by-default, behavior-preserving codegen layout options
  ([`CodegenOptions`](../../tools/koto-compiler/src/codegen.rs): `relocate_preamble`,
  `outline_cold_blocks`), surfaced as CLI flags and seven equivalence/layout tests.
- **build pipeline**: a per-app `codegen` block in `apps/apps.json` (threaded by
  `harness/build_apps.py`); only KotoBlocks opts in, so **every other app's committed
  `*.kbc` is byte-identical** (`git status` shows only `koto_blocks.kbc` changed; `--check`
  passes). KotoBlocks' rendered behavior is unchanged (the
  `koto_blocks_play_uses_retained_game2d_layers` retained-layer assertions still hold; the
  golden-frame trace, which covers the shell + hello-text, is untouched).
- No `koto-vm`, app *source*, graphics/audio/input, ABI, opcode, hostcall-ID,
  `RuntimeLimits`, verifier, PSRAM, or CodeWindow-policy changes.

## Verification

- `cargo test -p koto-vm` — ok (35 + 19).
- `cargo test -p koto-core -p koto-sim` — ok (incl. `koto_blocks_code_window_tile_profile`
  now asserting 0 refills / 1 tile, and `koto_blocks_play_uses_retained_game2d_layers`).
- `cargo test -p koto-compiler` — ok (63 tests incl. 7 KOTO-0156 equivalence/layout tests).
- `cargo build -p koto-pico --target thumbv6m-none-eabi --bins` — ok.
- `python harness/build_apps.py --check` — OK (only `koto_blocks.kbc` rebuilt).
- `python harness/check_golden_frames.py` — OK.

