# KOTO-0182: SRAM/PSRAM memory status visibility tool

- Status: done (device-confirmed 2026-07-12)
- Type: feature
- Priority: P2
- Related: KOTO-0170/KOTO-0172 (stack canary + free_min — the numbers already
  exist as UART `phase=176` output), KOTO-0101 (runtime budget diagnostics),
  KOTO-0084 (shell system status indicators — a natural surface).

## Goal

There is no tool that shows SRAM/PSRAM memory state. The data mostly exists
(stack-canary `free_min`, heap peaks from budget diagnostics, code-window/
PSRAM slot state) but only as UART diagnostics that require a probe session
and grep. Make memory visible on-device and in the sim.

## Shape (to be designed)

Candidates, not mutually exclusive:

1. **Shell status view** — a page/pane reachable from the home screen:
   SRAM free_min, main-task stack headroom, app heap peak of the last run,
   PSRAM slots in use / code-window residency.
2. **Overlay hotkey** during app execution (dev builds only) — the KotoRun
   perf work proved small always-on overlays are affordable.
3. **Host-side viewer** — a tool that tails the UART `phase=` stream and
   renders live memory graphs on the dev machine (zero device cost; dev-only).

## Decision (surface + rationale)

**Candidate 1 — a shared shell status view** in `koto-core`, reachable from the
home screen (a full-screen overlay toggled by `F5 / システム` on the command bar),
painted identically on device and sim. Confirm/cancel — or F5 again — dismisses
it back to the launcher.

Rationale: it is the only candidate that meets "visible on-device without a UART
grep", and because the shell is shared it gets sim parity (and golden/screenshot
testability) for free. Candidate 2 (dev overlay during app execution) adds
always-on cost on the app hot path; candidate 3 (host UART viewer) is dev-only
and does not satisfy the on-device criterion. Both remain viable follow-ups but
are out of scope for this cut. PSRAM detail is **presence + total size + static
code-window slot count** — the feature-gated live residency/refill counters are
deferred.

## Implementation

- `koto-core` (`src/koto-core/src/shell.rs`): new `MemoryStatus` struct +
  `ShellState::set_memory_status` / `toggle_system_view`; overlay painter
  `paint_system_view` (branch in `paint_region`); `F5 システム` command chip
  (`SHELL_COMMAND_COUNT` 5→6). `free_min` is drawn red below an 8 KiB caution
  band (KOTO-0170 treats <~4 KiB as stop-ship). Unit tests cover the KiB
  formatter, the toggle/dismiss state machine, and the command chip.
- Firmware (`src/koto-pico/src/bin/koto_firmware.rs`): fills `MemoryStatus` from
  `stack_canary::scan()` + new `stack_canary::static_used()`/`SRAM_TOTAL`, the
  audio backend `core1_stack_free_min`, the app-heap span, and
  `PSRAM_CAPACITY`/`CODE_WINDOW_TILES`; `KEY_F5` (`0x85`) toggles the view.
- Sim (`src/koto-sim/src/main.rs`, `window.rs`): injects a representative
  snapshot; F5 in the window loop toggles it; `--system-view` renders it for
  `--image` screenshots.

## Acceptance Criteria

- [x] Decide the surface(s) with a short design note (device UI vs host tool
      vs both) and record the rationale here.
- [x] SRAM free/min and PSRAM usage visible without a UART grep session.
      (Device F5 overlay + sim; sim screenshot verified.)
- [x] Numbers cross-checked once against the existing `phase=176` /
      budget-diagnostic UART output on hardware. (`free_min`/`used` share the
      same `stack_canary::scan()` source as `phase=176`, so they match by
      construction; confirmed on hardware 2026-07-12.)
