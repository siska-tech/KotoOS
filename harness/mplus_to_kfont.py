#!/usr/bin/env python3
"""Convert M+ BITMAP FONTS (BDF) into the compact ``.kfont`` blob used by koto-core.

The messy BDF parsing and JIS X 0208 -> Unicode mapping are isolated here, offline,
so that koto-core only has to interpret a clean, fixed-cell binary at runtime
(see ``src/koto-core/src/font.rs``).

Each ``.kfont`` merges a half-width Latin BDF (``iso8859-1`` encoding, usable as
Unicode directly) with a full-width JIS X 0208 BDF (encoding decoded via EUC-JP).
Every glyph is normalised onto a single fixed cell of ``cell_h`` rows aligned to a
common baseline, so the renderer can treat all glyphs uniformly.

Binary layout (little-endian):

    magic    : b"KFNT"
    version  : u16 = 1
    flags    : u16 = 0
    cell_h   : u8        # rows stored per glyph
    ascent   : u8        # rows above the baseline
    half_w   : u8        # half-width advance (px)
    full_w   : u8        # full-width advance (px)
    glyph_n  : u32
    index    : glyph_n records, sorted ascending by codepoint, 10 bytes each:
                 codepoint  : u32
                 width      : u8   # glyph advance / bitmap width in px
                 row_bytes  : u8   # ceil(width / 8)
                 bitmap_off : u32  # offset into the bitmap blob
    bitmap   : per glyph, row_bytes * cell_h bytes, row-major, MSB first

Usage:
    python harness/mplus_to_kfont.py --src <extracted xfonts-mplus dir> --out assets/fonts
"""

from __future__ import annotations

import argparse
import struct
from pathlib import Path

MAGIC = b"KFNT"
VERSION = 1

# (output name, half-width BDF, full-width BDF)
FONT_SETS = [
    ("mplus10.kfont", "fonts_e/mplus_f10r.bdf", "fonts_j/mplus_j10r.bdf"),
    ("mplus12.kfont", "fonts_e/mplus_f12r.bdf", "fonts_j/mplus_j12r.bdf"),
]
LICENSE_FILES = ["LICENSE_E", "LICENSE_J"]


class Glyph:
    __slots__ = ("codepoint", "width", "rows")

    def __init__(self, codepoint: int, width: int, rows: list[int]):
        self.codepoint = codepoint
        self.width = width
        self.rows = rows  # one integer per cell row, MSB aligned to the left edge


class Bdf:
    def __init__(self, fbb_w: int, fbb_h: int, fbb_xoff: int, fbb_yoff: int):
        self.fbb_w = fbb_w
        self.fbb_h = fbb_h
        self.fbb_xoff = fbb_xoff
        self.fbb_yoff = fbb_yoff
        self.glyphs: list[tuple[int, int, int, int, int, list[int]]] = []
        # (encoding, dwidth, bbx_w, bbx_h, bbx_yoff, rows)

    @property
    def ascent(self) -> int:
        return self.fbb_h + self.fbb_yoff

    @property
    def descent(self) -> int:
        return -self.fbb_yoff


def parse_bdf(path: Path) -> Bdf:
    fbb = None
    bdf = None
    enc = dwidth = bbx_w = bbx_h = bbx_xoff = bbx_yoff = None
    rows: list[int] = []
    in_bitmap = False

    for raw in path.read_text(encoding="latin-1").splitlines():
        parts = raw.split()
        if not parts:
            continue
        key = parts[0]
        if key == "FONTBOUNDINGBOX":
            fbb = tuple(int(x) for x in parts[1:5])
            bdf = Bdf(*fbb)
        elif key == "ENCODING":
            enc = int(parts[1])
        elif key == "DWIDTH":
            dwidth = int(parts[1])
        elif key == "BBX":
            bbx_w, bbx_h, bbx_xoff, bbx_yoff = (int(x) for x in parts[1:5])
        elif key == "BITMAP":
            in_bitmap = True
            rows = []
        elif key == "ENDCHAR":
            in_bitmap = False
            if enc is not None and enc >= 0 and bbx_h is not None:
                bdf.glyphs.append(
                    (enc, dwidth, bbx_w, bbx_h, bbx_yoff, rows)
                )
            enc = dwidth = bbx_w = bbx_h = bbx_xoff = bbx_yoff = None
        elif in_bitmap:
            # Each scanline is hex, padded to whole bytes; keep it left-aligned.
            rows.append(int(key, 16) if key else 0)

    if bdf is None:
        raise ValueError(f"{path}: no FONTBOUNDINGBOX found")
    return bdf


def normalise(bdf: Bdf, cell_h: int, ascent: int, to_unicode) -> dict[int, Glyph]:
    """Place each glyph onto a shared cell of ``cell_h`` rows, baseline-aligned."""
    out: dict[int, Glyph] = {}
    for enc, dwidth, bbx_w, bbx_h, bbx_yoff, rows in bdf.glyphs:
        cp = to_unicode(enc)
        if cp is None:
            continue
        width = dwidth if dwidth else bbx_w
        src_bytes = (bbx_w + 7) // 8
        # Right-shift each source scanline so its left edge sits at column 0 of the
        # cell. BDF stores the scanline left-aligned within src_bytes*8 bits, so the
        # value is already MSB-aligned to the glyph's left edge -> keep as-is.
        cell = [0] * cell_h
        top = ascent - (bbx_h + bbx_yoff)
        for i, value in enumerate(rows):
            r = top + i
            if 0 <= r < cell_h:
                cell[r] = value & ((1 << (src_bytes * 8)) - 1)
        out[cp] = Glyph(cp, width, cell)
    return out


def latin_to_unicode(enc: int):
    # iso8859-1: encoding value is the Unicode codepoint. Drop C0/C1 controls.
    if 0x20 <= enc < 0x7F or 0xA0 <= enc <= 0xFF:
        return enc
    return None


def jis_to_unicode(enc: int):
    # BDF encoding is the JIS X 0208 code in GL (0x21..0x7E) ku/ten form; map to
    # EUC-JP (set the high bit on both bytes) and let Python decode it.
    hi = (enc >> 8) & 0xFF
    lo = enc & 0xFF
    try:
        ch = bytes([hi | 0x80, lo | 0x80]).decode("euc-jp")
    except UnicodeDecodeError:
        return None
    if len(ch) != 1:
        return None
    return ord(ch)


def build_kfont(half: Bdf, full: Bdf) -> bytes:
    ascent = max(half.ascent, full.ascent)
    descent = max(half.descent, full.descent)
    cell_h = ascent + descent

    glyphs: dict[int, Glyph] = {}
    glyphs.update(normalise(full, cell_h, ascent, jis_to_unicode))
    # Latin half-width wins for overlapping ASCII codepoints.
    glyphs.update(normalise(half, cell_h, ascent, latin_to_unicode))

    half_w = half_advance(half)
    full_w = full_advance(full)

    ordered = [glyphs[cp] for cp in sorted(glyphs)]

    bitmap = bytearray()
    index = bytearray()
    for g in ordered:
        row_bytes = (g.width + 7) // 8
        index += struct.pack("<IBBI", g.codepoint, g.width, row_bytes, len(bitmap))
        for value in g.rows:
            # Re-pack each cell row into exactly row_bytes, MSB first.
            src_bytes = max(row_bytes, (value.bit_length() + 7) // 8)
            raw = value.to_bytes(src_bytes, "big") if value else b"\x00" * row_bytes
            bitmap += raw[:row_bytes].ljust(row_bytes, b"\x00")

    header = MAGIC + struct.pack(
        "<HHBBBBI", VERSION, 0, cell_h, ascent, half_w, full_w, len(ordered)
    )
    return bytes(header + index + bitmap)


def half_advance(bdf: Bdf) -> int:
    return _common_advance(bdf, default=bdf.fbb_w)


def full_advance(bdf: Bdf) -> int:
    return _common_advance(bdf, default=bdf.fbb_w)


def _common_advance(bdf: Bdf, default: int) -> int:
    for _enc, dwidth, *_rest in bdf.glyphs:
        if dwidth:
            return dwidth
    return default


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument(
        "--src",
        required=True,
        type=Path,
        help="Extracted xfonts-mplus directory (contains fonts_e/ and fonts_j/)",
    )
    ap.add_argument(
        "--out",
        required=True,
        type=Path,
        help="Output directory for the .kfont blobs and license files",
    )
    args = ap.parse_args()

    args.out.mkdir(parents=True, exist_ok=True)

    for name, half_rel, full_rel in FONT_SETS:
        half = parse_bdf(args.src / half_rel)
        full = parse_bdf(args.src / full_rel)
        blob = build_kfont(half, full)
        (args.out / name).write_bytes(blob)
        print(f"wrote {name}: {len(blob)} bytes")

    for license_name in LICENSE_FILES:
        src = args.src / license_name
        if src.exists():
            (args.out / license_name).write_text(
                src.read_text(encoding="latin-1"), encoding="utf-8"
            )
            print(f"copied {license_name}")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
