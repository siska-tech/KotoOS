# KOTO-0155: KotoBlocks CodeWindow refill analysis

Why does KotoBlocks steady gameplay touch **two** code tiles per frame
(`refills=2`, `code_tiles=2`, `cw_refill_us` ≈ 2.5–2.8 ms)? This is an
**observation + diagnosis** document. It changes no VM semantics, opcode values,
the bytecode ABI, hostcall IDs, `RuntimeLimits`, the verifier, the PSRAM backend,
the CodeWindow refill policy, or graphics/audio/input. The only code change is a
host-side, test-only diagnostic in
[fixture_runner.rs](../../src/koto-sim/tests/fixture_runner.rs)
(`koto_blocks_code_window_tile_profile`); firmware is untouched.

## TL;DR

`refills=2 / code_tiles=2` is **not** a refill ping-pong and **not** a pathology to
fix. It is the **structural floor** of a per-frame loop body that is larger than one
16 KiB code tile. Steady gameplay executes a hot path spanning code words
**1391–6528 (~20 KiB)**, which crosses the single tile boundary at word 4096 exactly
once going forward and once on the loop-back — the irreducible minimum of two refills
for a >1-tile hot loop. The device is already at its best achievable steady state for
the current bytecode layout. Driving it to **0 steady refills** (title-screen speed)
requires shrinking the *per-frame-resident* code below one tile (~4 KiB / ~1040
words), which is a compiler/app-bytecode lever — not a CodeWindow change.

## The mechanism

The device streams PSRAM-resident code through `PsramCodeWindow`
([psram.rs](../../src/koto-core/src/psram.rs)), a **single-tile** SRAM cache. The window
is `CODE_WINDOW_BYTES = 16 KiB` → **4096 code words per tile**
([config.rs](../../src/koto-pico/src/firmware/config.rs)). Word `i` is served from the
tile `[base, base+4096)` where `base = (i / 4096) * 4096`; a fetch outside the cached
tile triggers one blocking refill. Sequential execution only refills on a boundary
crossing; the cache persists across frames, and only the per-frame counters reset.

> **Doc nit (not fixed here, out of scope):** the comment block above
> `CODE_WINDOW_BYTES` still narrates the old 8 KiB window and warns that 16 KiB
> overflowed the boot stack. The constant is now `16 * 1024`; the comment is stale
> (the koto-psram stack-headroom work made room). Flag for a follow-up doc fix.

### KotoBlocks bytecode shape (`koto_blocks.kbc`, post-KOTO-0154)

Parsed from the KBC header + debug map:

| Field | Value |
| --- | --- |
| Code segment | 6,531 words / 26,124 bytes (file bytes 64–26,188) |
| Tiles spanned (16 KiB) | **2** — tile 0 = words [0,4096), tile 1 = words [4096,6531) |
| Tile boundary | code word 4096 = file byte 16,448 (≈ source line 468) |
| Entry preamble | words 0–~1390 — one-time string/const init, runs once at launch |
| `main` body (inlined) | words ~1391–6526 — helpers (`pmid`/`shape`/`blit_piece`/`sfx_*`) are **inlined**, so there is no `CALL`/`RET` (matches the profile's `call_depth=0`) |

So KotoBlocks is effectively one big `main` function. Its per-frame loop
(`loop { … }` at source line 202, `yield_frame()` at line 891) spans roughly source
lines 153→891, i.e. nearly the whole body — and physically straddles the word-4096
boundary.

## Evidence: host replay of the exact device tiling

`koto_blocks_code_window_tile_profile` wraps the resident code source with the
**device's own single-tile math** (`base = (index/4096)*4096`, one cached tile,
cache persists across frames) and steps the real
`koto_blocks_play_uses_retained_game2d_layers` input script (title bake → start →
spawn → hard-drop → steady play). Per-frame, it reports refills, distinct tiles, and
the executed hot-word extent. Reproduce:

```sh
cargo test -p koto-sim --test fixture_runner koto_blocks_code_window_tile_profile -- --nocapture
```

| Phase | Frames | Hot words [min..max] | Hot extent | refills | code_tiles |
| --- | --- | --- | --- | --- | --- |
| Title screen | 1–34 | 1391..3428 | ~7 KiB | **0** | 1 (tile 0) |
| **Steady play** | 38–43 | 1391..**6528** | **~20 KiB** | **2** | **2** (tiles 0+1) |

Key reads:

1. **The title screen fits one tile and never refills** — its hot path (words
   1391–3428) is wholly inside tile 0. Immediate-draw title rendering is *not* what
   crosses the boundary.
2. **Steady gameplay's hot path is ~20 KiB and spans both tiles.** The lowest hot
   word (~1391, source ~153) and the highest (~6528, source ~774–891: the
   status-number render + `game2d_present` + `yield_frame`) are both executed every
   frame, so the per-frame footprint genuinely reaches from one end of `main` to the
   other.
3. **`refills == distinct_tiles` in every steady frame (2 == 2).** A `main`↔helper
   ping-pong would show `refills ≫ code_tiles` (many refills, few tiles). It does not.
   The pattern is a monotone tile0→tile1 forward walk plus the once-per-frame
   loop-back to the top — exactly two refills, no thrash. The KOTO-0134 ping-pong
   worry does **not** apply to KotoBlocks.
4. **Gameplay is ~21.6K instructions/frame**, ~2.6× the title screen's ~8.3K that
   the earlier (empty-input) profile measured — a reminder that
   [KOTO_VM_PROFILE_KOTOBLOCKS.md](KOTO_VM_PROFILE_KOTOBLOCKS.md) captured the title
   screen, not play.

### Mapping `cw_refill_us` ≈ 2.5–2.8 ms

Two refills/frame move tile 0 (16 KiB) + tile 1 tail (≈9.7 KiB) ≈ **26 KiB of PSRAM
reads per frame**. At ~2.5–2.8 ms that is ~1.25–1.4 ms per refill, i.e. the cost is
the PSRAM transfer of the two tiles, not extra fetches. Against a ~16.6 ms 60 fps
frame this is ~15–17% of the budget — real but bounded, and already minimal for the
current layout.

## Can the KotoBlocks hot path fit in one code tile?

**Not as currently laid out.** The per-frame hot extent is ~20 KiB > the 16 KiB
tile, and (unlike the title screen) the executed words reach both the low and high
ends of `main`. Relocating the one-time entry preamble (words 0–~1390) alone does not
help: the per-frame loop body itself is ~20 KiB and would still straddle a boundary.
Reaching 0 steady refills requires reducing the *per-frame-resident* code by ~4 KiB
(~1040 words) **or** clustering the steadily-executed blocks into a single
tile-sized, contiguous region with the rare blocks (game-over, pause overlay,
line-clear, title setup) outlined out of that span.

## Proposed next actions, ranked by risk

Compiler/app-bytecode layout first, as required; refill-policy changes explicitly
deferred and out of scope.

| # | Action | Layer | Risk | Expected effect |
| --- | --- | --- | --- | --- |
| 1 | **Confirm on hardware** that the existing `phase=163 cw` line shows `cw_hist=0:1,1:1` and a monotone `cw_trans` (e.g. `0>1:1,1>0:1`), not a high-count pair. The host replay already proves this; this is the zero-cost field check. | firmware diag (already present) | none | Confirms diagnosis |
| 2 | **Keep shrinking the per-frame bytecode footprint** toward <4096 resident words. KOTO-0154 cut 880 B; the profile's still-open target #1 (template-level `swap`/`dup` boolean-normalization rewrite) and further constant folding are the safe continuations. If the per-frame extent drops under 16 KiB, gameplay refills go to **0**, like the title screen. | koto-compiler | low | Removes both steady refills if it crosses the threshold |
| 3 | **Cold-block outlining / hot-path clustering**: emit the rare per-frame branches (game-over render, pause/banner overlays, line-clear, one-time title setup) outside the contiguous extent of the steadily-executed code so the hot loop body packs into one tile. No VM/ABI change, but the compiler does no basic-block reordering today, so this is a new codegen pass or an app-source refactor. | koto-compiler / app source | medium | Can reach 0 refills without shrinking total code |
| 4 | **Relocate the one-time entry/string-init preamble** (words 0–~1390) to the code tail. On its own it does not fix the straddle, but it frees ~5.4 KiB of tile-0 space and is a prerequisite that makes #2/#3 land sooner. | koto-compiler | low–med | Enabler for #2/#3 |
| 5 | *(Deferred — out of scope)* Window/refill-policy changes: a larger window that covers the whole 26 KiB segment (0 refills ever), or the KOTO-0134 2-tile LRU cache. Both are CodeWindow/refill-policy and SRAM-budget changes the task forbids, and the 2-tile cache previously coincided with a launch hang. **Do not pursue under this task.** | firmware / koto-core | higher | Not now |

## What was (intentionally) not changed

- No `koto-vm` semantics, opcode values, bytecode ABI, hostcall IDs,
  `RuntimeLimits`, verifier, PSRAM backend, or CodeWindow refill policy.
- No graphics/audio/input behavior; KotoBlocks bytecode is byte-identical.
- The only added code is the host-only, assertion-bearing diagnostic test; it prints
  only under `--nocapture` and adds no firmware log noise.

## Verification

- `cargo test -p koto-vm` — ok (35 + 19 passed; VM untouched).
- `cargo test -p koto-core -p koto-sim` — ok (incl. the new
  `koto_blocks_code_window_tile_profile` and the existing
  `koto_blocks_play_uses_retained_game2d_layers`).
- `cargo build -p koto-pico --target thumbv6m-none-eabi --bins` — ok (firmware
  unchanged).
