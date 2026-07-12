# KOTO-0134: Why `__embassy_main::POOL` Is ~128 KiB

- Status: in-progress
- Type: research
- Priority: P1
- Requirements: FR-RT-5

## Goal

Understand why the embassy main-task future (`__embassy_main::POOL`) is ~128 KiB
— nearly half the RP2040's 264 KiB SRAM — and propose ways to shrink it. This is
**investigation only**: no behavior changes. The motivation is the firmware's
thin stack headroom (KOTO-0131 measured ~59 KiB free above `.bss`, and an 8 KiB
StaticCell bump overflowed the boot stack), and a hang on KotoBlocks launch right
after `phase=152 app-started`. A smaller main future frees SRAM for the stack.

## Measurement

`llvm-nm --print-size --size-sort` on the `--release` `thumbv6m-none-eabi`
binary, largest statics:

```
128952  b  koto_firmware::__embassy_main::POOL
 30721  b  RGB666_STRIP
 20481  b  RASTER_STRIP
 16385  b  APP_HEAP
  8193  b  CODE_WINDOW
  2305  b  MANIFEST_BYTES
  2049  b  KICON_SCRATCH
```

`POOL` (the main task's future) alone exceeds RGB666 + RASTER + APP_HEAP +
CODE_WINDOW combined. Total `.bss` is 210176; `POOL` is ~61% of it.

## Why It Is So Large

The `#[embassy_executor::main]` macro stores the whole main async fn's state
machine in `POOL`. Its size is the worst-case set of locals **live across an
`.await`**, plus state-machine slop. Two structural facts make it huge:

1. **`run_device_app().await` is called from inside the shell `loop`**
   (`koto_firmware.rs:345`), so the entire app-session subtree future is
   co-resident with the still-live shell-loop locals (`ShellState`, font refs,
   `held`, `input`, `line`). The app future does not replace the shell future; it
   stacks on top of it.

2. **Large async fns overlap poorly.** The compiler often fails to reuse space
   between disjoint-lifetime locals across a big state machine, so e.g. the boot
   `present_pixel_diagnostic` host and the launch-path hosts can each get their
   own slot instead of sharing one — inflating `POOL` beyond the theoretical max.

### Dominant cross-await locals (sizes computed from the type definitions)

- **`DeviceRuntimeHost` = `{ commands: [AppDrawCommand; 384], len }` ≈ 30.7 KiB.**
  `AppDrawCommand` is **80 B** — its `Text` variant inlines `bytes: [u8; 64]`
  (`MAX_APP_TEXT_BYTES`), so the 384-entry array (`MAX_APP_DRAW_COMMANDS`) is
  ~30 KiB even though KotoBlocks peaks at **106/384** used (`phase=155`). This
  type appears live at multiple await points:
  - `present_pixel_diagnostic`'s `host` (boot, `app_render.rs`).
  - `DeviceHost::draw` inside `host` in `run_app_session`.
  - `previous_draw` local in `run_app_session`.
  Three to four ~30 KiB instances that overlap poorly ≈ the bulk of `POOL`.
- **`DeviceHost` memo/IME/SKK state ≈ 6 KiB**, carried even by games:
  `skk_dict: [u8; 4096]`, `editor: MemoEditor<1024>` (`bytes:[u8;1024]` + layout
  + dirty ≈ 1.3 KiB), `ime: KotoMemoIme`, `skk_index: Option<SkkLeadingIndex>`,
  `diag: LineBuffer` (224 B).
- **`ShellState` ≈ 30 KiB** (live across the launch await): `packages:
  PackageList = [Option<PackageInfo>; 32]`, and `PackageInfo` is ~900 B of fixed
  buffers (`app_id[64]`, `name[64]`, `icon_path[128]`, `entry[128]`,
  `description[128]`, `category[32]`, runtime/permission buffers, optional icon).

Note `APP_HEAP`, `RASTER_STRIP`, `RGB666_STRIP`, `CODE_WINDOW` are already
StaticCells borrowed by reference, so they cost only a pointer in `POOL` — the
draw-command lists and `DeviceHost`/`ShellState` are the parts still living
inside the future by value.

## Result (implemented)

Proposal 1 was implemented: the current + previous frame draw-command lists (and
the boot pixel-diagnostic host) now live in a binary `StaticCell<[DeviceRuntimeHost;
2]>` (`APP_DRAW`) borrowed `&mut` through `run_device_app` / `run_app_session` /
`DeviceHost`, instead of as locals in the async run loop. Measured:

- **`__embassy_main::POOL`: 128,952 → 70,568 bytes** (−57 KiB, −45%). The main
  future no longer carries the two ~30 KiB lists.
- **Total `.bss`: unchanged** (210,176). The new `APP_DRAW` static is 58,380 B —
  the buffers *moved out of the future* but were not eliminated, so stack headroom
  is the same (~59 KiB). The boot diagnostic's copy turned out to already be
  overlapped by the compiler, so there was no triplication to collapse.

So relocating alone did **not** free SRAM. Proposal 3 was then applied to actually
shrink the buffers:

### `MAX_APP_DRAW_COMMANDS` 384 → 160 (measured SRAM reduction)

`AppDrawCommand` layout is unchanged; only the per-list capacity dropped, halving
both retained lists in `APP_DRAW`. Measured:

| metric              | before (384) | after (160) | delta        |
| :------------------ | -----------: | ----------: | :----------- |
| `APP_DRAW`          |       58,380 |      24,332 | −34,048      |
| total `.bss`        |      210,176 |     176,136 | −34,040      |
| stack headroom      |  58.8 KiB    |   92.0 KiB  | +33.2 KiB    |
| `__embassy_main::POOL` | 70,568    |      70,576 | ~0           |

Stack headroom (`RAM 270,336 − .bss`) grows from ~59 KiB to ~92 KiB — the margin
that an 8 KiB StaticCell bump overflowed in KOTO-0131 now has real room.

Behaviour change & instrumentation: 160 sits above KotoBlocks' observed normal-play
peak (106) but below its theoretical board-full/busy-play worst case (~310-340),
so a very full board can overflow and drop tail commands (panels/overlays render
after the board). This is now observable — `phase=160 ... peak= ovf=` (session
draw peak and overflow-frame count) and a one-shot `phase=162 app-draw-overflow`
when the cap is first hit — and is a one-line revert to 384. Existing overflow
behaviour (drop tail, app swallows `NO_MEMORY`) is unchanged.

Validation: `cargo fmt`, `cargo build` (thumbv6m release), `cargo test`
(core 169, sim 90, workspace all ok), `python harness/check_all.py` — all pass.

## Proposed Reductions

Behavior-preserving (proposal 1 done above):

1. **StaticCell the draw-command lists.** Move `previous_draw` and `DeviceHost`'s
   `draw` (and the boot `present_pixel_diagnostic` host) into one or two
   `StaticCell<DeviceRuntimeHost>` borrowed `&mut`, mirroring `RASTER_STRIP`. This
   pulls ~60–90 KiB out of `POOL`, and because a shared StaticCell is reused
   across the boot and launch await points (which the async state machine fails
   to overlap), it should also cut **total `.bss`**, directly widening the stack
   margin. Two buffers are needed (current + previous frame for the delta).
2. **StaticCell `DeviceHost`'s memo/SKK block.** The `skk_dict[4096]` + `editor` +
   `ime` + `skk_index` (~6 KiB) are only meaningful for Memo; move them behind a
   StaticCell (or instantiate lazily) so games do not carry them in `POOL`.

Behavior-changing (defer / weigh separately — flag clearly if taken):

3. **Right-size `MAX_APP_DRAW_COMMANDS`** (384 → e.g. 160). Observed peak is
   106/384; 160 keeps headroom and shrinks each `DeviceRuntimeHost` from ~30.7 to
   ~12.8 KiB. Risk: apps that exceed it drop tail commands.
4. **Move `AppDrawCommand::Text` bytes out of line** (side ring buffer, or reduce
   `MAX_APP_TEXT_BYTES` 64 → 32). Shrinks the 80 B element directly. Risk: longer
   strings truncate / added indirection.
5. **Spawn the app session as its own task** so its future is not co-resident with
   the shell loop. Trade-off: embassy task pools are statically sized too, so this
   moves rather than removes the cost unless paired with (1)/(2).

## Notes — launch hang (resolved)

The KotoBlocks hang after `phase=152` was bisected to the `PsramCodeWindow` 2-tile
cache (KOTO-0131): reverting it to the single-tile window restored launch on
hardware (user-confirmed). The cache is reverted; the PSRAM code-fetch ping-pong
it tried to fix is still open (see `config.rs` `CODE_WINDOW_BYTES` note) and should
be re-attempted with a different approach (e.g. the stateful Game2D host API in the
KOTO-0131 notes, which avoids the per-instruction code-fetch entirely).

## Code-fetch thrash diagnostics (added)

After KOTO-0135 Phase 1 moved the locked board to a host tilemap, the render path
went idle but the frame stayed VM-bound: `vm_us ~75-78 ms` against `fuel ~23k` at
`fps 10-13`, i.e. ~3 us/instruction — the PSRAM window-refill thrash signature
(SRAM execution is ~0.1 us/instruction). To quantify it on hardware before any fix
(and per the constraint to leave `CODE_WINDOW_BYTES` at 8 KiB), the
[`PsramCodeWindow`](../../../src/koto-core/src/psram.rs) now counts, per frame:

- `refills` — window refills (one full-tile PSRAM read each), and
- `code_tiles` — distinct tiles those refills touched (a `main`↔helper ping-pong
  shows **many refills across few tiles**).

These ride the throttled `phase=160` line as `refills= code_tiles=`. Plumbed
through the `CodeSource` trait as default-zero methods (`reset_fetch_metrics` /
`fetch_refills` / `fetch_distinct_tiles`), so the resident `SliceCode` fallback and
the simulator report nothing; only the device PSRAM window populates them. This is
**instrumentation only** — no window-size or layout change yet. The frame loop
resets the counters before each `step_frame_with` and logs them after.

### Confirmed hardware results (KotoBlocks, `code_size=65988` = 8× 8 KiB tiles)

| phase | frame | `refills` | `code_tiles` | `vm_us` | `fuel` | `fps` |
| :---- | ----: | --------: | -----------: | ------: | -----: | ----: |
| title / early | 30-90 | 3 | 3 | ~25,600 | — | 37 |
| gameplay | 120-540 | 8 | 8 | 68k-86k | ~18k-29k | 10-12 |

The hypothesis that this was a 2-3-tile `main`↔helper ping-pong was **wrong**. The
data shows **`refills == code_tiles` in every sample** — each touched tile is
refilled *exactly once* per frame, i.e. there is no intra-frame reloading. The
per-frame **code working set is the whole code segment** (all 8 tiles / 64 KiB at
gameplay; 3 tiles early), and the 8 KiB single-tile window reloads every tile it
visits, once, each frame.

`vm_us` tracks `refills` almost linearly — 3 refills → ~25 ms, 8 refills → ~80 ms,
i.e. **~10 ms per 8 KiB tile refill** (an 8 KiB PSRAM SPI read plus overhead). So
`vm_us ≈ refills × ~10 ms` and the refills are essentially the entire frame cost
(fuel execution from cache is a few ms; `fuel ~28k × ~0.1 us` ≈ 3 ms).

**Implication for the fix.** Because the working set is the *entire* code, partial
remedies do **not** help: code-layout colocation of a couple of hot helpers, or a
small 2-tile victim cache, still leaves most of the 8-tile working set uncached and
reloading each frame. The effective fix is a window (or multi-tile cache) large
enough to hold the working set — at the limit a window ≥ the code size, so the whole
app is one tile: one fill at launch, **zero refills per frame** (exactly the
`config.rs` `CODE_WINDOW_BYTES` prediction). KotoBlocks needs ~64.4 KiB.

**SRAM trade (not taken yet — `CODE_WINDOW_BYTES` stays 8 KiB per the current
constraint).** Growing the window from 8 KiB toward 64 KiB costs +56 KiB `.bss`.
The KOTO-0135 160-cap reclaim left ~90 KiB stack headroom, so +56 KiB would leave
~34 KiB — above the margin that the +8 KiB bump overflowed at ~59 KiB headroom in
KOTO-0131, but it must be measured before adopting (and the per-app window could be
sized to the app's code, not a fixed 64 KiB, to avoid charging small apps). A
right-sized multi-tile LRU cache is the alternative if a fixed large window is too
costly. Decision deferred to the next issue.
