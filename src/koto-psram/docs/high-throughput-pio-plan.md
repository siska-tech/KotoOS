# High-Throughput PIO Realignment Plan

This plan realigns the RP2040 Embassy backend with the original PIO design and
performance target in `docs/koto-psram-design.md`.

The design target remains:

- Theoretical QPI bandwidth at `clkdiv = 2`: about 33.25 MB/s.
- Large sequential transfers: roughly 20-25 MB/s effective bandwidth.

Recent hardware measurements are far below that target:

- Byte path: about 70 KB/s.
- `polling_burst` path: about 93 KB/s write and about 118 KB/s read.

Those numbers mean the current concrete backend is a correctness-oriented MVP
and diagnostic implementation. It should not be polished as though the current
byte or `polling_burst` payload loop can grow into the 20 MB/s-class design.

## Current Backend Inspection

The current concrete backend in `src/rp2040_embassy/embassy_hal.rs` loads two
small command clocking programs:

- `spi_command_program`: full-duplex `out pins, 1` plus `in pins, 1`.
- `qpi_command_program`: full-duplex `out pins, 4` plus `in pins, 4`.

Both programs are command-style byte/nibble clocks. They are useful for SPI
recovery, QPI enter/exit, command/address/dummy phases, device ID reads, and
known-good bring-up. They are not specialized streaming payload programs.

The current QPI read/write chunks both use this same QPI command program:

- Writes send command, address, optional dummy bytes, then send each payload
  byte through the full-duplex transfer path.
- Reads send command, address, dummy bytes, switch SIO pins to input, then send
  dummy TX bytes to generate clocks and pull one RX byte per payload byte.

The `PayloadTransferPath` selector correctly labels the current modes:

- `Byte`: known-good path, one TX FIFO push and one RX FIFO pull per byte.
- `PollingBurst`: diagnostic prototype, opportunistically fills/drains FIFOs
  during payload only.

`PollingBurst` reduces some per-byte call structure, but it still operates on
one byte per TX word and one RX word per byte using the same shared
out/in command program. It does not convert the data phase into a word-streaming
PIO path.

## MVP / Diagnostic Parts

The following parts are intentionally kept but must be treated as MVP or
diagnostic infrastructure:

- Byte transfer path:
  - Known-good fallback.
  - One byte becomes one `u32` TX FIFO word.
  - Each payload byte waits for a matching RX FIFO word.
  - Appropriate for correctness, bring-up, and small sanity tests.

- `polling_burst` path:
  - Diagnostic-only benchmark path.
  - Fills TX FIFO and drains RX FIFO opportunistically.
  - Still transfers one payload byte per FIFO word.
  - Still uses the shared full-duplex command program.
  - Useful evidence that CPU/FIFO polling is the current bottleneck.

- Shared out/in QPI command program:
  - Good for command/address/dummy and small full-duplex byte-style transfers.
  - Bad fit for high-throughput payload writes because it samples RX during
    write payloads and forces discard work.
  - Bad fit for high-throughput payload reads because it requires dummy TX
    words per byte instead of continuous receive-side streaming.

- CPU FIFO polling bottleneck:
  - Current loops pay per-byte software overhead and FIFO status overhead.
  - FIFO words carry only one useful byte.
  - Read and write payload phases both couple TX and RX progress even when the
    bus direction is actually one-way.

## Target Architecture

Keep the public `PsramBus::read_exact` and `PsramBus::write_all` APIs unchanged.
The driver can continue to split public byte slices into bounded
`QpiTransaction` chunks. The backend chooses the concrete payload engine for a
chunk.

Payload engines should become explicit:

- `ByteFallback`
  - Current byte path.
  - Default production-safe mode until streaming is proven on hardware.

- `PollingBurstDiagnostic`
  - Current `polling_burst` path.
  - Hidden diagnostic mode for regression comparison.

- `WordStreamPolling`
  - New CPU-polling high-throughput path.
  - Command/address/dummy remains handled by the existing command program.
  - Payload switches to a dedicated write or read stream program.
  - CPU pushes or pulls packed `u32` words.
  - Tail bytes are packed/unpacked safely outside the PIO hot loop.
  - Experimental and opt-in until acceptance stages are met.

- `WordStreamDma`
  - Later DMA-backed high-throughput path.
  - Uses the same stream PIO programs as `WordStreamPolling`.
  - Write payloads use TX DMA into the PIO TX FIFO.
  - Read payloads use RX DMA out of the PIO RX FIFO.
  - Optional chained setup can later combine command setup and payload DMA.

The first implementation milestone should add `WordStreamPolling`, not DMA.
That keeps failure modes inspectable while proving the PIO stream programs,
direction timing, word packing, tail handling, and benchmark shape.

## Required PIO Programs

### Existing Command / Address / Dummy Program

Purpose:

- SPI recovery and device ID command clocks.
- QPI enter/exit command clocks.
- QPI command byte.
- 24-bit QPI address.
- Read dummy cycles and turnaround setup.

This program can remain a small full-duplex command clocker. It is not the
payload performance path.

Use it for:

- `EXIT_QPI` as quad.
- `EXIT_QPI` as SPI.
- `READ_ID` in SPI mode.
- `ENTER_QPI` in SPI mode.
- QPI read/write command and address.
- Read dummy/turnaround clocks until the SIO pins are safely input.

### `qpi_write_stream`

Purpose:

- Output-only QPI write payload phase.

Required behavior:

- Configure SIO0-3 as output before enabling the stream.
- Keep CS asserted across the command/address phase and payload phase.
- Use `autopull`.
- Consume 32-bit words from TX FIFO.
- Emit `out pins, 4` once per PIO data cycle.
- Transfer one 32-bit FIFO word as eight QPI nibbles, i.e. four payload bytes.
- Avoid RX sampling during write payload.
- Avoid RX FIFO traffic and discard work during write payload.
- Stall naturally on TX FIFO empty rather than emitting extra payload clocks.

CPU contract for `WordStreamPolling`:

- Pack payload bytes into big-endian or otherwise explicitly documented nibble
  order matching the existing QPI byte order tests.
- Push full `u32` words while TX FIFO has room.
- For tail lengths 1-3, push one padded final word and configure the stream
  count so only the real tail nibbles are clocked.
- Do not rely on PSRAM ignoring extra bytes.

Open design point:

- The stream program needs a bounded payload count. Prefer a setup word that
  encodes nibble count or word count plus tail nibble count before the stream
  loop. The final implementation must make the PIO stop exactly after the
  requested byte length.

### `qpi_read_stream`

Purpose:

- Input-only QPI read payload phase.

Required behavior:

- Complete command/address/dummy and bus turnaround before enabling payload
  input.
- Configure SIO0-3 as input before payload sampling.
- Use `autopush`.
- Execute `in pins, 4` once per PIO data cycle.
- Push one 32-bit RX FIFO word per eight sampled nibbles, i.e. four payload
  bytes.
- Generate continuous read clocks for the requested payload length.
- Avoid requiring one dummy TX FIFO word per payload byte.
- Stop after exactly the requested byte length.

CPU contract for `WordStreamPolling`:

- Pull full `u32` words while RX FIFO has data.
- Unpack words into bytes according to the documented QPI nibble order.
- For tail lengths 1-3, read one final pushed word only if the PIO program
  intentionally pushes the partial final data, or configure the program to
  clock/push a final partial unit through an explicit tail path.
- The implementation must not overrun the caller's output slice.

Dummy/turnaround handling:

- The existing command program may handle read dummy clocks before switching to
  `qpi_read_stream`.
- Alternatively, `qpi_read_stream` may include a tiny dummy/turnaround prelude.
- In either case, SIO direction must change to input before the PSRAM begins
  driving data, and sampling must occur on the intended edge.

## State Machine and Program Loading Strategy

Use the existing single state machine initially:

1. Assert CS.
2. Run command/address/dummy with the command program.
3. Disable the state machine.
4. Reconfigure the same state machine to `qpi_write_stream` or
   `qpi_read_stream`.
5. Preserve CS asserted and SCK idle state across reconfiguration.
6. Run the stream payload.
7. Disable the state machine.
8. Deassert CS, idle SCK, and return SIO pins to input.

This may not be the final lowest-overhead structure, but it isolates the
payload performance problem without spending another state machine. If the
state-machine reconfiguration cost dominates small transfers, route only large
chunks through the stream path and keep byte fallback for small transfers.

Program inventory:

- Keep `spi_command_program`.
- Keep existing `qpi_command_program`.
- Add `qpi_write_stream_program`.
- Add `qpi_read_stream_program`.

The stream programs must be written independently from public datasheet-level
PIO behavior and local design requirements. Do not copy GPL code or
translate GPL PIO assembly.

## WordStreamPolling Stage

Add this before DMA.

Backend-facing behavior:

- Leave `read_exact` and `write_all` untouched.
- Keep chunk validation through `QpiTransaction`.
- Add an opt-in concrete backend payload path for word streaming.
- Do not make streaming default until hardware correctness and benchmarks pass.

Write flow:

1. Existing command program sends QPI write command and 24-bit address.
2. Existing command program emits write dummy cycles if configured.
3. Backend switches to `qpi_write_stream`.
4. CPU polling loop packs and pushes `u32` payload words.
5. PIO clocks four bits per PIO cycle and consumes packed words with
   `autopull`.
6. Backend waits for the stream program to complete exactly the requested
   payload length.

Read flow:

1. Existing command program sends QPI fast-read command and 24-bit address.
2. Existing command program performs dummy/turnaround handling.
3. Backend switches SIO pins to input.
4. Backend switches to `qpi_read_stream`.
5. PIO clocks four bits per PIO cycle and fills RX FIFO with `autopush`.
6. CPU polling loop pulls `u32` payload words and safely unpacks tail bytes.
7. Backend waits for the stream program to complete exactly the requested
   payload length.

Polling-loop expectation:

- The hot loop should move `u32` words, not bytes.
- The write loop should not touch RX FIFO during payload.
- The read loop should not feed dummy TX bytes during payload.
- Tail handling should be outside the steady-state hot loop where possible.

## WordStreamDma Stage

Add this after `WordStreamPolling` has proven correctness.

Phase 4-6E CPU-polling freeze:

- `WordStreamPolling` write is the fastest measured CPU-polling write path,
  around 3.0 MB/s on the current PicoCalc payload benchmark.
- `PollingBurstDiagnostic` read is still the fastest measured CPU-polling read
  path, around 2.8 MB/s.
- `WordStreamPolling` read works but is slower, around 2.4 MB/s, and the best
  diagnostic batch sizes are `4` or `8` words.
- Larger read batches reduce unpack time but increase RX pull overhead, so the
  current word-stream read path is RX-pull limited rather than unpack limited.
- `rx_fifo_join = true` is unsupported with the current `qpi_read_stream`
  setup because `FifoJoin::RxOnly` disables TX FIFO access while the stream
  program still receives its payload count through TX.

Next read-DMA experiment:

- First target read DMA only; do not add write DMA in the first experiment.
- Use an aligned 256 KB transfer as the initial case.
- Keep public `read_exact` and `write_all` APIs unchanged.
- Do not change the default payload path.
- Use the existing `qpi_read_stream` setup where possible.
- Handle tail bytes and unaligned caller buffers in a later step.

Write DMA:

- Pack or expose aligned payload words for DMA.
- Configure TX DMA paced by the PIO TX DREQ.
- Feed `qpi_write_stream` TX FIFO.
- Use a small CPU-prepared tail word when the byte length is not divisible by
  four.

Read DMA:

- Configure RX DMA paced by the PIO RX DREQ.
- Drain `qpi_read_stream` RX FIFO into a word buffer or caller-compatible
  destination strategy.
- Handle unaligned/tail bytes with a staging word if needed.

Later optional chaining:

- Chain command/setup DMA into payload DMA only after the polling stream path
  and simple DMA path are stable.
- Keep command/address and payload responsibilities observable in diagnostics.

## Acceptance Targets

Stage 1: correctness with `write_stream` and `read_stream`

- Streaming path is opt-in.
- Byte fallback remains default and still passes existing compare tests.
- `polling_burst` remains available for diagnostics.
- Stream write/read pass hardware compare at lengths:
  `1`, `2`, `3`, `4`, `5`, `31`, `32`, `255`, `256`, `257`, `512`, and at least
  one larger sequential transfer.
- Tail-byte tests explicitly cover all alignments modulo four.

Stage 2: more than 1 MB/s

- `WordStreamPolling` exceeds 1 MB/s for large sequential writes and reads on
  PicoCalc-class hardware.
- Bench output clearly labels payload engine, chunk length, clkdiv, dummy
  cycles, total bytes, and payload timing.

Stage 3: more than 5 MB/s

- `WordStreamPolling` or a minimally optimized stream path exceeds 5 MB/s for
  large sequential writes and reads.
- If polling cannot pass this target, record whether the bottleneck is TX FIFO
  refill, RX FIFO drain, program reconfiguration, or read sampling margin.

Stage 4: 20 MB/s-class with DMA/streaming

- `WordStreamDma` approaches 20 MB/s-class effective bandwidth for large
  sequential transfers.
- The benchmark should be compared against the design expectation of 20-25 MB/s
  and the theoretical `clkdiv = 2` ceiling of about 33.25 MB/s.

## Non-Goals

- Do not remove the known-good byte path.
- Do not remove the `polling_burst` diagnostic path.
- Do not change public `read_exact` or `write_all` APIs.
- Do not make experimental streaming default.
- Do not integrate VM, graphics, assets, or KotoOS in this phase.
- Do not copy or translate GPL code from external firmware.

## Next Implementation Steps

1. Add internal payload engine names that distinguish fallback, diagnostic,
   polling word stream, and future DMA word stream.
2. Load `qpi_write_stream_program` and `qpi_read_stream_program` alongside the
   existing programs.
3. Implement stream setup and exact-length completion rules.
4. Implement `WordStreamPolling` write packing and read unpacking helpers with
   host unit tests for byte order and tails.
5. Add opt-in hardware example coverage for stream correctness.
6. Add benchmark output that reports the selected payload engine and acceptance
   stage metrics.
