# KOTO-0120: Pico Shell Dirty-Rectangle Performance

- Status: done
- Type: feature
- Priority: P0
- Requirements: FR-SHELL-3, FR-SDK-1, NFR-PERF-1, NFR-DRAW-1, NFR-DRAW-2, NFR-MEM-3

## Goal

Make normal KotoShell interaction responsive on RP2040 by treating the LCD
controller GRAM as retained display memory and transferring only pixels whose
visual content changed.

## Acceptance Criteria

- [x] Selection movement updates only the previous tile, current tile, and
  genuinely changed status/detail content.
- [x] Unchanged details-pane and chrome regions are not retransmitted.
- [x] Text and icon raster work is bounded to dirty rectangles rather than
  rerunning the full shell painter for every transfer band.
- [x] Release firmware records raster time, transfer time, dirty pixel count,
  and total interaction latency over UART0.
- [x] Same-page selection movement reaches a measured target of 33 ms or less
  on the validated PicoCalc, or a follow-up issue records the measured hardware
  limit and accepted UX target. (Measured 24 ms at SPI 62.5 MHz, 2026-06-22.)
- [x] No full-screen SRAM framebuffer is introduced.

## Notes

Depends on KOTO-0119. Physical measurements on 2026-06-21 were approximately
339 ms for a full release redraw and 193–194 ms for a selection redraw while
the details pane was included. The LCD retains all untouched pixels in GRAM;
the backend must exploit that property.

## Resolution

Two changes exploit the retained GRAM:

- `ShellState::paint_rect` (koto-core) rasterizes only the components that
  intersect a clip rectangle. It shares all drawing code with the full
  `paint`/`paint_with` path — pixels remain `Canvas`-clipped — but the clip now
  also skips the CPU cost of the header, command bar, status strip, details
  pane, page indicator, and off-rectangle tiles. The firmware's strip loop calls
  `paint_rect` with each transfer band's viewport, so a selection redraw no
  longer reruns the whole shell painter ~20 times per frame. A new core test
  (`paint_rect_matches_full_paint_inside_clip_and_leaves_outside_untouched`)
  asserts the bounded output equals the full paint inside the clip and leaves
  every pixel outside it untouched.
- The selection command set is still produced by `render_selection_change`,
  which emits only the previous tile, current tile, and (when shown) the details
  pane plus the status strip — the header and command bar are never re-sent.

Product firmware now records a `PaintMetrics` breakdown per redraw and logs it
over UART0 as `raster_us`, `transfer_us`, `dirty_px`, and total `latency_ms`
(spanning state update, raster, and transfer) for the first, full, and dirty
redraw paths. No full-screen SRAM framebuffer is introduced; the 32-line / 20
KiB strip is unchanged.

Verified:

- `cargo test -p koto-core --offline`
- `cargo clippy -p koto-core --offline -- -D warnings`
- `cargo build -p koto-pico --bin koto_firmware --target thumbv6m-none-eabi --release --offline`
- `cargo clippy -p koto-pico --bin koto_firmware --target thumbv6m-none-eabi --release --offline -- -D warnings`
- `python harness/check_project.py`

## Hardware validation (2026-06-22)

First on-device UART0 capture of the bounded-raster build (pane hidden,
per-scanline transfer):

| Redraw            | dirty_px | raster_us | transfer_us | latency_ms |
| ----------------- | -------- | --------- | ----------- | ---------- |
| First full        | 102400   | 41081     | 216449      | 257        |
| Same-page select  | 17920    | ~10600    | ~39000      | 51         |
| Pane-shown select | 48632    | ~33300    | ~105500     | 140        |
| Pane toggle (full)| 102400   | ~40800    | ~216500     | 257–279    |

Findings:

- The dirty-rect raster is working: a same-page selection touches exactly
  17,920 px (two 80×84 tiles + the 320×14 status strip), down from the full
  102,400 px, and `paint_rect` keeps raster to ~10.6 ms.
- Transfer, not raster, dominates. At SPI 20 MHz the per-scanline path cost
  ~2.1 µs/px, of which only ~1.2 µs/px is raw SPI; the remaining ~96 µs/row was
  fixed DMA-setup overhead repeated for every scanline (~17 ms over a selection
  redraw).

Transport fix: `PicoCalcLcd::write_rgb565_rect` now converts the whole band into
a caller-owned RGB666 scratch buffer and ships it in a single DMA transfer
instead of one write per scanline. The firmware adds a 30,720-byte `RGB666_STRIP`
scratch (3 bytes/px for one 32-line strip); this is still far below the forbidden
204,800-byte full-screen framebuffer.

### Second hardware capture (2026-06-22, batched DMA)

| Redraw            | dirty_px | raster_us | transfer_us | latency_ms |
| ----------------- | -------- | --------- | ----------- | ---------- |
| First full        | 102400   | 41114     | 204984      | 246        |
| Same-page select  | 17920    | ~10600    | ~36040      | 48–49      |
| Pane-shown select | 48632    | ~33400    | ~97780      | 133        |

Batching removed only ~3 ms (the per-row DMA setup was smaller than estimated).
Transfer is a clean 2.0 µs/px across every redraw, decomposing into:

- ~1.2 µs/px raw SPI at the KOTO-0119-validated 20 MHz (RGB666 is 24 bits/px) —
  a hard floor at this clock.
- ~0.8 µs/px RGB565→RGB666 conversion on the CPU.

A same-page selection therefore costs ~48 ms: ~10.6 ms raster + ~21.5 ms SPI +
~14.3 ms conversion. The remaining ~25 ms of CPU (raster + conversion) and
~21.5 ms of SPI currently run serially per band.

Routes to the 33 ms target, in increasing effort:

1. Raise the SPI clock from 20 MHz (e.g. 40–62.5 MHz). Cuts the 1.2 µs/px SPI
   floor proportionally but needs physical signal-integrity validation; the
   20 MHz value is the conservative KOTO-0119-validated setting.
2. Pipeline conversion/DMA so the ~25 ms of CPU overlaps the ~21.5 ms of SPI
   (double-buffered RGB666 strips). No hardware risk; bounded by CPU at ~27 ms.
3. Accept the measured ~48 ms and record it with an agreed UX target in a
   follow-up issue (the escape hatch in acceptance criterion 5).

Chosen direction: route 1. `ILI9488_SPI.spi_hz` is raised to 62.5 MHz (the
RP2040 SPI ceiling, clk_peri 125 MHz / 2).

### Third hardware capture (2026-06-22, SPI 62.5 MHz) — target met

| Redraw            | dirty_px | raster_us | transfer_us | latency_ms |
| ----------------- | -------- | --------- | ----------- | ---------- |
| First full        | 102400   | 41091     | 64948       | 106        |
| Same-page select  | 17920    | ~10600    | ~11550      | 24         |
| Pane-shown select | 48632    | ~33300    | ~31290      | 66         |

Same-page selection is **24 ms**, comfortably under the 33 ms target, and the
panel showed no tearing or corruption at 62.5 MHz.

The measurement also corrects the earlier cost model. Transfer scaled from
36.0 ms to 11.5 ms when the clock went 20 MHz → 62.5 MHz — a 3.13× speedup that
matches the 3.125× clock ratio exactly. Transfer is therefore entirely
SPI-clock-bound; the RGB565→RGB666 conversion overlaps the DMA and is
effectively free, so the earlier "~0.8 µs/px conversion" split was wrong. The
only material CPU cost left is raster (~10.6 ms), which the dirty-rect bounding
already minimizes.

Because transfer is clock-bound and conversion is hidden, routes 2 (DMA/convert
pipelining) and dual-core offload are unnecessary for shell responsiveness. The
RP2040's second core stays reserved for the soft-PCM audio mixer
(REQUIREMENTS.md), where it does useful concurrent work.
