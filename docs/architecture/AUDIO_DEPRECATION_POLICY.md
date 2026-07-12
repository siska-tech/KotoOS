# KotoOS audio deprecation policy (KOTO-0162)

Status: **active policy**. Establishes the KotoAudio generated-sequence runtime as
the primary KotoOS audio path and marks the original KotoOS audio implementation as
**legacy**. This document is the source of truth for which audio path new work
targets; the KotoBlocks bridge reference is
[KOTO_KOTOBLOCKS_AUDIO_BRIDGE.md](../devlog/KOTO_KOTOBLOCKS_AUDIO_BRIDGE.md).

> **KOTO-0165 update:** on **Pico** the legacy path has since been **deleted**, not
> just deprecated — the device now runs the ported `koto-audio` runtime directly and
> every app cue is a compiled table
> (see [KOTO-0165](../issues/main/KOTO-0165-port-koto-audio-runtime-to-pico.md)). The
> legacy paths below survive only in **KotoSim** (`crate::audio`), pending the same
> retirement.

## Background

The original KotoOS audio path grew exploratively and reached a working state:

- a runtime KotoMML (`.kmml`) subset parser + square/pulse/triangle/saw/noise synth;
- a KotoWaveTable (`.kwt`) custom-instrument loader;
- multi-track BGM, bounded one-shot SFX, and a raw-PCM (`audio_submit_i16`) queue;
- on Pico, a CPU1 PWM worker with the same MML/`.kwt` model and a tone fallback.

It works, but it was bolted on after the fact: the sound quality is limited and the
design boundaries (host-owned MML synth, per-app wavetable banks, tone fallbacks) are
harder to reason about and extend than a purpose-built runtime.

Meanwhile the sibling `koto-audio` crate matured into a bounded, deterministic audio
runtime with: PCM16 clip playback, experimental SLDPCM4, monophonic/polyphonic
sequences, a BGM/SFX bus split, P/ECE/MusLib-style built-in instruments, fixed PCM
drums, a compact sequence representation, an MML-subset → compact-table generator, a
generated Rust table, render-to-WAV, and KotoSim/KotoBlocks + Pico first-slice
bridges.

## Policy

1. **Primary path.** New music and SFX target the KotoAudio generated-sequence
   runtime: compact sequences / generated tables, built-in instruments, fixed PCM
   drums, and the KACL clip path. KotoBlocks is the reference integration (SIM and
   Pico) — see the bridge doc.

2. **Legacy path (deprecated on SIM; deleted on Pico, KOTO-0165).** These are
   **legacy** and receive no new features:
   - the runtime `.kmml` subset parser (KotoSim `crate::audio`; the Pico
     `parse_pico_bgm_mml` was removed);
   - the `.kwt` wavetable loader (KotoSim `InstrumentBank`; the Pico
     `parse_pico_kwt` was removed);
   - the tone-fallback "music" path (removed on Pico together with the tone owners).

3. **No compatibility guarantee.** Old KotoMML behavior, the `.kmml`/`.kwt` on-disk
   formats, and the legacy synth voicing are **not** guaranteed to be preserved. Do
   not build new work on them; they may change or be removed once no shipped app
   depends on them.

4. **Safety is still mandatory.** Deprecation is not permission to regress
   robustness. The legacy paths must continue to **not panic**, stop safely, and keep
   their diagnostics. Bounded buffers, saturating mixing, and drop-on-overflow
   remain in force on both SIM and Pico.

## Diagnostics convention

The audio per-call trace (`phase=172`, DIAG-0001 §3.2) records a `result=` label so a
UART/host capture shows which path served each hostcall. The label distinguishes the
primary sequence path from the legacy fallbacks:

| `result=` label      | Path                          | Status  |
| -------------------- | ----------------------------- | ------- |
| `seq-bgm`            | KotoAudio generated BGM        | primary |
| `seq-sfx`            | KotoAudio authored/embedded SFX| primary |
| `legacy-mml`         | `.kmml` parse + synth          | legacy  |
| `legacy-pcm`         | raw `.kmml`-as-PCM asset       | legacy  |
| `legacy-tone`        | tone SFX / `play_sfx`          | legacy  |
| `legacy-tone-fallback` | tone stand-in for failed BGM MML | legacy |
| `*-error`, `dropped` | rejected / failed              | error   |

Legacy results are logged with a `legacy-` prefix so a mostly-`seq-*` capture makes it
obvious when an app falls back to a deprecated path. The health counters
(`unsupported_count`, `command_drops`, `mixer_saturations`, `drops`, `underruns`) are
**retained unchanged** — deprecation does not drop instrumentation.

Since KOTO-0165 the **Pico** firmware can no longer emit `legacy-*` labels: every
hostcall is served by the KotoAudio runtime (`seq-bgm` / `seq-sfx` / `ok`), and a
path with no route logs `result=unrouted` and plays nothing. `legacy-*` labels can
still appear in SIM captures.

A **routing miss** on the primary path (KOTO-0164) — a primary-path asset whose path
did not match its route table entry — falls through to a `legacy-*` label. That is
safe but is a defect for a primary-path app; the asset → cue routing model and this
failure signature are documented in
[PRIMARY_AUDIO_CUE_MODEL.md](PRIMARY_AUDIO_CUE_MODEL.md).

## Non-goals

Deliberately out of scope for this policy and its first slice:

- full deletion of the old audio path (**superseded on Pico**: KOTO-0165 deleted it;
  the SIM copy remains the last holdout);
- reimplementing the `.kmml` or `.kwt` loaders;
- preserving old KotoMML compatibility;
- freezing a stable audio ABI;
- adding dynamic allocation to the runtime.

On SIM the legacy code stays isolated behind clear module banners and Rustdoc, so the
knowledge (MML grammar, ADSR model, tone fallback) stays available while the primary
path takes over. On Pico that knowledge now lives in this document trail and in
`tools/koto-audio-gen`, which encodes the legacy dialect conversion (volume scale,
instrument mapping, tick resolution) as the offline build step.
