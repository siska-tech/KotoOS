"""KOTO-0227 concrete CYW43 target-layout ELF gate."""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_ELF = ROOT / "target/thumbv6m-none-eabi/release/probe_wifi_residency"
DEFAULT_OUTPUT = ROOT / "target/koto-dev/wifi_residency_layout.json"
ARENA_BYTES = 36 * 1024
MIN_RESERVE_BYTES = 16 * 1024
SYMBOLS = {
    "state": "CYW43_STATE_SIZE",
    "runner": "CYW43_RUNNER_SIZE",
    "control": "CYW43_CONTROL_SIZE",
    "net_driver": "CYW43_NET_DRIVER_SIZE",
    "driver_storage": "CYW43_DRIVER_STORAGE_SIZE",
    "reserve": "CYW43_DRIVER_RESERVE_SIZE",
    "layout": "WIFI_RESIDENCY_LAYOUT_SIZE",
    "fetch_mailbox": "CYW43_FETCH_MAILBOX_SIZE",
    "driver_spare": "CYW43_DRIVER_STORAGE_SPARE_SIZE",
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

    if sizes["layout"] != ARENA_BYTES:
        failures.append(f"layout is {sizes['layout']} bytes, expected {ARENA_BYTES}")
    if sizes["driver_storage"] + sizes["reserve"] != ARENA_BYTES:
        failures.append("driver storage and reserve do not fill the arena exactly")
    if sizes["reserve"] < MIN_RESERVE_BYTES:
        failures.append(
            f"runner-future/network reserve {sizes['reserve']} is below {MIN_RESERVE_BYTES}"
        )
    if sizes["fetch_mailbox"] > sizes["driver_storage"] - sizes["state"]:
        failures.append("Fetch mailbox does not fit after CYW43 State")

    report: dict[str, object] = {
        "schema": "koto.wifi-residency-layout.v1",
        "board": "picocalc-picow",
        "target": "thumbv6m-none-eabi",
        "arena_bytes": ARENA_BYTES,
        "components": {key: sizes[key] for key in ("state", "runner", "control", "net_driver")},
        "runner_poll_scratch_bytes": 512,
        "driver_storage_bytes": sizes["driver_storage"],
        "fetch_mailbox_bytes": sizes["fetch_mailbox"],
        "driver_storage_spare_bytes": sizes["driver_spare"],
        "runner_future_and_network_reserve_bytes": sizes["reserve"],
        "checks": {
            "layout_exact_36_kib": sizes["layout"] == ARENA_BYTES,
            "reserve_at_least_16_kib": sizes["reserve"] >= MIN_RESERVE_BYTES,
            "fetch_mailbox_inside_driver_reservation": (
                sizes["fetch_mailbox"] <= sizes["driver_storage"] - sizes["state"]
            ),
        },
        "lifecycle_constraint": (
            "cyw43::Runner::run consumes self and returns a non-terminating future; "
            "runtime integration must store/poll that cancellable future in the arena "
            "and acknowledge shutdown only after it is dropped"
        ),
    }
    return report, failures


def self_test() -> None:
    text = (
        "10000000 00000010 R CYW43_CONTROL_SIZE\n"
        "10000010 00000020 R CYW43_NET_DRIVER_SIZE\n"
        "10000030 0000002c R CYW43_RUNNER_SIZE\n"
        "1000005c 00003190 R CYW43_STATE_SIZE\n"
        "100031ec 000033f0 R CYW43_DRIVER_STORAGE_SIZE\n"
        "100065dc 00005c10 R CYW43_DRIVER_RESERVE_SIZE\n"
        "1000c1ec 00009000 R WIFI_RESIDENCY_LAYOUT_SIZE\n"
        "100151ec 0000025c R CYW43_FETCH_MAILBOX_SIZE\n"
        "10015448 00000004 R CYW43_DRIVER_STORAGE_SPARE_SIZE\n"
    )
    report, failures = build_report(parse_symbols(text))
    assert not failures, failures
    assert report["driver_storage_bytes"] == 13_296
    assert report["runner_future_and_network_reserve_bytes"] == 23_568
    assert report["fetch_mailbox_bytes"] == 604
    assert report["driver_storage_spare_bytes"] == 4
    print("Wi-Fi residency layout parser: OK")


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
        print(f"missing Wi-Fi layout probe ELF: {elf}", file=sys.stderr)
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
    print("Wi-Fi residency layout gate: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
