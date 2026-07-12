# KotoAudio Compact Sequence Representation

## 1. Background

M11 and M12 moved KotoAudio from clip-only playback toward a small BGM-capable
runtime:

- M11 added static-slice monophonic sequence playback with note, rest, loop,
  end, tempo, and built-in tone instruments.
- M12 added a fixed-capacity polyphonic sequence foundation, a BGM/SFX bus
  model, P/ECE/MusLib-style built-in instrument ids, and fixed built-in drums.
- PCM16 clip playback remains the golden clip path, and SLDPCM4 remains an
  experimental clip codec path.

The next bridge is not a full parser. Before KotoMML text, `.kmml` assets, or a
Pico CPU1 worker command path are added, KotoAudio needs a compact
runtime-ready sequence representation that is small enough to pass as a bounded
command and direct enough for the mixer to interpret without allocation.

This document defines that representation boundary. It is a design target for
future parser/tool output and generated Rust tables. It does not freeze a stable
numeric hostcall ABI, raw byte format, or production Pico backend.

## 2. Runtime-ready sequence model

The compact sequence is an already-validated, already-budgeted object graph
made of static slices. The runtime consumes it directly; it does not parse MML,
allocate memory, load wavetable files, parse WAV, resample audio, or repair
malformed assets.

Conceptual shape:

```rust
pub struct CompactSequence<'a> {
    pub instruments: &'a [CompactInstrument],
    pub tracks: &'a [CompactTrack<'a>],
    pub tempo: CompactTempo,
}

pub struct CompactTrack<'a> {
    pub events: &'a [CompactEvent],
    pub gain: u8,
    pub initial_instrument_id: u8,
}
```

The exact Rust names may continue to evolve from the current
`Sequence`/`PolyphonicSequence` foundation. The required representation
properties are more important than the names:

- static slice ownership: all tables are caller-owned or generated static data.
- no heap: runtime interpretation never allocates or grows tables.
- bounded tables: instrument count, track count, voice count, and event count
  are validated before playback.
- mixer-ready timing: tempo and tick fields are expressed in small integer
  values usable by the runtime without floating point.
- source-ready role: the same compact sequence can be admitted as SFX-role
  polyphonic sequence playback or BGM-role `play_bgm_sequence` playback.

### Instrument table

The instrument table maps small numeric ids used by events to runtime-ready
instrument descriptors. For v1-style compact assets, the first required mode is
to reference KotoAudio built-ins rather than carrying arbitrary waveform data.

Instrument entries should be limited to:

- built-in tone id reference.
- fixed built-in drum id reference.
- small gain/envelope profile values already supported by the runtime.
- reserved fields written as zero for future compatible expansion.

The table is intentionally not a `.kwt` loader. Future external wavetable import
must still happen in tools/host code and produce a reviewed compact table, not a
runtime file parser.

### Built-in instrument ids

Compact sequence assets should preserve the current M12 built-in ids so parsed
KotoMML and generated Rust tables can target one stable musical vocabulary:

| Id | Instrument |
|---:|---|
| 0 | square fast |
| 1 | saw fast |
| 2 | triangle fast |
| 3 | square |
| 4 | saw |
| 5 | triangle |
| 6 | bass drum |
| 7 | snare drum 2 |
| 8 | snare drum 1 |
| 9 | open hi-hat |
| 10 | closed hi-hat |
| 11 | crash cymbal |
| 13 | synth tom high |
| 14 | synth tom mid |
| 15 | synth tom low |
| 16 | clap |

Ids not listed above are reserved. A compact asset that references an unknown or
reserved id should be rejected by the host/tool parser or by CPU0 validation
before it reaches a CPU1 worker command. Runtime validation may still return a
malformed/unsupported asset error when defensive checks are compiled in.

### Fixed drum ids

Fixed drums are sequence instruments, not arbitrary clip assets. They are
selected by built-in ids, play from static PCM16 mono slices compiled into the
runtime, ignore note pitch, and become silent when the built-in sample slice
ends. Normal note volume, per-track gain, source volume, bus volume, app volume,
and master volume still apply.

The current placeholder drum table sample rate is 16000 Hz. Replacing those
placeholder samples must continue to go through host-side table generation and
license review. Compact sequence playback must not add runtime WAV parsing,
runtime resampling, `.kwt` loading, or heap allocation.

### Track and voice table

A compact sequence contains one or more tracks. Each track is a monophonic event
stream with track-local gain. Polyphony is represented by multiple tracks/voices
rather than by dynamically allocating notes at runtime.

Current M12 interpretation caps active sequence voices at `MAX_SEQUENCE_VOICES`
(currently 4). The compact representation should keep that bound visible so
tools can report and enforce it before runtime:

- zero tracks is malformed.
- more than the configured voice limit is unsupported.
- each track has one event stream and one gain value.
- BGM playback uses the BGM bus and BGM replacement policy.
- SFX-role sequence playback uses the SFX bus and normal bounded source policy.

Per-track gain is a small integer volume field, equivalent in spirit to the
current `MixerVolume` voice-local gain. It is multiplied with note, source, bus,
app, and master gain using the same saturating mono mixdown path as existing
sequence playback.

### Event stream

The minimum compact event vocabulary is:

| Event | Required fields | Runtime meaning |
|---|---|---|
| note | pitch, duration ticks, volume, instrument id | start a tone or fixed drum and advance by duration |
| rest | duration ticks | emit silence and advance by duration |
| loop start | none | mark the beginning of the single supported loop region |
| loop end | repeat count | jump to loop start for finite repeats or forever |
| end | none | mark track completion |

The current M12 foundation supports a single loop depth per track. Compact
assets should preserve that bound:

- nested loops are unsupported.
- loop end without loop start is malformed.
- missing end event is malformed.
- zero-duration note/rest is malformed.
- infinite loop uses the existing infinite-repeat sentinel.

The event stream is not a tracker effect stream. Arpeggio, vibrato, arbitrary
per-tick command effects, runtime macro expansion, and dynamic channel
allocation are outside this compact v1 representation.

### Tempo and tick model

Tempo is stored as a small integer tick model, not as parsed MML notation. Tools
may accept BPM, note length, dotted notes, ties, or textual tempo commands, but
the compact sequence should carry runtime-ready values:

- `tick_rate_hz` or an equivalent integer tick duration accepted by the runtime.
- event durations in integer ticks.
- optional source-level tempo metadata for diagnostics and generated table
  readability.

The runtime should not do floating-point BPM conversion during playback. The
host/tool/parser side converts musical notation into ticks and validates that
durations fit the compact integer fields.

## 3. Parser boundary

KotoMML text and `.kmml` parsing belong on the host/tool side or CPU0 side.
The runtime and a future CPU1 worker should receive only compact sequence data.

Parser/tool responsibilities:

- parse KotoMML text or `.kmml` files.
- expand macros, defaults, note lengths, ties, rests, tempo commands, and
  instrument declarations.
- map requested instruments to KotoAudio built-in ids or approved compact table
  entries.
- split polyphonic music into bounded monophonic tracks.
- validate event count, track count, instrument count, loop structure, duration
  ranges, and expected mixer cost.
- reject malformed input.
- apply an explicit truncate policy only if the tool mode says truncation is
  allowed and reports it clearly.

Runtime/CPU1 responsibilities:

- accept compact sequence references or copied command payloads.
- validate cheap structural invariants when available.
- interpret bounded event streams.
- mix through the existing sequence, BGM/SFX bus, source, and mixer paths.
- report malformed/unsupported assets as visible errors rather than attempting
  repair.

Malformed or oversized assets should normally be rejected by tools or CPU0
before playback. A truncate policy must be a tool/host decision, not an
implicit realtime behavior.

## 4. Pico CPU1 compatibility

The compact representation is designed so it can later be transported as a Pico
CPU1 worker command without changing the musical model:

- no heap allocation on CPU1.
- bounded voices and bounded event streams.
- small numeric ids for instruments and drums.
- static or pre-copied slices with known lengths.
- integer tempo/tick interpretation.
- deterministic completion for finite tracks.
- explicit infinite-loop state for BGM loops.

The command boundary can pass a compact descriptor, a pointer/length pair owned
by CPU0, or a generated static table reference. The exact transport and numeric
ABI are intentionally not fixed by this document.

High-priority control remains separate from ordinary sequence data:

- `stop_bgm` must be able to stop BGM-role playback without parsing or draining
  event streams.
- `stop_all` or system stop must remain a separate control path.
- BGM replacement policy stays in `AudioService`/source admission, not inside
  the sequence event stream.
- queue full, source drop, and bus policy counters remain visible diagnostics.

This keeps CPU1 work bounded: it interprets compact events and mixes audio, but
does not become a general parser, allocator, asset loader, or policy engine.

## 5. Future asset flow

Target flow for future KotoMML and `.kmml` work:

```text
.kmml or KotoMML text
        |
        v
host/tool or CPU0 parser
        |
        v
validated compact sequence
        |
        +--> runtime play_bgm_sequence / play_poly_sequence
        |
        +--> optional generated Rust static table
```

The optional generated Rust path is useful for built-in game jingles and
firmware-owned audio. A tool can emit static event, instrument, and track tables
that compile into the app or runtime, preserving the no-heap representation.

The binary `.kmml` path can be added later as a host/CPU0-loaded asset format,
but it should still resolve to the same compact model before CPU1/runtime
playback. `.kmml` should not force a separate realtime parser.

## 6. Generated Rust table path

Generated Rust tables are the first concrete compact sequence export path. They
are intended for firmware-owned or game-owned audio: short jingles, simple loop
BGM, and reviewed built-in sound tables that should compile directly into the
program. They are not a replacement for a future KotoMML parser or `.kmml`
loader.

The generated source should keep each compact layer visible:

- an instrument table.
- one event table per track.
- a track table that borrows event tables and stores track gain/defaults.
- one `CompactSequence<'static>` that borrows instruments and tracks.

Example line clear jingle table:

```rust
use koto_audio::{
    CompactEvent, CompactInstrument, CompactSequence, CompactTempo, CompactTrack, MixerVolume,
};

// Generated by koto-audio-tools from a validated compact sequence table.
// Runtime representation: borrowed CompactSequence static tables.
// KotoMML/.kmml parsing remains a future host/tool-side step.
pub static LINE_CLEAR_COMPACT_INSTRUMENTS: [CompactInstrument; 2] = [
    CompactInstrument { builtin_id: 3, volume: 240, attack_ticks: 0, release_ticks: 0, decay_ticks: 0 },
    CompactInstrument { builtin_id: 10, volume: 180, attack_ticks: 0, release_ticks: 0, decay_ticks: 0 },
];

pub static LINE_CLEAR_COMPACT_TRACK_0_EVENTS: [CompactEvent; 6] = [
    CompactEvent::LoopStart,
    CompactEvent::Note {
        pitch: 262,
        duration_ticks: 2,
        volume: 210,
        instrument_id: 0,
    },
    CompactEvent::Rest { duration_ticks: 1 },
    CompactEvent::Note {
        pitch: 1,
        duration_ticks: 1,
        volume: 255,
        instrument_id: 1,
    },
    CompactEvent::LoopEnd { repeat_count: 0 },
    CompactEvent::End,
];

pub static LINE_CLEAR_COMPACT_TRACKS: [CompactTrack<'static>; 1] = [
    CompactTrack::new(&LINE_CLEAR_COMPACT_TRACK_0_EVENTS, MixerVolume::new(224), 0),
];

pub static LINE_CLEAR_COMPACT: CompactSequence<'static> = CompactSequence::new(
    &LINE_CLEAR_COMPACT_INSTRUMENTS,
    &LINE_CLEAR_COMPACT_TRACKS,
    CompactTempo { tick_rate_hz: 8, bpm: 120, ticks_per_beat: 4 },
);
```

The tools helper for this path accepts a small in-memory representation and
emits a Rust source fragment. Before emitting, it converts that representation
back through the runtime compact sequence shape and calls
`validate_compact_sequence`. That keeps the tool output aligned with the
runtime consumer boundary without adding parsing, dynamic allocation, WAV
handling, or resampling to runtime playback.

Responsibility split:

- tools validate rich source concerns before generation: parser errors, symbol
  names, event/track budgets, diagnostics, and any future truncation policy.
- runtime validation remains a defensive compact-table check: tempo, track
  count, built-in ids, instrument references, durations, loop structure, and end
  events.
- runtime playback consumes compact tables. It does not parse KotoMML, load
  `.kmml`, load `.kwt`, parse WAV, resample, allocate, or repair malformed
  sequences.

## 7. M13 parser bridge

M13-001 adds the first host-side parser bridge in `koto-audio-tools`. It is an
experimental KotoMML-style subset parser, not full KotoMML compatibility. The
parser accepts a small text surface and produces the same
`CompactSequenceTable` used by the generated Rust formatter:

- `T120` tempo commands become `CompactTempo` with integer tick rate metadata.
- `L4`, `L8`, `L16`, explicit note/rest lengths, and `O4` octave commands
  become integer event durations and MIDI-derived note frequencies.
- Natural notes `c d e f g a b` and rests `r` are supported.
- Sharps may be written with `+` or `#`, such as `c+` or `c#`; flats such as
  `b-` are accepted as a small experimental convenience. Accidentals may cross
  octave boundaries when the resulting MIDI note remains in range.
- Single dotted note/rest lengths such as `c8.` and `r4.` are supported when
  they resolve to integer compact ticks. Durations that would require
  fractional ticks, and double dots such as `c8..`, are rejected.
- Line comments start with `;` or `//` and run to the end of the line. `#TRACK`
  directives are still recognized before commented line tails are discarded.
- Built-in instrument commands `@0` through `@16` preserve the M12 id table,
  with id `12` and unlisted ids rejected.
- Practical drum aliases such as `!bd`, `!sd`, `!s2`, `!hh`, `!oh`, `!cy`,
  `!th`, `!tm`, `!tl`, and `!cl` expand to M12 fixed built-in drum note events.
  Explicit alias lengths such as `!bd8` and `!hh16` are supported; omitted
  lengths use the current default length. These aliases are an experimental
  convenience, not full MusLib macro compatibility.
- `V0` through `V127` becomes note-local compact event volume.
- `#TRACK name` starts a new monophonic track. One to `MAX_SEQUENCE_VOICES`
  tracks are accepted; empty tracks and additional tracks are rejected.
- Each `#TRACK` has independent parser state for tempo, default length, octave,
  current instrument, and volume. The current subset requires every track to
  resolve to the same tempo; mismatched `T` commands are rejected.
- One-level loops like `[cdef]2` are supported; nested loops are rejected.
  Full macro definition syntax remains future work.

Example multiple-track subset input:

```text
#TRACK melody ; lead
T120 L8 O5 @0 c d e g c+
#TRACK bass
T120 L4 O3 @2 c r4. g r
#TRACK drums
T120 L16 !bd !hh !sd !hh // hats can use normal line comments
```

The helper path is:

```text
experimental MML subset text
        |
        v
koto-audio-tools::mml parser
        |
        v
CompactSequenceTable
        |
        +--> validate_compact_sequence
        |
        +--> format_compact_sequence_table / koto-audio-mml-table
```

The parser lives entirely on the host/tool side and may allocate while
building vectors. Runtime remains a `CompactSequence` consumer and does not
gain a parser, `.kmml` loader, `.kwt` loader, runtime WAV parsing, runtime
resampling, heap allocation, stable numeric hostcall ABI, or Pico CPU1 command
transport in this slice.

M13-005 adds practical subset examples under [`examples/mml`](../../examples/mml)
so the parser bridge can be smoke-tested with short game-style content rather
than only unit-test snippets:

- `blocks_like_bgm.mml` is a three-track looping BGM example with melody, bass,
  fixed built-in drums, comments, accidentals, dotted durations, and drum
  aliases.
- `line_clear_jingle.mml` is a short multi-track jingle example.

[`examples/generated/blocks_like_bgm.rs`](../../examples/generated/blocks_like_bgm.rs)
is a checked-in `koto-audio-mml-table` formatter output sample. It demonstrates
the current generated Rust table shape only; it is not a runtime loader, a
`.kmml` asset, a `.kwt` asset, or Pico CPU1 transport data.

## 8. Non-goals

This task does not implement or require:

- a full KotoMML parser.
- `.kmml` loading.
- `.kwt` loading.
- dynamic allocation.
- runtime WAV parsing.
- runtime arbitrary resampling.
- arbitrary PCM clips as sequence drums.
- a stable numeric hostcall ABI.
- a stable raw memory ABI for sequence assets.
- a production Pico backend.
- Pico CPU1 worker migration.
- tracker effects, MOD import, or high-channel-count music playback.

PCM16 clip playback, the experimental SLDPCM4 clip path, monophonic sequence
playback, polyphonic sequence playback, the BGM/SFX bus model, and fixed
built-in drums must keep working independently of this parser bridge design.
