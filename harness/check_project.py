from __future__ import annotations

import json
import re
import struct
import sys
from pathlib import Path
from urllib.parse import unquote

from asset_pipeline import KFont, pixels_to_kicon, pixels_to_pbm, read_p1_pbm, render_text, verify_layout


ROOT = Path(__file__).resolve().parents[1]
DOCS = ROOT / "docs"
ISSUES = DOCS / "issues"
FIXTURE = ROOT / "harness" / "fixtures" / "sample_app.kpa.json"
FIXTURE_LAYOUT = ROOT / "harness" / "fixtures" / "sample_app.layout.csv"
NON_MONOTONIC_LAYOUT = ROOT / "harness" / "fixtures" / "non_monotonic.layout.csv"
ASSET_PIPELINE_FIXTURE = ROOT / "harness" / "fixtures" / "asset_pipeline"
KOTOUI_ABI_FIXTURES = ROOT / "harness" / "fixtures" / "koto_ui_abi"
NETWORK_SERVICE_FIXTURES = ROOT / "harness" / "fixtures" / "network_service"
SDCARD_APPS = ROOT / "sdcard_mock" / "apps"
PACKAGE_MANIFESTS = ROOT / "package_inputs" / "manifests"
PACKAGE_INPUTS = ROOT / "package_inputs"
CARGO_ROOT = ROOT / "Cargo.toml"
KOTO_PICO_SRC = ROOT / "src" / "koto-pico" / "src"
BOARD_SRC = KOTO_PICO_SRC / "board"
KPA_REQUIRED_KEYS = ["format", "version", "app_id", "name", "entry", "runtime", "assets", "permissions"]

REQ_ID_RE = re.compile(r"\b(?:HC-\d+|FR-[A-Z]+-\d+|NFR-[A-Z]+-\d+)\b")
ISSUE_ID_RE = re.compile(r"\bKOTO-\d{4}\b")
REQ_DEFINITION_RE = re.compile(
    r"^\s*(?:-\s+|\|\s*)(?P<id>(?:HC-\d+|FR-[A-Z]+-\d+|NFR-[A-Z]+-\d+))(?::|\s*\|)"
)
MD_LINK_RE = re.compile(r"\[[^\]]+\]\(([^)]+)\)")


def iter_markdown_files() -> list[Path]:
    files = [ROOT / "README.md"]
    files.extend(sorted(DOCS.rglob("*.md")))
    return [path for path in files if path.exists()]


def check_requirement_ids(errors: list[str]) -> None:
    seen: dict[str, Path] = {}
    for path in iter_markdown_files():
        lines = path.read_text(encoding="utf-8").splitlines()
        for line in lines:
            match = REQ_DEFINITION_RE.match(line)
            if not match:
                continue
            req_id = match.group("id")
            if req_id in seen:
                errors.append(
                    f"duplicate requirement id {req_id}: "
                    f"{seen[req_id].relative_to(ROOT)} and {path.relative_to(ROOT)}"
                )
            else:
                seen[req_id] = path

    if not seen:
        errors.append("no requirement IDs found")


def is_external_link(target: str) -> bool:
    return (
        "://" in target
        or target.startswith("mailto:")
        or target.startswith("#")
    )


def check_markdown_links(errors: list[str]) -> None:
    for path in iter_markdown_files():
        text = path.read_text(encoding="utf-8")
        for raw_target in MD_LINK_RE.findall(text):
            target = raw_target.split("#", 1)[0].strip()
            if not target or is_external_link(target):
                continue
            local = (path.parent / unquote(target)).resolve()
            try:
                local.relative_to(ROOT)
            except ValueError:
                errors.append(f"link escapes repository: {path.relative_to(ROOT)} -> {raw_target}")
                continue
            if not local.exists():
                errors.append(f"broken link: {path.relative_to(ROOT)} -> {raw_target}")


# Issue tracks: docs/issues/<track>/ directory, ID prefix, and index document.
ISSUE_TRACKS = [
    ("main", "KOTO", "ISSUES_main.md"),
    ("kotogfx", "GFX", "ISSUES_kotogfx.md"),
    ("diagnostics", "DIAG", None),
]


def check_issue_files(errors: list[str]) -> None:
    if not ISSUES.exists():
        errors.append(f"missing issue directory: {ISSUES.relative_to(ROOT)}")
        return

    valid_statuses = (
        "todo", "in-progress", "in progress", "done",
        "implemented", "proposed", "proposal",
    )
    total_seen = 0
    for track, prefix, index_name in ISSUE_TRACKS:
        track_dir = ISSUES / track
        if not track_dir.exists():
            errors.append(f"missing issue track directory: {track_dir.relative_to(ROOT)}")
            continue

        seen: dict[str, Path] = {}
        id_re = re.compile(rf"\b{prefix}-\d{{4}}\b")
        title_re = re.compile(rf"^#\s+({prefix}-\d{{4}}):", re.MULTILINE)
        issue_files = sorted(track_dir.glob(f"{prefix}-*.md"))
        for path in issue_files:
            text = path.read_text(encoding="utf-8")
            title_match = title_re.search(text)
            if not title_match:
                errors.append(f"issue file missing title ID: {path.relative_to(ROOT)}")
                continue

            issue_id = title_match.group(1)
            if issue_id in seen:
                errors.append(
                    f"duplicate issue id {issue_id}: "
                    f"{seen[issue_id].relative_to(ROOT)} and {path.relative_to(ROOT)}"
                )
            seen[issue_id] = path

            if issue_id not in path.name:
                errors.append(f"issue filename does not include title ID: {path.relative_to(ROOT)}")

            status_match = re.search(r"^- Status:\s+(.+)$", text, re.MULTILINE)
            if not status_match:
                errors.append(f"issue missing status: {path.relative_to(ROOT)}")
                continue
            # Statuses are free-form in practice ("**DONE 2026-07-12** —
            # device-confirmed…", "Stage 0 implemented (observe-only)");
            # require only that a recognizable state word appears.
            status = status_match.group(1).strip().lower().replace("*", "")
            if not any(tok in status for tok in valid_statuses):
                errors.append(f"issue has invalid status {status}: {path.relative_to(ROOT)}")

        total_seen += len(seen)

        if index_name is None:
            continue
        index = DOCS / index_name
        if not index.exists():
            errors.append(f"missing issue index: docs/{index_name}")
            continue
        index_text = index.read_text(encoding="utf-8")
        for issue_id, path in seen.items():
            if path.name not in index_text:
                errors.append(f"issue not listed in docs/{index_name}: {issue_id}")

        listed_ids = set(id_re.findall(index_text))
        listed_ids.discard(f"{prefix}-0000")
        missing_files = sorted(listed_ids - set(seen))
        for issue_id in missing_files:
            errors.append(f"docs/{index_name} lists issue without file: {issue_id}")

    if total_seen == 0:
        errors.append("no issue files found")


def check_sample_manifest(errors: list[str]) -> None:
    if not FIXTURE.exists():
        errors.append(f"missing fixture: {FIXTURE.relative_to(ROOT)}")
        return

    try:
        manifest = json.loads(FIXTURE.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        errors.append(f"invalid JSON in {FIXTURE.relative_to(ROOT)}: {exc}")
        return

    required = ["format", "version", "app_id", "name", "entry", "runtime", "assets", "permissions"]
    for key in required:
        if key not in manifest:
            errors.append(f"sample manifest missing key: {key}")

    if manifest.get("format") != "kpa-manifest":
        errors.append("sample manifest format must be kpa-manifest")

    assets = manifest.get("assets")
    if not isinstance(assets, list) or not assets:
        errors.append("sample manifest assets must be a non-empty list")
        return

    entry = manifest.get("entry")
    asset_paths = {asset.get("path") for asset in assets if isinstance(asset, dict)}
    if entry not in asset_paths:
        errors.append("sample manifest entry must exist in assets")
    icon = manifest.get("icon")
    if icon is not None and icon not in asset_paths:
        errors.append("sample manifest icon must exist in assets")

    for index, asset in enumerate(assets):
        if not isinstance(asset, dict):
            errors.append(f"asset {index} must be an object")
            continue
        for key in ["path", "type", "sequential"]:
            if key not in asset:
                errors.append(f"asset {index} missing key: {key}")


def validate_kpa_manifest(
    manifest_path: Path,
    manifest: dict[str, object],
    asset_root: Path,
    errors: list[str],
    app_dir: Path | None = None,
    app_asset_sources: dict[str, Path] | None = None,
) -> None:
    rel = manifest_path.relative_to(ROOT)
    for key in KPA_REQUIRED_KEYS:
        if key not in manifest:
            errors.append(f"{rel} missing key: {key}")

    if manifest.get("format") != "kpa-manifest":
        errors.append(f"{rel} format must be kpa-manifest")

    assets = manifest.get("assets")
    if not isinstance(assets, list) or not assets:
        errors.append(f"{rel} assets must be a non-empty list")
        return

    asset_paths: set[str] = set()
    for index, asset in enumerate(assets):
        if not isinstance(asset, dict):
            errors.append(f"{rel} asset {index} must be an object")
            continue
        for key in ["path", "type", "sequential"]:
            if key not in asset:
                errors.append(f"{rel} asset {index} missing key: {key}")
        path = asset.get("path")
        if not isinstance(path, str) or not path:
            errors.append(f"{rel} asset {index} path must be a non-empty string")
            continue
        if Path(path).is_absolute() or ".." in Path(path).parts:
            errors.append(f"{rel} asset {index} path must stay under sdcard_mock: {path}")
            continue
        if path in asset_paths:
            errors.append(f"{rel} contains duplicate asset path: {path}")
        asset_paths.add(path)
        asset_file = asset_root / path
        if not asset_file.exists() and app_asset_sources is not None:
            asset_file = app_asset_sources.get(path, asset_file)
        if (
            not asset_file.exists()
            and app_dir is not None
            and asset.get("type") == "data"
            and path.endswith(".map")
        ):
            asset_file = app_dir / path
        if not asset_file.exists():
            errors.append(f"{rel} references missing asset: {path}")
            continue
        if asset.get("type") == "image" and path.endswith(".kicon"):
            validate_kicon(asset_file, errors)

    entry = manifest.get("entry")
    if not isinstance(entry, str) or entry not in asset_paths:
        errors.append(f"{rel} entry must exist in assets")
    icon = manifest.get("icon")
    if icon is not None and (not isinstance(icon, str) or icon not in asset_paths):
        errors.append(f"{rel} icon must exist in assets")


def validate_kicon(path: Path, errors: list[str]) -> None:
    rel = path.relative_to(ROOT)
    lines = path.read_text(encoding="ascii").splitlines()
    if len(lines) != 41 or lines[0] != "KICON1":
        errors.append(f"{rel} must be KICON1 plus 40 bitmap rows")
        return
    for row_index, row in enumerate(lines[1:], start=1):
        if len(row) != 40 or any(char not in "#." for char in row):
            errors.append(f"{rel} row {row_index} must contain exactly 40 #/. pixels")
            return


def read_layout_csv(path: Path, errors: list[str]) -> list[dict[str, str]]:
    if not path.exists():
        errors.append(f"missing fixture layout: {path.relative_to(ROOT)}")
        return []

    lines = path.read_text(encoding="utf-8").splitlines()
    if not lines:
        errors.append(f"fixture layout is empty: {path.relative_to(ROOT)}")
        return []
    if lines[0] != "path,type,offset,size,alignment,flags,padding_before":
        errors.append(f"fixture layout has unexpected header: {path.relative_to(ROOT)}")
        return []

    rows: list[dict[str, str]] = []
    fields = lines[0].split(",")
    for line_number, line in enumerate(lines[1:], start=2):
        values = line.split(",")
        if len(values) != len(fields):
            errors.append(
                f"fixture layout row {line_number} has {len(values)} fields: {path.relative_to(ROOT)}"
            )
            continue
        rows.append(dict(zip(fields, values)))
    return rows


def layout_monotonic_error(rows: list[dict[str, str]]) -> str | None:
    previous_path: str | None = None
    previous_end: int | None = None
    for row in rows:
        try:
            offset = int(row["offset"])
            size = int(row["size"])
        except (KeyError, ValueError):
            return "layout offsets and sizes must be integers"

        if previous_end is not None and offset < previous_end:
            return (
                f"layout offset for {row.get('path', '<unknown>')} is non-monotonic "
                f"after {previous_path}"
            )
        previous_path = row.get("path")
        previous_end = offset + size
    return None


def check_fixture_layout(errors: list[str]) -> None:
    rows = read_layout_csv(FIXTURE_LAYOUT, errors)
    if not rows:
        return

    manifest_errors: list[str] = []
    manifest = json.loads(FIXTURE.read_text(encoding="utf-8"))
    manifest_paths = [asset["path"] for asset in manifest["assets"]]
    layout_paths = [row["path"] for row in rows]
    if layout_paths != manifest_paths:
        errors.append("fixture layout paths must match manifest asset order")

    if layout_monotonic_error(rows) is not None:
        errors.append("fixture layout must use monotonic asset offsets")

    if not all(row.get("offset", "").isdigit() for row in rows):
        errors.append("fixture layout records must include numeric asset offsets")

    bad_rows = read_layout_csv(NON_MONOTONIC_LAYOUT, manifest_errors)
    if manifest_errors:
        errors.extend(manifest_errors)
    elif layout_monotonic_error(bad_rows) is None:
        errors.append("layout harness failed to detect non-monotonic asset offsets")


def check_asset_pipeline(errors: list[str]) -> None:
    src_icon = ASSET_PIPELINE_FIXTURE / "icon_40.pbm"
    generated_root = ASSET_PIPELINE_FIXTURE / "package_assets"
    generated_icon = generated_root / "icons" / "pipeline.kicon"
    generated_preview = generated_root / "previews" / "pipeline_icon.pbm"
    font_path = generated_root / "fonts" / "mplus10.kfont"
    font_preview = ASSET_PIPELINE_FIXTURE / "font_preview.txt"
    manifest_path = ASSET_PIPELINE_FIXTURE / "asset_pipeline.kpa.json"
    layout_path = ASSET_PIPELINE_FIXTURE / "asset_pipeline.layout.csv"

    for path in [src_icon, generated_icon, generated_preview, font_path, font_preview, manifest_path, layout_path]:
        if not path.exists():
            errors.append(f"missing asset pipeline fixture: {path.relative_to(ROOT)}")
            return

    try:
        pixels = read_p1_pbm(src_icon)
        if generated_icon.read_text(encoding="ascii") != pixels_to_kicon(pixels):
            errors.append("asset pipeline icon fixture is stale")
        expected_preview = pixels_to_pbm(pixels, f"preview generated from {src_icon.name}")
        if generated_preview.read_text(encoding="ascii") != expected_preview:
            errors.append("asset pipeline icon preview fixture is stale")

        font = KFont.read(font_path)
        if font.cell_h <= 0 or font.ascent <= 0 or font.half_w <= 0 or font.full_w <= 0:
            errors.append("asset pipeline font metrics must be non-zero")
        rendered = render_text(font, "Koto日本語")
        if font_preview.read_text(encoding="utf-8") != rendered:
            errors.append("asset pipeline font preview fixture is stale")

        verify_layout(manifest_path, layout_path)
        rows = read_layout_csv(layout_path, errors)
        for row in rows:
            asset_path = generated_root / row["path"]
            if not asset_path.exists():
                errors.append(f"asset pipeline manifest references missing asset: {row['path']}")
                continue
            if asset_path.stat().st_size != int(row["size"]):
                errors.append(f"asset pipeline layout size mismatch: {row['path']}")
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        errors.append(f"asset pipeline fixture invalid: {exc}")


def check_sdcard_mock(errors: list[str]) -> None:
    if not SDCARD_APPS.exists():
        errors.append(f"missing simulator app directory: {SDCARD_APPS.relative_to(ROOT)}")
        return

    manifests = sorted(PACKAGE_MANIFESTS.glob("*.kpa.json"))
    if not manifests:
        errors.append(f"no package manifests in {PACKAGE_MANIFESTS.relative_to(ROOT)}")
        return

    app_dirs: dict[str, Path] = {}
    app_asset_sources: dict[str, dict[str, Path]] = {}
    for descriptor_path in sorted((ROOT / "apps").glob("**/app.json")):
        try:
            descriptor = json.loads(descriptor_path.read_text(encoding="utf-8"))
        except json.JSONDecodeError:
            continue
        package = descriptor.get("package")
        if isinstance(package, str):
            app_dirs[package] = descriptor_path.parent
            sources = {}
            for asset in descriptor.get("assets", []):
                if isinstance(asset, dict) and isinstance(asset.get("source"), str):
                    output = asset.get("output", asset["source"])
                    if isinstance(output, str):
                        sources[output] = descriptor_path.parent / asset["source"]
            app_asset_sources[package] = sources

    for manifest_path in manifests:
        try:
            manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        except json.JSONDecodeError as exc:
            errors.append(f"invalid JSON in {manifest_path.relative_to(ROOT)}: {exc}")
            continue
        package_stem = manifest_path.name.removesuffix(".kpa.json")
        app_dir = app_dirs.get(package_stem)
        validate_kpa_manifest(
            manifest_path,
            manifest,
            PACKAGE_INPUTS,
            errors,
            app_dir,
            app_asset_sources.get(package_stem),
        )
        package_path = SDCARD_APPS / f"{package_stem}.kpa"
        if app_dir is not None and package_path.exists():
            try:
                payloads = read_kpa_payloads(package_path)
                sources = app_asset_sources.get(package_stem, {})
                for asset in manifest.get("assets", []):
                    if (
                        isinstance(asset, dict)
                        and asset.get("type") == "data"
                        and isinstance(asset.get("path"), str)
                    ):
                        source = sources.get(asset["path"])
                        if source is None and asset["path"].endswith(".map"):
                            source = app_dir / asset["path"]
                        if source is not None and payloads.get(asset["path"]) != source.read_bytes():
                            errors.append(
                                f"{package_path.relative_to(ROOT)} data payload differs from "
                                f"{source.relative_to(ROOT)}"
                            )
            except (OSError, ValueError, struct.error) as exc:
                errors.append(f"invalid KPA map payloads in {package_path.relative_to(ROOT)}: {exc}")
    packages = sorted(SDCARD_APPS.glob("*.kpa"))
    if len(packages) != len(manifests) - int((PACKAGE_MANIFESTS / "sample_app.kpa.json").exists()):
        errors.append("binary KPA count does not match registered package manifests")
        if manifest.get("app_id") == "dev.koto.memo":
            if manifest.get("runtime") != "kotoruntime-bytecode":
                errors.append(f"{manifest_path.relative_to(ROOT)} must use kotoruntime-bytecode")
            if manifest.get("entry") != "bytecode/memo.kbc":
                errors.append(f"{manifest_path.relative_to(ROOT)} must launch bytecode/memo.kbc")
            permissions = manifest.get("permissions")
            if not isinstance(permissions, dict) or permissions.get("fs") != "sandbox":
                errors.append(f"{manifest_path.relative_to(ROOT)} must request sandbox fs permission")


def read_kpa_payloads(path: Path) -> dict[str, bytes]:
    """Read package path->payload pairs for app-source parity checks."""
    data = path.read_bytes()
    if len(data) < 64 or data[:4] != b"KPA1":
        raise ValueError("bad KPA1 header")
    entry_count, table_offset, string_offset, string_size = struct.unpack_from("<IIII", data, 16)
    if string_offset + string_size > len(data):
        raise ValueError("string table outside package")
    payloads: dict[str, bytes] = {}
    for index in range(entry_count):
        entry = table_offset + index * 64
        if entry + 64 > len(data):
            raise ValueError("entry table outside package")
        path_offset, path_len = struct.unpack_from("<II", data, entry)
        data_offset, data_size = struct.unpack_from("<II", data, entry + 16)
        start = string_offset + path_offset
        end = start + path_len
        payload_end = data_offset + data_size
        if end > string_offset + string_size or payload_end > len(data):
            raise ValueError("entry range outside package")
        package_path = data[start:end].decode("utf-8")
        payloads[package_path] = data[data_offset:payload_end]
    return payloads


def check_rust_workspace(errors: list[str]) -> None:
    expected = [
        CARGO_ROOT,
        ROOT / "src" / "koto-core" / "Cargo.toml",
        ROOT / "src" / "koto-core" / "src" / "lib.rs",
        ROOT / "src" / "koto-core" / "src" / "fs.rs",
        ROOT / "src" / "koto-core" / "src" / "hal.rs",
        ROOT / "src" / "koto-core" / "src" / "package.rs",
        ROOT / "src" / "koto-core" / "src" / "shell.rs",
        ROOT / "src" / "koto-sim" / "Cargo.toml",
        ROOT / "src" / "koto-sim" / "src" / "main.rs",
    ]
    for path in expected:
        if not path.exists():
            errors.append(f"missing Rust workspace file: {path.relative_to(ROOT)}")

    if CARGO_ROOT.exists():
        cargo_text = CARGO_ROOT.read_text(encoding="utf-8")
        for member in ['"src/koto-core"', '"src/koto-sim"']:
            if member not in cargo_text:
                errors.append(f"Cargo workspace missing member {member}")


def check_board_boundary(errors: list[str]) -> None:
    """Keep physical GPIO type names inside the selected board adapter."""
    if not BOARD_SRC.exists():
        errors.append(f"missing board profile directory: {BOARD_SRC.relative_to(ROOT)}")
        return
    pin_type = re.compile(r"\bPIN_\d+\b")
    for path in sorted(KOTO_PICO_SRC.rglob("*.rs")):
        if BOARD_SRC in path.parents:
            continue
        if pin_type.search(path.read_text(encoding="utf-8")):
            errors.append(
                "board GPIO type escaped src/koto-pico/src/board: "
                f"{path.relative_to(ROOT)}"
            )


def check_koto_ui_abi_fixtures(errors: list[str]) -> None:
    valid_path = KOTOUI_ABI_FIXTURES / "valid_panel_button_mount.hex"
    truncated_path = KOTOUI_ABI_FIXTURES / "invalid_truncated_mount.hex"
    capabilities_path = KOTOUI_ABI_FIXTURES / "valid_en_us_capabilities.hex"
    for path in (
        valid_path,
        truncated_path,
        capabilities_path,
        KOTOUI_ABI_FIXTURES / "README.md",
    ):
        if not path.exists():
            errors.append(f"missing KotoUI ABI fixture: {path.relative_to(ROOT)}")
    if not valid_path.exists() or not truncated_path.exists():
        return

    def decode(path: Path) -> bytes | None:
        encoded = path.read_text(encoding="ascii").strip()
        if not re.fullmatch(r"[0-9a-f]+", encoded) or len(encoded) % 2 != 0:
            errors.append(f"invalid lowercase hex fixture: {path.relative_to(ROOT)}")
            return None
        return bytes.fromhex(encoded)

    valid = decode(valid_path)
    truncated = decode(truncated_path)
    capabilities = decode(capabilities_path) if capabilities_path.exists() else None
    if valid is not None:
        if len(valid) != 142:
            errors.append(f"KotoUI valid mount must be 142 bytes, got {len(valid)}")
        elif valid[:4] != b"KUI1":
            errors.append("KotoUI valid mount has wrong magic")
        else:
            total_len, = struct.unpack_from("<I", valid, 8)
            node_count, node_stride = struct.unpack_from("<HH", valid, 12)
            nodes_offset, data_offset, data_len = struct.unpack_from("<III", valid, 16)
            root_id, initial_focus_id = struct.unpack_from("<HH", valid, 28)
            second_id, second_parent = struct.unpack_from("<HH", valid, 88)
            expected = (142, 2, 48, 40, 136, 6, 1, 2, 2, 1)
            actual = (
                total_len, node_count, node_stride, nodes_offset, data_offset,
                data_len, root_id, initial_focus_id, second_id, second_parent,
            )
            if actual != expected:
                errors.append(f"KotoUI valid mount header/node contract drifted: {actual}")
            if valid[136:142] != b"DemoOK":
                errors.append("KotoUI valid mount data payload drifted")
    if truncated is not None and len(truncated) >= 40:
        errors.append("KotoUI truncated mount fixture must be shorter than its header")
    if capabilities is not None:
        if len(capabilities) != 64:
            errors.append(
                f"KotoUI capabilities must be 64 bytes, got {len(capabilities)}"
            )
        elif capabilities[:4] != b"KUC1":
            errors.append("KotoUI capabilities have wrong magic")
        else:
            locale_len, direction = struct.unpack_from("<BB", capabilities, 32)
            generation, = struct.unpack_from("<I", capabilities, 36)
            locale = capabilities[40:40 + locale_len]
            if (locale_len, direction, generation, locale) != (5, 0, 1, b"en-US"):
                errors.append("KotoUI canonical locale capability drifted")
            if any(capabilities[40 + locale_len:64]):
                errors.append("KotoUI canonical locale padding must be zero")


def check_fake_network_fixtures(errors: list[str]) -> None:
    """Keep KotoSim fake networking deterministic and host-network-free."""
    paths = sorted(NETWORK_SERVICE_FIXTURES.glob("*.json"))
    if not paths:
        errors.append(
            f"no fake-network fixtures in {NETWORK_SERVICE_FIXTURES.relative_to(ROOT)}"
        )
        return

    expected_limits = {
        "ssid_bytes": 32,
        "scan_results": 16,
        "credential_bytes": 63,
        "status_records": 8,
        "command_queue": 4,
        "event_queue": 8,
    }
    nondeterministic_fields = {
        "timestamp", "timestamp_ms", "wall_clock", "wall_clock_ms",
        "random", "random_seed", "rng_seed", "dns", "hostname", "url",
        "socket", "interface", "host_network", "sleep_ms",
    }

    def reject_duplicate_keys(pairs: list[tuple[str, object]]) -> dict[str, object]:
        result: dict[str, object] = {}
        for key, value in pairs:
            if key in result:
                raise ValueError(f"duplicate JSON field {key!r}")
            result[key] = value
        return result

    def inspect_value(value: object, rel: Path, location: str = "$") -> None:
        if isinstance(value, dict):
            for key, child in value.items():
                if key in nondeterministic_fields:
                    errors.append(f"{rel} adds nondeterministic field {location}.{key}")
                inspect_value(child, rel, f"{location}.{key}")
        elif isinstance(value, list):
            for index, child in enumerate(value):
                inspect_value(child, rel, f"{location}[{index}]")
        elif isinstance(value, float):
            errors.append(f"{rel} uses non-integer number at {location}")

    for path in paths:
        rel = path.relative_to(ROOT)
        try:
            fixture = json.loads(
                path.read_text(encoding="utf-8"),
                object_pairs_hook=reject_duplicate_keys,
            )
        except (OSError, ValueError, json.JSONDecodeError) as exc:
            errors.append(f"invalid fake-network fixture {rel}: {exc}")
            continue
        inspect_value(fixture, rel)
        if not isinstance(fixture, dict):
            errors.append(f"{rel} fake-network fixture root must be an object")
            continue
        if fixture.get("schema") != "koto.fake-network-service.v1":
            errors.append(f"{rel} has unsupported fake-network schema")
        if fixture.get("tick_unit_ms") != 100:
            errors.append(f"{rel} tick_unit_ms must remain the fixed 100 ms unit")
        if fixture.get("limits") != expected_limits:
            errors.append(f"{rel} limits differ from the bounded NetworkService contract")
        if not isinstance(fixture.get("networks"), list):
            errors.append(f"{rel} networks must be a list")
        if not isinstance(fixture.get("scenarios"), list) or not fixture["scenarios"]:
            errors.append(f"{rel} scenarios must be a non-empty list")

    cargo = (ROOT / "src" / "koto-sim" / "Cargo.toml").read_text(encoding="utf-8")
    for dependency in ("reqwest", "ureq", "hyper", "socket2", "dns-lookup"):
        if re.search(rf"(?m)^\s*{re.escape(dependency)}\s*=", cargo):
            errors.append(f"koto-sim fake path must not add host-network dependency {dependency}")

    fake_source = (
        ROOT / "src" / "koto-sim" / "src" / "runtime" / "fake_network.rs"
    ).read_text(encoding="utf-8")
    forbidden_apis = (
        "std::net", "tokio::net", "TcpStream", "UdpSocket", "ToSocketAddrs",
        "SystemTime", "Instant::now", "thread_rng", "rand::",
    )
    for api in forbidden_apis:
        if api in fake_source:
            errors.append(f"fake NetworkService uses forbidden host/nondeterministic API {api}")


def main() -> int:
    # Windows consoles may default to a legacy code page (e.g. cp932) that
    # cannot encode characters quoted from the docs; never let that mask the
    # actual check result.
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(errors="replace")
    errors: list[str] = []
    check_requirement_ids(errors)
    check_markdown_links(errors)
    check_issue_files(errors)
    check_sample_manifest(errors)
    check_fixture_layout(errors)
    check_asset_pipeline(errors)
    check_sdcard_mock(errors)
    check_rust_workspace(errors)
    check_board_boundary(errors)
    check_koto_ui_abi_fixtures(errors)
    check_fake_network_fixtures(errors)

    if errors:
        print("KotoOS harness: FAIL")
        for error in errors:
            print(f"- {error}")
        return 1

    print("KotoOS harness: OK")
    print(f"- checked {len(iter_markdown_files())} markdown files")
    issue_count = sum(
        len(list((ISSUES / track).glob(f"{prefix}-*.md")))
        for track, prefix, _ in ISSUE_TRACKS
        if (ISSUES / track).exists()
    )
    print(f"- checked {issue_count} issue files")
    print(f"- checked {FIXTURE.relative_to(ROOT)}")
    print(f"- checked {ASSET_PIPELINE_FIXTURE.relative_to(ROOT)}")
    print(f"- checked {SDCARD_APPS.relative_to(ROOT)}")
    print(f"- checked {CARGO_ROOT.relative_to(ROOT)}")
    print("- checked koto-pico board/GPIO boundary")
    print(f"- checked {KOTOUI_ABI_FIXTURES.relative_to(ROOT)}")
    print(f"- checked {NETWORK_SERVICE_FIXTURES.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
