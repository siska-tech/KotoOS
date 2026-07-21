# KotoAudio codec demo assets

Both KACL files use the complete 16 kHz mono `sample.wav`, so the demo compares
long-form streaming with the same source audio. Assets larger than the bounded
owned-clip buffer automatically use the host streaming path.

Press `3` to run the KOTO-0226 regression: PCM16 and SLDPCM4 stream stages each
run for 9,000 frames, while two deterministic KMML cues alternate every 60
frames. The first cue loads are cold PSRAM writes/read-backs and all later loads
are cache hits. Firmware emits the final machine-readable `phase=226
audio-scratch-regression ... result=pass|fail` line before the app exits.

Committed fixture SHA-256 values (also enforced by
`harness/check_audio_scratch.py`):

- `pcm16.kacl`: `6f826ffe6845fd034b34cf21028e50c5e457efd4ed818ad5d10671c8359cde58`
- `sld4.kacl`: `6ef8b073fe1b80549c53908566b31bb662bba540d6aceadc83204d00ccba39c7`
- `cue_a.kmml`: `4822aedac252b43d7788d6db3a1d2f517653a42aa56980464b7e6b340ec533df`
- `cue_b.kmml`: `8274c19d28aa3503fc9528a8499b77fbd2c7321a14a717cd8311d5b9ab33df5e`

```powershell
cargo run --manifest-path src/koto-audio/Cargo.toml -p koto-audio-tools `
  --bin koto-audio-convert -- --codec pcm16 `
  apps/samples/audio_codecs/audio/sample.wav `
  apps/samples/audio_codecs/audio/pcm16.kacl

cargo run --manifest-path src/koto-audio/Cargo.toml -p koto-audio-tools `
  --bin koto-audio-convert --features experimental-sldpcm4 -- `
  --codec experimental-sldpcm4 --sldpcm4-fallback force `
  apps/samples/audio_codecs/audio/sample.wav `
  apps/samples/audio_codecs/audio/sld4.kacl
```

For intentionally short one-shots, add `--max-samples COUNT` after the codec
options.

sample.wav is based from ReactOS boot.ogg
https://freesound.org/people/jobro/sounds/244604/
