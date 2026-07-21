"""KOTO-0245 RP2040 TLS workspace admission-envelope check.

This is deliberately a preflight check, not proof that a TLS implementation
fits.  It makes the currently measured arena costs and the remaining budget
for the as-yet-unmeasured TLS connection/future state explicit.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_WIFI_REPORT = ROOT / "target/koto-dev/wifi_residency_layout.json"
DEFAULT_NETWORK_REPORT = ROOT / "target/koto-dev/network_service_budget.json"
DEFAULT_FETCH_REPORT = ROOT / "target/koto-dev/app_fetch_budget.json"
DEFAULT_OUTPUT = ROOT / "target/koto-dev/app_fetch_tls_feasibility.json"

# On-hardware KOTO-0239 measurement. This future includes the 9,264-byte
# embassy-net stack but not the separately installed CYW43 driver storage.
DEFAULT_NETWORK_FUTURE_BYTES = 15_048

# A candidate needs this much space after its record buffers and known Fetch
# state. The TLS connection/future has not been measured yet, so satisfying
# this floor only admits a target probe; it never enables product HTTPS.
UNMEASURED_TLS_STATE_AND_MARGIN_FLOOR = 4 * 1024
# KOTO-0227 measured permanent stream-audio residency. TLS exclusion keeps the
# 8 KiB CPU1 stack reserved until a separate stop/restart proof exists, and
# reclaims only PCM, decode/refill scratch, and DMA storage.
PERMANENT_STREAM_AUDIO_BYTES = 22_584
CORE1_AUDIO_STACK_BYTES = 8_192
TLS_AUDIO_EXCLUSIVE_RECLAIM_BYTES = (
    PERMANENT_STREAM_AUDIO_BYTES - CORE1_AUDIO_STACK_BYTES
)
IMPLEMENTED_TLS_PCM_WORKSPACE_BYTES = 8_192


def scenario(
    name: str,
    arena_free: int,
    portable_fetch: int,
    rx: int,
    tx: int,
    reclaimed: int = 0,
    overlaid: int = 0,
    audio_reclaimed: int = 0,
) -> dict[str, object]:
    concurrent_fetch = portable_fetch - overlaid
    headroom = (
        arena_free
        + reclaimed
        + audio_reclaimed
        - concurrent_fetch
        - rx
        - tx
    )
    return {
        "name": name,
        "record_rx_bytes": rx,
        "record_tx_bytes": tx,
        "known_concurrent_fetch_bytes": concurrent_fetch,
        "reclaimed_socket_window_bytes": reclaimed,
        "tls_exclusive_audio_reclaimed_bytes": audio_reclaimed,
        "lifecycle_overlaid_bytes": overlaid,
        "tls_state_and_margin_headroom_bytes": headroom,
        "probe_admitted": headroom >= UNMEASURED_TLS_STATE_AND_MARGIN_FLOOR,
    }


def build_report(
    *,
    arena: int,
    driver: int,
    network_future: int,
    socket_windows: int,
    socket_count: int,
    portable_fetch: int,
    http_decoder: int,
) -> tuple[dict[str, object], list[str]]:
    failures: list[str] = []
    if socket_count <= 0 or socket_windows % socket_count:
        failures.append("application socket windows must divide exactly by socket count")
        return {}, failures
    arena_free = arena - driver - network_future
    if arena_free < 0:
        failures.append("measured Wi-Fi residency exceeds the switchable arena")
        return {}, failures
    one_socket_window = socket_windows // socket_count
    scenarios = [
        scenario("16k_duplex", arena_free, portable_fetch, 16_640, 16_640),
        scenario("8k_rx_2k_tx", arena_free, portable_fetch, 8_192, 2_048),
        scenario("4k_rx_1k_tx", arena_free, portable_fetch, 4_096, 1_024),
        scenario(
            "4k_rx_1k_tx_decoder_overlay",
            arena_free,
            portable_fetch,
            4_096,
            1_024,
            overlaid=http_decoder,
        ),
        scenario(
            "4k_rx_1k_tx_one_socket",
            arena_free,
            portable_fetch,
            4_096,
            1_024,
            reclaimed=one_socket_window,
        ),
        scenario(
            "4k_rx_1k_tx_one_socket_decoder_overlay",
            arena_free,
            portable_fetch,
            4_096,
            1_024,
            reclaimed=one_socket_window,
            overlaid=http_decoder,
        ),
        scenario(
            "4k_rx_1k_tx_pcm_workspace",
            arena_free,
            portable_fetch,
            4_096,
            1_024,
            audio_reclaimed=IMPLEMENTED_TLS_PCM_WORKSPACE_BYTES,
        ),
        scenario(
            "4k_rx_1k_tx_tls_audio_exclusive",
            arena_free,
            portable_fetch,
            4_096,
            1_024,
            audio_reclaimed=TLS_AUDIO_EXCLUSIVE_RECLAIM_BYTES,
        ),
        scenario(
            "8k_rx_2k_tx_tls_audio_exclusive",
            arena_free,
            portable_fetch,
            8_192,
            2_048,
            audio_reclaimed=TLS_AUDIO_EXCLUSIVE_RECLAIM_BYTES,
        ),
    ]
    admitted = [item["name"] for item in scenarios if item["probe_admitted"]]
    return {
        "schema": "koto.app-fetch-tls-feasibility.v1",
        "board": "picocalc-picow",
        "target": "thumbv6m-none-eabi",
        "measured_inputs": {
            "switchable_arena_bytes": arena,
            "cyw43_driver_storage_bytes": driver,
            "network_future_including_ip_stack_bytes": network_future,
            "arena_free_before_fetch_tls_bytes": arena_free,
            "application_socket_windows_bytes": socket_windows,
            "application_socket_count": socket_count,
            "portable_fetch_control_plane_bytes": portable_fetch,
            "http_decoder_overlay_candidate_bytes": http_decoder,
            "permanent_stream_audio_bytes": PERMANENT_STREAM_AUDIO_BYTES,
            "core1_audio_stack_retained_bytes": CORE1_AUDIO_STACK_BYTES,
            "tls_exclusive_audio_reclaim_bytes": TLS_AUDIO_EXCLUSIVE_RECLAIM_BYTES,
            "implemented_tls_pcm_workspace_bytes": IMPLEMENTED_TLS_PCM_WORKSPACE_BYTES,
        },
        "admission_floor": {
            "unmeasured_tls_state_and_margin_bytes": (
                UNMEASURED_TLS_STATE_AND_MARGIN_FLOOR
            ),
            "meaning": (
                "Preflight room for the unmeasured TLS connection/future and "
                "safety margin; admission is not product-fit proof."
            ),
        },
        "scenarios": scenarios,
        "admitted_probe_scenarios": admitted,
        "decision": (
            "RP2040 may continue only with TLS-scoped audio exclusion. Ordinary "
            "Wi-Fi retains stream audio; HTTPS must quiesce and release the "
            "implemented PCM workspace before TLS starts, then erase and "
            "drop TLS state before rebuilding audio. Product HTTPS remains "
            "disabled until exact peak SRAM/flash and endpoint tests pass."
        ),
    }, failures


def read_inputs(
    wifi_path: Path, network_path: Path, fetch_path: Path, network_future: int
) -> tuple[dict[str, object], list[str]]:
    try:
        wifi = json.loads(wifi_path.read_text(encoding="utf-8"))
        network = json.loads(network_path.read_text(encoding="utf-8"))
        fetch = json.loads(fetch_path.read_text(encoding="utf-8"))
        return build_report(
            arena=wifi["arena_bytes"],
            driver=wifi["driver_storage_bytes"],
            network_future=network_future,
            socket_windows=network["rows"]["ip_stack"]["components"][
                "application_socket_windows"
            ],
            socket_count=2,
            portable_fetch=fetch["control_plane"]["bytes"],
            http_decoder=fetch["control_plane"]["components"]["http_decoder"],
        )
    except (OSError, KeyError, TypeError, ValueError, json.JSONDecodeError) as error:
        return {}, [f"could not read measured input reports: {error}"]


def self_test() -> None:
    report, failures = build_report(
        arena=36_864,
        driver=13_296,
        network_future=15_048,
        socket_windows=6_144,
        socket_count=2,
        portable_fetch=2_450,
        http_decoder=1_076,
    )
    assert not failures, failures
    assert report["measured_inputs"]["arena_free_before_fetch_tls_bytes"] == 8_520
    scenarios = {item["name"]: item for item in report["scenarios"]}
    assert scenarios["16k_duplex"]["tls_state_and_margin_headroom_bytes"] == -27_210
    assert scenarios["4k_rx_1k_tx"]["tls_state_and_margin_headroom_bytes"] == 950
    assert not scenarios["4k_rx_1k_tx_one_socket"]["probe_admitted"]
    constrained = scenarios["4k_rx_1k_tx_one_socket_decoder_overlay"]
    assert constrained["tls_state_and_margin_headroom_bytes"] == 5_098
    assert constrained["probe_admitted"]
    tls_exclusive = scenarios["4k_rx_1k_tx_tls_audio_exclusive"]
    assert tls_exclusive["tls_exclusive_audio_reclaimed_bytes"] == 14_392
    assert tls_exclusive["tls_state_and_margin_headroom_bytes"] == 15_342
    assert tls_exclusive["probe_admitted"]
    pcm_workspace = scenarios["4k_rx_1k_tx_pcm_workspace"]
    assert pcm_workspace["tls_exclusive_audio_reclaimed_bytes"] == 8_192
    assert pcm_workspace["tls_state_and_margin_headroom_bytes"] == 9_142
    assert pcm_workspace["probe_admitted"]
    print("App Fetch TLS feasibility model: OK")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--wifi-report", type=Path, default=DEFAULT_WIFI_REPORT)
    parser.add_argument("--network-report", type=Path, default=DEFAULT_NETWORK_REPORT)
    parser.add_argument("--fetch-report", type=Path, default=DEFAULT_FETCH_REPORT)
    parser.add_argument(
        "--network-future-bytes", type=int, default=DEFAULT_NETWORK_FUTURE_BYTES
    )
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return 0

    wifi_report = args.wifi_report.resolve()
    network_report = args.network_report.resolve()
    fetch_report = args.fetch_report.resolve()
    output = args.output.resolve()
    report, failures = read_inputs(
        wifi_report,
        network_report,
        fetch_report,
        args.network_future_bytes,
    )
    for failure in failures:
        print(f"FAIL {failure}", file=sys.stderr)
    if failures:
        return 1
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, indent=2))
    try:
        display_output = output.relative_to(ROOT)
    except ValueError:
        display_output = output
    print(f"wrote {display_output}")
    print("App Fetch TLS feasibility preflight: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
