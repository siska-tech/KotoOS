"""KOTO-0245 portable app-Fetch SRAM budget ELF gate."""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_ELF = ROOT / "target/thumbv6m-none-eabi/release/probe_app_fetch_service"
DEFAULT_OUTPUT = ROOT / "target/koto-dev/app_fetch_budget.json"
CONTROL_PLANE_CEILING = 3 * 1024
TRANSPORT_MAILBOX_CEILING = 640

SYMBOLS = {
    "service": "APP_FETCH_SERVICE_CONTROL_SIZE",
    "allowlist": "APP_FETCH_ALLOWLIST_SIZE",
    "http_decoder": "APP_FETCH_HTTP_DECODER_SIZE",
    "pin_table": "APP_FETCH_PIN_TABLE_SIZE",
    "transport_mailbox": "APP_FETCH_TRANSPORT_MAILBOX_SIZE",
}


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
    return sizes


def build_report(sizes: dict[str, int]) -> tuple[dict[str, object], list[str]]:
    failures = [
        f"missing ELF symbol {symbol}"
        for key, symbol in SYMBOLS.items()
        if key not in sizes
    ]
    if failures:
        return {}, failures
    mailbox = sizes["transport_mailbox"]
    control_sizes = {key: value for key, value in sizes.items() if key != "transport_mailbox"}
    total = sum(control_sizes.values())
    if total > CONTROL_PLANE_CEILING:
        failures.append(
            f"Fetch control plane {total} exceeds {CONTROL_PLANE_CEILING} bytes"
        )
    if mailbox > TRANSPORT_MAILBOX_CEILING:
        failures.append(
            f"Fetch transport mailbox {mailbox} exceeds {TRANSPORT_MAILBOX_CEILING} bytes"
        )
    return {
        "schema": "koto.app-fetch-budget.v1",
        "board": "picocalc-pico",
        "target": "thumbv6m-none-eabi",
        "control_plane": {
            "bytes": total,
            "ceiling": CONTROL_PLANE_CEILING,
            "within": total <= CONTROL_PLANE_CEILING,
            "components": control_sizes,
        },
        "transport_mailbox": {
            "bytes": mailbox,
            "ceiling": TRANSPORT_MAILBOX_CEILING,
            "within": mailbox <= TRANSPORT_MAILBOX_CEILING,
            "placement": "Wi-Fi arena driver reservation; no product static allocation",
        },
        "excluded_until_device_https": [
            "dns_query_storage",
            "tcp_socket_buffers",
            "tls_workspace",
        ],
        "note": (
            "The service row uses UnavailableFetchBackend, which is zero-sized "
            "and links no DNS, socket, TLS, timer, or retry implementation."
        ),
    }, failures


def self_test() -> None:
    sample = (
        "10000000 00000048 R APP_FETCH_SERVICE_CONTROL_SIZE\n"
        "10000048 0000040a R APP_FETCH_ALLOWLIST_SIZE\n"
        "10000452 00000434 R APP_FETCH_HTTP_DECODER_SIZE\n"
        "10000886 0000010c R APP_FETCH_PIN_TABLE_SIZE\n"
        "10000992 00000254 R APP_FETCH_TRANSPORT_MAILBOX_SIZE\n"
    )
    report, failures = build_report(parse_symbols(sample))
    assert not failures, failures
    assert report["control_plane"]["bytes"] == 2_450
    assert report["control_plane"]["within"] is True
    assert report["transport_mailbox"]["bytes"] == 596
    _, failures = build_report(
        {
            "service": 100,
            "allowlist": 1_500,
            "http_decoder": 1_500,
            "pin_table": 268,
            "transport_mailbox": 596,
        }
    )
    assert any("exceeds" in failure for failure in failures)
    print("App Fetch budget parser: OK")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--elf", type=Path)
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        if args.elf is None:
            return 0

    elf_arg = args.elf or DEFAULT_ELF
    elf = elf_arg if elf_arg.is_absolute() else ROOT / elf_arg
    if not elf.is_file():
        print(f"missing app Fetch budget probe ELF: {elf}", file=sys.stderr)
        return 1
    completed = subprocess.run(
        ["rust-nm", "-S", str(elf)], cwd=ROOT, text=True, capture_output=True
    )
    if completed.returncode != 0:
        print(completed.stderr, end="", file=sys.stderr)
        return completed.returncode
    report, failures = build_report(parse_symbols(completed.stdout))
    if report:
        output = args.output if args.output.is_absolute() else ROOT / args.output
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
        print(json.dumps(report, indent=2))
        print(f"wrote {output.relative_to(ROOT)}")
    for failure in failures:
        print(f"FAIL {failure}", file=sys.stderr)
    if failures:
        return 1
    print("App Fetch budget gate: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
