# KOTO-0133: Connect audio hostcalls to PicoCalc audio backend

- Status: done (minimal backend path validated on hardware)
- Type: feature / integration
- Priority: P1
- Related: KOTO-0095, KOTO-0132

## Background

KotoOS already has app/runtime-side audio concepts, but the embedded PicoCalc
path is currently disconnected. Audio calls appear to return `Unsupported`, so
apps cannot produce sound on hardware.

After KOTO-0132 stabilized PSRAM-backed CodeWindow execution, the next user-visible
gap is audio output.

## Goal

Connect the audio path end-to-end on PicoCalc hardware.

The first goal is not high-quality audio. The first goal is simply:

- audio hostcall reaches runtime
- runtime reaches hardware backend
- backend produces at least one audible tone or accepts PCM samples
- KotoBlocks can trigger at least one sound event

## Non-goals

- full music engine
- polished synthesizer
- PSRAM audio streaming
- multi-channel mixer optimization
- QPI/TX-DMA work
- unrelated rendering/runtime refactors

## Plan

1. Trace current audio hostcall path.
2. Identify where `Unsupported` is returned.
3. Add concise audio diagnostics.
4. Implement or connect minimal PicoCalc audio backend.
5. Add 440Hz beep / tiny PCM diagnostic.
6. Connect one KotoBlocks sound event.
7. Document remaining unsupported audio features.

## Success criteria

- Audio diagnostic produces sound on hardware.
- Audio hostcalls no longer all return `Unsupported`.
- KotoBlocks triggers at least one audible sound.
- No panic or crash if audio backend is unavailable.
- Unsupported features are logged clearly.

## Implementation Notes (2026-06-25)

- Pico firmware now owns a minimal PWM square-wave backend (`GP26/GP27`, PWM slice 5)
	and runs a launch-time diagnostic tone:
	- `audio_diag start` (440Hz / 160ms)
	- `audio_diag done` with backend + counters
- `DeviceHost` now implements audio hostcalls directly instead of inheriting
	`VmHost` defaults:
	- `audio_submit_i16`: validates shape, emits fallback tone, returns accepted frames
	- `play_sfx` / `play_bgm`: map to short tones
	- `play_sfx_asset` / `play_bgm_asset`: map known `.kmml` asset names to tones
	- `stop_bgm`: stops the active tone
- Audio hostcall diagnostics are emitted as concise per-call lines (`phase=172`) with:
	- hostcall id + name
	- sample_rate (or `-1` when not part of ABI)
	- frames/channels/bytes (when available)
	- backend + result
- Throttled runtime summaries (`phase=173`) now report:
	- `audio_events`
	- `samples_submitted`
	- `drops`
	- `underruns`
	- `unsupported_count`

## Remaining Limitations (Intentional for KOTO-0133)

- Backend is a minimal beep/tone fallback, not full PCM playback quality.
- No ADSR/instrument-bank/mixer/music engine parity yet on Pico firmware.
- `sample_rate` is not currently carried by `audio_submit_i16` ABI and is logged as unavailable.

## Hardware Validation (2026-06-25)

- User-confirmed on real PicoCalc:
	- audio output is audible (`ばっちり鳴ってます`)
	- hostcall path reaches backend (`play_sfx_asset`, id `0x35`)
	- summary counters stay healthy during run:
		- `audio_events=60`
		- `drops=0`
		- `underruns=0`
		- `unsupported_count=0`