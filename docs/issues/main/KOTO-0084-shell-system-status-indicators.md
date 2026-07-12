# KOTO-0084: Shell System Status Indicators

- Status: done
- Type: feature
- Priority: P1
- Requirements: FR-SHELL-5, FR-SDK-7, FR-SIM-1

## Goal

Make the shell status bar show compact system state: time, battery, SD/storage,
and save/health indicators.

## Acceptance Criteria

- [x] Battery state renders as an icon gauge plus a compact percentage when
  available (green normal, red low).
- [x] Missing or unsupported battery data degrades to a `--%` placeholder and a
  hollow gauge.
- [x] SD/storage availability has a visible indicator (`SD` / `SD×` / `SD?`).
- [x] The clock area is deterministic: it renders an injected `ShellClock`
  (`YYYY/MM/DD HH:MM`) or a `----/--/-- --:--` placeholder when unset.
- [x] Tests cover normal, low battery, unknown battery, and unsupported states.

## Notes

The current shell can show battery text. This issue moves that into a stronger
visual status model without requiring real hardware battery reporting yet.

Implemented: header now draws a home icon + title, a centered clock, and a
right-hand cluster (battery gauge + percent, storage badge, save badge). The
clock/storage/save state is injected via `ShellState::set_clock`,
`set_storage_status`, and `set_save_status`; the simulator sets a fixed clock and
`Present`/`Saved` for the demo views. A real-time clock source and live save/SD
wiring are left to the embedded HAL bring-up.
