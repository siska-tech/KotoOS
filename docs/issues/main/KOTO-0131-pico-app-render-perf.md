# KOTO-0131: Pico App Render Performance and Metrics

- Status: in-progress
- Type: feature
- Priority: P1
- Requirements: NFR-DRAW-2

## Goal

Make PicoCalc games (starting with KotoBlocks) playable on physical hardware.
After KOTO-0127 staged large bytecode into PSRAM, every game *launches* and
*runs*, but rendering is so slow it is effectively unplayable: observed frames
report `dirty_px=102400` (320×320 — a full-surface transfer), `raster_us≈101636`
and `transfer_us≈73724`, i.e. ~175 ms/frame (≈6 fps). The dirty-rectangle
optimisation is not holding; the frame is being repainted whole.

This issue (a) adds the per-frame measurement needed to triage the slowdown on
hardware, and (b) makes the smallest firmware-side render change that stops the
delta path from collapsing into full-surface transfers. It does **not** touch the
KOTO-0127 PSRAM bytecode staging / window-fetch / launch path.

## Diagnosis (from reading the pipeline + KotoBlocks)

The app renders immediate-mode: every frame it re-emits the whole command list —
a full-screen `draw_rect(0,0,320,320,C_PAGE)` clear, the well + grid (~35 stable
rects), then a *variable* number of board-cell blits, then ghost/active-piece
overlays, then ~50 side-panel commands.

`present_app_delta` diffs the previous and current command lists **positionally**
(`command[i]` old vs new). This is cheap while the command *count* is stable
(free-fall: only the ~4 active-piece blits move). But the moment the board cell
count changes — a piece locks (+cells), a line clears (−cells), the ghost
appears/disappears (±16 rects) — every later index is **misaligned**: panel
command `i` is compared against a board tile, producing a spurious union
rectangle that can span most of the screen. Two failure modes follow:

1. A single misaligned union exceeds one raster strip (`used > strip.len()`), and
   the old code fell back to `present_app_commands` — a **full 320×320 repaint**.
   A piece respawning top↔bottom (union ≈ 64×304) triggers this every spawn.
2. Many misaligned commands each emit their own near-full-screen transfer, and
   `paint_app_commands` re-rasters the **entire** command list per rectangle, so
   `raster_us` explodes (the dominant term in the observed numbers).

## Acceptance Criteria

- [x] Per-frame render metrics over UART on a throttled cadence (frame 1, then
  every 30): `phase=160 app-frame app=… frame=… vm_us=… raster_us=… transfer_us=…
  dirty_px=… rects=… hostcalls=… rect=… text=… pixels=… full=… fps=… lat_ms=…`.
- [x] First-frames draw-pattern audit (frames 1–3, ≤16 commands each):
  `phase=161 hc f=… i=… op=… x/y/w/h… clip=…` plus a
  `summary len=… fullscreen_base=…` line, so the per-frame full clear and the
  tile/text mix are visible without flooding UART.
- [x] `BytecodeSession::last_frame_host_calls()` exposes the per-frame host-call
  count (mirrors `last_frame_fuel`); no change to VM execution.
- [x] Delta no longer escalates a single oversize dirty rectangle to a full
  repaint: `present_rect_banded` repaints just that rectangle in strip-high bands.
- [x] Delta sizes its work first (`FULL_REPAINT_AREA`, `FULL_REPAINT_RECTS`) and
  takes one clean full compose only when the partial transfers would cost more —
  bounding the worst case instead of death-spiralling into many large transfers.
- [x] `cargo fmt`, `cargo build -p koto-pico --bin koto_firmware --target
  thumbv6m-none-eabi --release --offline`, `cargo test` (core 169, workspace),
  and the runtime budget gate all pass.
- [x] Hardware (render): KotoBlocks reacts to input and plays without breaking;
  `phase=160` shows `full=0`, `dirty_px` 102400→0–1920, `transfer_us`
  73724→0–1764, `raster_us`→0–8254, fps 6→11–12. Render target met.
- [x] Hardware exposed a new bottleneck: `vm_us≈76–84 ms/frame` (≈3.1 µs/insn),
  i.e. PSRAM code-window thrash. Addressed by the `PsramCodeWindow` 2-tile LRU
  cache, keeping the original 8 KiB window (two 4 KiB tiles) — both a 64 KiB and
  even a 16 KiB window overflowed the boot stack on hardware, so the fix spends no
  extra SRAM (firmware `.bss` leaves only ~59 KiB for the stack).
- [ ] Hardware re-run with the 2-tile cache to confirm `vm_us` drops and fps
  rises (≥15 target). *(Pending user run.)*

## Changed Files

- `src/koto-core/src/runtime.rs` — `last_frame_host_calls()` on `BytecodeVm` and
  `BytecodeSession`.
- `src/koto-core/src/psram.rs` — `PsramCodeWindow` is now an LRU N-way tile cache
  (`MAX_TILE_SLOTS=2`) instead of a single tile, so the helpers' low tile stays
  resident alongside `main`'s current tile and the call/return ping-pong stops
  refilling. New unit test covers the steady-state hit.
- `src/koto-pico/src/firmware/config.rs` — `CODE_WINDOW_BYTES` stays 8 KiB
  (net-zero SRAM). A 16 KiB bump overflowed the boot stack on hardware (HardFault
  hang in the default handler loop; the linker can't catch a runtime stack/`.bss`
  collision), so the cache splits the original 8 KiB into two 4 KiB tiles instead
  of enlarging the window. Comment records the measured ceiling.
- `src/koto-pico/src/firmware/diag.rs` — `PaintMetrics` gains a transfer (dirty
  rect) count, a `full_repaint` flag, and accessors; new `log_app_frame_metrics`.
- `src/koto-pico/src/firmware/app_render.rs` — metrics threaded through both
  present paths; `present_rect_banded`; two-pass delta with full-repaint
  thresholds; `log_command_sample`.
- `src/koto-pico/src/firmware/app_runtime.rs` — measures `vm_us` / `work_us`,
  reads host-call count, emits `phase=160` / `phase=161`.
- `src/koto-pico/src/dashboard.rs` — `DASHBOARD_LINE_BUFFER_BYTES` 160→224 for
  the longer `phase=160` line.

## Notes — Next Issues (mid-term design)

The positional diff is the root limit; metrics + banding + thresholds tame the
worst case but do not make a board grow/collapse cheap. Two follow-ups:

- **KOTO-0132 — Game2D stateful tile/sprite host API.** Hold tilemap + sprite
  table + per-tile dirty bitset on the *host*; the VM updates only changed cells
  (`game2d_set_tile/set_sprite/set_palette/present`) instead of replaying a full
  immediate-mode list. KotoBlocks is the natural first adopter. The ABI was
  already sketched in KOTO-0097. This is the real fix for tile games.
- **KOTO-0133 — content-keyed (stable-slot) app delta.** Cheaper interim step:
  make the delta diff by command identity/position rather than list index (e.g.
  the app reserves a fixed slot per board cell, drawing empties too), so a count
  change no longer misaligns every later command. Firmware- or app-side.

Memo and text apps want a separate text-cell renderer, not this tile model.
