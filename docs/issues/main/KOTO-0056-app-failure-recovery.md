# KOTO-0056: App Failure Recovery Screen

- Status: done
- Type: feature
- Priority: P1
- Requirements: FR-SHELL-2, FR-RT-4, FR-SIM-5, NFR-REL-1, NFR-DEV-3
- Prerequisites: KOTO-0043

## Goal

Ensure bytecode app failures do not look like simulator or OS hangs. When an app
fails verification, traps at runtime, exhausts fuel according to policy, or
receives a fatal host-call failure, KotoOS should return to a controlled shell
state and show a readable error summary.

## Acceptance Criteria

- [x] Runtime verification failure, VM trap, and app exit are represented as
      distinct shell-visible outcomes.
- [x] KotoSim can render or print an app failure summary with app ID, failure
      kind, and diagnostic detail.
- [x] The shell can return to the app list after a failed app without losing its
      package list state.
- [x] Tests cover at least one bad bytecode package and one runtime trap package.

## Notes

This is product polish and reliability infrastructure at the same time. It helps
users evaluate KotoOS without interpreting a blank or frozen window.

Completed: app execution failures now flow through `AppFailureSummary`, which
separates app ID, failure kind, and diagnostic detail for both headless CLI output
and the interactive window failure screen. `run_app_scenario` reports verifier
failures as `verification-failed` summaries, VM traps as `runtime-trap` summaries
with retained diagnostics/source locations, and successful app exits remain
normal scenario reports. Window mode keeps the existing shell state and returns to
the app list with Backspace after launch or runtime failure.
