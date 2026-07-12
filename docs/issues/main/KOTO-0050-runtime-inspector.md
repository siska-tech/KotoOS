# KOTO-0050: Runtime Inspector

- Status: done
- Type: feature
- Priority: P1
- Requirements: FR-SIM-5, FR-RT-4, NFR-DEV-3, NFR-DEV-4
- Prerequisites: KOTO-0043

## Goal

Add a simulator-facing runtime inspector so developers can see the state of a
running bytecode app without guessing from screen output alone. The inspector
should expose enough VM, host-call, input, drawing, and file state to make
interactive app debugging practical.

## Acceptance Criteria

- [x] KotoSim can report app ID, VM run state, PC, frame fuel consumed, last host
      call, last VM error, and last input snapshot for the active app.
- [x] The inspector reports open sandboxed file handles and captured draw output
      counts without exposing host paths outside the sandbox model.
- [x] Inspector data is available in a deterministic text or structured output
      mode suitable for tests.
- [x] A test or scripted scenario proves inspector state updates after at least
      one yielded frame and one host call.

## Notes

This issue supports KOTO-0049. The first version can be CLI/text based; a visual
overlay can come later if needed.

Implemented as a CLI/text inspector. The VM (`koto-core`) now tracks the last
`HOST_CALL` id (`BytecodeVm::last_host_call`) and the fuel consumed in the most
recent frame (`BytecodeVm::last_frame_fuel`), and `host_call::name` maps ids to
short names. `BytecodeAppSession::inspect` in `koto-sim` returns an
`InspectorReport` combining VM state (run state, PC, fuel, last host call, last VM
error), the last `VmInputSnapshot`, and host counts (open sandboxed file handles
via occupancy only, plus captured `draw_rect`/`draw_text` counts) — no host paths
are exposed. `describe_inspector_report` renders one deterministic line, also
surfaced on the CLI via `--app APP_ID --inspect`, which prints the final-frame
inspector snapshot. `run_app_scenario` carries the final snapshot on
`AppScenarioReport::inspector`. Tests
`inspector_reports_vm_and_host_state_after_yielded_frame`,
`inspector_reports_open_sandboxed_file_handles`, and
`inspector_reports_last_vm_error_after_trap` prove the state transitions.
