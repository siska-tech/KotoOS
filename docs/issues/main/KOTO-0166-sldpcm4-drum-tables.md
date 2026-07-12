# KOTO-0166: SLDPCM4 built-in drum tables (flash diet)

- Status: done (hardware-verified 2026-07-04: drums indistinguishable from
  PCM16 by ear on device)
- Type: optimization
- Priority: P2
- Related: KOTO-0165, KOTO-0164

## Goal

Cut the dominant flash cost KOTO-0165 introduced: the koto-audio built-in PCM16
drum tables (~524 KiB) were compiled into the firmware whole. Store them as
SLDPCM4 nibble payloads (4-bit fixed-delta DPCM, exactly 4x smaller) and decode
in the drum voice path.

## What changed

### koto-audio (sibling repo, KA-M14)

- New opt-in crate feature **`sldpcm4-drums`** (default stays PCM16, per the
  crate's "PCM16 remains the required path" policy): the PCM16 drum module is
  compiled out and replaced by `builtin_drums_sldpcm4_generated.rs` — per drum a
  `*_SLDPCM4: &[u8]` payload plus `*_SAMPLE_COUNT: u32`.
- The drum voice (`ActiveTone`) decodes incrementally: sample `i` is the
  high/low nibble of payload byte `i / 2`, added (saturating) to the previous
  decoded sample — bit-identical semantics to the experimental `Sldpcm4Decoder`,
  via the shared `SLDPCM4_DELTAS_V0` table. One extra `i16` of decoder state per
  voice; per-sample cost is a table lookup + add (cheaper than the PCM16 path is
  not, but equal-order; no divisions).
- New generator `koto-audio-drum-sldpcm4-table` (koto-audio-tools): parses the
  vendored PCM16 module and re-encodes it, so the SLDPCM4 tables always track
  the drum data actually in the tree (not the WAVs it once came from):

  ```console
  cargo run -p koto-audio-tools --bin koto-audio-drum-sldpcm4-table -- \
    crates/koto-audio/src/builtin_drums_generated.rs \
    crates/koto-audio/src/builtin_drums_sldpcm4_generated.rs
  ```

  Encoding report: 536,918 PCM16 bytes -> 134,230 payload bytes, **zero
  saturating reconstructions** across all nine drums.

### KotoOS

- `koto-pico` and `koto-sim` both enable `sldpcm4-drums` explicitly, so the SIM
  hears the same (lossy) drums the device plays regardless of which packages a
  build invocation includes — parity instead of feature-unification surprises.

## Measured (release, thumbv6m)

| | text | data+bss |
| --- | --- | --- |
| before | 1,116,520 | 188,992 |
| after | 713,916 | 188,992 |

**-393 KiB flash (35 % of the 2 MiB part used)**, RAM unchanged.

## Verification

- koto-audio: 153 tests green in both variants (default and
  `--features sldpcm4-drums`, including a new payload-shape test);
  koto-audio-tools: 67 tests green. Fixed a stale drum test that still
  referenced the removed `PLACEHOLDER_DRUM_CLAP` symbol.
- KotoOS workspace tests green (26 suites), including the SIM KotoBlocks bridge
  non-silence checks with the SLDPCM4 bass-drum thud.
- **Device listening pass: done (2026-07-04).** Drums sound indistinguishable
  from the PCM16 originals by ear, consistent with the zero-saturation encode
  report.

## Notes

- SLDPCM4 drums are quality-lossy by design. If a drum ever sounds unacceptable,
  the escape hatch is dropping the feature from one consumer (that consumer
  reverts to PCM16 at the old flash cost) — no code changes.
