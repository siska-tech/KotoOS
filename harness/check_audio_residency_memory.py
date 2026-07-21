"""KOTO-0227 RP2040 Audio/Wi-Fi residency ELF report and gate."""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_ELF = ROOT / "target/thumbv6m-none-eabi/release/koto_firmware"
DEFAULT_OUTPUT = ROOT / "target/koto-dev/audio_residency_memory.json"
RP2040_SRAM_BASE = 0x2000_0000
RP2040_SRAM_END = 0x2004_0000
RICH_ARENA_BYTES = 36 * 1024
TLS_EXCLUSIVE_RECLAIM_FLOOR = 14 * 1024
TLS_PCM_WORKSPACE_BYTES = 8 * 1024
KOTO_0226_DATA_BSS_BASELINE = 205_436
KOTO_0226_STATIC_SPAN_BASELINE = 205_752

SYMBOLS = {
    "rich_arena": "AUDIO_RICH_RESIDENCY",
    "stream_shared": "AUDIO_STREAM_SHARED",
    "stream_scratch": "AUDIO_STREAM_SCRATCH",
    "dma_ring": "AUDIO_DMA_RING",
    "core1_stack": "AUDIO_CORE1_STACK",
}


def parse_size_sections(text: str) -> dict[str, tuple[int, int]]:
    sections: dict[str, tuple[int, int]] = {}
    for line in text.splitlines():
        fields = line.split()
        if len(fields) != 3 or not fields[0].startswith("."):
            continue
        try:
            sections[fields[0]] = (int(fields[1]), int(fields[2]))
        except ValueError:
            continue
    return sections


def parse_nm_symbols(text: str) -> dict[str, tuple[int, int, str]]:
    found: dict[str, tuple[int, int, str]] = {}
    for line in text.splitlines():
        fields = line.split(maxsplit=3)
        if len(fields) != 4:
            continue
        raw_address, raw_size, _kind, name = fields
        try:
            address = int(raw_address, 16)
            size = int(raw_size, 16)
        except ValueError:
            continue
        for key, marker in SYMBOLS.items():
            if marker in name:
                if key in found:
                    raise ValueError(f"duplicate ELF symbol matching {marker}")
                found[key] = (address, size, name)
    return found


def run_tool(command: list[str]) -> str:
    completed = subprocess.run(command, cwd=ROOT, text=True, capture_output=True)
    if completed.returncode != 0:
        if completed.stderr:
            print(completed.stderr, end="", file=sys.stderr)
        raise RuntimeError(f"command failed: {' '.join(command)}")
    return completed.stdout


def optional_measurement(value: int | None, source: str) -> dict[str, int | str | None]:
    return {"bytes": value, "source": source if value is not None else "not-captured"}


def build_report(
    elf: Path,
    sections: dict[str, tuple[int, int]],
    symbols: dict[str, tuple[int, int, str]],
    cpu0_free_min: int | None,
    cpu1_stack_free_min: int | None,
) -> tuple[dict[str, object], list[str]]:
    failures: list[str] = []
    for section in (".data", ".bss"):
        if section not in sections:
            failures.append(f"missing ELF section {section}")
    for key, marker in SYMBOLS.items():
        if key not in symbols:
            failures.append(f"missing ELF symbol {marker}")
    if failures:
        return {}, failures

    data_size, data_addr = sections[".data"]
    bss_size, bss_addr = sections[".bss"]
    static_start = min(data_addr, bss_addr)
    static_end = max(data_addr + data_size, bss_addr + bss_size)
    data_bss = data_size + bss_size
    static_span = static_end - static_start

    symbol_sizes = {key: symbols[key][1] for key in SYMBOLS}
    symbol_addresses = {key: symbols[key][0] for key in SYMBOLS}
    permanent_stream_bytes = sum(
        symbol_sizes[key]
        for key in ("stream_shared", "stream_scratch", "dma_ring", "core1_stack")
    )
    tls_exclusive_reclaimed_bytes = sum(
        symbol_sizes[key] for key in ("stream_shared", "stream_scratch", "dma_ring")
    )
    tls_exclusive_retained_bytes = symbol_sizes["core1_stack"]
    arena_size = symbol_sizes["rich_arena"]

    if arena_size != RICH_ARENA_BYTES:
        failures.append(
            f"rich arena is {arena_size} bytes, expected exactly {RICH_ARENA_BYTES}"
        )
    if symbol_addresses["rich_arena"] % 8 != 0:
        failures.append("rich arena is not 8-byte aligned")
    if not (RP2040_SRAM_BASE <= static_start < static_end <= RP2040_SRAM_END):
        failures.append(
            f"static SRAM span 0x{static_start:08x}..0x{static_end:08x} is outside RP2040 SRAM"
        )
    if arena_size < RICH_ARENA_BYTES:
        failures.append("WifiStreamAudio recovers less than 36 KiB")
    if tls_exclusive_reclaimed_bytes < TLS_EXCLUSIVE_RECLAIM_FLOOR:
        failures.append(
            "TLSExclusive recovers less than 14 KiB without reclaiming the CPU1 stack"
        )
    if symbol_sizes["stream_shared"] < TLS_PCM_WORKSPACE_BYTES:
        failures.append("TLS PCM workspace is smaller than 8 KiB")

    report: dict[str, object] = {
        "schema": "koto.audio-residency-memory.v1",
        "board": "picocalc-picow",
        "target": "thumbv6m-none-eabi",
        "elf": str(elf.resolve().relative_to(ROOT.resolve())).replace("\\", "/"),
        "sections": {
            "data_bytes": data_size,
            "bss_bytes": bss_size,
            "data_bss_bytes": data_bss,
            "static_span_bytes": static_span,
            "static_start": f"0x{static_start:08x}",
            "static_end": f"0x{static_end:08x}",
        },
        "named_residency": {
            key: {"address": f"0x{symbol_addresses[key]:08x}", "bytes": symbol_sizes[key]}
            for key in SYMBOLS
        },
        "modes": {
            "FullAudio": {
                "permanent_stream_bytes": permanent_stream_bytes,
                "rich_audio_bytes": arena_size,
                "wifi_arena_bytes": 0,
            },
            "WifiStreamAudio": {
                "permanent_stream_bytes": permanent_stream_bytes,
                "rich_audio_bytes": 0,
                "wifi_arena_bytes": arena_size,
                "reclaimed_bytes": arena_size,
            },
            "TlsExclusive": {
                "audio_available": False,
                "wifi_arena_bytes": arena_size,
                "core1_stack_retained_bytes": tls_exclusive_retained_bytes,
                "implemented_pcm_workspace_bytes": TLS_PCM_WORKSPACE_BYTES,
                "tls_exclusive_reclaimed_bytes": tls_exclusive_reclaimed_bytes,
                "maximum_candidate_workspace_bytes": (
                    arena_size + tls_exclusive_reclaimed_bytes
                ),
            },
        },
        "hardware_margins": {
            "cpu0_phase176_free_min": optional_measurement(cpu0_free_min, "phase=176"),
            "cpu1_stack_free_min": optional_measurement(
                cpu1_stack_free_min, "phase=173 audio-summary"
            ),
        },
        "baseline_delta": {
            "koto_0226_data_bss_bytes": KOTO_0226_DATA_BSS_BASELINE,
            "data_bss_delta_bytes": data_bss - KOTO_0226_DATA_BSS_BASELINE,
            "koto_0226_static_span_bytes": KOTO_0226_STATIC_SPAN_BASELINE,
            "static_span_delta_bytes": static_span - KOTO_0226_STATIC_SPAN_BASELINE,
        },
        "checks": {
            "rich_arena_exact_36_kib": arena_size == RICH_ARENA_BYTES,
            "rich_arena_aligned_8": symbol_addresses["rich_arena"] % 8 == 0,
            "wifi_reclaimed_at_least_36_kib": arena_size >= RICH_ARENA_BYTES,
            "tls_exclusive_reclaims_at_least_14_kib": (
                tls_exclusive_reclaimed_bytes >= TLS_EXCLUSIVE_RECLAIM_FLOOR
            ),
            "tls_exclusive_retains_core1_stack": (
                tls_exclusive_retained_bytes == symbol_sizes["core1_stack"]
                and tls_exclusive_reclaimed_bytes
                == permanent_stream_bytes - symbol_sizes["core1_stack"]
            ),
            "implemented_pcm_workspace_is_8_kib": (
                symbol_sizes["stream_shared"] >= TLS_PCM_WORKSPACE_BYTES
            ),
            "static_span_inside_rp2040_sram": (
                RP2040_SRAM_BASE <= static_start < static_end <= RP2040_SRAM_END
            ),
        },
    }
    return report, failures


def self_test() -> None:
    sections = parse_size_sections(
        ".data 66248 536870912\n.bss 139336 536937472\nTotal 1054957\n"
    )
    symbols = parse_nm_symbols(
        "20032000 00000400 b hash::AUDIO_DMA_RING\n"
        "2002e838 0000142c B AUDIO_STREAM_SCRATCH\n"
        "2001f680 00002000 b hash::AUDIO_CORE1_STACK\n"
        "2002fc64 0000200c B AUDIO_STREAM_SHARED\n"
        "20025838 00009000 B AUDIO_RICH_RESIDENCY\n"
    )
    report, failures = build_report(
        DEFAULT_ELF, sections, symbols, cpu0_free_min=15_724, cpu1_stack_free_min=4096
    )
    assert not failures, failures
    assert report["modes"]["WifiStreamAudio"]["reclaimed_bytes"] == RICH_ARENA_BYTES
    assert report["modes"]["TlsExclusive"]["tls_exclusive_reclaimed_bytes"] == 14_392
    assert report["modes"]["TlsExclusive"]["core1_stack_retained_bytes"] == 8_192
    assert report["modes"]["TlsExclusive"]["implemented_pcm_workspace_bytes"] == 8_192
    assert report["modes"]["TlsExclusive"]["maximum_candidate_workspace_bytes"] == 51_256
    assert report["sections"]["data_bss_bytes"] == 205_584
    assert report["hardware_margins"]["cpu0_phase176_free_min"]["bytes"] == 15_724
    print("Audio residency memory parser: OK")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--elf", type=Path)
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument("--cpu0-free-min", type=int)
    parser.add_argument("--cpu1-stack-free-min", type=int)
    parser.add_argument("--self-test", action="store_true")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if args.self_test:
        self_test()
        if args.elf is None:
            return 0
    elf = args.elf or DEFAULT_ELF
    if not elf.is_absolute():
        elf = ROOT / elf
    if not elf.is_file():
        print(f"missing Pico W ELF: {elf}", file=sys.stderr)
        return 1

    try:
        sections = parse_size_sections(run_tool(["rust-size", "-A", str(elf)]))
        symbols = parse_nm_symbols(run_tool(["rust-nm", "-S", str(elf)]))
        report, failures = build_report(
            elf,
            sections,
            symbols,
            args.cpu0_free_min,
            args.cpu1_stack_free_min,
        )
    except (RuntimeError, ValueError) as error:
        print(error, file=sys.stderr)
        return 1

    if report:
        output = args.output if args.output.is_absolute() else ROOT / args.output
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
        print(json.dumps(report, indent=2))
        print(f"wrote {output.relative_to(ROOT)}")
    if failures:
        for failure in failures:
            print(f"FAIL {failure}", file=sys.stderr)
        return 1
    print("Audio residency memory gate: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
