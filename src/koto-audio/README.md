# KotoAudio

KotoAudio v0 is a bounded audio runtime boundary for short mono SFX clips.
The required path is runtime-ready PCM16 clip playback through `AudioService`,
logical source IDs, events, counters, hostcall adapters, and an abstract backend
boundary.

## v0 Scope

Required in v0:

- PCM16 little-endian mono clip playback.
- KACL runtime-ready clip assets parsed by the runtime.
- `AudioService` source admission, mixing, backend submit, completion events,
  source release, and public counter snapshots.
- Experimental M11 simple monophonic sequence playback for short BGM, jingles,
  and melodies using static slices, built-in instruments, and minimal
  attack/release fades for note-boundary click reduction.
- Experimental M12 fixed-capacity polyphonic sequence foundation for up to four
  static BGM voices, with P/ECE-style built-in tone/drum ids, per-voice gain,
  and saturating mono mixdown.
- A minimal host-owned BGM/SFX bus model with app-facing high-level playback,
  bus volumes, BGM replacement, and BGM-only stop control.
- Normal/system/debug hostcall adapter separation, without a stable numeric
  hostcall ABI.
- Host-side asset conversion and decode tools.

Experimental or explicitly outside v0:

- SLDPCM4 is an `experimental-sldpcm4` feature-gated codec experiment. PCM16
  remains the compatibility and fallback path.
- `picocalc-backend` is an experiment boundary, not a production backend
  commitment.
- Full MOD/tracker-style BGM, multi-channel music, effects, streams, a stable
  numeric hostcall ABI, runtime WAV parsing, runtime downmixing, and runtime
  resampling are non-goals for v0.

## M11 Simple Sequence

M11 adds a minimal experimental sequence source for KotoBlocks and launcher
jingles. A sequence is a no-heap static event slice with note, rest, single
loop, and end events. It is monophonic, uses a fixed tick rate, and generates
PCM16-equivalent mixer samples from built-in square, saw, or triangle tone
instruments.
Instruments can apply lightweight linear attack/release fades and volume
scaling to make short BGM and jingles less clicky without adding heap use or a
full ADSR engine.

Helpers are available for short game-side melodies without changing the
experimental event layout:

```rust
use koto_audio::{
    Sequence, SequenceEvent, SequenceInstrument, SequencePitch, SequenceTempo,
};

const TEMPO: SequenceTempo = SequenceTempo::from_bpm(120, 4);
const INSTRUMENTS: [SequenceInstrument; 1] = [SequenceInstrument::square_lead()];
const EVENTS: [SequenceEvent; 5] = [
    SequenceEvent::note(SequencePitch::C4, TEMPO.eighth(), 200, 0),
    SequenceEvent::note(SequencePitch::E4, TEMPO.eighth(), 200, 0),
    SequenceEvent::note(SequencePitch::G4, TEMPO.eighth(), 200, 0),
    SequenceEvent::rest(TEMPO.eighth()),
    SequenceEvent::End,
];
const JINGLE: Sequence<'static> = Sequence::with_tempo(&EVENTS, &INSTRUMENTS, TEMPO);
```

Practical blocks-like examples for move/rotate/drop sounds, line-clear and
game-over jingles, and a stop-driven monophonic loop live in
[`crates/koto-audio/examples/sequence_game_sounds.rs`](crates/koto-audio/examples/sequence_game_sounds.rs).

This is not an Amiga MOD player, tracker engine, stable sequence asset ABI, or
production BGM system. MOD/tracker import, multi-channel music, effects such as
arpeggio or vibrato, full ADSR, richer instruments, bus policy, and stable
hostcall numbering remain future work.

## M12 Polyphonic Sequence Foundation

M12 adds an experimental no-heap polyphonic sequence foundation for BGM-like
playback. `PolyphonicSequence` borrows a static slice of
`PolyphonicSequenceVoice` values and caps interpretation at
`MAX_SEQUENCE_VOICES` (currently 4). Each voice reuses the existing
`Sequence`/`SequenceEvent`/`SequenceInstrument` model, applies voice-local gain,
and mixes into the same mono PCM16-equivalent block path with saturation.

M12-003 also adds a P/ECE/MusLib-style built-in instrument model for sequence
content. `BUILTIN_SEQUENCE_INSTRUMENTS` maps the numeric ids used by
`SequenceEvent::instrument_id`:

- `0`: square fast
- `1`: saw fast
- `2`: triangle fast
- `3`: square
- `4`: saw
- `5`: triangle
- `6`: bass drum
- `7`: snare drum 2
- `8`: snare drum 1
- `9`: open hi-hat
- `10`: closed hi-hat
- `11`: crash cymbal
- `13`: synth tom high
- `14`: synth tom mid
- `15`: synth tom low
- `16`: clap

The fast tone ids are distinct now, but initially share the same simple
oscillator implementation as their non-fast variants. They reserve room for a
future quality/performance profile without changing sequence ids. Id `12` and
all unlisted ids are reserved.

Drums are fixed built-in PCM16 mono sequence instruments, not arbitrary PCM
clips. The runtime representation is a static `&'static [i16]` sample slice at
the current placeholder table rate of 16000 Hz. Their note pitch is ignored,
note-on starts the built-in sample from the beginning, the sample tail becomes
silent after its static placeholder data ends, and normal note volume, source
volume, bus gain, and app/master gain still apply. The current drum data is
intentionally small placeholder static sample data; MusLib/P/ECE-derived drum
data is not copied into this repository without license verification.

`AudioService::play_poly_sequence` enqueues a polyphonic sequence as an SFX-role
logical source for game sounds. `AudioService::play_bgm_sequence` enqueues the
same sequence type on the BGM bus: `stop` stops that source by ID, `stop_bgm`
stops only BGM sources, finite voices complete when every voice reaches `End`,
and any infinite-loop voice keeps the source active until stopped.

The runtime owns the BGM/SFX model in a P/ECE-style boundary: apps call
high-level APIs such as `play_clip`, `play_sequence`, `play_bgm_sequence`,
`set_bgm_volume`, and `set_sfx_volume`, while source admission, bus gain, and
replacement policy stay inside the host-owned audio service. PCM16 clips and
normal sequence playback default to the SFX bus. Starting a new BGM sequence
replaces existing BGM instead of letting ordinary SFX bursts steal it; if the
bounded source queue is full, normal queue/drop policy still applies. The
public counter snapshot keeps this model visible with
`active_bgm_source_count`, `active_sfx_source_count`, `bgm_start_count`,
`bgm_stop_count`, and `bgm_replaced_count`.

This is still only a foundation. KotoMML parsing, `.kmml` loading, `.kwt`
wavetable loading, compatibility with the current KotoOS/Pico worker
KotoMML/`.kwt` path, stable hostcall numbering, and the Pico CPU1 worker port
remain future work. The compact runtime-ready representation that should sit
between those future parsers/assets and the runtime is sketched in
[`docs/design/KOTO_AUDIO_COMPACT_SEQUENCE.md`](docs/design/KOTO_AUDIO_COMPACT_SEQUENCE.md).
M12-006 adds the first Rust skeleton for that boundary:
`CompactSequence`/`CompactTrack`/`CompactInstrument`/`CompactEvent` use
caller-owned static slices and validation before playback. This is a
runtime-ready representation produced by parser/tool code; full KotoMML
compatibility, `.kmml` loading, binary compact assets, and stable hostcall
numeric ABI remain future work.

M13-001 adds an experimental host-side KotoMML-style subset parser in
`koto-audio-tools::mml`, plus `koto-audio-mml-table <input.mml> <output.rs>`
for generated Rust compact sequence tables. The subset currently covers tempo,
default length, octave, natural notes/rests, sharps such as `c+` and `c#`,
single dotted note/rest lengths such as `c8.` and `r4.`, line comments using
`;` or `//`, built-in instrument ids, volume, one-level loops, `#TRACK name`
separators for up to `MAX_SEQUENCE_VOICES` independent monophonic tracks, and
practical fixed-drum aliases such as `!bd`, `!sd`, `!hh16`, `!oh`, `!cy`,
`!th`, `!tm`, `!tl`, and `!cl`. Flats such as `b-` are accepted as a small
musical convenience. Dotted durations must resolve to integer compact ticks;
double dots are rejected. All tracks must currently use the same tempo. These
aliases are an experimental convenience for M12 built-in drum ids, not full
MusLib macro compatibility. Full macro syntax, including macro definitions,
remains future work. It is not a runtime parser and is not full KotoMML or
`.kmml` compatibility.

Practical M13 subset examples live in [`examples/mml`](examples/mml):

- [`examples/mml/blocks_like_bgm.mml`](examples/mml/blocks_like_bgm.mml) is a
  short looping three-track blocks-style BGM with melody, bass, built-in drums,
  comments, accidentals, dotted durations, built-in instruments, and drum
  aliases.
- [`examples/mml/line_clear_jingle.mml`](examples/mml/line_clear_jingle.mml) is
  a short multi-track line-clear jingle.

The checked-in generated Rust table fragment in
[`examples/generated/blocks_like_bgm.rs`](examples/generated/blocks_like_bgm.rs)
is produced with:

```console
cargo run -p koto-audio-tools --bin koto-audio-mml-table -- \
  --symbol BLOCKS_LIKE_BGM_COMPACT \
  --prefix BLOCKS_LIKE_BGM_COMPACT \
  examples/mml/blocks_like_bgm.mml \
  examples/generated/blocks_like_bgm.rs
```

The same experimental MML subset can be rendered on the host for listening
checks with `koto-audio-mml-render`:

```console
cargo run -p koto-audio-tools --bin koto-audio-mml-render -- \
  --bgm --seconds 8 --sample-rate 22050 \
  examples/mml/blocks_like_bgm.mml \
  target/blocks_like_bgm.wav

cargo run -p koto-audio-tools --bin koto-audio-mml-render -- \
  examples/mml/line_clear_jingle.mml \
  target/line_clear_jingle.wav
```

`koto-audio-mml-render` parses MML in `koto-audio-tools`, builds and validates
a `CompactSequenceTable`/`CompactSequence`, then renders through the existing
host-side `AudioService` sequence path into PCM16 mono WAV. It is a listening
tool for `examples/mml` and generated compact sequence workflow checks, not a
runtime WAV writer/parser. `.kmml` loading, `.kwt` loading, Pico CPU1
transport, production Pico backend work, and stable hostcall ABI remain future
work.

Example multiple-track subset input:

```text
#TRACK melody
T120 L8 O5 @0 c d e g c+
#TRACK bass
T120 L4 O3 @2 c r4. g r
#TRACK drums
T120 L16 !bd !hh !sd !hh // comments are ignored
```

## Runtime And Tools Boundary

The runtime consumes assets that are already mixer-ready. It validates KACL
headers, channel count, sample rate, codec support, payload size, and loop
metadata, then decodes supported clip payloads to PCM16 mono samples for the
mixer.

Arbitrary PCM effects stay on the KACL clip path: PCM16 is the required clip
codec, and the experimental SLDPCM4 path remains feature-gated. Fixed sequence
drums are separate built-in instruments selected by sequence instrument id; they
are not loaded as KACL PCM16/SLDPCM4 clips and do not add runtime WAV parsing,
runtime resampling, heap allocation, `.kmml` loading, or `.kwt` loading.

The runtime does not parse WAV files, downmix channels, resample sample rates,
or repair arbitrary asset data. Those jobs belong to `koto-audio-tools`:

- `koto-audio-convert` reads PCM16 WAV, optionally downmixes and resamples on
  the host, and writes KACL.
- `koto-audio-decode` reads KACL and writes PCM16 mono WAV for reference and
  listening checks.
- `koto-audio-drum-table` reads license-verified PCM16 WAV on the host and
  writes a Rust `pub static NAME: &[i16]` fragment for future fixed built-in
  sequence drum table replacement.
- `koto-audio-mml-table` reads the experimental M13 MML subset on the host and
  writes validated `CompactSequence` Rust table fragments.
- `koto-audio-mml-render` reads the same experimental M13 MML subset on the
  host and writes PCM16 mono WAV for listening checks only; it does not add WAV
  writing or MML parsing to the runtime.

## Codec Policy

PCM16 is the required v0 path. Converter output defaults to PCM16, runtime
validation accepts PCM16 without optional features, and minimal app tests should
exercise PCM16 playback first.

SLDPCM4 is experimental. It uses KotoAudio's own fixed table and does not claim
compatibility with existing SLDPCM files, containers, or tables. Runtime and
tools SLDPCM4 decode support is enabled only with `experimental-sldpcm4`; builds
without that feature report SLDPCM4 assets as unsupported. Host conversion can
request experimental SLDPCM4 explicitly, but PCM16 fallback is the expected
operational policy until the experiment is promoted.

## Sample Rate Tiers

The current sample-rate tiers are provisional:

- 16000 Hz: low-cost default candidate.
- 22050 Hz: quality SFX candidate.
- 44100 Hz: host-side/reference comparison.

The runtime accepts the configured mixer rate. Cross-rate conversion is a
host-tool responsibility.
