# KOTO-0216: KotoShell pilot adoption of KotoUI

- Status: done
- Type: feature
- Priority: P2
- Requirements: FR-SHELL-2, FR-SHELL-3, FR-SHELL-4, NFR-PERF-1, NFR-DRAW-1
- Related: KOTO-0057, KOTO-0086, KOTO-0120, KOTO-0214, KOTO-0215
- Roadmap: [KotoUI GUI Component Roadmap](../../planning/KOTOUI_ROADMAP.md)

## Goal

Validate KotoUI in production by migrating the bounded KotoShell details pane
and command-bar controls while preserving launcher behavior, appearance intent,
and dirty-rectangle performance.

## Acceptance Criteria

- [x] Replace duplicated label/panel/button-like painting in the details pane
  and command bar with KotoUI components; package grid navigation remains out of
  scope.
- [x] Existing package metadata, favorite/category/sort state, command enablement,
  shortcuts, launch behavior, and system-page behavior remain unchanged.
- [x] Shell input mapping drives the component event model without adding a
  second device-specific input path.
- [x] Selection, focus, command enablement, and action responses preserve current
  Shell semantics and sound triggers.
- [x] Idle Shell frames add no render commands; selection and command-state
  changes repaint no larger region than before unless documented with evidence.
- [x] Simulator golden frames and Shell behavior tests are updated only for
  intentional component-style differences.
- [x] Compare before/after release code size, Shell state size, maximum render
  command count, and representative dirty-rectangle traces.
- [x] The Pico firmware builds and a device validation checklist covers launch,
  navigation, command shortcuts, details visibility, and return to Shell.
- [x] KotoUI adoption and the remaining bespoke Shell regions are documented so
  later migrations do not duplicate component logic.
- [x] Workspace tests, simulator fixtures, golden-frame checks,
  `cargo check -p koto-pico --target thumbv6m-none-eabi`, and
  `python harness/check_project.py` pass.

## Notes

The package grid is intentionally excluded because KotoUI v1 has a linear list,
not a spatial grid component. Memo migration and a VM-facing declarative UI ABI
require separate follow-up decisions after this pilot.

## Implementation Evidence

- The details pane uses KotoUI `Panel` and `Label`; command key chips use
  `Button`, with adjacent labels/state rendered by `Label`. The Shell-derived
  theme keeps the existing RGB565 output and geometry; the simulator golden
  trace is unchanged.
- `ShellCommandId` is the shared semantic boundary. Enter/Backspace and both
  simulator and Pico F2-F5 mappings activate the same KotoUI button event path.
- The `ShellState` layout is unchanged: 29,672 bytes on the 64-bit host. The
  maximum render-list capacity remains 16 commands, and existing exact dirty
  rectangle assertions remain unchanged.
- Pico release comparison (`thumbv6m-none-eabi`): `.text` 345,132 -> 347,060
  bytes (+1,928), `.rodata` 411,744 -> 411,776 (+32), `.data` 54,588 and `.bss`
  175,032 unchanged.
- `cargo check -p koto-pico --target thumbv6m-none-eabi`, the Pico release
  build, Shell tests, KotoUI/Core/Simulator clippy, simulator golden checks, and
  `python harness/check_all.py` pass on 2026-07-15 after synchronizing the
  committed KPA packages.
- PicoCalc hardware validation passed on 2026-07-15: Shell navigation, launch,
  F2-F5 shortcuts, details visibility, system-page return, and return from an
  app were confirmed by the project owner.
- Adoption boundary and device checklist:
  [KotoUI Shell adoption](../../architecture/KOTOUI_SHELL_ADOPTION.md).
