from __future__ import annotations

import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]

CHECKS = [
    (
        "RP2040 embedded bins",
        [
            "cargo",
            "check",
            "-p",
            "koto-pico",
            "--bins",
            "--target",
            "thumbv6m-none-eabi",
        ],
    ),
    (
        "RP2040 Pico W embedded bins",
        [
            "cargo",
            "check",
            "-p",
            "koto-pico",
            "--bins",
            "--target",
            "thumbv6m-none-eabi",
            "--no-default-features",
            "--features",
            "board-picocalc-picow,ram_interpreter,ram_audio_mixer",
        ],
    ),
    (
        "RP2040 Pico W wifi-config embedded bins",
        [
            "cargo",
            "check",
            "-p",
            "koto-pico",
            "--bins",
            "--target",
            "thumbv6m-none-eabi",
            "--no-default-features",
            "--features",
            "board-picocalc-picow,ram_interpreter,ram_audio_mixer,network_service",
        ],
    ),
    (
        "RP2040 Pico W HTTPS-fetch embedded bins",
        [
            "cargo",
            "check",
            "-p",
            "koto-pico",
            "--bins",
            "--target",
            "thumbv6m-none-eabi",
            "--no-default-features",
            "--features",
            "board-picocalc-picow,ram_interpreter,ram_audio_mixer,network_service,app_fetch_https",
        ],
    ),
    (
        "RP2350A embedded bins",
        [
            "cargo",
            "check",
            "-p",
            "koto-pico",
            "--bins",
            "--target",
            "thumbv8m.main-none-eabihf",
            "--no-default-features",
            "--features",
            "board-picocalc-pico2w,ram_interpreter,ram_audio_mixer",
        ],
    ),
    (
        "RP2350A Pico 2 W HTTPS-fetch embedded bins",
        [
            "cargo",
            "check",
            "-p",
            "koto-pico",
            "--bins",
            "--target",
            "thumbv8m.main-none-eabihf",
            "--no-default-features",
            "--features",
            "board-picocalc-pico2w,ram_interpreter,ram_audio_mixer,network_service,app_fetch_https",
        ],
    ),
]


def main() -> int:
    for name, command in CHECKS:
        print(f"\n== {name} ==", flush=True)
        print(f"$ {' '.join(command)}", flush=True)
        result = subprocess.run(command, cwd=ROOT)
        if result.returncode != 0:
            print(f"{name} failed with exit code {result.returncode}")
            return result.returncode
    print("\nKotoOS embedded cross-build checks: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
