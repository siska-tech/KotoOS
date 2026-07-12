# KOTO-0024: Power Status Model and Shell Indicator

- Status: done
- Type: feature
- Priority: P1
- Requirements: FR-SHELL-5, FR-SDK-7, NFR-REL-4

## Goal

Represent optional power/battery information and expose it to KotoShell.

## Acceptance Criteria

- [x] Core model distinguishes unsupported, unknown, charging, and percent states.
- [x] Shell status text can include battery state.
- [x] Tests cover unavailable and low-battery cases.

## Notes

Actual STM32 polling belongs to the device backend.
