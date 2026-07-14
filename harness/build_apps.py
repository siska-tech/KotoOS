"""Reproducible build loop for source-authored KotoOS apps.

Each app is a self-contained folder under ``apps/`` described by its own
``apps/<dir>/app.json`` descriptor (KOTO-0195). This tool discovers every
descriptor, compiles the app's source into its committed bytecode, generates
the packaging manifest and stages the app's icon and assets, and packs
bytecode, images, sprites, icons, and compiled Native KotoAudio into one
committed ``APPS/*.kpa`` archive. Each app declares a ``kind``:

- ``koto``: high-level Koto source compiled by ``koto-compiler``.
- ``asm``: low-level ``kbc-asm`` assembly / IR.

An ``app.json`` is the single authoring surface: it carries the build recipe
(``source``, optional ``codegen`` / ``maps`` / ``images`` / ``audio``) *and*
the package descriptor (``name``, ``description``, ``category``, ``icon``,
``shell_icon``, ``memory``, ``permissions``). In-app paths are app-relative so
the folder is copy-paste portable; the staged ``.kpa.json`` manifest and the
``package_inputs`` intermediates are generated from it.

Usage:
  python harness/build_apps.py            # rebuild committed outputs
  python harness/build_apps.py --check    # fail if a committed output is stale
  python harness/build_apps.py --app ID   # rebuild only the app with this app_id

The ``--check`` mode is wired into ``harness/check_all.py`` so drift between an
app's source and its committed ``.kbc`` / manifest / ``.kpa`` fails the local
checks. ``koto-compiler`` runs ``verify_kbc`` on its output.
"""

from __future__ import annotations

import argparse
import json
import shutil
import struct
import subprocess
import sys
import tempfile
import zlib
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
APPS_DIR = ROOT / "apps"
PACKAGE_INPUTS = ROOT / "package_inputs"

TOOL_FOR_KIND = {
    "koto": "koto-compiler",
    "asm": "kbc-asm",
}

# Required top-level fields every app.json must declare.
REQUIRED_FIELDS = ("app_id", "kind", "package", "name", "source")


def pkg_path(package_local: str) -> Path:
    """Resolve a package-local path (`audio/x.kmml`) under package_inputs."""
    return PACKAGE_INPUTS / package_local


def in_app_path(app: dict, relative: str) -> Path:
    """Resolve an app-relative path (`src/main.koto`) under the app folder."""
    return app["_dir"] / relative

def discover_apps(errors: list[str]) -> list[dict]:
    """Load every ``apps/**/app.json`` descriptor, sorted and validated.

    Duplicate ``app_id`` and missing required fields fail per file — a job the
    central registry used to do implicitly. Each returned dict gains ``_dir``
    (the app folder) and ``_package`` (the staging/archive stem).
    """
    apps: list[dict] = []
    seen: dict[str, Path] = {}
    for descriptor in sorted(APPS_DIR.glob("**/app.json")):
        rel = descriptor.relative_to(ROOT)
        try:
            app = json.loads(descriptor.read_text(encoding="utf-8"))
        except json.JSONDecodeError as exc:
            errors.append(f"{rel}: invalid JSON: {exc}")
            continue
        missing = [field for field in REQUIRED_FIELDS if not app.get(field)]
        if missing:
            errors.append(f"{rel}: missing required field(s) {missing}")
            continue
        app_id = app["app_id"]
        if app_id in seen:
            errors.append(
                f"{rel}: duplicate app_id {app_id!r} (already in {seen[app_id]})"
            )
            continue
        seen[app_id] = rel
        app["_dir"] = descriptor.parent
        app["_package"] = app["package"]
        apps.append(app)
    if not apps and not errors:
        errors.append("no apps/**/app.json descriptors found")
    return apps


def build_manifest(app: dict) -> dict:
    """Generate the packaging manifest (`.kpa.json`) from an app descriptor.

    Asset order is bytecode, icon, maps, images, audio — the layout the packer turns
    into the archive, so it is what keeps a ``.kpa`` reproducible.
    """
    package = app["_package"]
    assets = [
        {"path": f"bytecode/{package}.kbc", "type": "bytecode", "sequential": True},
        {"path": f"icons/{package}.kicon", "type": "image", "sequential": False},
    ]
    for map_asset in app.get("_map_assets", []):
        assets.append({"path": map_asset["path"], "type": "data", "sequential": True})
    for image in app.get("images", []):
        assets.append({"path": image["output"], "type": "image", "sequential": True})
        if image.get("tilemap_output"):
            assets.append(
                {"path": image["tilemap_output"], "type": "data", "sequential": True}
            )
    for audio in app.get("audio", []):
        assets.append({"path": audio["output"], "type": "audio", "sequential": True})
    manifest = {
        "format": "kpa-manifest",
        "version": 1,
        "app_id": app["app_id"],
        "name": app["name"],
        "entry": f"bytecode/{package}.kbc",
        "runtime": app.get("runtime", "kotoruntime-bytecode"),
        "icon": f"icons/{package}.kicon",
    }
    if "shell_icon" in app:
        manifest["shell_icon"] = app["shell_icon"]
    manifest["description"] = app.get("description", "")
    manifest["category"] = app.get("category", "")
    if "memory" in app:
        manifest["memory"] = app["memory"]
    manifest["assets"] = assets
    manifest["permissions"] = app.get("permissions", {"fs": "sandbox", "network": False})
    return manifest


def generate_manifest(app: dict, check: bool, errors: list[str]) -> None:
    """Write (or, with --check, verify) the generated ``.kpa.json`` manifest."""
    manifest_path = pkg_path(f"manifests/{app['_package']}.kpa.json")
    manifest = build_manifest(app)
    paths = [asset["path"] for asset in manifest["assets"]]
    duplicates = sorted({path for path in paths if paths.count(path) > 1})
    if duplicates:
        errors.append(f"{app['app_id']}: duplicate package asset path(s) {duplicates}")
        return
    text = json.dumps(manifest, ensure_ascii=False, indent=2) + "\n"
    if check:
        if not manifest_path.exists():
            errors.append(f"{app['app_id']}: committed manifest {manifest_path.name} is missing")
        elif manifest_path.read_text(encoding="utf-8") != text:
            errors.append(
                f"{app['app_id']}: {manifest_path.relative_to(ROOT).as_posix()} is stale; "
                f"rebuild with `python harness/build_apps.py`"
            )
    else:
        manifest_path.parent.mkdir(parents=True, exist_ok=True)
        manifest_path.write_text(text, encoding="utf-8")


def stage_icon(app: dict, check: bool, errors: list[str]) -> None:
    """Stage the app's ``icon.kicon`` into ``package_inputs/icons/<package>``."""
    source = in_app_path(app, app.get("icon", "icon.kicon"))
    output = pkg_path(f"icons/{app['_package']}.kicon")
    if not source.exists():
        errors.append(f"{app['app_id']}: missing icon {app.get('icon', 'icon.kicon')!r}")
        return
    if check:
        if not output.exists():
            errors.append(f"{app['app_id']}: committed icon {output.name} is missing")
        elif source.read_bytes() != output.read_bytes():
            errors.append(
                f"{app['app_id']}: {output.relative_to(ROOT).as_posix()} is stale; "
                f"rebuild with `python harness/build_apps.py`"
            )
    else:
        output.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, output)


def package_path_parts(value: str) -> list[str] | None:
    """Return clean package path parts, rejecting absolute/traversal syntax."""
    normalized = value.replace("\\", "/")
    parts = normalized.split("/")
    if not normalized or normalized.startswith("/") or ":" in parts[0]:
        return None
    if any(part in ("", ".", "..") for part in parts):
        return None
    return parts


def load_maps(app: dict, maps: dict, app_id: str, errors: list[str]) -> list[dict]:
    dir_value = maps.get("dir")
    if not isinstance(dir_value, str):
        errors.append(f"{app_id}: maps.dir must be a relative package path")
        return []
    dir_parts = package_path_parts(dir_value)
    if dir_parts is None:
        errors.append(f"{app_id}: maps.dir must stay inside the app/package: {dir_value!r}")
        return []
    map_dir = app["_dir"].joinpath(*dir_parts)
    width = int(maps["width"])
    height = int(maps["height"])
    allowed = set(maps.get("glyphs", ""))
    if not map_dir.is_dir():
        errors.append(f"{app_id}: missing maps dir {maps['dir']!r}")
        return []
    files = sorted(map_dir.glob("*.map"))
    if not files:
        errors.append(f"{app_id}: no .map files in {maps['dir']!r}")
        return []
    assets: list[dict] = []
    max_glyph_bytes = max((len(glyph.encode("utf-8")) for glyph in allowed), default=1)
    max_asset_bytes = (width * max_glyph_bytes + 2) * height
    for path in files:
        raw = path.read_bytes()
        try:
            text = raw.decode("utf-8")
        except UnicodeDecodeError as exc:
            errors.append(f"{app_id}: {path.name} is not UTF-8: {exc}")
            continue
        canonical = text.replace("\r\n", "\n")
        if "\r" in canonical:
            errors.append(f"{app_id}: {path.name} uses a bare CR line ending")
            continue
        if canonical.endswith("\n"):
            canonical = canonical[:-1]
        rows = canonical.split("\n")
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
        if len(raw) > max_asset_bytes:
            errors.append(
                f"{app_id}: {path.name} is {len(raw)} bytes, exceeds declared map maximum "
                f"{max_asset_bytes}"
            )
            continue
        package_path = "/".join([*dir_parts, path.name])
        assets.append(
            {
                "path": package_path,
                "source_path": path,
                "flat": flat.encode("utf-8"),
                "max_bytes": max_asset_bytes,
            }
        )
    return assets


def generate_maps(app: dict, check: bool, errors: list[str]) -> None:
    """Validate `.map` sources and register them as package-local data assets."""
    del check  # maps are packed directly from their app-local source files
    maps = app.get("maps")
    if not maps:
        app["_map_assets"] = []
        return
    app_id = app.get("app_id")
    required = ("dir", "width", "height", "glyphs")
    missing = [field for field in required if field not in maps]
    if missing:
        errors.append(f"{app_id}: maps missing required field(s) {missing}")
        return
    if (
        not isinstance(maps["width"], int)
        or isinstance(maps["width"], bool)
        or maps["width"] < 1
        or not isinstance(maps["height"], int)
        or isinstance(maps["height"], bool)
        or maps["height"] < 1
    ):
        errors.append(f"{app_id}: maps width and height must be positive integers")
        return
    if not isinstance(maps["glyphs"], str) or not maps["glyphs"]:
        errors.append(f"{app_id}: maps.glyphs must be a non-empty string")
        return
    before = len(errors)
    assets = load_maps(app, maps, app_id, errors)
    if len(errors) != before or not assets:
        return
    app["_map_assets"] = assets


def reject_embedded_map_payloads(app: dict, bytecode: bytes, errors: list[str]) -> None:
    """Regression guard: complete flattened maps must never return to KBC rodata."""
    for asset in app.get("_map_assets", []):
        payload = asset["flat"]
        if payload and payload in bytecode:
            errors.append(
                f"{app['app_id']}: flattened map payload {asset['path']!r} is embedded in bytecode"
            )


# Per-app KOTO-0156 code-window layout opt-ins: app.json `codegen` booleans mapped to
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
    source_path = in_app_path(app, source) if source else None
    if not source_path or not source_path.exists():
        errors.append(f"{app.get('app_id')}: missing source {source!r}")
        return False
    extra = codegen_flags(app, errors) if kind == "koto" else []
    # Pass the repo-relative posix path: the compiler embeds the source path in
    # the bytecode's KDBG debug data, so an absolute path would change the bytes.
    source_arg = source_path.relative_to(ROOT).as_posix()
    completed = subprocess.run(
        ["cargo", "run", "-q", "-p", tool, "--", source_arg, str(dest), *extra],
        cwd=ROOT,
    )
    if completed.returncode != 0:
        errors.append(f"{app.get('app_id')}: build failed ({tool})")
        return False
    return True


def rgb565_le(hex6: str) -> bytes:
    """Pack an `RRGGBB` hex colour into little-endian RGB565."""
    r, g, b = int(hex6[0:2], 16), int(hex6[2:4], 16), int(hex6[4:6], 16)
    return struct.pack("<H", ((r & 0xF8) << 8) | ((g & 0xFC) << 3) | (b >> 3))


def decode_png_rgb(path: Path, app_id: str, errors: list[str]) -> tuple[int, int, bytes] | None:
    """Decode the small, dependency-free PNG subset used by tile-mosaic sources."""
    raw = path.read_bytes()
    if not raw.startswith(b"\x89PNG\r\n\x1a\n"):
        errors.append(f"{app_id}: tile-mosaic source is not a PNG: {path}")
        return None
    pos = 8
    width = height = bit_depth = color_type = interlace = 0
    compressed = bytearray()
    while pos + 12 <= len(raw):
        size = struct.unpack_from(">I", raw, pos)[0]
        kind = raw[pos + 4 : pos + 8]
        data = raw[pos + 8 : pos + 8 + size]
        pos += 12 + size
        if kind == b"IHDR":
            width, height, bit_depth, color_type, _compression, _filter, interlace = (
                struct.unpack(">IIBBBBB", data)
            )
        elif kind == b"IDAT":
            compressed += data
        elif kind == b"IEND":
            break
    channels = {2: 3, 6: 4}.get(color_type)
    if not width or not height or bit_depth != 8 or channels is None or interlace != 0:
        errors.append(
            f"{app_id}: tile-mosaic PNG must be non-interlaced 8-bit RGB/RGBA"
        )
        return None
    try:
        scanlines = zlib.decompress(compressed)
    except zlib.error as exc:
        errors.append(f"{app_id}: cannot decompress tile-mosaic PNG: {exc}")
        return None
    stride = width * channels
    expected = (stride + 1) * height
    if len(scanlines) != expected:
        errors.append(f"{app_id}: malformed tile-mosaic PNG scanline data")
        return None

    pixels = bytearray(width * height * 3)
    previous = bytearray(stride)
    source_pos = 0
    dest_pos = 0
    for _y in range(height):
        filter_type = scanlines[source_pos]
        source_pos += 1
        row = bytearray(scanlines[source_pos : source_pos + stride])
        source_pos += stride
        for x in range(stride):
            left = row[x - channels] if x >= channels else 0
            above = previous[x]
            upper_left = previous[x - channels] if x >= channels else 0
            if filter_type == 1:
                row[x] = (row[x] + left) & 0xFF
            elif filter_type == 2:
                row[x] = (row[x] + above) & 0xFF
            elif filter_type == 3:
                row[x] = (row[x] + ((left + above) >> 1)) & 0xFF
            elif filter_type == 4:
                estimate = left + above - upper_left
                pa, pb, pc = abs(estimate - left), abs(estimate - above), abs(estimate - upper_left)
                predictor = left if pa <= pb and pa <= pc else above if pb <= pc else upper_left
                row[x] = (row[x] + predictor) & 0xFF
            elif filter_type != 0:
                errors.append(f"{app_id}: unsupported PNG filter {filter_type}")
                return None
        for x in range(width):
            src = x * channels
            pixels[dest_pos : dest_pos + 3] = row[src : src + 3]
            dest_pos += 3
        previous = row
    return width, height, bytes(pixels)


def png_to_rgb565_kim(
    path: Path, app_id: str, source: str, errors: list[str]
) -> bytes | None:
    """Convert a 320x320 PNG to a normal row-major RGB565 KIM1 image.

    No tiling, clustering, resampling, or palette reduction occurs; RGB888 to
    RGB565 quantization is the only colour loss.
    """
    decoded = decode_png_rgb(path, app_id, errors)
    if decoded is None:
        return None
    width, height, pixels = decoded
    if width != 320 or height != 320:
        errors.append(f"{app_id}: {source!r} is {width}x{height}, expected 320x320")
        return None
    kim = bytearray(b"KIM1")
    kim += struct.pack("<HH", width, height)
    for i in range(0, len(pixels), 3):
        r, g, b = pixels[i : i + 3]
        kim += struct.pack("<H", ((r & 0xF8) << 8) | ((g & 0xFC) << 3) | (b >> 3))
    return bytes(kim)


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
    """Compile authored sprite sheets and full-colour PNG images."""
    app_id = app.get("app_id")
    for image in app.get("images", []):
        source = image.get("source")
        output = image.get("output")
        if not source or not output:
            errors.append(f"{app_id}: image requires `source` and `output`")
            continue
        source_path = in_app_path(app, source)
        output_path = pkg_path(output)
        if not source_path.exists():
            errors.append(f"{app_id}: missing image source {source!r}")
            continue
        if image.get("full_color_image"):
            kim = png_to_rgb565_kim(source_path, app_id, source, errors)
            if kim is None:
                continue
        else:
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


def sync_audio(app: dict, check: bool, errors: list[str]) -> None:
    """Stage each `audio` entry's source into package_inputs for packing."""
    for asset in app.get("audio", []):
        source = asset.get("source")
        output = asset.get("output")
        if not source or not output:
            errors.append(f"{app.get('app_id')}: audio requires `source` and `output`")
            continue
        source_path = in_app_path(app, source)
        output_path = pkg_path(output)
        if not source_path.exists():
            errors.append(f"{app.get('app_id')}: missing audio source {source!r}")
            continue
        if check:
            if not output_path.exists():
                errors.append(f"{app.get('app_id')}: committed audio {output!r} is missing")
            elif source_path.read_bytes() != output_path.read_bytes():
                errors.append(
                    f"{app.get('app_id')}: {output} is stale; "
                    f"rebuild with `python harness/build_apps.py`"
                )
        else:
            output_path.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source_path, output_path)
            print(f"copied: {source} -> {output}")


def pack_app(app: dict, check: bool, errors: list[str]) -> None:
    """Build the real KPA1 archive from the generated manifest.

    The staged ``package_inputs`` files are build inputs only. ``kpa-packer``
    compiles ``type: audio`` KMML entries to KAQ1 while copying every other
    declared asset byte-for-byte into the same archive.
    """
    manifest = pkg_path(f"manifests/{app['_package']}.kpa.json")
    package = ROOT / "sdcard_mock" / "apps" / f"{app['_package']}.kpa"
    with tempfile.TemporaryDirectory() as tmp:
        temp_root = Path(tmp)
        assets_root = temp_root / "assets"
        manifest_data = json.loads(manifest.read_text(encoding="utf-8"))
        map_sources = {
            asset["path"]: asset["source_path"] for asset in app.get("_map_assets", [])
        }
        # Every pack gets an isolated input root. This lets two self-contained
        # apps both use `maps/world.map` without a shared package_inputs path
        # overwriting one app's source with the other's.
        for asset in manifest_data["assets"]:
            package_path = asset["path"]
            source = map_sources.get(package_path, pkg_path(package_path))
            destination = assets_root.joinpath(*package_path.split("/"))
            if not source.exists():
                errors.append(f"{app.get('app_id')}: missing staged asset {package_path!r}")
                return
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source, destination)

        built = temp_root / package.name if check else package
        completed = subprocess.run(
            [
                "cargo",
                "run",
                "-q",
                "-p",
                "kpa-packer",
                "--",
                "--manifest",
                str(manifest),
                "--assets-root",
                str(assets_root),
                "--out",
                str(built),
            ],
            cwd=ROOT,
        )
        if completed.returncode != 0:
            errors.append(f"{app.get('app_id')}: KPA pack failed")
            return
        if check:
            if not package.exists():
                errors.append(f"{app.get('app_id')}: committed {package.relative_to(ROOT)} is missing")
            elif package.read_bytes() != built.read_bytes():
                errors.append(
                    f"{app.get('app_id')}: {package.relative_to(ROOT)} is stale; "
                    "rebuild with `python harness/build_apps.py`"
                )
            else:
                print(
                    f"ok: {app.get('app_id')} -> {package.relative_to(ROOT)} "
                    f"({built.stat().st_size} bytes)"
                )


def main() -> int:
    parser = argparse.ArgumentParser(description="Build source-authored KotoOS apps.")
    parser.add_argument(
        "--check",
        action="store_true",
        help="verify committed bytecode matches its source instead of rewriting it",
    )
    parser.add_argument(
        "--app",
        metavar="APP_ID",
        help="rebuild only the app with this app_id "
        "(the koto-sim --watch loop's incremental path, KOTO-0191)",
    )
    args = parser.parse_args()

    errors: list[str] = []
    apps = discover_apps(errors)
    if args.app:
        apps = [app for app in apps if app.get("app_id") == args.app]
        if not apps and not errors:
            errors.append(f"--app {args.app}: no such app_id under apps/**/app.json")

    for app in apps:
        app_error_count = len(errors)
        package = app["_package"]
        output = f"bytecode/{package}.kbc"
        committed = pkg_path(output)
        generate_maps(app, args.check, errors)
        generate_images(app, args.check, errors)
        sync_audio(app, args.check, errors)
        stage_icon(app, args.check, errors)
        generate_manifest(app, args.check, errors)

        if args.check:
            with tempfile.TemporaryDirectory() as tmp:
                dest = Path(tmp) / "app.kbc"
                if not build_one(app, dest, errors):
                    continue
                built = dest.read_bytes()
            reject_embedded_map_payloads(app, built, errors)
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
                reject_embedded_map_payloads(app, committed.read_bytes(), errors)
                print(f"built: {app.get('app_id')} -> {output}")
        if len(errors) == app_error_count:
            pack_app(app, args.check, errors)

    if errors:
        print("\nKotoOS app build: FAIL")
        for error in errors:
            print(f"- {error}")
        return 1
    print("\nKotoOS app build: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
