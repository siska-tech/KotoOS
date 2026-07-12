# Koto Audio PicoCalc Backend Experiment

## Purpose

This note defines the experiment boundary for a future PicoCalc/RP2040 audio
backend. The backend must remain an internal implementation of the existing
`AudioBackend` trait so `AudioService` continues to depend only on logical audio
operations: `start`, `stop`, `suspend`, `resume`, `reset`, and
`submit_block`.

The current implementation is a stub/skeleton. Its job is to establish feature
gating, policy recording, state transitions, counters, and report hooks before
any production hardware output is attempted.

## Non-Goals

- Production PicoCalc audio output.
- Final hardware register or driver implementation.
- Stable hostcall numeric ABI.
- SLDPCM4, PCM8, ADPCM4, sequence, BGM, stream, or WAV conversion support.
- Public exposure of backend-owned buffers, handles, registers, or raw hardware
  controls.

## Backend Candidates

Real hardware validation should compare these implementation strategies inside
the PicoCalc backend module only:

- PWM output.
- PIO-fed output.
- DMA-assisted output.
- Timer-paced output.
- I2S-like output if the target board wiring supports it.

Normal applications must not receive direct access to these mechanisms. They
interact with `AudioService` and the `AudioBackend` trait boundary only.

## Sample Rate Candidates

- 16 kHz.
- 22.05 kHz.

These are experiment candidates, not compatibility promises.

## Block Size Candidates

- 128 frames.
- 256 frames.

The backend policy records the selected candidate so hardware runs can compare
latency, CPU cost, underrun behavior, and queue pressure.

## Buffer and Prefill Candidates

The stub records backend depth in blocks and silent prefill count. Initial
hardware runs should compare shallow and moderate queue depths, such as two,
three, and four blocks. Silent prefill should be measured separately because it
can hide startup underruns while adding startup latency.

## Measurement Items

- Submitted block count.
- Underrun count and underrun report behavior.
- Submit failure count and failure report behavior.
- Maximum submit latency placeholder, in implementation-defined ticks until a
  stable measurement source is chosen.
- Abstract backend state.
- Start, restart, and resume count.
- Silent prefill block count.

Measurements should flow through `BackendReport` where possible so service-level
counters and events continue to work without PicoCalc-specific call sites.

## Safety Rule

No raw hardware exposure to normal app code. Hardware details, handles, register
access, pacing engines, and backend-owned storage stay inside
`crates/koto-audio/src/backend/picocalc.rs` or lower private modules. The crate
root may expose only abstract PicoCalc experiment configuration, counters,
snapshots, and an `AudioBackend` implementation.

## Next Steps

1. Compile the feature-gated stub on host and target builds.
2. Bring up one hardware candidate behind the same `AudioBackend` methods.
3. Record startup behavior with each sample rate, block size, queue depth, and
   silent prefill setting.
4. Measure underruns, submit failures, and submit latency under idle and loaded
   CPU conditions.
5. Compare candidates and choose the smallest private hardware abstraction that
   keeps `AudioService` unchanged.
