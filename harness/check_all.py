from __future__ import annotations

import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


CHECKS = [
    ("Rust format", ["cargo", "fmt", "--check"]),
    (
        "Rust Clippy",
        ["cargo", "clippy", "--all-targets", "--", "-D", "warnings"],
    ),
    ("Rust tests", ["cargo", "test"]),
    ("App build sync", [sys.executable, "harness/build_apps.py", "--check"]),
    ("Memo validation", ["cargo", "run", "-p", "koto-sim", "--", "--memo-validation"]),
    ("Golden frame validation", [sys.executable, "harness/check_golden_frames.py"]),
    ("Runtime budget gate", [sys.executable, "harness/check_budgets.py"]),
    ("Project harness", [sys.executable, "harness/check_project.py"]),
]


def run_check(name: str, command: list[str]) -> int:
    print(f"\n== {name} ==", flush=True)
    print(f"$ {' '.join(command)}", flush=True)
    completed = subprocess.run(command, cwd=ROOT)
    if completed.returncode != 0:
        print(f"\n{name} failed with exit code {completed.returncode}")
    return completed.returncode


def main() -> int:
    for name, command in CHECKS:
        result = run_check(name, command)
        if result != 0:
            return result

    print("\nKotoOS local checks: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
