from __future__ import annotations

import subprocess
import sys
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
TEST_SOURCE = ROOT / "harness" / "arena_future_host.rs"


def main() -> int:
    with tempfile.TemporaryDirectory(prefix="koto-arena-future-") as temp_dir:
        executable = Path(temp_dir) / (
            "arena_future_tests.exe" if sys.platform == "win32" else "arena_future_tests"
        )
        compile_result = subprocess.run(
            [
                "rustc",
                "--edition=2021",
                "--test",
                str(TEST_SOURCE),
                "-o",
                str(executable),
            ],
            cwd=ROOT,
        )
        if compile_result.returncode != 0:
            return compile_result.returncode
        return subprocess.run([str(executable)], cwd=ROOT).returncode


if __name__ == "__main__":
    sys.exit(main())
