from __future__ import annotations

import difflib
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
EXPECTED = ROOT / "harness" / "fixtures" / "golden_frames" / "sim.trace"


def main() -> int:
    if not EXPECTED.exists():
        print(f"missing golden frame fixture: {EXPECTED.relative_to(ROOT)}")
        return 1

    completed = subprocess.run(
        ["cargo", "run", "-q", "-p", "koto-sim", "--", "--golden-frames"],
        cwd=ROOT,
        text=True,
        capture_output=True,
    )
    if completed.returncode != 0:
        print("golden frame trace command failed")
        if completed.stderr:
            print(completed.stderr, end="")
        return completed.returncode

    expected = EXPECTED.read_text(encoding="utf-8")
    actual = completed.stdout
    if actual != expected:
        print("golden frame trace mismatch")
        diff = difflib.unified_diff(
            expected.splitlines(keepends=True),
            actual.splitlines(keepends=True),
            fromfile=str(EXPECTED.relative_to(ROOT)),
            tofile="actual golden frame trace",
        )
        print("".join(diff), end="")
        return 1

    print("Golden frame validation: OK")
    print(f"- checked {EXPECTED.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
