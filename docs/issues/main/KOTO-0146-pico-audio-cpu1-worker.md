# KOTO-0146: Pico audio CPU1 worker for stable PCM service

- Status: in-progress
- Type: feature / integration
- Priority: P1
- Related: KOTO-0133, KOTO-0145, KOTO-0114, KOTO-0095, KOTO-0098, KOTO-0029

## Background

KOTO-0145 introduced a minimal PCM playback path on the Pico PWM backend.
The current service path still runs on the same core as app VM and rendering.
Under heavy app frames, audio submit bursts can overrun the queue and increase
`drops`, even when output eventually recovers.

Recent hardware logs show:

- frequent queue saturation (`buffer_level` near/at capacity)
- rising `drops` during heavy frame load
- service cadence coupled to frame timing

This indicates the single-core schedule is a stability bottleneck for PCM
service timing.

The Pico host-call path currently accepts the same audio ABI as KotoSim, but it
does not yet implement the KotoSim music engine. `play_bgm` / `play_bgm_asset`
still map to short tone fallbacks on Pico; there is no looping BGM voice state,
KotoMML parsing/playback, or BGM+SFX mixer parity yet.

KotoSim's reference model is `SimAudio`: a host-owned mixer with looping BGM
voices, bounded one-shot SFX voices, a raw PCM queue fed by `audio_submit_i16`,
and separate BGM/SFX mix gains. Pico should not copy the `Vec`/`alloc`-based host
implementation directly, but KOTO-0146 should make the CPU1 protocol and worker
state compatible with that model.

## Goal

Move Pico PCM output service to CPU1 so audio pacing is decoupled from app VM and
rendering on CPU0.

First milestone goal:

- keep existing PWM backend and hostcall API
- run a dedicated CPU1 audio service loop
- feed it via a bounded queue from CPU0
- reduce queue saturation and `drops` under KotoBlocks load
- leave a clear upgrade path for `play_bgm_asset` / `stop_bgm` to control a
      host-owned looping BGM stream, matching KotoSim semantics over time
- preserve current safety behavior (no panic on full/empty)

## Non-goals

- I2S or external DAC migration
- PSRAM audio streaming
- full KotoMML mixer parity in the first CPU1 milestone
- advanced resampling
- changes to KOTO-0132 PSRAM DMA / CodeWindow behavior

## Plan

1. Add a CPU0->CPU1 bounded SPSC command/data queue for audio.
2. Introduce a CPU1 audio worker loop with fixed-rate PWM sample service.
3. Convert CPU0 hostcalls to enqueue work only (`audio_submit_i16`, selected SFX).
4. Keep tone fallback path for unsupported/overload cases.
5. Add worker-aware diagnostics and summary counters.
6. Validate behavior under KotoBlocks stress frames.
7. Add the CPU1 command/state shape needed for KotoSim-style host audio:
       `PlayBgm`, `StopBgm`, `PlaySfx`, raw PCM submit, and deterministic reset/silence.
8. Keep SD/FAT and `.kmml` asset loading on CPU0. For `play_bgm_asset`, CPU0 should
       validate `audio/*.kmml`, load/parse or precompile the asset into a bounded event
       representation, then enqueue a compact BGM command to CPU1.
9. Start BGM with a minimal fixed-rate path: one looping voice or pre-rendered
       event stream, mixed with selected SFX and raw PCM using KotoSim's model
       (BGM below unity, SFX at higher priority). Expand toward 2-4 BGM voices only
       after worker timing is measured.
10. Make `stop_bgm` a high-priority control command that clears BGM voice state even
            when sample/data queues are saturated; a full audio reset remains separate and
            must force hardware silence.
11. Add BGM-aware counters to `phase=173`: active BGM voices, active SFX voices,
            BGM starts/stops, command drops, mixer saturations/overruns, and worker late/max
            jitter.

## KotoSim reference points

- `src/koto-sim/src/audio.rs::SimAudio` owns looping BGM voices, one-shot SFX
      voices, raw PCM, and BGM/SFX gains.
- `SimAudio::render()` mixes BGM + SFX + raw PCM into mono i16 output.
- `SimAudio::play_bgm_mml_banked()` restarts looping multi-track KotoMML BGM;
      `stop_bgm()` clears only the BGM voices.
- `src/koto-sim/src/runtime/host.rs::play_bgm_asset()` validates package-local
      `audio/*.kmml`, loads optional `#INST` wavetable references, and starts the BGM
      engine.
- Pico CPU1 should implement the same host-owned boundary, but with fixed-capacity
      data structures and no dynamic allocation in the worker.

## Acceptance criteria

- [x] PCM service runs on CPU1 while CPU0 continues app VM/rendering.
- [x] `audio_submit_i16` and selected SFX path enqueue successfully to CPU1.
- [x] CPU1 command protocol includes BGM control commands, even if the first
      milestone still routes unsupported BGM to the existing fallback.
- [x] `play_bgm_asset` no longer disappears silently in the implementation plan:
      it is either a bounded KotoMML/event-stream BGM start command or a clearly
      logged unsupported/fallback case.
- [x] `stop_bgm` is serviced as a high-priority command and clears BGM state without
      depending on PCM queue space.
- [x] During representative KotoBlocks gameplay, `drops` is reduced versus the
      current CPU0-only path.
- [x] `underruns` and `unsupported_count` remain bounded and clearly logged.
- [x] BGM/SFX/raw PCM mixing preserves KotoSim's ownership model: apps request
      high-level audio actions; host/worker owns synthesis and PCM output.
- [x] No panic on queue full/empty; fallback remains functional.
- [x] Existing square-wave fallback still works.
- [x] KOTO-0132 PSRAM/CodeWindow behavior remains unchanged.
- [x] CPU1 does not access VM/app/render state directly.
- [x] Stop/silence command is serviced even when the PCM data queue is saturated.
- [x] CPU1 logging is counter-based; high-frequency worker loop does not emit per-sample logs.
- [x] Worker late/max jitter is summarized in phase=173.

## Implementation notes

- CPU1 PCM service milestone and bounded KotoMML BGM playback are implemented and
      hardware-audible on Pico. KOTO-0146 remains open while the Pico device
      profile quality is refined.
- `PicoAudioBackend` is now a CPU0 host-call handle; `PicoAudioWorker` owns the
      PWM peripheral on CPU1 and services the 8 kHz PCM queue at a fixed cadence.
- CPU0 feeds a bounded critical-section protected PCM ring plus a bounded command
      queue. High-priority `StopBgm` / `StopAll` commands bypass normal command
      queue capacity so silence is not blocked by PCM saturation.
- The CPU1 protocol includes raw PCM submit, `PlaySfx`, `PlayBgm`,
      `StartBgmScore`, `StopBgm`, and `StopAll`.
- Pico now parses a fixed-capacity KotoMML subset on CPU0 and transfers a compact
      BGM score to CPU1. The CPU1 worker mixes looping BGM voices with raw PCM at
      8 kHz. Long scores are truncated to the Pico event budget and loop from the
      retained prefix.
- Pico loads `#INST` / `.kwt` custom instruments into a fixed-capacity wavetable
      bank: up to four custom ids, each with a 2-64 sample single-cycle table plus
      ENV/GAIN, stored without allocation.
- Pico BGM voices now apply fixed-point ADSR envelopes and per-instrument gains,
      matching the KotoSim built-in profile more closely without floating point or
      allocation on CPU1.
- Pico SFX `.kmml` assets are now parsed as one-shot MML and rendered into the
      existing raw PCM queue before falling back to fixed beep tones, so common
      KotoBlocks effects no longer need the `sample_rate=-1` tone path.
- Pico SFX `.kmml` playback now runs as a CPU1 one-shot synth voice instead of a
      pre-rendered 2048-sample PCM burst, so longer effects such as game-over can
      play to completion without being clipped by the raw PCM ring capacity.
- CPU0 keeps a single pending SFX score payload to avoid growing SRAM use; when
      dense bursts arrive before CPU1 consumes the command, longer effects replace
      shorter pending effects so game-over is favored over incidental line-clear
      ticks.
- PCM-mode silence now outputs the centered PWM duty instead of duty 0 while BGM
      is active, avoiding clicks at zero crossings/rests and reducing false
      underrun diagnostics.
- `phase=173` now summarizes `command_drops`, BGM/SFX voice state,
      `mixer_saturations`, `worker_late`, and `worker_max_jitter_us`.

## Remaining work

- Hardware-validate Pico `.kwt` custom wavetable playback with KotoSnake BGM/SFX
      assets; malformed or oversized KWT assets should fail clearly rather than
      silently falling back.
- Revisit the Pico BGM event budget after SRAM/stack pressure is measured; current
      long scores are intentionally truncated to keep CPU1 boot stable.
- Subjectively tune SFX-over-BGM behavior across both raw PCM SFX and tone fallback
      SFX.
- Hardware-check that `play_sfx_asset` logs use 8 kHz PCM frames for KotoBlocks
      `.kmml` effects instead of the `sample_rate=-1` tone fallback.
- Hardware-check that game-over SFX is no longer clipped and that `sfx_voices`
      reports one active one-shot voice during longer effects.
- Watch for missing SFX during dense line-clear and game-over bursts. A multi-entry
      SFX score queue was avoided after it increased static memory enough to risk
      Pico boot failure.

## Validation

- Build: `cargo check -p koto-pico --bin koto_firmware --target thumbv6m-none-eabi`
      passes.
- Build after Pico ADSR/instrument fallback tuning:
      `cargo check -p koto-pico --bin koto_firmware --target thumbv6m-none-eabi`
      passes.
- Build after Pico one-shot SFX MML rendering and PWM zero-duty click fix:
      `cargo check -p koto-pico --bin koto_firmware --target thumbv6m-none-eabi`
      passes.
- Build after moving SFX `.kmml` from pre-rendered PCM bursts to a CPU1 one-shot
      synth voice:
      `cargo check -p koto-pico --bin koto_firmware --target thumbv6m-none-eabi`
      passes.
- Build after reverting the multi-entry SFX score queue to a single pending slot
      with longer-effect replacement:
      `cargo check -p koto-pico --bin koto_firmware --target thumbv6m-none-eabi`
      passes.
- Build after adding fixed-capacity Pico `.kwt` loading for `#INST` custom
      instruments:
      `cargo check -p koto-pico --bin koto_firmware --target thumbv6m-none-eabi`
      passes.
- Hardware KotoBlocks run through frame 750 produced audible output with
      `drops=0`, `command_drops=0`, `unsupported_count=0`, and
      `samples_submitted == samples_played`:
      `samples_submitted=2016 samples_played=2016 drops=0 underruns=34
      command_drops=0`.
- Hardware KotoBlocks BGM validation: user confirmed BGM is audible, though rough.
      The frame 420-750 summaries show a stable BGM start with four active BGM
      voices and no transport/mixer failures:
      `bgm_starts=1 bgm_voices=4 drops=0 command_drops=0 mixer_saturations=0`,
      with `samples_submitted == samples_played` through
      `samples_submitted=2144 samples_played=2144` and
      `worker_max_jitter_us=581`.

## Notes

- Start with minimal queue protocol and one fixed sample rate.
- Favor deterministic shutdown: stop command must force hardware silence.
- Keep logs concise (`phase=172` event lines and `phase=173` summaries).
