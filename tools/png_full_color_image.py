"""Convert a 320x320 RGB/RGBA PNG to a row-major RGB565 KIM1 image.

The output preserves every source pixel in its original position. This CLI
shares the dependency-free converter used by ``harness/build_apps.py``.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "harness"))

from build_apps import png_to_rgb565_kim  # noqa: E402


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", type=Path, help="320x320 RGB/RGBA PNG")
    parser.add_argument("--output", required=True, type=Path, help="output KIM1 path")
    args = parser.parse_args()

    errors: list[str] = []
    converted = png_to_rgb565_kim(args.input, "png-full-color-image", str(args.input), errors)
    if converted is None:
        for error in errors:
            print(error, file=sys.stderr)
        return 1

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_bytes(converted)
    print(f"wrote {args.output} ({len(converted)} bytes, 320x320 RGB565)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
