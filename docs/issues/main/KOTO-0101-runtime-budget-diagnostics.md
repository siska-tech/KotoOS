# KOTO-0101: Runtime budget diagnostics

- Status: done
- Type: harness
- Priority: P1
- Requirements: NFR-MEM-1, NFR-MEM-2

## Goal

Expose per-app VM and host working-set usage so KotoOS can validate SRAM, heap,
local-slot, stack, and frame-fuel budgets before device bring-up — turning "it
runs" into "this VM profile continues to hold" as features (larger SKK dictionary,
Game2D host service, audio multi-track, PSRAM cache) are added.

## Acceptance Criteria

- [x] KotoSim prints a per-app VM budget report after a scripted run (`--budget`).
- [x] The report includes stack peak, call-depth peak, local-slot peak, app-heap
  request/peak, frame-fuel peak, and host-calls-per-frame peak.
- [x] Render/file/audio host working sets are reported as category-level peaks.
- [x] Memo and KotoBlocks have worst-ish scripted scenarios that emit reports.
- [x] Reports distinguish SRAM-resident VM state (stack/locals/heap) from the
  per-frame fuel budget and from host-owned working sets (pixel/PCM bytes never
  live in the VM heap).
- [x] `check_all.py` runs a budget gate that can fail when an app exceeds its
  configured threshold, and warns at >=90% of a fixed capacity.

## Design

### VM instrumentation (`koto-core`)

`BytecodeVm` accumulates a `VmBudget` of session high-water marks, never reset:
operand-stack depth, call depth, highest local slot touched (+1), highest heap
byte addressed by `LOAD*`/`STORE*`, instructions stepped in the busiest frame, and
host calls dispatched in the busiest frame. Heap usage is measured as the
directly-addressed high-water: host calls move data through caller-supplied heap
buffers, but the app writes those buffers (then the host reads) or reads them back
(after the host writes), so the addressed high-water captures the heap the program
actually uses.

### Report and capacities (`koto-sim`)

`BytecodeAppSession::budget()` pairs the VM peaks with the canonical simulator
profile capacities (`RuntimeLimits::simulator_default()` — 16 stack slots, 4 call
frames, 60 000 fuel), `VM_LOCAL_SLOTS` (48), the program's KBC heap request, and
the manifest's declared `sram_work_bytes` SRAM ceiling. It also tracks host
working-set peaks (open files, draw-rects, draw-pixels, text-draws, audio-events),
captured at frame end because the host clears its per-frame draw lists each frame.
`describe_app_budget_report` renders one deterministic `key=value` line; `--budget`
prints it after the scenario report, and `AppScenarioReport.budget` carries it.

### Scenarios and gate (`harness`)

`harness/fixtures/budget/{memo,koto_blocks}.script` walk each app's busiest paths
(Memo: IME conversion + candidate navigation, multi-line growth, editing, save-as
prompt, open dialog; KotoBlocks: title bake, start, move/rotate/soft-drop/hard-drop,
hold, pause+resume, top-out, retry). `harness/check_budgets.py` runs each scenario,
parses the budget line, warns at >=90% of a fixed capacity, and fails when a peak
exceeds the per-scenario threshold. Thresholds carry headroom over today's peaks so
the gate is green now but catches regressions. Wired into `check_all.py`.

## Notes

- Current apps compile with every function **inlined** (no `CALL`/`RET` emitted),
  so `call_peak` is 0 and all locals flatten into `main`'s frame. The flip side:
  KotoBlocks sits at `local_peak` 48/48 — the VM local ceiling — which the gate
  reports as a standing 100% warning. That is the live signal to teach the compiler
  per-scope reuse before adding locals, not to raise `VM_LOCAL_SLOTS`
  (see KOTO-0092 and the budget-sizing rationale).
- Measured worst-ish peaks: Memo `local 18/48`, `heap 1111/24576`, `fuel 2454`;
  KotoBlocks `local 48/48`, `heap 3981/24576`, `fuel 44277/60000` (the hard-drop
  frame is the fuel high-water).
- A device-side memory diagnostic screen (SRAM usage + subsystem high-water over
  USB CDC) is deferred to the Embedded Bring-Up track as a separate issue.
