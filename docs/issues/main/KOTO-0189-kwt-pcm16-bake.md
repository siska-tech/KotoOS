# KOTO-0189: Native KotoAudio phrase bake to PCM16 KACL

- Status: done; revised 2026-07-13 after removal of the former wavetable format.
- Type: feature
- Priority: P3
- Related: KOTO-0188, KOTO-0165, KotoAudio KACL asset format.

## Decision

The retired custom-wavetable source format is no longer an authoring or runtime
option. Native KotoAudio KMML is the sequence source of truth. `koto-mml bake`
renders that same source to a runtime-ready PCM16 KACL clip when a pre-rendered
one-shot is preferable. SLD4 KACL is produced by `koto-audio-convert` and uses
the same package playback route.

Both codecs are stored inside the app's KPA and are played through
`play_sfx_asset("audio/*.kacl", len)`. SIM and Pico copy the KACL into bounded
host-owned storage before decoding, so playback never borrows an SD/KPA scratch
buffer.

## Acceptance Criteria

- [x] `koto-mml bake IN.kmml OUT.kacl` accepts Native KotoAudio KMML, validates
      the PCM16 KACL, supports loop metadata, and reports payload cost.
- [x] The committed fixture is format-neutral:
      `harness/fixtures/kacl_bake/native_pcm16_jingle.{kmml,kacl}`.
- [x] PCM16 and SLD4 KACL assets can be packaged without KAQ1 recompilation.
- [x] The `KotoAudio PCM16 / SLD4` sample app exercises both codecs through the
      common KPA asset route on SIM and Pico.
- [x] No retired wavetable asset is shipped or registered by the SDK tooling.
