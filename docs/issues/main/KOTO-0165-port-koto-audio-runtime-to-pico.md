# KOTO-0165: Port the koto-audio runtime to Pico and delete the legacy audio engine

- Status: done (hardware-verified 2026-07-04: BGM + SFX both play on device, better
  quality than the legacy synth)
- Type: feature / removal
- Priority: P1
- Related: KOTO-0164, KOTO-0163, KOTO-0162, KOTO-0161, KOTO-0160, KOTO-0146, KOTO-0148

Policy: [AUDIO_DEPRECATION_POLICY.md](../../architecture/AUDIO_DEPRECATION_POLICY.md).
Cue model: [PRIMARY_AUDIO_CUE_MODEL.md](../../architecture/PRIMARY_AUDIO_CUE_MODEL.md).
Bridge reference: [KOTO_KOTOBLOCKS_AUDIO_BRIDGE.md](../../devlog/KOTO_KOTOBLOCKS_AUDIO_BRIDGE.md).

## Goal

Run the bounded `koto-audio` runtime itself on the device — the same crate the SIM
bridge uses — and **delete** the legacy in-tree engine: the runtime KotoMML parser,
the `.kwt` wavetable loader, the hand-rolled synth (`PicoTrackPlayer` /
`PicoBgmPlayer`), the mirrored 16-event `PicoBgmScore` tables, and the tone-fallback
paths. This retires the KOTO-0161 known limitation (the full generated BGM table did
not fit the worker's event budget) and removes the "half-legacy" state KOTO-0162
left behind.

## What changed

### 1. CPU1 worker drives `koto_audio::DefaultAudioService`

`src/koto-pico/src/firmware/audio.rs` was rewritten. CPU1 owns a
`DefaultAudioService<'static, PwmBlockSink>` in its own `StaticCell` (KOTO-0148),
renders fixed 128-frame mixer blocks into a small output ring, and mixes the raw
`audio_submit_i16` stream additively per sample — the device mirror of how the SIM
folds the bridge output into `SimAudio`. Commands from CPU0 carry `&'static`
references into compiled cue tables (`PlayBgm` / `PlaySfx` / `StopBgm` / `StopAll`);
no score data is copied through the queue.

Output moved from 8 kHz to **16 kHz** (`AudioLimits::v0_default()`, the rate the
koto-audio built-in drum tables are authored at).

**Sample pacing is hardware-owned** (second hardware iteration): a DMA channel
(PAC ch 11, free of the LCD `DMA_CH0` and koto-psram `DMA_CH1` claims) paced by
DMA timer 0 streams precomputed duty words from a 256-word aligned ring into the
PWM slice-5 compare register at exactly 16 kHz — the KOTO-0114 `probe_audio`
pattern. The worker only *fills* the ring ahead of the DMA read position on a
~1 ms pass. The first cut paced samples from the CPU1 loop instead; the koto-audio
decode path costs tens of µs per sample on the M0+ (software i64 mul/div in the
envelope/scale math), which chronically overran the 62.5 µs slot and burst-played
the backlog — audible as severe distortion (`worker_late` ≈ 74 % of ticks,
`worker_max_jitter_us` ≈ 28 ms). With DMA pacing only *average* render cost
matters, and ~50 % CPU1 load has 16 ms of hardware lead to hide bursts.
`worker_late` / `worker_max_jitter_us` now describe fill-pass duration
(late = pass > 8 ms), and `underruns` counts real DMA-overtook-the-writer events.

### 2. Compiled cue tables replace runtime parsing

- `tools/koto-audio-gen` (new workspace tool): parses every `apps/*/audio/*.kmml`
  with the koto-audio-tools MML frontend, converts the **legacy dialect** (0-15 `V`
  volumes ×17 onto the 0-255 note scale; legacy synth/`.kwt` `@` ids remapped to the
  closest built-in instruments; 96 ticks/quarter so `L32` and dotted lengths stay
  integral), wraps BGM tracks in infinite loops, validates everything with the
  runtime validators, and emits `SequenceEvent` statics.
- `src/koto-pico/src/firmware/audio_cues_generated.rs` (vendored output): 42 cues —
  9 BGM (`PolyphonicSequence`) and 33 SFX (`Sequence`) covering kotomines, kotorogue,
  kotorun, kotoshogi, kotosnake, sokoban, and the KotoBlocks BGM — plus the two
  route arrays. Regenerate with
  `cargo run -p koto-audio-gen -- src/koto-pico/src/firmware/audio_cues_generated.rs`.
- `src/koto-pico/src/firmware/audio_cues.rs` (hand-written): the KotoBlocks SFX as
  verbatim ports of the SIM authored sequences (envelope/drum voices the compact
  path cannot express), the `play_sfx(id)` blip cues and `play_bgm(id)` built-in
  loops that replace the tone path, and the single `primary_audio_route` lookup.
  The KotoBlocks BGM is generated from the same `blocks_like_bgm.mml` source as the
  SIM table, so SIM and device stay cue-identical for the reference app.

### 3. Host dispatch is route-only

`app_host.rs`: `play_bgm_asset` / `play_sfx_asset` resolve the path through
`primary_audio_route` and hand the sequence to the worker — every declared app path
routes, nothing is read from SD, and a routing miss logs `result=unrouted` and
returns `UNSUPPORTED` (no legacy fallback chain). `play_sfx(id)` / `play_bgm(id)`
now play authored KotoAudio cues (`seq-sfx` / `seq-bgm`). `audio_submit_i16` is
unchanged apart from the 16 kHz device rate. The `AudioAssetScratch` SD buffers
(4.5 KiB), the `#INST` collector, and both SD asset loaders are gone.

### 4. Deleted

`parse_pico_bgm_mml`, `parse_pico_kwt`, `PicoInstrumentBank`, `PicoBgmScore`,
`PicoSfxScore`, `PicoTrackPlayer`, `PicoBgmPlayer`, `pico_instrument`,
`adsr_envelope_256`, `midi_phase_step`, the embedded `koto_blocks_*_score` mirror
tables, `AudioOutputMode::Tone` / `ToneOwner` and all tone commands,
`play_bgm_fallback`, `tone_for_sfx_id`, `tone_for_asset`, `submit_pcm_asset`
(`legacy-pcm`), and `AudioAssetScratch`.

## Budgets (measured, release)

- **RAM**: `data+bss` 182,812 → 188,992 bytes (**+6.0 KiB**: the CPU1 stack bump
  below plus the 1 KiB aligned DMA duty ring). The ~3 KiB service + output ring
  are covered by the removed scratch buffers and score slots. Const asserts:
  `AudioShared ≤ 6 KiB`, service `≤ 4 KiB`.
- **CPU1 stack**: 4 KiB → **8 KiB** (`AUDIO_CORE1_STACK_BYTES`). The koto-audio
  mixer keeps ~1.3 KiB of locals in one `tick()` frame (`[i64; 128]` accumulator +
  output block); the old 4 KiB budget was sized for the removed ≤2 KiB legacy
  synth. The service itself is **constructed on CPU0** and parked in its
  StaticCell *before* `spawn_core1` — building the ~3 KiB service on the core1
  stack overflowed it at worker start (first flash attempt was fully silent).
- **Flash**: text 552 KiB → 1,090 KiB (**+538 KiB** of 2,048 KiB; ~46% headroom
  left). Almost all of it is the koto-audio built-in PCM16 drum tables (~500 KiB at
  16 kHz), which the sequence decoder references as a whole. Acceptable today (apps
  live on SD); if flash gets tight, the follow-up is an upstream koto-audio feature
  to select a reduced drum set, not re-growing a device-side synth.

## Behavior notes

- **Timbre drift is expected and accepted** (per the KOTO-0162 no-compatibility
  policy): pulse voices become squares, noise percussion becomes closed hi-hat, and
  the kotosnake `.kwt` lead/bass/drum instruments map to saw/triangle/hi-hat.
  Loudness maps V15 → 255 with the SIM bus gains (BGM 150/256, SFX 200/256).
- **SIM is unchanged in this slice**: non-KotoBlocks apps still take the SIM legacy
  MML synth, so SIM and device timbres differ for those apps until the SIM legacy
  path is retired the same way (natural follow-up: route the SIM through the same
  generated tables and delete `koto-sim/src/audio.rs`'s MML synth).
- `probe_audio` (KOTO-0114 hardware probe) is self-contained on `koto-core` and
  untouched.

## Verification

- `cargo test` (workspace) green, including 4 new koto-audio-gen tests; generation
  re-validates every emitted cue with the koto-audio runtime validators.
- `cargo build -p koto-pico --target thumbv6m-none-eabi --bins` green (dev +
  release); firmware clippy adds no new finding (18 → 14 pre-existing warnings).
- `--memo-validation` green.
- **Hardware listening pass: done (2026-07-04).** First flash was silent (the
  CPU1-stack service construction, fixed above); second played with severe
  distortion (the CPU-paced output overrun, replaced by DMA pacing); third plays
  BGM and SFX cleanly, subjectively better than the legacy synth. The
  `phase=171 audio_pcm_diag done` line now also reports `samples_played`, which
  distinguishes a dead CPU1 worker (`samples_played=0`) from a routing problem
  (`unrouted` in `phase=172`).

## Follow-ups (not in this slice)

- ~~**SLDPCM4 drum tables** (flash)~~ — **done**, see
  [KOTO-0166](KOTO-0166-sldpcm4-drum-tables.md): drums are SLDPCM4 payloads on
  both Pico and SIM (-393 KiB flash; firmware at 35 % of the 2 MiB part).
- **Retire the SIM legacy MML synth** the same way (route the SIM through the
  same generated tables), restoring SIM↔device timbre parity for the six
  converted apps.
- If per-sample render cost ever becomes the bottleneck again, hoist the
  software i64 divisions out of koto-audio's per-sample envelope/scale math
  (power-of-two scales / incremental envelopes).
