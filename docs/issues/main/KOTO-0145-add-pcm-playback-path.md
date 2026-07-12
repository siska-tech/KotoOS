# KOTO-0145: Add PCM playback path for Pico audio backend

- Status: todo
- Type: feature / integration
- Priority: P1
- Related: KOTO-0133

## Background

KOTO-0133 connected the audio hostcall path to a minimal Pico audio backend.
The current backend can produce square-wave tones through PWM on GP26/GP27 and
maps KotoBlocks SFX asset names to simple tones.

Current verified state:

- audio hostcalls no longer all return `Unsupported`
- startup audio diagnostic tone works
- KotoBlocks SFX asset calls reach the backend
- backend reports `result=ok`
- `drops=0`
- `underruns=0`
- `unsupported_count=0`

However, this is not real PCM playback yet. `play_sfx_asset` currently maps asset
names to generated tones, and `audio_submit_i16` does not yet drive a real sample
stream.

## Goal

Add a minimal PCM playback path for the Pico audio backend.

The first goal is not high-quality audio. The goal is:

- accept signed 16-bit PCM samples from `audio_submit_i16`
- queue them safely
- output them through the existing PWM backend
- track drops and underruns
- play at least one short PCM effect on hardware

## Non-goals

- full music engine
- high-quality DAC output
- I2S
- PSRAM audio streaming
- compression
- CPU1 audio worker
- advanced mixer
- resampling beyond a simple fixed sample-rate path

## Plan

1. Add a small PCM ring buffer.
2. Implement `audio_submit_i16` to enqueue samples.
3. Add a PWM sample service path that consumes queued PCM.
4. Support one fixed sample rate first, for example 8kHz, 11.025kHz, or 16kHz.
5. Convert i16 PCM to PWM duty safely.
6. Add counters:
   - samples_submitted
   - samples_played
   - drops
   - underruns
   - buffer_level
7. Add a PCM diagnostic tone or short waveform.
8. Connect one KotoBlocks sound event to PCM playback.
9. Keep the existing square-wave asset fallback.

## Success criteria

- `audio_submit_i16` accepts a short PCM buffer and returns ok.
- A PCM diagnostic sound is audible on hardware.
- KotoBlocks can trigger at least one PCM-backed sound.
- `unsupported_count=0`
- no panic if buffer is full or empty
- drops/underruns are logged clearly
- KOTO-0132 PSRAM/CodeWindow behavior remains unchanged