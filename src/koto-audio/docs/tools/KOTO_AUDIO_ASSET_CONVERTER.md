# KotoAudio Runtime-Ready Clip Assets

M9 defines a host-side converter boundary for v0 PCM16 mono SFX clips. PCM16 is
the required v0 asset path. Runtime assets are already decoded into mixer-ready
mono payloads; the runtime does not parse WAV files, downmix channels, resample
arbitrary input, or repair source media.

## Clip Asset Header

All integer fields are little-endian. The v1 header is 48 bytes:

| Offset | Size | Field |
|---:|---:|---|
| 0 | 4 | magic `KACL` |
| 4 | 2 | version, currently `1` |
| 6 | 2 | header size, currently `48` |
| 8 | 2 | codec id, `1` for PCM16, `16` reserved for experimental SLDPCM4 |
| 10 | 2 | channel count, v0 accepts `1` |
| 12 | 4 | sample rate in Hz |
| 16 | 4 | mono sample count |
| 20 | 4 | loop start sample, inclusive |
| 24 | 4 | loop end sample, exclusive |
| 28 | 4 | loop count, `0` none, `u32::MAX` infinite |
| 32 | 4 | payload byte size |
| 36 | 2 | placement hint, `0` unspecified, `1` resident |
| 38 | 4 | optional budget hint bytes, `0` unspecified |
| 42 | 6 | reserved, written as zero for v1 |

Payload bytes begin at `header_size`. Required version 1 payloads are signed
PCM16 little-endian mono samples, and `payload_size` must equal
`sample_count * 2`. Experimental SLDPCM4 payloads are accepted only by runtime
builds compiled with the `experimental-sldpcm4` feature; otherwise the codec id
is parsed but rejected as unsupported. The v1 runtime ignores reserved bytes
after validating `header_size`, so future compatible headers can append data
after the 48-byte base header without moving the payload start.

## Converter Boundary

The host tool crate is `koto-audio-tools`. Its CLI is:

```text
koto-audio-convert [--codec pcm16|experimental-sldpcm4] [--sldpcm4-fallback pcm16|reject|force] [--target-rate hz] [--strict-input|--no-resample] <input.wav> <output.kacl>
```

v0 tools accept PCM16 WAV input. Stereo and multi-channel input is downmixed to
mono on the host with an average mix, and mismatched sample rates are resampled
on the host with linear interpolation. The default target rate is the current
mixer policy rate, 16000 Hz. `--target-rate <hz>` selects a different output
rate, and `--strict-input` or `--no-resample` restores the older
reject-on-mismatch behavior for non-mono or mismatched-rate input. The runtime
still does not parse WAV, downmix, or resample.

Provisional sample-rate tiers:

- 16000 Hz: low-cost default candidate.
- 22050 Hz: quality SFX candidate.
- 44100 Hz: host-side/reference comparison.

The tools crate also provides a debug/listening validation decoder:

```text
koto-audio-decode <input.kacl> <output.wav>
```

It decodes runtime-ready PCM16 KACL assets, and decodes experimental SLDPCM4 KACL
assets only when `koto-audio-tools` is built with `experimental-sldpcm4`. The
output is always PCM16 mono WAV at the sample rate stored in the KACL header.
Malformed assets and codecs unsupported by the current build are reported as
decode errors. This is a host-side listening tool; it does not add WAV writing
or codec promotion to the runtime crate.

## Built-In Drum Table Generation

Fixed sequence drums are not arbitrary KACL clips. They are built into the
runtime as signed PCM16 mono static slices and selected by sequence instrument
id. The current placeholder drum tables are authored for 16000 Hz and return
silence after the static slice ends; pitch is ignored, while note volume,
source volume, bus gain, and app/master gain still apply.

For future replacement of placeholder data, the tools crate provides a
host-side Rust table generator:

```text
koto-audio-drum-table [--symbol NAME] [--target-rate hz] [--strict-input|--no-resample] <input.wav> <output.rs>
```

The input is PCM16 WAV. By default the tool uses the same host-side mono
downmix and linear resampling policy as `koto-audio-convert`, then emits a Rust
fragment such as:

```rust
pub static DRUM_BD: &[i16] = &[
    0, 1200, -800,
];
```

This output is source text for reviewed built-in sequence drum tables; it is
not a KACL asset and does not add WAV parsing, resampling, heap allocation,
`.kmml` loading, or `.kwt` loading to the runtime crate. Do not import
MusLib/P/ECE-derived sample data, or any other third-party drum data, unless the
license has been checked and the result is documented.

The default converter output remains PCM16, which is the required v0 and golden
path. `--codec experimental-sldpcm4` explicitly requests the host-side SLDPCM4
experiment and writes KACL codec id `16` when accepted. SLDPCM4 is not v0
required, depends on PCM16 fallback as the operational compatibility path, does
not claim compatibility with existing SLDPCM files or containers, and should be
treated as a listening-test candidate.

The experimental encoder uses the runtime decoder's fixed `standard16` delta
table. This is the KotoAudio experimental SLDPCM4 table and does not imply
compatibility with existing SLDPCM files or containers. For each normalized
PCM16 mono sample it selects the nibble whose
`previous_sample + delta` reconstruction is closest to the reference sample,
packs high nibble first, pads the final low nibble for odd sample counts, and
replays the same saturating reconstruction internally before computing metrics.

SLDPCM4 loop policy is intentionally narrow: no loop, whole-clip loop, and
forward loops with `loop_start == 0` are allowed. Non-zero forward loops fall
back to PCM16 by default because the experimental payload has no stable loop
checkpoint format. `--sldpcm4-fallback reject` rejects those cases instead.
`--sldpcm4-fallback force` can keep experimental output for quality decisions,
but invalid loop shapes are still rejected rather than written.

The converter emits a human-readable report containing source path, source
sample rate, source channels, bit depth, output sample rate, decoded sample
count, target sample rate, original frame count, output sample count, downmix
and resample flags, resampler name, codec, SLDPCM4 table id, experimental flag,
encoded payload bytes, total asset bytes, compression ratio versus PCM16, peak
absolute error, RMS error, SNR dB, low signal SNR note, saturation count, loop
validation result, converter decision, fallback codec, validation result, and
warnings.

The decoder emits a human-readable report containing source path, codec,
experimental flag, KACL sample rate, channel count, decoded sample count,
payload bytes, output WAV bytes, output format, SLDPCM4 table id when present,
and validation result.

## Runtime Alignment

The converter validates generated bytes with `koto_audio::parse_clip_asset`.
Runtime validation rejects invalid magic, unsupported version, unsupported
codec, sample-rate mismatch, non-mono assets, payload-size mismatch, and invalid
loop ranges using the same `ClipAssetError` classification.
