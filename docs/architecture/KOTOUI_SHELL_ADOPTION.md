# KotoUI Shell Adoption

KOTO-0216 introduces KotoUI into a bounded production surface without changing
the KotoShell application model or its device/simulator rendering boundary.

## Adopted Regions

- Details pane frame/background: KotoUI `Panel`.
- Details title, favorite marker, and metadata label/value rows: KotoUI `Label`.
- Command-bar key chips: KotoUI `Button`.
- Command-bar action labels and state suffixes: KotoUI `Label`.
- Command activation: a physical key is mapped to `ShellCommandId`, then routed
  through the corresponding KotoUI `Button` as a normalized `UiEvent`.

The controls are ephemeral borrowed views over `ShellState`. Package, favorite,
sort, category, pane, and system-view state remain owned by `ShellState`, so the
pilot adds no retained component fields or allocation.

## Remaining Bespoke Shell Regions

The launcher package grid remains bespoke because KotoUI v1 has a linear list,
not a spatial grid. Header/status indicators, package icons, wrapped description
layout, animation, and the system/memory page also retain their existing Shell
painters. A later migration should add a suitable component contract before
moving these regions; it should not imitate a grid with independent buttons.

## App-Side Boundary

KotoUI is currently a native Rust UI toolkit used by OS-owned surfaces such as
KotoShell and the simulator Gallery. Koto apps continue to use the existing SDK
host calls and KotoGFX drawing model; they do not construct KotoUI controls and
no KotoVM declarative UI ABI is introduced by this pilot. Exposing components to
apps requires a separate ABI/lifecycle design covering stable widget identity,
bounded app-owned state, event delivery, damage, and compatibility. That work is
tracked by the [KotoUI App ABI roadmap](../planning/KOTOUI_APP_ABI_ROADMAP.md).

## Performance Record

| Measure | Before | After | Delta |
| :-- | --: | --: | --: |
| Pico release `.text` | 345,132 B | 347,060 B | +1,928 B |
| Pico release `.rodata` | 411,744 B | 411,776 B | +32 B |
| Pico release `.data` | 54,588 B | 54,588 B | 0 B |
| Pico release `.bss` | 175,032 B | 175,032 B | 0 B |
| `ShellState` (64-bit host) | 29,672 B | 29,672 B | 0 B |
| Maximum render commands | 16 | 16 | 0 |

The existing selection trace still repaints the previous tile, current tile,
details pane, and status strip. Page changes still repaint the grid area. Idle
behavior and command-list construction are unchanged. The simulator golden
frame trace is byte-for-byte unchanged.

## Device Validation Checklist

- [x] Boot to Shell and navigate in all four directions across a page boundary.
- [x] Press Enter and confirm the selected package launches with the confirm sound.
- [x] Press F2 and verify favorite star, ordering where applicable, details, and
  persisted preference after reboot.
- [x] Press F3/F4 and verify sort/category state and command enablement.
- [x] Press Backspace twice and verify details closes/reopens with the cancel sound.
- [x] Press F5, verify the system page, then return with F5 or Enter.
- [x] Exit an app and verify Shell selection, command shortcuts, and rendering still
  work without full-screen corruption.

All checklist items were confirmed on PicoCalc hardware by the project owner on
2026-07-15.
