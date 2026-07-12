# KOTO-0035: KotoSim Runtime Launch Path

- Status: done
- Type: feature
- Priority: P1
- Requirements: FR-SHELL-2, FR-RT-1, FR-RT-4, FR-SIM-3, FR-SIM-4, FR-SIM-5, FR-PKG-1

## Goal

Connect KotoShell launch actions in KotoSim to the first bytecode runtime path:
load the selected package manifest entry, open its bytecode asset from
`sdcard_mock`, verify it, run one bounded frame, and report runtime outcome in a
headless harness-friendly form.

## Acceptance Criteria

- [x] KotoSim can resolve the selected package manifest `runtime` and `entry`
      fields without losing the existing package list and icon behavior.
- [x] Launching a `kotoruntime-bytecode` package reads the entry asset through
      the host filesystem adapter, verifies it with KOTO-0033, and starts the VM
      from KOTO-0034.
- [x] A minimal sample bytecode fixture is added under `sdcard_mock` or
      `harness/fixtures` and can exit cleanly from the simulator.
- [x] The headless simulator prints a deterministic launch/runtime result that
      can be asserted by tests or the project harness.
- [x] Unsupported runtimes, missing entry assets, verifier errors, and VM errors
      are reported without crashing the shell.

## Notes

Depends on KOTO-0033 and KOTO-0034. This issue is the first visible MVP thread:
it turns the launcher from "select package" into "start a sandboxed app" while
still keeping all behavior host-testable before real hardware arrives.

Implemented through `koto_sim::launch_package`, which reloads the selected
manifest by `app_id`, reads the package entry through `HostFs`, verifies `KBC1`,
and executes one bounded VM frame. `sdcard_mock/bytecode/main.kbc` and
`sdcard_mock/bytecode/memo.kbc` are minimal exit fixtures for headless smoke
testing.
