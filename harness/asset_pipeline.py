#!/usr/bin/env python3
"""Host-side asset pipeline prototype for Koto package assets.

The tool intentionally stays dependency-free so it can run in the existing
project harness. It covers the first concrete paths for KOTO-0054:

- convert 40x40 ASCII PBM (P1) launcher icons into package-ready KICON1 files;
- emit a PBM preview for converted icons;
- inspect .kfont metrics and render sample text previews;
- verify generated asset layout rows against a KPA source manifest.
"""

from __future__ import annotations

import argparse
import csv
import json
import struct
import sys
from dataclasses import dataclass
from pathlib import Path


KICON_SIZE = 40
KFNT_HEADER_LEN = 16
KFNT_INDEX_ENTRY_LEN = 10


def read_p1_pbm(path: Path) -> list[list[bool]]:
    tokens: list[str] = []
    for raw_line in path.read_text(encoding="ascii").splitlines():
        line = raw_line.split("#", 1)[0]
        tokens.extend(line.split())

    if len(tokens) < 3 or tokens[0] != "P1":
        raise ValueError(f"{path}: expected ASCII PBM P1 header")
    width = int(tokens[1])
    height = int(tokens[2])
    if width != KICON_SIZE or height != KICON_SIZE:
        raise ValueError(f"{path}: icon PBM must be {KICON_SIZE}x{KICON_SIZE}")

    bits = tokens[3:]
    expected = width * height
    if len(bits) != expected:
        raise ValueError(f"{path}: expected {expected} pixels, found {len(bits)}")
    if any(bit not in {"0", "1"} for bit in bits):
        raise ValueError(f"{path}: PBM pixels must be 0 or 1")

    rows: list[list[bool]] = []
    for y in range(height):
        start = y * width
        rows.append([bit == "1" for bit in bits[start : start + width]])
    return rows


def pixels_to_kicon(pixels: list[list[bool]]) -> str:
    if len(pixels) != KICON_SIZE or any(len(row) != KICON_SIZE for row in pixels):
        raise ValueError(f"KICON1 pixels must be {KICON_SIZE}x{KICON_SIZE}")
    lines = ["KICON1"]
    lines.extend("".join("#" if pixel else "." for pixel in row) for row in pixels)
    return "\n".join(lines) + "\n"


def pixels_to_pbm(pixels: list[list[bool]], comment: str) -> str:
    width = len(pixels[0]) if pixels else 0
    height = len(pixels)
    lines = ["P1", f"# {comment}", f"{width} {height}"]
    lines.extend(" ".join("1" if pixel else "0" for pixel in row) for row in pixels)
    return "\n".join(lines) + "\n"


def convert_icon(src: Path, out: Path, preview: Path) -> None:
    pixels = read_p1_pbm(src)
    out.parent.mkdir(parents=True, exist_ok=True)
    preview.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(pixels_to_kicon(pixels), encoding="ascii")
    preview.write_text(pixels_to_pbm(pixels, f"preview generated from {src.name}"), encoding="ascii")


@dataclass(frozen=True)
class Glyph:
    codepoint: int
    width: int
    row_bytes: int
    rows: tuple[bytes, ...]

    def pixel(self, x: int, y: int) -> bool:
        if x < 0 or y < 0 or x >= self.width or y >= len(self.rows):
            return False
        byte = self.rows[y][x // 8]
        return ((byte >> (7 - (x % 8))) & 1) == 1


@dataclass(frozen=True)
class KFont:
    cell_h: int
    ascent: int
    half_w: int
    full_w: int
    glyphs: dict[int, Glyph]

    @classmethod
    def read(cls, path: Path) -> "KFont":
        data = path.read_bytes()
        if len(data) < KFNT_HEADER_LEN:
            raise ValueError(f"{path}: truncated KFNT header")
        if data[:4] != b"KFNT":
            raise ValueError(f"{path}: bad KFNT magic")
        version, flags = struct.unpack_from("<HH", data, 4)
        if version != 1 or flags != 0:
            raise ValueError(f"{path}: unsupported KFNT version/flags")

        cell_h, ascent, half_w, full_w = data[8], data[9], data[10], data[11]
        glyph_count = struct.unpack_from("<I", data, 12)[0]
        index_off = KFNT_HEADER_LEN
        bitmap_off = index_off + glyph_count * KFNT_INDEX_ENTRY_LEN
        if bitmap_off > len(data):
            raise ValueError(f"{path}: truncated KFNT index")

        glyphs: dict[int, Glyph] = {}
        previous_cp = -1
        for i in range(glyph_count):
            base = index_off + i * KFNT_INDEX_ENTRY_LEN
            codepoint, width, row_bytes, bitmap_rel = struct.unpack_from("<IBBI", data, base)
            if codepoint <= previous_cp:
                raise ValueError(f"{path}: glyph index is not sorted")
            previous_cp = codepoint
            size = row_bytes * cell_h
            start = bitmap_off + bitmap_rel
            end = start + size
            if end > len(data):
                raise ValueError(f"{path}: glyph U+{codepoint:04X} bitmap is truncated")
            rows = tuple(data[start + y * row_bytes : start + (y + 1) * row_bytes] for y in range(cell_h))
            glyphs[codepoint] = Glyph(codepoint, width, row_bytes, rows)

        return cls(cell_h, ascent, half_w, full_w, glyphs)

    def glyph(self, char: str) -> Glyph | None:
        return self.glyphs.get(ord(char))


def render_text(font: KFont, sample: str) -> str:
    glyphs = [font.glyph(char) for char in sample]
    missing = [char for char, glyph in zip(sample, glyphs) if glyph is None]
    if missing:
        names = ", ".join(f"U+{ord(char):04X}" for char in missing)
        raise ValueError(f"font is missing sample glyphs: {names}")

    lines = [
        f"# sample: {sample}",
        f"# cell_h={font.cell_h} ascent={font.ascent} half_w={font.half_w} full_w={font.full_w} glyphs={len(font.glyphs)}",
    ]
    for y in range(font.cell_h):
        parts = []
        for glyph in glyphs:
            assert glyph is not None
            parts.append("".join("#" if glyph.pixel(x, y) else "." for x in range(glyph.width)))
        lines.append(".".join(parts).rstrip("."))
    return "\n".join(lines) + "\n"


def write_font_preview(font_path: Path, sample: str, out: Path) -> None:
    font = KFont.read(font_path)
    preview = render_text(font, sample)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(preview, encoding="utf-8")


def read_layout(path: Path) -> list[dict[str, str]]:
    with path.open("r", encoding="utf-8", newline="") as handle:
        return list(csv.DictReader(handle))


def verify_layout(manifest_path: Path, layout_path: Path) -> None:
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    assets = manifest.get("assets", [])
    manifest_paths = [asset["path"] for asset in assets]
    rows = read_layout(layout_path)
    layout_paths = [row["path"] for row in rows]
    if layout_paths != manifest_paths:
        raise ValueError("layout paths must match generated manifest asset order")

    previous_end = None
    for row in rows:
        offset = int(row["offset"])
        size = int(row["size"])
        alignment = int(row["alignment"])
        if alignment <= 0 or offset % alignment != 0:
            raise ValueError(f"{row['path']}: offset {offset} is not aligned to {alignment}")
        if previous_end is not None and offset < previous_end:
            raise ValueError(f"{row['path']}: starts before previous asset ends")
        previous_end = offset + size


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)

    icon = sub.add_parser("convert-icon", help="convert a 40x40 P1 PBM to KICON1")
    icon.add_argument("--src", required=True, type=Path)
    icon.add_argument("--out", required=True, type=Path)
    icon.add_argument("--preview", required=True, type=Path)

    font = sub.add_parser("font-preview", help="validate a .kfont and render sample text")
    font.add_argument("--font", required=True, type=Path)
    font.add_argument("--sample", default="Koto日本語")
    font.add_argument("--out", required=True, type=Path)

    layout = sub.add_parser("verify-layout", help="verify generated asset layout rows")
    layout.add_argument("--manifest", required=True, type=Path)
    layout.add_argument("--layout", required=True, type=Path)

    args = parser.parse_args(argv)
    try:
        if args.command == "convert-icon":
            convert_icon(args.src, args.out, args.preview)
        elif args.command == "font-preview":
            write_font_preview(args.font, args.sample, args.out)
        elif args.command == "verify-layout":
            verify_layout(args.manifest, args.layout)
        else:
            parser.error(f"unknown command: {args.command}")
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        print(f"asset pipeline: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
