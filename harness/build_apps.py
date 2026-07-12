"""Reproducible build loop for source-authored KotoOS apps.

Reads the app registry at ``apps/apps.json`` and compiles each app's source into
its committed ``sdcard_mock`` bytecode. Each app declares a ``kind``:

- ``koto``: high-level Koto source compiled by ``koto-compiler``.
- ``asm``: low-level ``kbc-asm`` assembly / IR.

Usage:
  python harness/build_apps.py            # rebuild committed bytecode
  python harness/build_apps.py --check    # fail if committed bytecode is stale

The ``--check`` mode is wired into ``harness/check_all.py`` so drift between an
app's source and its committed ``.kbc`` (or a manifest entry that does not point
at the built output) fails the local checks. Both tools already emit
verifier-valid bytecode (``koto-compiler`` runs ``verify_kbc``; the committed
assembly fixtures are verified by their crate tests).
"""

from __future__ import annotations

import argparse
import json
import shutil
import struct
import subprocess
import sys
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
REGISTRY = ROOT / "apps" / "apps.json"

TOOL_FOR_KIND = {
    "koto": "koto-compiler",
    "asm": "kbc-asm",
}

# Markers that bracket the build-time generated stage table inside a Koto source.
STAGE_BEGIN_PREFIX = "// === BEGIN GENERATED STAGES"
STAGE_END_MARKER = "// === END GENERATED STAGES ==="


def render_stage_block(stages: list[tuple[str, str]]) -> str:
    """Render the generated Koto between the stage markers (markers excluded).

    Each map's printable tilemap is embedded flat (row-major, newlines stripped)
    as a string literal; the app decodes the glyphs at runtime. Adding a stage is
    just dropping another `*.txt` map, so this block grows automatically.
    """
    lines = ["// Regenerate with: python harness/build_apps.py"]
    lines.append("fn stage_count() -> int {")
    lines.append(f"    return {len(stages)};")
    lines.append("}")
    lines.append("")
    lines.append("fn stage_data(level: int) -> int {")
    for index, (name, flat) in enumerate(stages, start=1):
        lines.append(f'    if level == {index} {{ return "{flat}"; }}   // {name}')
    lines.append(f'    return "{stages[0][1]}";')  # fallback to the first stage
    lines.append("}")
    return "\n".join(lines)


def replace_stage_region(text: str, block: str) -> tuple[str, bool]:
    """Replace the lines between the stage markers with `block`. LF-normalized."""
    lines = text.split("\n")
    begin = end = None
    for index, line in enumerate(lines):
        if begin is None and line.startswith(STAGE_BEGIN_PREFIX):
            begin = index
        elif begin is not None and line.strip() == STAGE_END_MARKER:
            end = index
            break
    if begin is None or end is None:
        return text, False
    merged = lines[: begin + 1] + block.split("\n") + lines[end:]
    return "\n".join(merged), True


def load_stages(maps: dict, app_id: str, errors: list[str]) -> list[tuple[str, str]]:
    map_dir = ROOT / maps["dir"]
    width = int(maps["width"])
    height = int(maps["height"])
    allowed = set(maps.get("glyphs", ""))
    if not map_dir.is_dir():
        errors.append(f"{app_id}: missing maps dir {maps['dir']!r}")
        return []
    files = sorted(map_dir.glob("*.txt"))
    if not files:
        errors.append(f"{app_id}: no .txt maps in {maps['dir']!r}")
        return []
    stages: list[tuple[str, str]] = []
    for path in files:
        rows = path.read_text(encoding="utf-8").splitlines()
        while rows and rows[-1] == "":
            rows.pop()
        if len(rows) != height:
            errors.append(f"{app_id}: {path.name} has {len(rows)} rows, expected {height}")
            continue
        bad_row = next((row for row in rows if len(row) != width), None)
        if bad_row is not None:
            errors.append(
                f"{app_id}: {path.name} row {bad_row!r} is {len(bad_row)} wide, expected {width}"
            )
            continue
        illegal = sorted({ch for row in rows for ch in row} - allowed)
        if illegal:
            errors.append(f"{app_id}: {path.name} has invalid glyphs {illegal}")
            continue
        flat = "".join(rows)
        if flat.count("@") != 1:
            errors.append(f"{app_id}: {path.name} must contain exactly one '@' start")
            continue
        stages.append((path.name, flat))
    return stages


def generate_maps(app: dict, check: bool, errors: list[str]) -> None:
    """Embed `*.txt` tilemaps into the app's Koto source between the markers."""
    maps = app.get("maps")
    if not maps:
        return
    app_id = app.get("app_id")
    source_path = ROOT / maps["source"]
    if not source_path.exists():
        errors.append(f"{app_id}: missing maps source {maps['source']!r}")
        return
    before = len(errors)
    stages = load_stages(maps, app_id, errors)
    if len(errors) != before or not stages:
        return
    text = source_path.read_text(encoding="utf-8")
    new_text, replaced = replace_stage_region(text, render_stage_block(stages))
    if not replaced:
        errors.append(f"{app_id}: missing generated-stage markers in {maps['source']!r}")
        return
    if new_text == text:
        return
    if check:
        errors.append(
            f"{app_id}: {maps['source']} stage block is stale; "
            f"rebuild with `python harness/build_apps.py`"
        )
    else:
        source_path.write_bytes(new_text.encode("utf-8"))
        print(f"generated: {len(stages)} stages -> {maps['source']}")


def load_registry(errors: list[str]) -> list[dict]:
    if not REGISTRY.exists():
        errors.append(f"missing app registry: {REGISTRY.relative_to(ROOT)}")
        return []
    try:
        data = json.loads(REGISTRY.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        errors.append(f"invalid JSON in {REGISTRY.relative_to(ROOT)}: {exc}")
        return []
    apps = data.get("apps")
    if not isinstance(apps, list) or not apps:
        errors.append("app registry must contain a non-empty `apps` list")
        return []
    return apps


# Per-app KOTO-0156 code-window layout opt-ins: apps.json `codegen` booleans mapped to
# koto-compiler CLI flags. Off unless a `codegen` block requests them, so every other
# app's bytecode is byte-identical to the baseline layout.
CODEGEN_FLAGS = {
    "relocate_preamble": "--relocate-preamble",
    "outline_cold_blocks": "--outline-cold-blocks",
    # KOTO-0169 Stage 4 opt-OUT: pin the pre-Stage-4 boolean/comparison
    # templates for apps whose code-window tile layout regresses under the
    # smaller default codegen (the KOTO-0156 lesson).
    "legacy_compare_templates": "--legacy-compare-templates",
}


def codegen_flags(app: dict, errors: list[str]) -> list[str]:
    codegen = app.get("codegen")
    if not codegen:
        return []
    if not isinstance(codegen, dict):
        errors.append(f"{app.get('app_id')}: `codegen` must be an object")
        return []
    flags = []
    for key, value in codegen.items():
        if key == "comment":
            continue
        flag = CODEGEN_FLAGS.get(key)
        if flag is None:
            errors.append(f"{app.get('app_id')}: unknown codegen option {key!r}")
            continue
        if value:
            flags.append(flag)
    return flags


def build_one(app: dict, dest: Path, errors: list[str]) -> bool:
    kind = app.get("kind")
    tool = TOOL_FOR_KIND.get(kind)
    source = app.get("source")
    if tool is None:
        errors.append(f"{app.get('app_id')}: unknown kind {kind!r}")
        return False
    if not source or not (ROOT / source).exists():
        errors.append(f"{app.get('app_id')}: missing source {source!r}")
        return False
    extra = codegen_flags(app, errors) if kind == "koto" else []
    completed = subprocess.run(
        ["cargo", "run", "-q", "-p", tool, "--", source, str(dest), *extra],
        cwd=ROOT,
    )
    if completed.returncode != 0:
        errors.append(f"{app.get('app_id')}: build failed ({tool})")
        return False
    return True


def check_manifest(app: dict, errors: list[str]) -> None:
    manifest_path = app.get("manifest")
    output = app.get("output")
    if not manifest_path:
        return
    manifest_file = ROOT / manifest_path
    if not manifest_file.exists():
        errors.append(f"{app.get('app_id')}: missing manifest {manifest_path!r}")
        return
    try:
        manifest = json.loads(manifest_file.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        errors.append(f"{app.get('app_id')}: invalid manifest JSON: {exc}")
        return
    # The manifest entry must point at the built output, relative to sdcard_mock.
    expected_entry = Path(output).relative_to("sdcard_mock").as_posix()
    if manifest.get("entry") != expected_entry:
        errors.append(
            f"{app.get('app_id')}: manifest entry {manifest.get('entry')!r} "
            f"does not match built output {expected_entry!r}"
        )
    if manifest.get("app_id") != app.get("app_id"):
        errors.append(
            f"{app.get('app_id')}: manifest app_id {manifest.get('app_id')!r} mismatch"
        )

def rgb565_le(hex6: str) -> bytes:
    """Pack an `RRGGBB` hex colour into little-endian RGB565."""
    r, g, b = int(hex6[0:2], 16), int(hex6[2:4], 16), int(hex6[4:6], 16)
    return struct.pack("<H", ((r & 0xF8) << 8) | ((g & 0xFC) << 3) | (b >> 3))


def kspr_to_kim(text: str, app_id: str, source: str, errors: list[str]) -> bytes | None:
    """Compile a `.kspr` sprite-sheet source into a `KIM1` image.

    Format: 16x16 tiles stacked top-to-bottom into a 16-wide strip. Lines are
    `# comment`, `color <char> <RRGGBB>` palette entries, `tile <id> <name>`
    headers, or 16-character pixel rows referencing palette chars. The output is
    `b"KIM1"`, width/height as little-endian u16, then row-major RGB565 pixels.
    """
    palette: dict[str, str] = {}
    tiles: list[list[str]] = []
    for raw in text.splitlines():
        s = raw.strip()
        if not s or s.startswith("#"):
            continue
        if s.startswith("color "):
            parts = s.split()
            if len(parts) != 3 or len(parts[2]) != 6:
                errors.append(f"{app_id}: bad palette line in {source!r}: {s!r}")
                return None
            palette[parts[1]] = parts[2]
        elif s.startswith("tile "):
            tiles.append([])
        else:
            if not tiles:
                errors.append(f"{app_id}: pixel row before any `tile` in {source!r}")
                return None
            if len(s) != 16:
                errors.append(f"{app_id}: row in {source!r} is {len(s)} chars, expected 16: {s!r}")
                return None
            tiles[-1].append(s)
    if not tiles:
        errors.append(f"{app_id}: no tiles defined in {source!r}")
        return None
    out = bytearray(b"KIM1")
    out += struct.pack("<HH", 16, 16 * len(tiles))
    for index, rows in enumerate(tiles):
        if len(rows) != 16:
            errors.append(f"{app_id}: tile {index} in {source!r} has {len(rows)} rows, expected 16")
            return None
        for row in rows:
            for ch in row:
                color = palette.get(ch)
                if color is None:
                    errors.append(f"{app_id}: undefined palette char {ch!r} in {source!r}")
                    return None
                out += rgb565_le(color)
    return bytes(out)


def generate_images(app: dict, check: bool, errors: list[str]) -> None:
    """Compile each `images` entry's `.kspr` source into a committed `.kim` asset."""
    app_id = app.get("app_id")
    for image in app.get("images", []):
        source = image.get("source")
        output = image.get("output")
        if not source or not output:
            errors.append(f"{app_id}: image requires `source` and `output`")
            continue
        source_path = ROOT / source
        output_path = ROOT / output
        if not source_path.exists():
            errors.append(f"{app_id}: missing image source {source!r}")
            continue
        kim = kspr_to_kim(source_path.read_text(encoding="utf-8"), app_id, source, errors)
        if kim is None:
            continue
        if check:
            if not output_path.exists():
                errors.append(f"{app_id}: committed image {output!r} is missing")
            elif output_path.read_bytes() != kim:
                errors.append(
                    f"{app_id}: {output} is stale; rebuild with `python harness/build_apps.py`"
                )
        else:
            output_path.parent.mkdir(parents=True, exist_ok=True)
            output_path.write_bytes(kim)
            print(f"image: {source} -> {output} ({len(kim)} bytes)")


def sync_assets(app: dict, check: bool, errors: list[str]) -> None:
    for asset in app.get("assets", []):
        source = asset.get("source")
        output = asset.get("output")
        if not source or not output:
            errors.append(f"{app.get('app_id')}: asset requires `source` and `output`")
            continue
        source_path = ROOT / source
        output_path = ROOT / output
        if not source_path.exists():
            errors.append(f"{app.get('app_id')}: missing asset source {source!r}")
            continue
        if check:
            if not output_path.exists():
                errors.append(f"{app.get('app_id')}: committed asset {output!r} is missing")
            elif source_path.read_bytes() != output_path.read_bytes():
                errors.append(
                    f"{app.get('app_id')}: {output} is stale; "
                    f"rebuild with `python harness/build_apps.py`"
                )
        else:
            output_path.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source_path, output_path)
            print(f"copied: {source} -> {output}")


def main() -> int:
    parser = argparse.ArgumentParser(description="Build source-authored KotoOS apps.")
    parser.add_argument(
        "--check",
        action="store_true",
        help="verify committed bytecode matches its source instead of rewriting it",
    )
    args = parser.parse_args()

    errors: list[str] = []
    apps = load_registry(errors)

    for app in apps:
        output = app.get("output")
        if not output:
            errors.append(f"{app.get('app_id')}: registry entry missing `output`")
            continue
        committed = ROOT / output
        generate_maps(app, args.check, errors)
        generate_images(app, args.check, errors)
        check_manifest(app, errors)
        sync_assets(app, args.check, errors)

        if args.check:
            with tempfile.TemporaryDirectory() as tmp:
                dest = Path(tmp) / "app.kbc"
                if not build_one(app, dest, errors):
                    continue
                built = dest.read_bytes()
            if not committed.exists():
                errors.append(f"{app.get('app_id')}: committed {output} is missing")
            elif committed.read_bytes() != built:
                errors.append(
                    f"{app.get('app_id')}: {output} is stale; "
                    f"rebuild with `python harness/build_apps.py`"
                )
            else:
                print(f"ok: {app.get('app_id')} -> {output} ({len(built)} bytes)")
        else:
            committed.parent.mkdir(parents=True, exist_ok=True)
            if build_one(app, committed, errors):
                print(f"built: {app.get('app_id')} -> {output}")

    if errors:
        print("\nKotoOS app build: FAIL")
        for error in errors:
            print(f"- {error}")
        return 1
    print("\nKotoOS app build: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
