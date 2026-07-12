# PicoCalc KotoShell Parity Validation

Release gate for KOTO-0126. This document defines and records the gate for a
PicoCalc KotoShell that is behaviorally and visually equivalent to the accepted
KotoSim shell, within documented hardware constraints. It is the final gate for
KOTO-0119 through KOTO-0125.

The shell rendering, state machine, manifest model, preference model, IME, and
bytecode runtime are shared portable code in `koto-core`. KotoSim and the
PicoCalc firmware provide only platform adapters. Parity therefore means the
device exercises the *same* portable code paths as the simulator and differs
only where a hardware constraint is explicitly recorded below.

- Status date: 2026-06-22
- Reference build: `koto-sim --golden-frames`
  (`harness/fixtures/golden_frames/sim.trace`)
- Device build: `koto_firmware`, `thumbv6m-none-eabi` release
- SPI clock: 62.5 MHz (KOTO-0120 third capture)

## 1. Parity Checklist

Each row states the KotoSim reference behavior, the PicoCalc behavior, the
verdict, and the evidence (UART phase and/or source issue). "Same" means the
device drives the identical portable `koto-core` path; any difference is an
explicit, linked deviation in section 5.

| # | Area | KotoSim reference | PicoCalc behavior | Verdict | Evidence |
| :- | :--- | :---------------- | :---------------- | :------ | :------- |
| 1 | Package order | `PackageList` insertion order, then `ShellState` sort/order | Same `PackageList` + `ShellState` ordering; SD scan fills the list | Parity | KOTO-0121 `phase=14 catalog-ready packages=15` |
| 2 | Pagination | 3×3 launcher grid, page indicator from `ShellState` | Same grid and page indicator painted by `ShellState::paint` | Parity | KOTO-0119 |
| 3 | Selection | `render_selection_change` emits prev + current tile (+ pane, status) | Same; `paint_selection_change` transfers only those rects | Parity | KOTO-0120 `dirty_px=17920` |
| 4 | Details-pane content | Shared pane painter; visible by default | Same painter; **hidden by default**, toggled with X/Escape | Parity (deviation D1) | KOTO-0119; [shell-detail-pane-toggle] |
| 5 | Command bar | Shared command-bar labels/availability from `ShellState` | Same labels/availability via the shared state machine | Parity | KOTO-0123 |
| 6 | Favorites | `toggle_selected_favorite`; persists | F2 → `toggle_selected_favorite`; persists to `SHELLPRF.TXT` | Parity | KOTO-0123 `phase=143 prefs-saved` / `phase=142 prefs-applied` |
| 7 | Sort | `cycle_sort` (Default/Name/Favorite) | F3 → `cycle_sort`; persists | Parity | KOTO-0123 |
| 8 | Categories | `cycle_category` filter | F4 → `cycle_category`; persists | Parity | KOTO-0123 |
| 9 | Status indicators | Storage / power / save from shared `ShellState` | SD detect, STM32 battery, save-health drive the same indicators | Parity (deviations D3, D4) | KOTO-0124 `phase=145 power-poll`, `phase=148/149` |
| 10 | Icons & metadata | Names, descriptions, categories, themes, `.kicon`, JP text | Same portable model; `.kicon` + theme parsed on device | Parity | KOTO-0122 `phase=140 icons-loaded count=15` |
| 11 | Launch | `ShellAction::Launch` → shared `BytecodeSession` | Confirm → same `BytecodeSession` lifecycle | Parity (deviation D5) | KOTO-0125 `phase=150/152` |
| 12 | Failure recovery | Verify/VM/trap errors return to a usable shell | Same; returns to shell with UART diagnostic, no reset | Parity | KOTO-0125 `phase=254/256/257`; KOTO-0121 `phase=19x` fallbacks |
| 13 | Return to shell | App exit repaints KotoShell | Exit → `paint_shell` repaint (`phase=30 ready app-return`) | Parity | KOTO-0125 |

[shell-detail-pane-toggle]: the home-screen right pane is a show/hide toggle,
not a permanent fixture, on both platforms.

## 2. Reference Frame vs Device Capture

Reference KotoSim frames are the deterministic render-command trace produced by
`cargo run -q -p koto-sim -- --golden-frames` and checked by
`harness/check_golden_frames.py` against
`harness/fixtures/golden_frames/sim.trace`. The trace records the exact dirty
rectangles for the shell list, a selection-feedback step, and a sample app
frame, in logical 320×320 `Rgb565` coordinates.

The device renders those same rectangles because it calls the same
`ShellState::paint` / `paint_rect` / `render_selection_change` code. The
device's UART `dirty_px` counters are the per-state cross-check:

| State | KotoSim reference rects (sim.trace) | Device dirty pixels | Match |
| :---- | :---------------------------------- | :------------------ | :---- |
| Shell list (full) | full 320×320 grid + chrome | 102,400 px first full redraw | Yes |
| Same-page selection | two 65×84 tiles + status strip | 17,920 px (two 80×84 tiles + 320×14 strip) | Geometry-equivalent (device uses full-width tiles, pane hidden) |
| Status change | system-status cluster | 2,400 px (120×20 cluster) | Yes |
| Sample app frame | `draw_rect` clear + two `draw_text` | same command list via `DeviceRuntimeHost` | Yes |

Visual capture: launcher with 15 themed `.kicon` assets was visually confirmed
to match the simulator (KOTO-0122). The tile width differs (full-width device
layout vs. pane-sharing sim layout) because the details pane is hidden by
default on device — deviation D1, not a rendering defect.

## 3. UART Timing Captures

All device timings below are from on-device UART0 captures (115200 8N1) on the
validated PicoCalc, 2026-06-22, at SPI 62.5 MHz unless noted. `latency_ms`
spans state update + raster + transfer.

| Interaction | UART phases | raster | transfer | latency | Source |
| :---------- | :---------- | :----- | :------- | :------ | :----- |
| Cold boot → ready | `10`→`131`/`181`/`132`→`137`/`139`/`140`→`14`→`20`→`30 first` | — | — | boot to first paint | KOTO-0121/0122 |
| Initial paint (full, fallback icons) | `phase=30 ready first` | 41 ms | 65 ms | **106 ms** | KOTO-0120 |
| Initial paint (full, 15 themed `.kicon`) | `phase=30 ready first` | ~92 ms | ~73 ms | ~165 ms | KOTO-0122/0126 |
| Same-page movement | `phase=40 dirty-redraw`→`30 ready dirty` | ~10.6 ms | ~11.5 ms | **24 ms** | KOTO-0120 |
| Page movement | `phase=40 dirty-redraw`→`30 ready dirty` | 43–79 ms | ~65 ms | **112–147 ms** (`dirty_px=90240`) | KOTO-0126 |
| Pane toggle (full + pref write) | `phase=40 full-redraw`→`30 ready full` | ~92 ms | ~73 ms | **279–280 ms** | KOTO-0126 |
| Storage status change | `phase=148/149`→`30 ready storage-status` | — | — | 4 ms (`dirty_px=2400`) | KOTO-0124 |
| App launch → start | `phase=150`→`152 app-started` | — | — | immediate (no measurable paint) | KOTO-0126 |
| App return → shell | `phase=153`→`30 ready app-return` | ~92 ms | ~73 ms | **165 ms** (`dirty_px=102400`) | KOTO-0126 |

Capture notes (KOTO-0126, 2026-06-22, SPI 62.5 MHz, 15 themed icons):

- **Page movement** takes the dirty-redraw path, not a forced full redraw, but
  every tile's content changes so it touches `dirty_px=90240` (~88% of the
  screen) — far above the 17,920 px same-page case. Its 112–147 ms is therefore
  expected and outside the 33 ms same-page UX target, which by design only
  bounds same-page selection (KOTO-0120). KotoSim repaints the whole grid on a
  page change as well, so this is parity, not a device-only cost.
- **Pane toggle** is a preference change: the 279–280 ms spans the
  `SHELLPRF.TXT` SD write *and* the full themed repaint. Raster (~92 ms) +
  transfer (~73 ms) account for ~165 ms; the remaining ~115 ms is the
  bounded preference write (`phase=143 prefs-saved`) before the redraw.
- **App return** is a full themed repaint (`dirty_px=102400`) at 165 ms; the
  app launch itself (`phase=150`→`152`) shows no measurable paint cost. Both the
  Sample App and Hello Text samples exit cleanly (`code=0`) and repaint the
  shell.

Cold-boot sequence on the validated card: `phase=131 sd-card-init-start` →
`phase=181 sd-fast-init-failed` → `phase=132 sd-card-init-ok clock=1000000` →
`phase=137 apps-list-ok manifests=15` → `phase=139 manifest-read-done
accepted=15` → `phase=140 icons-loaded count=15` → `phase=14 catalog-ready
packages=15`.

## 4. Physical Validation Records

Measured from the `thumbv6m-none-eabi` release ELF
(`target/thumbv6m-none-eabi/release/koto_firmware`) on 2026-06-22.

| Metric | Value | Notes |
| :----- | :---- | :---- |
| Release firmware flash image | 451.8 KiB (462,688 B) | `.boot2` + `.vector_table` + `.text` 181 KiB + `.rodata` 270 KiB; UF2 payload 462,848 B |
| Static SRAM (`.bss`/`.uninit`) | 143.0 KiB (146,428 B) | of the RP2040's 264 KiB; ~118 KiB left for stack + heap |
| Maximum working buffer | 30,720 B | `RGB666_STRIP` (one 320×32 strip @ 3 B/px); no 204,800 B full framebuffer |
| Other notable static buffers | — | `RASTER_STRIP` 20,480 B, `APP_HEAP` 16,384 B, `APP_BYTECODE` 8,192 B, `KICON_SCRATCH` 2,048 B, `MANIFEST_BYTES` 2,304 B |
| Catalog working set | ~27 KiB | `PackageList` owned local (32 × `PackageInfo`) |
| Observed interaction latency | 24 ms same-page selection | under the 33 ms UX target (KOTO-0120) |

The large `.rodata` is dominated by the embedded M+ bitmap font and the SKK
dictionary; both are required for shell + IME parity with KotoSim.

## 5. Accepted Deviations from KotoSim

Each deviation is explicit and linked to a requirement or follow-up. None are
hidden; all are bounded.

- **D1 — Details pane hidden by default.** The device uses the shared shell's
  full-width launcher layout so normal navigation stays at two dirty tiles plus
  the status strip. The pane is the same portable painter and is shown with
  X/Escape. (KOTO-0119; FR-SHELL-3, NFR-PERF-1.)
- **D2 — Final-state rendering, no multi-frame animation.** The device presents
  the final shell state immediately (`advance_feedback` is fast-forwarded);
  KotoSim retains the multi-frame selection/launch animations. Behaviorally
  equivalent end state. (KOTO-0120; FR-SHELL-3.)
- **D3 — SD reinsertion requires reboot.** Removal is a fail-safe transition
  (`SD×`, launches/writes disabled); reinsertion shows `SD?` but automatic FAT
  remount is deliberately unsupported this increment. (KOTO-0124; FR-FS-1.)
- **D4 — Clock unset.** The available STM32 protocol exposes no clock source, so
  the shell clock stays visibly unknown rather than inventing data. (KOTO-0124;
  FR-SHELL-5.)
- **D5 — Game packages exceed the device bytecode budget.** Games (20–96 KiB
  bytecode) fail launch with `phase=253 launch-bytecode-oversize` against this
  slice's deliberate 8 KiB bytecode buffer and 16 KiB heap. Running them needs a
  deliberate RP2040 SRAM budget re-sizing, tracked in
  [KOTO-0127](../issues/main/KOTO-0127-pico-large-bytecode-budget.md). (KOTO-0125.)
- **D6 — Dirty-Rects sample flicker (resolved).** The Dirty Rects SDK sample
  previously flickered on the device LCD adapter only. Fixed in
  [KOTO-0128](../issues/main/KOTO-0128-pico-app-frame-flicker.md): the app present path
  now composites partial-background frames off-screen and transfers atomically,
  so the moving square no longer flickers (hardware-confirmed 2026-06-22). No
  longer an accepted deviation. (KOTO-0125 / KOTO-0120.)

## 6. Capture Completeness

All six required interaction captures are now recorded from the validated
PicoCalc at SPI 62.5 MHz: cold boot, initial paint, same-page movement, page
movement, pane toggle, and app launch/return (section 3). The three captures
that were previously open — page-movement latency, app launch/return latency,
and a direct pane-toggle capture at 62.5 MHz — were recorded on 2026-06-22 and
folded into section 3.

The captures confirmed one correction to an earlier derived estimate: a pane
toggle costs ~280 ms, not the ~106 ms previously extrapolated, because the
interaction includes the bounded `SHELLPRF.TXT` preference write in addition to
the full themed repaint. This is recorded openly rather than smoothed over; the
write is bounded and recoverable (KOTO-0123) and the redraw itself is the same
~165 ms full-screen cost as an app return.
