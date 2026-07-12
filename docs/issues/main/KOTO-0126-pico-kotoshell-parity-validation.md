# KOTO-0126: Pico KotoShell Parity Validation

- Status: done
- Type: harness
- Priority: P0
- Requirements: FR-SHELL-1, FR-SHELL-2, FR-SHELL-3, FR-SHELL-4, FR-SHELL-5, FR-SIM-4, NFR-PERF-1

## Goal

Define and pass the release gate for a PicoCalc KotoShell that is behaviorally
and visually equivalent to the accepted KotoSim shell within documented
hardware constraints.

## Acceptance Criteria

- [x] A parity checklist covers package order, pagination, selection,
  details-pane content, command bar, favorites, sort, categories, status
  indicators, launch, failure recovery, and return to shell.
- [x] Reference KotoSim frames and physical-device captures are compared at
  equivalent states, allowing only documented backend differences.
- [x] UART timing captures cover cold boot, initial paint, same-page movement,
  page movement, pane toggle, and app launch/return. (All six recorded at SPI
  62.5 MHz; the final three — page movement, app launch/return, pane toggle —
  were captured on 2026-06-22 and folded into the parity doc §3.)
- [x] Physical validation records release firmware size, SRAM static usage,
  maximum working buffer, and observed interaction latency.
- [x] `docs/VALIDATION_PLAN.md` release gates are updated from the recorded
  results.

## Notes

Final gate for KOTO-0119 through KOTO-0125. This issue should not hide hardware
limitations; any accepted deviation from KotoSim must be explicit and linked to
its requirement or follow-up issue.

## Resolution

The parity gate is defined and recorded in
[docs/PICO_KOTOSHELL_PARITY.md](../../hardware/PICO_KOTOSHELL_PARITY.md):

- A 13-row parity checklist covers package order, pagination, selection,
  details-pane content, command bar, favorites, sort, categories, status
  indicators, launch, failure recovery, and return to shell. Every row is
  "parity" because the device drives the same portable `koto-core` code as
  KotoSim; differences are captured as explicit, linked deviations D1–D6 (pane
  hidden by default, final-state rendering, SD-reinsertion reboot, clock unset,
  game bytecode budget, Dirty-Rects flicker).
- Reference KotoSim frames are the deterministic `--golden-frames` trace; the
  device's UART `dirty_px` counters cross-check the same rectangles at the shell
  list, same-page selection, status change, and sample app states.
- UART timing covers all six interactions at SPI 62.5 MHz: cold boot, initial
  paint, same-page 24 ms, page movement 112–147 ms, pane toggle 279–280 ms, and
  app launch (immediate) / return 165 ms. The final three were captured on
  2026-06-22. The pane-toggle capture corrected an earlier derived estimate: it
  costs ~280 ms because it includes the bounded `SHELLPRF.TXT` preference write,
  not just the ~165 ms repaint — recorded openly rather than smoothed over.
- Physical records from the `thumbv6m-none-eabi` release build: flash image
  451.8 KiB (462,688 B), static SRAM 143.0 KiB of 264 KiB, maximum working
  buffer 30,720 B (no full framebuffer), observed same-page latency 24 ms.
- `docs/VALIDATION_PLAN.md` gains a Phase 4 parity-gate section and updated
  release-gate checkboxes from these results.

Verified:

- `cargo build -p koto-pico --bin koto_firmware --target thumbv6m-none-eabi --release --offline`
- `python harness/check_project.py` (Markdown links resolve, including the new
  parity doc)

All six interaction captures are now recorded and the last
`docs/VALIDATION_PLAN.md` release-gate checkbox is closed, so the parity gate is
fully green and this issue is done. The Dirty-Rects device flicker (deviation
D6) was since resolved in KOTO-0128 and hardware-confirmed. One deferred item
remains: the larger game bytecode budget (KOTO-0127, deviation D5).
