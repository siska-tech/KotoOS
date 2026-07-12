# KOTO-0049: KotoSim App Development Experience

- Status: done
- Type: feature
- Priority: P1
- Requirements: FR-SIM-1, FR-SIM-2, FR-SIM-3, FR-SIM-5, NFR-DEV-3, NFR-DEV-4
- Prerequisites: KOTO-0043, KOTO-0046

## Goal

Make KotoSim useful as the primary development and user-evaluation surface for
bytecode apps, including direct app launch, readable diagnostics, and repeatable
interactive scenarios.

## Acceptance Criteria

- [x] KotoSim can launch a specific app by app ID from the command line without
      manually navigating the shell.
- [x] Runtime failures report app ID, bytecode PC, host-call ID when relevant,
      VM error, and source location when debug data is available.
- [x] Scripted scenarios can drive shell launch, text input, IME conversion,
      save, exit, and relaunch.
- [x] Window mode routes the same input model used by scripted scenarios.
- [x] Documentation explains the app development loop from source edit to
      simulator run.

## Notes

This is the step that makes implemented features touchable. The simulator should
remain faithful to the runtime path rather than replacing app behavior with
native Rust shortcuts.

Implemented in `koto-sim`: `--app APP_ID` launches a single app headlessly (via
`run_app_scenario` over a throwaway sdcard copy) and `--app-script PATH` drives it
one `VmInputSnapshot` per frame, parsed by `parse_app_script` from single-quoted
characters and intent names — the same input model window mode feeds the live VM.
`BytecodeAppSession` retains the faulting PC and VM error, surfaced as an
`AppDiagnostic` ("app … trapped at frame N pc M: <error>"); bytecode-PC-to-source
mapping arrives with debug data in KOTO-0051. The loop is documented end to end in
[docs/APP_DEV_LOOP.md](../../guides/APP_DEV_LOOP.md).

Related focused issues:

- [KOTO-0050](KOTO-0050-runtime-inspector.md) for VM and host state inspection.
- [KOTO-0051](KOTO-0051-bytecode-debug-data.md) for source-location diagnostics.
- [KOTO-0056](KOTO-0056-app-failure-recovery.md) for controlled app failure
  presentation and shell recovery.
- [KOTO-0058](KOTO-0058-golden-frame-validation.md) for stable frame regression
  checks.
