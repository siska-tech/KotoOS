"""KOTO-0239 bounded NetworkService SRAM budget ELF gate.

Parses the `probe_network_service` ELF size symbols and checks the KOTO-0224
RP2040 network allocation rows this issue owns against their ceilings:

* the bounded embassy-net IP-stack storage (resources + application sockets),
* the CPU0 NetworkService plus page controller state.

The CYW43 driver storage, runner-future reserve, and the exact 36 KiB switchable
arena total are owned by `check_wifi_residency_layout.py` (KOTO-0227); this gate
covers only the network-service rows added by KOTO-0239.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_ELF = ROOT / "target/thumbv6m-none-eabi/release/probe_network_service"
DEFAULT_OUTPUT = ROOT / "target/koto-dev/network_service_budget.json"

# KOTO-0224 RP2040 Pico W ceilings (octets, including padding).
IP_STACK_CEILING = 12_288
SERVICE_CEILING = 4_096
ARENA_BYTES = 36 * 1024

SYMBOLS = {
    "ip_stack_storage": "NETWORK_STACK_STORAGE_SIZE",
    "stack_resources": "NETWORK_STACK_RESOURCES_SIZE",
    "stack_runner_value": "NETWORK_STACK_RUNNER_SIZE",
    "network_service": "NETWORK_SERVICE_SIZE",
    "page_controller": "NETWORK_PAGE_CONTROLLER_SIZE",
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
        name = fields[3]
        for key, marker in SYMBOLS.items():
            if name == marker:
                sizes[key] = size
    return sizes


def build_report(sizes: dict[str, int]) -> tuple[dict[str, object], list[str]]:
    failures = [f"missing ELF symbol {name}" for key, name in SYMBOLS.items() if key not in sizes]
    if failures:
        return {}, failures

    service_row = sizes["network_service"] + sizes["page_controller"]
    ip_stack_row = sizes["ip_stack_storage"]

    if ip_stack_row > IP_STACK_CEILING:
        failures.append(f"IP-stack storage {ip_stack_row} exceeds {IP_STACK_CEILING}")
    if service_row > SERVICE_CEILING:
        failures.append(
            f"NetworkService+page state {service_row} exceeds {SERVICE_CEILING}"
        )

    report: dict[str, object] = {
        "schema": "koto.network-service-budget.v1",
        "board": "picocalc-picow",
        "target": "thumbv6m-none-eabi",
        "arena_bytes": ARENA_BYTES,
        "rows": {
            "ip_stack": {
                "bytes": ip_stack_row,
                "ceiling": IP_STACK_CEILING,
                "within": ip_stack_row <= IP_STACK_CEILING,
                "components": {
                    "stack_resources": sizes["stack_resources"],
                    "application_socket_windows": ip_stack_row - sizes["stack_resources"],
                },
            },
            "network_service_and_page": {
                "bytes": service_row,
                "ceiling": SERVICE_CEILING,
                "within": service_row <= SERVICE_CEILING,
                "components": {
                    "network_service": sizes["network_service"],
                    "page_controller": sizes["page_controller"],
                },
            },
        },
        "stack_runner_value_bytes": sizes["stack_runner_value"],
        "note": (
            "CYW43 driver storage, runner-future reserve, and the exact 36 KiB "
            "switchable arena are gated by check_wifi_residency_layout.py; the "
            "network residency is placed inside that arena. networking-disabled "
            "product builds exclude the network_service feature and link no "
            "embassy-net/NetworkService state."
        ),
    }
    return report, failures


def self_test() -> None:
    text = (
        "10000000 0000030a R NETWORK_PAGE_CONTROLLER_SIZE\n"
        "10000400 000003e0 R NETWORK_SERVICE_SIZE\n"
        "10000800 00000c30 R NETWORK_STACK_RESOURCES_SIZE\n"
        "10001500 00000024 R NETWORK_STACK_RUNNER_SIZE\n"
        "10001600 00002430 R NETWORK_STACK_STORAGE_SIZE\n"
    )
    report, failures = build_report(parse_symbols(text))
    assert not failures, failures
    rows = report["rows"]
    assert rows["ip_stack"]["bytes"] == 9_264
    assert rows["ip_stack"]["within"] is True
    assert rows["network_service_and_page"]["bytes"] == 1_770
    assert rows["network_service_and_page"]["within"] is True

    over = dict(
        ip_stack_storage=20_000,
        stack_resources=3_120,
        stack_runner_value=36,
        network_service=992,
        page_controller=778,
    )
    _, failures = build_report(over)
    assert any("IP-stack storage" in f for f in failures), failures
    print("NetworkService budget parser: OK")


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

    elf = args.elf or DEFAULT_ELF
    if not elf.is_absolute():
        elf = ROOT / elf
    if not elf.is_file():
        print(f"missing NetworkService budget probe ELF: {elf}", file=sys.stderr)
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
    print("NetworkService budget gate: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
