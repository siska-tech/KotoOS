"""Runtime budget gate (KOTO-0101, KOTO-0102).

For each app's worst-ish scripted scenario this:

  * runs it through KotoSim with ``--budget`` and parses the runtime memory/fuel
    high-water report, and
  * runs ``koto-compiler --slot-map`` on the app source for the static user-local
    -slot attribution (which inlined function owns which slots).

It WARNS when a VM peak reaches >=90% of its fixed profile capacity (operand stack,
call depth, frame fuel) or when user-local-slot usage nears the user-slot cap, and
FAILS when a tracked peak exceeds the per-scenario threshold configured below.

Note on locals: the runtime ``local_peak`` counts the highest VM slot touched. The
three codegen scratch slots (one of them the return slot) now *float* just above the
program's user-slot high-water mark instead of pinning at the top of the 48-slot file
(KOTO-0146), so ``local_peak`` is roughly ``user_slots_used`` plus the scratch slots
the app touches -- it tracks real local pressure rather than always reaching 48. The
*actionable* number is still the static ``user_slots_used`` from the slot map -- the
slots user code actually owns, bounded by the 45 user slots below the scratch region.
The gate warns and thresholds on that, not on the runtime peak.

Thresholds carry headroom over today's peaks so the gate is green now but catches
regressions; raising one is the moment to confirm the new budget is justified
(see [[deliberate-vm-budget-sizing]]).
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]

WARN_RATIO = 0.90

# Runtime VM peak -> the capacity field it is bounded by. A peak >=90% of its cap
# warns. (`local_peak` is deliberately absent: see the module docstring -- the
# user-slot map is the meaningful local-pressure signal.)
CAP_OF = {
    "stack_peak": "stack_cap",
    "call_peak": "call_cap",
    "fuel_peak": "fuel_cap",
}

# Per-scenario hard thresholds: a peak strictly above its threshold fails the gate.
# `max_user_slots` thresholds the slot-map's post-reuse user-slot peak. KOTO-0104
# call-site inline slot reuse brought koto-blocks to 42/45; the KOTO-0103 score-popup
# value (`pop`) spends one more main local for 43/45, with two still free.
SCENARIOS = [
    {
        "app": "dev.koto.memo",
        "source": "apps/memo/src/main.koto",
        "script": "harness/fixtures/budget/memo.script",
        "max": {
            "stack_peak": 14,
            "heap_peak": 2048,
            "fuel_peak": 20000,
            "host_calls_peak": 160,
            "max_user_slots": 25,
        },
    },
    {
        "app": "dev.koto.games.koto-blocks",
        "source": "apps/koto_blocks/src/main.koto",
        "script": "harness/fixtures/budget/koto_blocks.script",
        "max": {
            "stack_peak": 14,
            # heap is dominated by `buf tiles[3584]` (7 tetromino tiles, 16x16
            # RGB565) plus the board/scratch buffers and string constants -> 4191
            # bytes. The package image/tile pipeline grew it past the old 4096
            # tripwire; 5120 restores ~22% headroom (matching memo's ratio) while
            # staying at 21% of the 24576 device heap budget. See
            # [[deliberate-vm-budget-sizing]].
            "heap_peak": 5120,
            "fuel_peak": 55000,
            "host_calls_peak": 220,
            "max_user_slots": 43,
        },
    },
    {
        "app": "dev.koto.samples.actor-array",
        "source": "apps/samples/actor_array/src/main.koto",
        "script": "harness/fixtures/budget/actor_array.script",
        "max": {
            "stack_peak": 14,
            "heap_peak": 512,
            "fuel_peak": 20000,
            "host_calls_peak": 80,
            "max_user_slots": 32,
        },
    },
]


def _parse_kv_line(line: str) -> dict[str, int | None]:
    fields: dict[str, int | None] = {}
    for token in line.split():
        if "=" not in token:
            continue
        key, _, raw = token.partition("=")
        if not raw.isascii() or not (raw.isdigit() or raw == "none"):
            continue
        fields[key] = None if raw == "none" else int(raw)
    return fields


def run_budget(app: str, script: str) -> dict[str, int | None]:
    """Run one scenario and return the parsed budget report as a field map."""
    command = [
        "cargo", "run", "-q", "-p", "koto-sim", "--",
        "--app", app, "--app-script", script, "--budget",
    ]
    completed = subprocess.run(command, cwd=ROOT, text=True, capture_output=True)
    if completed.returncode != 0:
        print(f"  scenario command failed for {app}")
        if completed.stderr:
            print(completed.stderr, end="")
        raise SystemExit(completed.returncode)

    line = next(
        (ln for ln in completed.stdout.splitlines() if ln.startswith("budget ")),
        None,
    )
    if line is None:
        print(f"  no budget report line in output for {app}")
        print(completed.stdout, end="")
        raise SystemExit(1)
    return _parse_kv_line(line)


def run_slot_map(source: str) -> tuple[dict[str, int | None], list[str]]:
    """Run `--slot-map` and return (summary fields, per-function lines)."""
    command = ["cargo", "run", "-q", "-p", "koto-compiler", "--", source, "--slot-map"]
    completed = subprocess.run(command, cwd=ROOT, text=True, capture_output=True)
    if completed.returncode != 0:
        print(f"  slot-map command failed for {source}")
        if completed.stderr:
            print(completed.stderr, end="")
        raise SystemExit(completed.returncode)

    lines = completed.stdout.splitlines()
    summary = next((ln for ln in lines if ln.startswith("slot-map ")), None)
    if summary is None:
        print(f"  no slot-map line in output for {source}")
        print(completed.stdout, end="")
        raise SystemExit(1)
    fn_lines = [ln for ln in lines if ln.startswith("fn ")]
    return _parse_kv_line(summary), fn_lines


def check_scenario(scenario: dict) -> tuple[list[str], list[str]]:
    """Return (warnings, failures) for one scenario."""
    app = scenario["app"]
    fields = run_budget(app, scenario["script"])
    slots, fn_lines = run_slot_map(scenario["source"])
    warnings: list[str] = []
    failures: list[str] = []

    print(f"\n  {app}  (frames={fields.get('frames')})")

    # Capacity warnings: VM peaks nearing their fixed profile capacity.
    for peak_key, cap_key in CAP_OF.items():
        peak = fields.get(peak_key)
        cap = fields.get(cap_key)
        if peak is None or not cap:
            continue
        ratio = peak / cap
        flag = "WARN" if ratio >= WARN_RATIO else "ok"
        print(f"    {peak_key:16} {peak:>7} / {cap:<7} ({ratio:5.0%}) {flag}")
        if ratio >= WARN_RATIO:
            warnings.append(f"{app}: {peak_key} {peak}/{cap} ({ratio:.0%} of capacity)")

    # Heap: the request is the SRAM working set; warn as it nears the device budget.
    heap_request = fields.get("heap_request")
    heap_budget = fields.get("heap_budget")
    if heap_request is not None and heap_budget:
        ratio = heap_request / heap_budget
        flag = "WARN" if ratio >= WARN_RATIO else "ok"
        print(
            f"    {'heap_request':16} {heap_request:>7} / {heap_budget:<7} "
            f"({ratio:5.0%}) {flag}  (peak addressed={fields.get('heap_peak')})"
        )
        if ratio >= WARN_RATIO:
            warnings.append(
                f"{app}: heap_request {heap_request}/{heap_budget} "
                f"({ratio:.0%} of SRAM budget)"
            )

    # User local slots: the actionable local-pressure number (static slot map).
    used = slots.get("user_slots_used")
    cap = slots.get("user_slots_cap")
    if used is not None and cap:
        ratio = used / cap
        flag = "WARN" if ratio >= WARN_RATIO else "ok"
        runtime_local = fields.get("local_peak")
        scratch_note = ""
        if runtime_local is not None and runtime_local > used:
            # Codegen scratch floats just above the user slots (KOTO-0146), so the
            # top `runtime_local - used` touched slots are scratch, not pinned at the
            # top of the file.
            n_scratch = runtime_local - used
            scratch_note = (
                f"; top {n_scratch} slot(s) are codegen scratch floating above "
                f"{used} user slots"
            )
        print(
            f"    {'user_slots':16} {used:>7} / {cap:<7} ({ratio:5.0%}) {flag}"
            f"  (runtime local_peak={runtime_local}{scratch_note})"
        )
        for fn_line in fn_lines:
            print(f"      {fn_line}")
        if ratio >= WARN_RATIO:
            warnings.append(f"{app}: user_slots {used}/{cap} ({ratio:.0%} of user cap)")

    # Hard thresholds: a peak above its configured limit is a regression failure.
    for key, limit in scenario["max"].items():
        peak = used if key == "max_user_slots" else fields.get(key)
        label = "user_slots" if key == "max_user_slots" else key
        if peak is None:
            failures.append(f"{app}: report missing '{label}'")
            continue
        if peak > limit:
            failures.append(f"{app}: {label} {peak} exceeds threshold {limit}")
            print(f"    threshold {label:16} {peak:>7} > {limit:<7} FAIL")

    return warnings, failures


def main() -> int:
    print("Runtime budget gate (KOTO-0101, KOTO-0102)")
    all_warnings: list[str] = []
    all_failures: list[str] = []
    for scenario in SCENARIOS:
        warnings, failures = check_scenario(scenario)
        all_warnings.extend(warnings)
        all_failures.extend(failures)

    if all_warnings:
        print("\nWarnings (budgets nearing capacity):")
        for warning in all_warnings:
            print(f"  WARN {warning}")

    if all_failures:
        print("\nFailures (budget thresholds exceeded):")
        for failure in all_failures:
            print(f"  FAIL {failure}")
        return 1

    print("\nRuntime budget gate: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
