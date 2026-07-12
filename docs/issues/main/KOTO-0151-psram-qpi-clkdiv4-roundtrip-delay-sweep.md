# KOTO-0151: PSRAM QPI clkdiv=4 roundtrip delay sweep

- Status: done
- Type: research / hardware validation
- Priority: P1
- Related: KOTO-0132, KOTO-0150

## Goal

Determine whether the `clkdiv=4` QPI roundtrip instability is caused by
insufficient write-to-read turnaround time between a QPI write transaction and
the following QPI read transaction.

## Context

`qpi_cpu120_sweep` showed that the Picoware-aligned QPI PIO path can often pass
read-only validation at `clkdiv=4`, `input_sync_bypass=0`, and `chunk=120`.
However, 64 KiB roundtrip validation failed frequently, commonly near
`fail_off=4` or `fail_off=5`.

That failure shape suggested a possible write-to-read turnaround, recovery, or
CS-high timing issue rather than a pure read data phase issue.

## Experiment

`psram_diag` was extended with `qpi_cpu120_stress` delay sweep logging:

- mode: `qpi_cpu120_stress`
- PIO profile: Picoware-aligned QPI read/write program
- clock: `clkdiv=4`, `sm_hz=33.25MHz`
- chunk: 120 bytes
- input synchronizer bypass: `0`
- RX DMA: disabled
- TX DMA: disabled
- benchmark kind: 64 KiB `roundtrip`, `verify=on`
- comparison benchmark: 64 KiB `read`, `verify=on`

The roundtrip path swept this delay after every QPI write and before the
matching QPI read:

- `write_read_delay_us=0`
- `write_read_delay_us=1`
- `write_read_delay_us=2`
- `write_read_delay_us=5`
- `write_read_delay_us=10`
- `write_read_delay_us=20`
- `write_read_delay_us=50`

Stress addresses:

- `0x00010100`
- `0x00020000`
- `0x00100000`
- `0x00300000`
- `0x00700000`

The stress mode logs one row per iteration, address, and delay:

```text
phase=336 psram-qpi-stress mode=qpi_cpu120_stress iteration=<n> addr=0x<addr> bench_kind=roundtrip verify=on write_read_delay_us=<us> bytes=65536 done_bytes=<n> elapsed_us=<us> mb_s=<x.xxx> chunk=120 txns_done=<n> txn_bytes=120 clkdiv=4.000 sm_hz=33250000 input_sync_bypass=0 ok=<0|1> fail=<reason> fail_off=<n> fail_exp=0x<xx> fail_got=0x<xx> first16=<hex>
```

## Result

The delay sweep did not materially change the failure behavior.

Increasing `write_read_delay_us` up to 50 us did not eliminate the
`clkdiv=4` roundtrip verify failures. Therefore, the observed instability is not
explained by a simple fixed wait requirement between write completion and the
following read command.

Read-only behavior remains a useful comparison signal: read-only can pass in
cases where roundtrip fails, but the failure is not resolved by adding a
post-write delay.

## Interpretation

The current evidence weakens the hypothesis that the dominant issue is PSRAM
write recovery time, CS-high wait, or a simple write-to-read turnaround delay.

More likely remaining causes:

- QPI write phase timing is marginal at `clkdiv=4`.
- The write command/data phase has an edge or side-set alignment issue that only
  appears after actual writes.
- The PIO program's transition between write and read paths leaves state that is
  not recovered by idle wall-clock delay alone.
- `clkdiv=4` is above the reliable timing envelope for roundtrip traffic even if
  read-only traffic sometimes passes.
- The first-byte/nibble failure pattern points to phase alignment or bus turn
  direction timing, not bulk FIFO drain.

## Decision

Do not treat `clkdiv=4` as a stable production candidate for QPI roundtrip or
future CodeWindow work yet.

Use `clkdiv=6` or `clkdiv=8` as the next practical validation target unless a
specific write-phase PIO timing fix is identified.

The next investigation should focus on one of:

- comparing QPI write phase timing against the Picoware reference instruction by
  instruction;
- adding targeted write-only validation that verifies through a slower stable
  read mode;
- testing whether a different write data launch edge or sideset pattern fixes
  the early-byte mismatch;
- moving bandwidth work to larger read-only transactions once a stable write
  baseline is available.

## Acceptance Criteria

- [x] `qpi_cpu120_stress` includes write-to-read delay sweep rows.
- [x] Sweep covers `0, 1, 2, 5, 10, 20, 50 us`.
- [x] Sweep covers the selected low, middle, and high PSRAM addresses.
- [x] Logs include delay, iteration, address, clock, bypass state, fail offset,
      expected byte, actual byte, and `first16`.
- [x] Hardware result is recorded: delay sweep did not resolve `clkdiv=4`
      roundtrip instability.
- [x] Follow-up decision is recorded: do not advance `clkdiv=4` as the stable
      QPI roundtrip profile without a write-phase fix.

## Notes

This issue records an experimental negative result. It should remain linked from
future QPI work so the project does not repeat the same turnaround-delay sweep
unless the PIO program or PSRAM initialization sequence changes materially.

## Follow-up: Picoware QPI Write Path Comparison

Reference files checked:

- `docs/Reference/PicoCalc/picoware_psram/psram_qspi.pio`
- `docs/Reference/PicoCalc/picoware_psram/psram_qspi.c`
- `docs/Reference/PicoCalc/picoware_psram/psram_qspi.h`
- `docs/Reference/PicoCalc/picoware_psram/picoware_psram_shared.h`

### Picoware write path summary

Picoware's normal QPI bulk write uses `qspi_psram_rw`:

- TX protocol is `[write_nibbles, read_nibbles, 0x38, addr_hi, addr_mid, addr_lo, data...]`.
- `write_nibbles = (4 + count) * 2`, where `4` is command byte plus 24-bit address.
- Address order is MSB first: bits 23..16, 15..8, 7..0.
- Data bytes are streamed in caller order after the 6-byte command header.
- Output shift is MSB first, autopull enabled, threshold 8 bits.
- Command/address/data are fed as 8-bit FIFO transfers (`DMA_SIZE_8` or `io_rw_8` fallback).
- The `qspi_psram_rw` PIO side-set owns CS and SCK; CS is lowered after the two count bytes and raised at the end of a transaction.
- Count fields are u8, so write count limits are `count <= 123`; Picoware's shared chunk size is 64.
- The separate `qspi_4wire_write` PIO program is write-only bulk output: `out pins, 4 side 0; nop side 1`. It clocks nibbles on SIO0..3 and uses only SCK side-set; CS must be controlled outside that program.

### Correspondence table

| Item | Picoware | KotoOS | Delta | Impact |
| --- | --- | --- | --- | --- |
| write command | `0x38` quad write | `0x38` | None | Command opcode is not the likely cause. |
| command/address/data order | count bytes, `0x38`, addr hi/mid/lo, data | Same in `write_chunk_cpu` | None | Header order matches reference. |
| address order | 24-bit MSB first | 24-bit MSB first | None | Address byte order is not the likely cause. |
| data nibble order | MSB nibble first via shift-left / MSB-first PIO | Same shift direction; RW path left-aligns bytes into u32 FIFO pushes | FIFO feed differs | If corruption has nibble-shift shape, TX feed packing remains a suspect. |
| write count | `(4 + count) * 2` nibbles, u8 | Same for `qspi_psram_rw` | None for chunks <=123 | Count error is less likely for <=120B, still tested at 1/2/4/8/16/64/120/256. |
| chunk size | `PSRAM_CHUNK_SIZE=64`; u8 count allows <=123 | QPI chunks 64, large diagnostic chunk 120 | KotoOS tests closer to limit | 120B stresses count/FIFO timing more than Picoware's default 64B. |
| TX autopull | enabled, threshold 8 | enabled, threshold 8 | None | Autopull config matches. |
| shift direction | MSB first (`shift_right=false`) | MSB first (`ShiftDirection::Left`) | None | Bit order config matches. |
| TX FIFO unit | 8-bit DMA/MMIO writes | RW path uses u32 pushes with byte in bits 31..24; new `qspi_4wire_write` diagnostic uses 8-bit MMIO feed | RW path differs | Direct A/B can now test whether FIFO feed width/packing matters. |
| CS control | `qspi_psram_rw` side-set controls CS; `qspi_4wire_write` needs external CS | RW side-set controls CS; new 4-wire wrapper manually lowers/raises CS with PIO pin writes | 4-wire wrapper added | Separates shared-RW CS/count handling from raw 4-bit write clocking. |
| SM/FIFO init | PIO init enables SM; 4-wire init clears FIFO | KotoOS clears FIFO/restarts at init and when switching programs | KotoOS more explicit | Should reduce stale FIFO/PC state, not explain deterministic early corruption. |
| read phase | Reference has `readloop_entry` before first input nibble | KotoOS removed it and passes read nibbles minus one | Intentional read-only fix | Should not affect write-only phase, but diagnostics use clkdiv=8 safe read to isolate writes. |
| DMA helper size | write/read DMA configured as 8-bit transfers | QPI write diagnostics are CPU-fed; 4-wire feed uses 8-bit MMIO | No write DMA optimization | Correctness check only; DMA optimization remains out of scope. |

### Added diagnostic

`psram_diag` now emits `phase=337 psram-qpi-write-diag` rows. The diagnostic:

- keeps readback on safe QPI read at `read_clkdiv=8`;
- tests write modes `qspi_psram_rw` and `qspi_4wire_write`;
- tests `qspi_psram_rw` write clocks `8, 6, 4`;
- tests `qspi_4wire_write` write clocks `8, 6, 4, 2`, with `2` last;
- tests patterns `00`, `ff`, `aa`, `55`, `0f`, `f0`, `inc00_3f`, and `pseudo_random`;
- tests write sizes `1, 2, 4, 8, 16, 64, 120, 256`;
- pre-reads a 64-byte guard window before small writes, then compares the full guard window after the write.

This should distinguish the leading candidates:

- `qspi_psram_rw` fails but `qspi_4wire_write` passes: suspect shared RW count/CS/write-loop transaction mechanics.
- Both fail at the same offset/nibble XOR: suspect bus timing, data nibble order, or PSRAM QPI write timing.
- Only 120/256 fail: suspect count/chunk boundary or FIFO feeding under longer bursts.
- 1/2/4/8 fail with guard bytes changed: suspect byte offset, overrun, or command/address/data phase alignment.

## KOTO-0151 result

Picoware reference based QPI port is mostly validated.

- QPI read-only works.
- RX DMA did not materially improve bandwidth in the current 64/120B transaction design.
- Picoware faithful `qspi_psram_rw` read phase improved high-speed behavior.
- `write_read_delay_us = 0,1,2,5,10,20,50` did not fix roundtrip instability.
- New phase=337 write diagnostic passed all cases:
  - `qspi_psram_rw`
  - Picoware-style `qspi_4wire_write`
  - patterns: 00/ff/aa/55/0f/f0/inc/pseudo
  - sizes: 1..256
  - `write_clkdiv=8,6,4,2`
  - safe read fixed at `read_clkdiv=8`

Interpretation:

- QPI write path is likely correct.
- Previous roundtrip failures are not caused by write corruption.
- The likely issue is `read_clkdiv=4` instability or roundtrip read-side state dependency.
- Do not use `read_clkdiv=4` for CodeWindow/bytecode fetch.

Recommended profiles:

```text
fast write:
  qspi_4wire_write
  write_clkdiv=2

safe read:
  qspi_psram_rw
  read_clkdiv=8

fast read candidate:
  qspi_psram_rw
  read_clkdiv=6
  needs stress verification

do not adopt:
  read_clkdiv=4
````

Follow-up status (2026-06-25):

1. [x] Added `qpi_cpu120_stress_read6` profile in `psram_diag` (`read_clkdiv=6`, `write_clkdiv=6`).
2. [x] Added `qpi_cpu120_stress_w2_r8` profile in `psram_diag` (`write_mode=qspi_4wire_write`, `write_clkdiv=2`, `read_clkdiv=8`).
3. [x] Connected safe QPI CodeWindow backend behind feature flag `psram_qpi_safe_read_code_window` in `koto_firmware`.
4. [x] Feature-gated CodeWindow HAL now uses `qspi_4wire_write` at `clkdiv=2` for writes.

Remaining validation:

1. [ ] Run hardware stress for `qpi_cpu120_stress_read6` and archive logs.
2. [ ] Run hardware stress for `qpi_cpu120_stress_w2_r8` and archive logs.
3. [ ] Measure CodeWindow refill timing with `--features psram_qpi_safe_read_code_window` and compare with `pio3v1` baseline.