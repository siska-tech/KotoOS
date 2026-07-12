# KOTO-0069: Pico Probe: PSRAM Round-Trip

- Status: done
- Type: feature
- Priority: P1
- Requirements: HC-3, FR-RT-5, NFR-MEM-4, NFR-MEM-5

## Goal

Prove the RP2040 PSRAM path as explicit block transfer storage, never as
pointer-addressable memory.

## Acceptance Criteria

- [x] The probe writes a known pattern to PSRAM through the HAL block API.
- [x] The probe reads the pattern back into an SRAM buffer and verifies it.
- [x] Out-of-range transfers fail cleanly.
- [x] Bring-up notes record timing, block size, and any fallback decisions.

## Notes

This corresponds to probe 5 in `docs/RP2040_BRINGUP.md`.

Implementation started on 2026-06-20 by adapting the MIT-licensed
`rp2040-psram` PIO protocol to `embassy-rp`. The first probe uses bounded PIO
FIFO transfers at an approximately 16.6 MHz serial clock; DMA remains a later
optimization after physical correctness and timing are recorded.

The first physical run completed 256-byte write/read calls in 2.459 ms and
1.627 ms respectively, and the out-of-range block check passed. Verification
failed at byte zero, so the follow-up firmware logs the first 16 expected and
actual bytes to distinguish power, wiring, and PIO sampling failures.

The diagnostic firmware now writes logs through UART0 TX on GP0 at 115200 8N1,
allowing the PicoCalc mainboard Type-C connection to provide both board power
and the host serial connection. UF2 flashing still uses the Pico BOOTSEL port.

The captured bytes revealed an exact one-bit left rotation per byte
(`5a→b5`, `a3→47`, `ec→d9`). The high-speed read "fudge" clock was therefore
incorrect at the current 16.6 MHz serial clock. The PIO program now uses the
reference implementation's non-fudge, low-speed sampling phase.

The non-fudge phase still showed the same stream-level one-bit advance,
including bits crossing byte boundaries. The follow-up removes the unsampled
clock before the first `in pins` instruction and passes `read_bits - 1` to the
PIO loop counter so the first PSRAM output bit is captured immediately.

Physical validation completed on 2026-06-20. A 256-byte block round-tripped
with exact byte equality at block 257. The measured write time was 2.439 ms and
the read time was 1.618 ms. Block index 32768 was rejected as out of range.
