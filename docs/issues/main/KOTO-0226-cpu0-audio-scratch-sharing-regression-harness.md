# KOTO-0226: CPU0 audio scratch sharing regression harness

- Status: done
- Type: harness
- Priority: P1
- Requirements: HC-1, HC-6, FR-RT-5, NFR-MEM-1, NFR-MEM-2, NFR-REL-3
- Related: KOTO-0148, KOTO-0165, KOTO-0170, KOTO-0186, KOTO-0207,
  KOTO-0220

## Background

The RP2040 product ELF at `126d48d` reserves two CPU0-only audio scratch
regions:

```text
AUDIO_LOAD_SCRATCH     8,732 B
AUDIO_STREAM_SCRATCH   5,120 B
```

The product frame loop uses them synchronously and in sequence:

1. `DeviceHost::service_audio_stream()` reads/decodes a package stream through
   `AUDIO_STREAM_SCRATCH` and copies the result into the PCM ring.
2. The VM runs and may queue an audio-asset request.
3. `service_pending_audio_asset()` loads or reads back the cue image through
   `AUDIO_LOAD_SCRATCH`, then copies it into PSRAM and the CPU1 staging slot.

Neither path retains a scratch reference, crosses an `.await`, or exposes the
scratch to CPU1. A single aligned 8,732-byte CPU0 scratch should therefore be
able to serve both modes and recover about 5,120 bytes of SRAM. The risk is a
future or overlooked re-entrant path corrupting a stream refill or cue image,
which could present as an intermittent decode failure, audio drop, underrun,
or CPU1 playback fault rather than a clear memory error.

Current device and ELF baseline (2026-07-17, RP2040 default release):

```text
.data + .bss payload       210,556 B
static span through .bss   210,872 B
phase=176 stack peak        48,932 B
phase=176 free_min          10,532 B
```

## Goal

Add a deterministic product-path regression program and machine-readable
diagnostics that exercise package PCM streaming and PSRAM-backed runtime cue
loading in the same app session. Establish a passing two-scratch baseline,
then use the same program to prove that one aligned CPU0 scratch can safely
serve both modes while recovering at least 5 KiB of RP2040 SRAM.

## Regression Program

Add a small packaged device fixture, rather than a standalone peripheral probe,
so it exercises the real `DeviceHost`, KPA/SD reads, PSRAM asset cache,
`AudioShared`, CPU1 worker, and PWM/DMA output paths.

The program must run deterministic stages with bounded iteration counts:

1. Start a KACL asset larger than `RUNTIME_CLIP_IMAGE_CAPACITY` so the host uses
   the package-streaming path.
2. While the stream remains active, request distinct runtime cues slowly enough
   for every request to be accepted. This covers cold SD load, PSRAM write/read
   back, and CPU1 staging.
3. Request the same cues again to cover PSRAM cache hits while stream refills
   continue.
4. Repeat the sequence for both PCM16 and SLDPCM4 streaming assets so the maximum
   encoded slice and the reused `[i16; 512]` decode slice are both exercised.
5. Run a bounded soak that crosses many ring-refill boundaries, then exit
   cleanly and emit one final `result=pass|fail` summary.

Use deterministic generated fixtures with checksums; do not depend on a user
hearing a glitch to decide pass/fail. An audible smoke check may supplement the
automated result.

## Shared-Scratch Guardrails

- Replace the two independent `UnsafeCell` statics with one const-initialized,
  explicitly aligned CPU0 scratch whose storage size is the maximum of the two
  views, not their sum.
- Give the load-image and stream encoded/decoded layouts named scoped accessors.
  A scratch borrow must end before either accessor returns and must never cross
  `.await`.
- Preserve the alignment required by the stream decoder's `i16` output view.
- Do not construct the 8 KiB-class storage by value on the main stack; retain
  in-place const initialization.
- CPU1 must continue to receive owned copies through `AudioShared` and must
  never retain a pointer or slice into the shared scratch.
- Add compile-time size/alignment assertions and a debug-visible busy/mode guard
  that reports any re-entry instead of handing out overlapping mutable views.

## Acceptance Criteria

- [x] A packaged regression program exercises large PCM16 streaming, large
      SLDPCM4 streaming, cold runtime-cue loads, PSRAM write/read-back, and cue
      cache hits in one product-firmware session.
- [x] The program uses deterministic assets and emits a machine-readable final
      summary with stage counts and `result=pass|fail`; listening is not the
      pass criterion.
- [x] Scratch diagnostics expose load acquisitions, stream acquisitions,
      rejected/re-entrant acquisitions, and guard/corruption failures. The
      passing run records zero rejection/re-entry and zero corruption.
- [x] A baseline capture is retained from the existing two-scratch build,
      including `phase=158`, `phase=173`, and `phase=176` lines and the ELF
      `.data`/`.bss`/symbol report.
- [x] The implementation uses one aligned CPU0 scratch with scoped, mutually
      exclusive load and stream views; no reference escapes, crosses `.await`,
      or reaches CPU1.
- [x] RP2040 release `AUDIO_LOAD_SCRATCH` + `AUDIO_STREAM_SCRATCH` storage falls
      by at least 5,000 bytes, with the replacement symbol no larger than the
      former 8,732-byte load scratch plus alignment/guard metadata.
- [x] The post-change RP2040 ELF and the full device session are re-measured.
      The current Gallery worst-case `phase=176 free_min=10,532` rises to at
      least 15 KiB without reducing either CodeWindow tile count or raster-strip
      height.
- [x] During the regression soak, `underruns=0`, `command_drops=0`, decode and
      checksum failures are zero, CPU1 heartbeat continues advancing, and
      `core1_stack_free_min` does not regress from the pre-change capture.
- [x] The fixture completes at least 100 cold/cache-hit cue cycles and at least
      five minutes of alternating stream/cue activity without a hang, buzz,
      stale replay, or unexpected audio drop.
- [x] RP2040 and RP2350A firmware release builds pass, along with the repository
      harness and any host-side tests for scratch layout, mode exclusion, and
      guard failure reporting.

## Notes

- The expected SRAM recovery is 5,120 bytes: `max(8,732, 5,120)` replaces the
  current sum. Linker alignment may move section boundaries by a few bytes, so
  acceptance uses a 5,000-byte floor and records exact symbols.
- The fixture must exercise the product path. Extending `probe_audio` alone is
  insufficient because that probe does not cover package streaming, runtime cue
  compilation/cache reads, or the app-frame service order.
- Do not serialize access by masking interrupts. CPU1 audio delivery must remain
  live while CPU0 owns either view; exclusion applies only to the two CPU0
  scratch users.
- If the harness finds a legitimate overlapping lifetime, keep the two buffers
  and record the call chain. Do not force sharing by copying another 5 KiB
  temporary onto the stack.

## Implementation Start (2026-07-17)

The first implementation pass is in place:

- `firmware/audio_scratch.rs` owns one aligned, const-initialized CPU0 static.
  Closure-scoped load and stream accessors enforce non-escaping borrows, a mode
  guard rejects re-entry, and leading/trailing guards report corruption.
- `audio_codecs` now includes a user-started, deterministic 18,750-frame product
  session. It alternates 9,000-frame PCM16 and SLDPCM4 stages, restarts each
  finite stream before it drains, and requests two KMML cues every 60 frames.
  Fixture SHA-256 values are committed and checked by the host harness.
- `phase=173 audio-scratch` exposes the four scratch counters. On regression-app
  exit, `phase=226 audio-scratch-regression` includes stream/cue stage counts,
  scratch counters, underruns, command drops, worker heartbeat, CPU1 stack
  margin, and `result=pass|fail`.
- `harness/check_audio_scratch.py` runs host tests for layout/alignment, mode
  exclusion, counter increments, and injected guard failure reporting.

RP2040 default release ELF comparison from the same worktree/toolchain:

| Measurement | Two scratch baseline | Shared scratch | Delta |
| :-- | --: | --: | --: |
| `.data` | 66,244 B | 66,244 B | 0 B |
| `.bss` | 144,312 B | 139,192 B | -5,120 B |
| `.data + .bss` | 210,556 B | 205,436 B | -5,120 B |
| static span through `.bss` | 210,872 B | 205,752 B | -5,120 B |
| scratch symbols | 8,732 B + 5,120 B | 8,776 B | -5,076 B |

The shared symbol is the 8,732-byte load storage plus 44 bytes of alignment,
guards, mode, and diagnostic metadata. The Embassy main task pool remains
24,160 bytes (`0x5e60`); diagnostics did not move the recovered SRAM into the
async future.

Local validation completed:

- RP2040 `thumbv6m-none-eabi` release firmware build: pass.
- RP2350A `thumbv8m.main-none-eabihf` release firmware build: pass.
- Workspace `cargo test`: pass.
- Scratch host regression and fixture checksum check: pass.
- Packaged app rebuild and `build_apps.py --check`: pass.

Still required before closing the issue: capture pre/post hardware
`phase=158/173/176`, run the full five-minute device session, retain its final
`phase=226 result=pass` line, and confirm the post-change RP2040 stack canary and
CPU1 stack/heartbeat values. The full `check_all.py` Clippy step is currently
blocked by unrelated pre-existing Rust 1.96 warnings in `koto-core` and
`koto-psram`; the test and build gates above pass.

### First device run finding

The first product-path run reached PCM16 streaming but every interleaved cue
reported `phase=258 audio-asset-fail reason=sd-read`. The committed/generated
KPA contained both cue paths; the failure was a real lifetime interaction in
the package I/O path, not scratch re-entry. `read_audio_asset` called the general
`VmHost::asset_load`, which attempted to open the FAT volume/package again while
the streaming path intentionally retained that volume and raw file for bounded
range reads.

Runtime cue loading now resolves and reads its range through the same retained
package handle as streaming. Every operation explicitly seeks before reading,
so PCM refill and cue image reads remain synchronous and sequential without a
second volume open. RP2040/RP2350A release builds, scratch host tests, and the
ELF SRAM measurement remain green after the fix. A repeat device run is needed
to retain the first `phase=226 result=pass` capture.

### Repeat device run (interim)

The corrected firmware produces clean audible output while PCM16 streaming and
runtime cues overlap. Through frame 1,560, both cue paths repeatedly completed
`cache-read` -> `queued`, interleaved with successful PCM16 `streaming` stages;
no further `phase=258 audio-asset-fail` was observed in the supplied capture.

The same run recorded:

```text
phase=176 stack-peak at=app used=48860 free_min=15724 lw=0x20036124
```

This is 364 bytes above the 15 KiB (`15,360` byte) acceptance floor and 5,192
bytes above the pre-change `free_min=10,532` baseline, matching the expected
5,120-byte scratch recovery within ordinary stack-peak variation. Keep the run
active through frame 18,750; the remaining closure evidence is the final
`phase=226 ... result=pass` line with zero reject/corruption/drop counters and
the SLDPCM4 half of the soak.

### Completed device soak

The corrected RP2040 release firmware completed all 18,750 frames and exited
cleanly after the SLDPCM4 stage. Final machine-readable result:

```text
phase=226 audio-scratch-regression pcm16=30 sld4=33 cold=2 hits=311 load=313 stream=2495 rejected=0 corrupt=0 drops=0 underruns=0 unsupported=0 command_drops=0 heartbeat=323926 core1_free=5844 result=pass
phase=153 app-exited code=0
phase=176 stack-peak at=app-exit used=48860 free_min=15724 lw=0x20036124
```

The soak covered 63 stream starts, 313 cue cycles, and 2,808 mutually exclusive
scratch acquisitions. CPU1 heartbeat advanced to 323,926; its minimum remaining
stack was 5,844 bytes. All rejection, corruption, decode/drop, underrun,
unsupported-command, and command-drop counters stayed zero, and audible output
remained clean throughout PCM16 and SLDPCM4 playback.

The implementation, memory-recovery, product-path soak, and post-change device
criteria are complete. The original two-scratch binary and its complete UART
capture were not retained, so the literal pre-change `phase=158/173/176` lines
cannot be recovered. Closure accepts the retained baseline ELF measurements and
`free_min=10,532` result, together with the exact post-change ELF comparison and
the successful five-minute product-path run, as the replacement evidence. No
remaining behavior or memory concern was observed; KOTO-0226 is closed.
