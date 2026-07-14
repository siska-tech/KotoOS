# KotoAudio codec demo assets

Both KACL files use the complete 16 kHz mono `sample.wav`, so the demo compares
long-form streaming with the same source audio. Assets larger than the bounded
owned-clip buffer automatically use the host streaming path.

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