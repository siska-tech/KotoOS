from __future__ import annotations

import hashlib
import subprocess
import sys
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
TEST_SOURCE = ROOT / "harness" / "audio_scratch_host.rs"
FIXTURE_HASHES = {
    "apps/samples/audio_codecs/audio/pcm16.kacl": "6f826ffe6845fd034b34cf21028e50c5e457efd4ed818ad5d10671c8359cde58",
    "apps/samples/audio_codecs/audio/sld4.kacl": "6ef8b073fe1b80549c53908566b31bb662bba540d6aceadc83204d00ccba39c7",
    "apps/samples/audio_codecs/audio/cue_a.kmml": "4822aedac252b43d7788d6db3a1d2f517653a42aa56980464b7e6b340ec533df",
    "apps/samples/audio_codecs/audio/cue_b.kmml": "8274c19d28aa3503fc9528a8499b77fbd2c7321a14a717cd8311d5b9ab33df5e",
}


def main() -> int:
    for relative, expected in FIXTURE_HASHES.items():
        actual = hashlib.sha256((ROOT / relative).read_bytes()).hexdigest()
        if actual != expected:
            print(f"audio scratch fixture checksum mismatch: {relative}")
            return 1
    with tempfile.TemporaryDirectory(prefix="koto-audio-scratch-") as temp_dir:
        executable = Path(temp_dir) / (
            "audio_scratch_tests.exe" if sys.platform == "win32" else "audio_scratch_tests"
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
