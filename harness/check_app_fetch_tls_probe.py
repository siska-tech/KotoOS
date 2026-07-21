"""KOTO-0245 isolated embedded-tls RP2040 ELF evaluation report."""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_BASELINE_ELF = (
    ROOT / "target/app-fetch-tls-base/thumbv6m-none-eabi/release/probe_app_fetch_tls"
)
DEFAULT_HANDSHAKE_ELF = (
    ROOT
    / "target/app-fetch-tls-handshake/thumbv6m-none-eabi/release/probe_app_fetch_tls"
)
DEFAULT_ADAPTER_ELF = (
    ROOT / "target/app-fetch-tls-adapter/thumbv6m-none-eabi/release/probe_app_fetch_tls"
)
DEFAULT_VERIFIER_ELF = (
    ROOT / "target/app-fetch-tls-verifier/thumbv6m-none-eabi/release/probe_app_fetch_tls"
)
DEFAULT_FEASIBILITY = ROOT / "target/koto-dev/app_fetch_tls_feasibility.json"
DEFAULT_OUTPUT = ROOT / "target/koto-dev/app_fetch_tls_probe.json"
IMPLEMENTED_TLS_PCM_WORKSPACE_BYTES = 8 * 1024

SYMBOLS = {
    "connection": "APP_FETCH_TLS_CONNECTION_SIZE",
    "record_rx": "APP_FETCH_TLS_RECORD_RX_SIZE",
    "record_tx": "APP_FETCH_TLS_RECORD_TX_SIZE",
    "provider": "APP_FETCH_TLS_PROVIDER_SIZE",
    "verifier": "APP_FETCH_TLS_VERIFIER_SIZE",
    "socket_adapter": "APP_FETCH_TLS_SOCKET_ADAPTER_SIZE",
}
TASK_POOL_MARKER = "probe_app_fetch_tls::tls_handshake_layout_task::POOL"


def parse_symbols(text: str) -> dict[str, int]:
    sizes: dict[str, int] = {}
    for line in text.splitlines():
        fields = line.split(maxsplit=3)
        if len(fields) != 4:
            continue
        try:
            size = int(fields[1], 16)
        except ValueError:
            continue
        for key, symbol in SYMBOLS.items():
            if fields[3] == symbol:
                sizes[key] = size
        if TASK_POOL_MARKER in fields[3]:
            sizes["handshake_task_pool"] = size
    return sizes


def parse_sections(text: str) -> dict[str, int]:
    sections: dict[str, int] = {}
    for line in text.splitlines():
        fields = line.split()
        if len(fields) >= 2 and fields[0].startswith("."):
            try:
                sections[fields[0]] = int(fields[1])
            except ValueError:
                pass
    return sections


def build_report(
    sizes: dict[str, int],
    adapter_sizes: dict[str, int],
    verifier_sizes: dict[str, int],
    baseline_sections: dict[str, int],
    handshake_sections: dict[str, int],
    adapter_sections: dict[str, int],
    verifier_sections: dict[str, int],
    available_before_tls: int,
    safety_floor: int,
) -> tuple[dict[str, object], list[str]]:
    failures = [
        f"missing handshake ELF symbol {name}"
        for key, name in SYMBOLS.items()
        if key != "socket_adapter" and key not in sizes
    ]
    if "socket_adapter" not in adapter_sizes:
        failures.append("missing adapter ELF socket-adapter symbol")
    if "handshake_task_pool" not in sizes:
        failures.append("missing handshake task-pool symbol")
    if "handshake_task_pool" not in adapter_sizes:
        failures.append("missing adapter handshake task-pool symbol")
    for key, name in SYMBOLS.items():
        if key not in verifier_sizes:
            failures.append(f"missing verifier ELF symbol {name}")
    if "handshake_task_pool" not in verifier_sizes:
        failures.append("missing verifier handshake task-pool symbol")
    for section in (".text", ".rodata", ".bss"):
        if (
            section not in baseline_sections
            or section not in handshake_sections
            or section not in adapter_sections
            or section not in verifier_sections
        ):
            failures.append(f"missing {section} from ELF section report")
    if failures:
        return {}, failures
    if sizes["record_rx"] != 4_096 or sizes["record_tx"] != 1_024:
        failures.append("probe record buffers no longer match the 4 KiB/1 KiB profile")

    task_pool = verifier_sizes["handshake_task_pool"]
    if task_pool > IMPLEMENTED_TLS_PCM_WORKSPACE_BYTES:
        failures.append("TLS task pool no longer fits the implemented 8 KiB PCM workspace")
    remaining = available_before_tls - task_pool
    admitted = remaining >= safety_floor
    section_delta = {
        section: handshake_sections[section] - baseline_sections[section]
        for section in (".text", ".rodata", ".bss")
    }
    adapter_section_delta = {
        section: adapter_sections[section] - handshake_sections[section]
        for section in (".text", ".rodata", ".bss")
    }
    verifier_section_delta = {
        section: verifier_sections[section] - adapter_sections[section]
        for section in (".text", ".rodata", ".bss")
    }
    return {
        "schema": "koto.app-fetch-tls-probe.v1",
        "board": "picocalc-picow",
        "target": "thumbv6m-none-eabi",
        "candidate": "embedded-tls 0.19.0",
        "layout": {
            "connection_bytes": sizes["connection"],
            "record_rx_bytes": sizes["record_rx"],
            "record_tx_bytes": sizes["record_tx"],
            "fail_closed_verifier_bytes": sizes["verifier"],
            "pinned_p256_verifier_bytes": verifier_sizes["verifier"],
            "pinned_provider_bytes": verifier_sizes["provider"],
            "complete_handshake_task_pool_bytes": task_pool,
            "socket_adapter_bytes": adapter_sizes["socket_adapter"],
            "implemented_pcm_workspace_bytes": IMPLEMENTED_TLS_PCM_WORKSPACE_BYTES,
            "workspace_slack_bytes": IMPLEMENTED_TLS_PCM_WORKSPACE_BYTES - task_pool,
        },
        "constrained_rp2040_envelope": {
            "available_before_tls_task_bytes": available_before_tls,
            "remaining_after_tls_task_bytes": remaining,
            "required_safety_floor_bytes": safety_floor,
            "probe_admitted": admitted,
        },
        "incremental_sections": section_delta,
        "adapter_incremental_sections": adapter_section_delta,
        "verifier_incremental_sections": verifier_section_delta,
        "decision": (
            "reject_for_rp2040" if not admitted else "continue_controlled_endpoint_probe"
        ),
        "limitations": [
            "The 0.6-to-0.7 adapter is linked and type-checked against embassy-net TcpSocket, but no live socket is exercised by this layout probe.",
            "SPKI pin hashing and P-256 CertificateVerify code are linked, but the EOF-only probe transport cannot complete a peer handshake.",
            "The reduced record profile still requires controlled-server negotiation and fail-closed oversized-record tests.",
        ],
    }, failures


def run_tool(command: list[str]) -> tuple[str, str | None]:
    completed = subprocess.run(command, cwd=ROOT, text=True, capture_output=True)
    if completed.returncode:
        return "", completed.stderr or f"{' '.join(command)} failed"
    return completed.stdout, None


def constrained_envelope(path: Path) -> tuple[int, int]:
    report = json.loads(path.read_text(encoding="utf-8"))
    scenario = next(
        item
        for item in report["scenarios"]
        if item["name"] == "4k_rx_1k_tx_pcm_workspace"
    )
    available = (
        scenario["tls_state_and_margin_headroom_bytes"]
        + scenario["record_rx_bytes"]
        + scenario["record_tx_bytes"]
    )
    return available, report["admission_floor"]["unmeasured_tls_state_and_margin_bytes"]


def self_test() -> None:
    nm = (
        "10000000 000004f0 R APP_FETCH_TLS_CONNECTION_SIZE\n"
        "100004f0 00001000 R APP_FETCH_TLS_RECORD_RX_SIZE\n"
        "100014f0 00000400 R APP_FETCH_TLS_RECORD_TX_SIZE\n"
        "100018f0 000000a8 R APP_FETCH_TLS_PROVIDER_SIZE\n"
        "10001998 000000a2 R APP_FETCH_TLS_VERIFIER_SIZE\n"
        "20000000 00001d68 b probe_app_fetch_tls::tls_handshake_layout_task::POOL::hash\n"
    )
    adapter_nm = (
        "10000000 00000008 R APP_FETCH_TLS_SOCKET_ADAPTER_SIZE\n"
        "20000000 00001d78 b probe_app_fetch_tls::tls_handshake_layout_task::POOL::hash\n"
    )
    verifier_nm = (
        "10000000 000004f0 R APP_FETCH_TLS_CONNECTION_SIZE\n"
        "100004f0 00001000 R APP_FETCH_TLS_RECORD_RX_SIZE\n"
        "100014f0 00000400 R APP_FETCH_TLS_RECORD_TX_SIZE\n"
        "100018f0 00000108 R APP_FETCH_TLS_PROVIDER_SIZE\n"
        "100019f8 00000100 R APP_FETCH_TLS_VERIFIER_SIZE\n"
        "10001af8 00000008 R APP_FETCH_TLS_SOCKET_ADAPTER_SIZE\n"
        "20000000 00001e40 b probe_app_fetch_tls::tls_handshake_layout_task::POOL::hash\n"
    )
    baseline = ".text 3376 0\n.rodata 6384 0\n.bss 312 0\n"
    handshake = ".text 74460 0\n.rodata 7492 0\n.bss 7840 0\n"
    adapter = ".text 74580 0\n.rodata 7500 0\n.bss 7856 0\n"
    verifier = ".text 99296 0\n.rodata 7812 0\n.bss 8056 0\n"
    report, failures = build_report(
        parse_symbols(nm),
        parse_symbols(adapter_nm),
        parse_symbols(verifier_nm),
        parse_sections(baseline),
        parse_sections(handshake),
        parse_sections(adapter),
        parse_sections(verifier),
        14_262,
        4_096,
    )
    assert not failures, failures
    assert report["layout"]["connection_bytes"] == 1_264
    assert report["layout"]["complete_handshake_task_pool_bytes"] == 7_744
    assert report["layout"]["pinned_p256_verifier_bytes"] == 256
    assert report["constrained_rp2040_envelope"]["remaining_after_tls_task_bytes"] == 6_518
    assert report["decision"] == "continue_controlled_endpoint_probe"
    assert report["incremental_sections"][".text"] == 71_084
    assert report["adapter_incremental_sections"][".text"] == 120
    assert report["adapter_incremental_sections"][".bss"] == 16
    assert report["verifier_incremental_sections"][".text"] == 24_716
    assert report["verifier_incremental_sections"][".bss"] == 200
    print("App Fetch embedded-tls probe parser: OK")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--baseline-elf", type=Path, default=DEFAULT_BASELINE_ELF)
    parser.add_argument("--handshake-elf", type=Path, default=DEFAULT_HANDSHAKE_ELF)
    parser.add_argument("--adapter-elf", type=Path, default=DEFAULT_ADAPTER_ELF)
    parser.add_argument("--verifier-elf", type=Path, default=DEFAULT_VERIFIER_ELF)
    parser.add_argument("--feasibility", type=Path, default=DEFAULT_FEASIBILITY)
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return 0

    nm_text, error = run_tool(["rust-nm", "-S", "-C", str(args.handshake_elf)])
    if error:
        print(f"FAIL {error}", file=sys.stderr)
        return 1
    adapter_nm_text, error = run_tool(["rust-nm", "-S", "-C", str(args.adapter_elf)])
    if error:
        print(f"FAIL {error}", file=sys.stderr)
        return 1
    verifier_nm_text, error = run_tool(["rust-nm", "-S", "-C", str(args.verifier_elf)])
    if error:
        print(f"FAIL {error}", file=sys.stderr)
        return 1
    baseline_size, error = run_tool(["rust-size", "-A", str(args.baseline_elf)])
    if error:
        print(f"FAIL {error}", file=sys.stderr)
        return 1
    handshake_size, error = run_tool(["rust-size", "-A", str(args.handshake_elf)])
    if error:
        print(f"FAIL {error}", file=sys.stderr)
        return 1
    adapter_size, error = run_tool(["rust-size", "-A", str(args.adapter_elf)])
    if error:
        print(f"FAIL {error}", file=sys.stderr)
        return 1
    verifier_size, error = run_tool(["rust-size", "-A", str(args.verifier_elf)])
    if error:
        print(f"FAIL {error}", file=sys.stderr)
        return 1
    try:
        available, safety = constrained_envelope(args.feasibility)
    except (OSError, KeyError, StopIteration, TypeError, ValueError, json.JSONDecodeError) as error:
        print(f"FAIL could not read feasibility report: {error}", file=sys.stderr)
        return 1
    report, failures = build_report(
        parse_symbols(nm_text),
        parse_symbols(adapter_nm_text),
        parse_symbols(verifier_nm_text),
        parse_sections(baseline_size),
        parse_sections(handshake_size),
        parse_sections(adapter_size),
        parse_sections(verifier_size),
        available,
        safety,
    )
    for failure in failures:
        print(f"FAIL {failure}", file=sys.stderr)
    if failures:
        return 1
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, indent=2))
    print(f"wrote {args.output.relative_to(ROOT)}")
    print("App Fetch embedded-tls probe report: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
