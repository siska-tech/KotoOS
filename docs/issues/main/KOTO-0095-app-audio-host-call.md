# KOTO-0095: App audio host call (BGM and sound effects)

- Status: done
- Type: feature
- Priority: P2
- Requirements: FR-AUD-1

## Goal

Let Koto apps play background music and sound effects so games such as
[KotoBlocks](KOTO-0094-koto-blocks-game.md) can have audio. The `audio_submit_i16`
host call (ABI id `0x30`) and the [PCM mixer core](KOTO-0023-audio-mixer-core.md)
already exist, but the call is not wired through the runtime, compiler, or simulator,
and KotoSim has no audio output backend.

## Acceptance Criteria

- [x] `VmHost::audio_submit_i16` trait method + VM dispatch arm (mirroring
  `draw_pixels_rgb565`), with a runtime test.
- [x] An `audio_submit` SDK prelude wrapper in `koto-compiler`, documented in
  `docs/KOTO_SDK.md`, with a compile test.
- [x] KotoSim host feeds submitted PCM frames into the mixer and plays them
  through a real audio backend (`cpal`) in window mode, with a deterministic
  headless capture path (`--app … --audio OUT.wav`) for scripted/golden runs.
- [x] A KotoMML-driven BGM track and a set of SFX wired into KotoBlocks (move,
  rotate, lock, line clear, game over).
- [x] `python harness\check_all.py` passes.

## Resolution

Shipped two layers. The low-level `audio_submit_i16` primitive (id `0x30`) is now
wired through the runtime trait/dispatch, the assembler, the compiler (`audio_submit`
wrapper), and the simulator mixer. On top of it, a **host-owned audio service**
(`play_sfx` / legacy `play_bgm` / `stop_bgm`, ids `0x31`–`0x33`, host ABI minor 8)
lets apps trigger sound while the host synthesizes KotoMML — keeping PCM out of the 4 KB
VM heap, per [KOTOMML_FORMAT.md](../../spec/KOTOMML_FORMAT.md). The synth lives in koto-sim
(`src/koto-sim/src/audio.rs`); window mode plays it through a `cpal` device stream and
the headless `--audio` path renders the same deterministic timeline to a WAV.
KotoBlocks uses the host-owned service. Host ABI minor 10 later added
`play_bgm_asset(path, len)`; all game BGM now ships as package-local `.kmml`
assets rather than host-embedded music IDs.

Out of scope (follow-up): an `audio_set_volume` call.

## Notes

Split out of [KOTO-0094](KOTO-0094-koto-blocks-game.md): the graphics half (the
`draw_pixels_rgb565` sprite/tile primitive) shipped there; audio is a larger,
cross-cutting effort (runtime + compiler + a simulator sound backend) and is deferred.
Determinism for scripted/golden runs must be preserved (capture, do not require a
real device).
