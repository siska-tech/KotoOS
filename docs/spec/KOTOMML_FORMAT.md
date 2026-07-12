# KotoMML Format And Playback Model

This document defines the first KotoMML subset. It is intentionally small:
enough to represent simple BGM and effects, while leaving room for a compiled
or richer text format later.

## Asset Shape

KotoMML assets are UTF-8 text files with the suggested extension `.kmml`.
They are stored as KPA `audio` assets and should be marked `sequential` unless
the packer later emits a separate index.

The first fixture is [twinkle.kmml](../../harness/fixtures/kotomml/twinkle.kmml).

```text
# KotoMML v0 fixture
T120 V10 O4 L4
C C G G A A G2
F F E E D D C2
R4 C8 D8 E8 F8 G2
```

## Text Rules

- Lines are UTF-8 text and may end with LF or CRLF.
- Empty lines and lines starting with `#` are ignored.
- Commands are ASCII and case-insensitive.
- Whitespace separates commands but is otherwise insignificant.
- A parser should reject unknown commands, invalid note lengths, and octave
  underflow or overflow.

## Supported Commands

| Command | Form | Range | Meaning |
| :------ | :--- | :---- | :------ |
| Note | `C D E F G A B` plus optional `#`, `+`, or `-` | Octave 0-8 | Queue a pitched tone. `#` and `+` sharpen; `-` flattens. |
| Rest | `R` | Uses length rules | Queue silence for the current track. |
| Tempo | `Tnnn` | 32-255 BPM | Set quarter-note tempo for following events. |
| Volume | `Vnn` | 0-15 | Set track amplitude for following notes. `0` is muted. |
| Octave | `On` | 0-8 | Set current octave. |
| Length | `Ln` | 1, 2, 4, 8, 16, 32 | Set default note/rest length. |
| Octave up | `>` | Stops at 8 | Increase current octave by one. |
| Octave down | `<` | Stops at 0 | Decrease current octave by one. |
| Instrument | `@n` | 0-5 built-in, 6-31 custom | Select the synth voice for following notes. Ids 0-5 are the host bank; 6-31 are package-defined wavetables (see below). Host extension. |
| Loop start | `[` | — | Mark the start of the looping body. Events before it are a one-shot intro. |
| Loop end | `]` | — | Mark the end of the looping body; a looping voice jumps back to `[` here. |
| Custom instrument | `#INST <id> <path>` | id 6-31 | Line directive: bind custom `@id` to a package KotoWaveTable (`.kwt`) asset. See below. |

### Instrument Selection (host extension)

`@n` selects a host synth voice for the notes that follow it (the default before any
`@` is `@0`). This is a host extension answering the "waveform and duty-cycle"
open question below; the id indexes the host's instrument bank rather than encoding a
waveform directly, so the same track can sound different on different hosts. The
KotoSim bank (`src/koto-sim/src/audio.rs`) is: `0` square lead, `1` thin pulse,
`2` soft triangle, `3` saw, `4` noise/percussion, `5` short pluck — each with its own
ADSR envelope. Unknown ids fall back to `@0` during playback; strict parsing (used by
tests and asset tooling) rejects them so typos surface early.

### Custom Wavetable Instruments (host extension)

Beyond the fixed `@0`..`@5` host bank, a package may define its own timbres as
**wavetable instruments**. The instrument *data lives in the KPA package* (not the
host): a score names a `.kwt` asset with a `#INST` directive and selects it with `@n`.
This keeps the host bank small while letting each app ship a signature sound.

A `#INST <id> <path>` line (case-insensitive keyword, like `#TRACK`) binds a custom id
to a KotoWaveTable asset:

```text
#INST 6 audio/kotosnake_lead.kwt
#INST 7 audio/kotosnake_bass.kwt
#TRACK lead
@6 T150 V12 O5 [ E G B G ]
#TRACK bass
@7 T150 V8 O3 [ C C G G ]
```

- The id must be in **6..=31** (below 6 is the built-in bank; ids never shadow it).
- A score may reference at most **8** custom instruments.
- The host loads each referenced `.kwt` asset from the package (it must be declared in
  the manifest `assets` and live under `audio/`) before playback and registers it in
  the score's instrument bank. The `#INST` lines are otherwise treated as comments by
  the note parser.
- `@n` for an undefined custom id falls back to `@0` during lenient playback; strict
  parsing (tests / asset tooling) rejects it, as it does an unknown built-in id.

#### KotoWaveTable (`.kwt`) Asset Format

A `.kwt` file is small, line-oriented UTF-8 text. Blank lines and `#` comments are
ignored; keywords are case-insensitive.

```text
KWT1
WAVE 0 49 90 117 127 117 90 49 0 -49 -90 -117 -127 -117 -90 -49
ENV 6 80 70 120
GAIN 110
```

| Line | Form | Meaning |
| :--- | :--- | :------ |
| Magic | `KWT1` | Required first directive; identifies the format/version. |
| Wave | `WAVE <s0> <s1> …` | One single-cycle period: **2-64** signed integers in **-100..100**, sampled by phase with linear interpolation. Values are normalised by 100 and clamped to `[-1, 1]`. |
| Envelope | `ENV <attack_ms> <decay_ms> <sustain_pct> <release_ms>` | Optional ADSR. `sustain_pct` is 0-100. Defaults to a soft lead (`2 40 70 30`). |
| Gain | `GAIN <pct>` | Optional output gain percent (100 = unity, max 400). Defaults to 100. |

`WAVE` is required; `ENV`/`GAIN` are optional. A malformed asset (missing magic, a
wave outside the length/range bounds, a non-integer, or the wrong `ENV` arity) is
rejected so authoring mistakes surface at load time.

The KotoSim implementation lives in `src/koto-sim/src/audio.rs` (`InstrumentBank`,
`parse_kwt`, `scan_instrument_refs`); the host loads the assets in
`play_bgm_asset` (`src/koto-sim/src/runtime/host.rs`).

### Loop Markers (host extension)

`[` and `]` bracket the looping body of a track. A looping voice (BGM) plays any
intro before `[` once, then repeats `[`…`]` forever; with no markers the whole track
is the loop body. One-shot voices (SFX) ignore the markers and play once. A `]`
without a matching `[` is rejected.

### Mix Balance

The host mixes BGM below unity and effects at unity (KotoSim defaults: BGM `0.55`,
SFX `1.0`) so short cues such as a line clear or lock stay audible over the music;
the balance is adjustable on the host engine.

Notes and rests may carry an explicit length suffix, such as `C8`, `F#16`, or
`R2`. When absent, the current `L` value is used. A dotted suffix is optional
for notes and rests (`C4.`) and multiplies the duration by 3/2.

## Event Model

Parsing produces a per-track sequence of events. Each event is absolute in the
sense that it has already captured the tempo, volume, octave, and duration that
were active at parse time.

| Field | Type | Notes |
| :---- | :--- | :---- |
| `kind` | `note` or `rest` | Notes create voices; rests only advance the track cursor. |
| `start_tick` | integer | Ticks are local to the track. |
| `duration_tick` | integer | Initial resolution: 96 ticks per quarter note. |
| `tempo_bpm` | integer | Used to convert ticks to samples. |
| `volume` | 0-15 | Later mapped to mixer gain. |
| `midi_note` | integer for notes | Middle C is C4 = MIDI 60. |

### Multi-Track Scores (host extension)

A single score may hold several simultaneous voices (lead + bass + drums),
separated by `#TRACK <name>` marker lines (case-insensitive; an ordinary
`# comment` is not a marker). Each section is parsed as an independent track with
its own tempo, octave, instrument, length, and loop, and they are mixed together at
playback. A score with no `#TRACK` marker is a single track, so this is a strict
superset of the single-voice syntax. KotoSim mixes up to four BGM voices.

```text
#TRACK lead
@0 T150 V8 O5 L8 [ E G B G E G B G ]
#TRACK bass
@2 T150 V6 O3 L4 [ C C G G ]
#TRACK drum
@4 T150 V6 O2 L8 [ C R C R C C R C ]
```

Give every track the same loop length (in beats) so they stay locked together. A
package may still split voices across separate `.kmml` assets instead; both models
feed the same mixer.

## Duration Conversion

Use 96 ticks per quarter note for parsed event durations:

```text
quarter_note_samples = sample_rate * 60 / tempo_bpm
event_samples = quarter_note_samples * duration_tick / 96
```

For example, at 22,050 Hz and `T120`, one quarter note lasts 11,025 samples.
`L8` lasts 5,512.5 samples; the renderer should carry a fractional accumulator
per track so repeated short notes do not drift.

## Pitch Conversion

The first synthesizer maps a note to a monophonic oscillator voice:

```text
frequency_hz = 440.0 * 2 ^ ((midi_note - 69) / 12)
phase_step = frequency_hz / sample_rate
```

The initial waveform is a low-cost square or triangle oscillator selected by
the host engine. The text format does not expose waveform selection yet.

## Mixer Voice Conversion

At playback time, each active track owns one melodic oscillator. When a parsed
note starts, the engine configures or restarts that track oscillator with:

| Voice Field | Source |
| :---------- | :----- |
| Frequency | Parsed `midi_note` converted to Hz. |
| Gain | `volume / 15`, scaled below clipping headroom. |
| Duration | Parsed duration converted to samples. |
| Envelope | Short fixed attack/release, initially 2-4 ms each. |
| Pan | Center; PicoCalc PWM output is treated as mono-mixed for now. |

The oscillator writes PCM into the existing software mixer path. BGM tracks and
sound effects are both mixer inputs; effects get separate higher-priority voices
so a short effect can play over BGM without rewriting the MML track state.

Recommended initial limits:

| Item | Limit | Reason |
| :--- | :---- | :----- |
| BGM tracks | 2-4 active melodic voices | Fits the RP2040 CPU budget until measured. |
| SFX voices | 2 active voices | Allows confirm/cancel effects over music. |
| Sample rate | 22,050 Hz first target | Reduces oscillator and mixer work. |
| Output | `i16` mono PCM | Matches `PcmMixer` and HAL audio buffers. |

The engine should clamp or scale gains before handing samples to `PcmMixer` so
combined BGM and SFX do not spend most of their time in final `i16` saturation.

## VM And Host Boundary

KotoMML parsing and oscillator rendering should live in host engine code. The VM
or app script should only request high-level actions such as `play_bgm_asset(path, len)`,
`stop_bgm()`, and `play_sfx(asset)`. Keeping synthesis host-side avoids a large
VM heap and lets the audio callback render bounded chunks directly into mixer
buffers.

## Open Questions

- Whether the packer should compile `.kmml` text into compact event tables.
- Whether to add named instrument aliases (e.g. `@lead`) over the numeric bank.
- Whether the packer should enforce stricter per-package audio asset namespaces.

Resolved in the first KotoSim implementation: loop points (`[` `]`), waveform/duty
selection via the `@n` instrument bank, and the BGM/SFX mix balance (KOTO-0095);
simultaneous multi-track scores via `#TRACK` markers (KOTO-0098).
Host ABI minor 10 also adds `play_bgm_asset(path, len)`, allowing app-specific
`.kmml` files to ship with the package instead of being compiled into the host.
Package-defined wavetable instruments (`#INST` + `.kwt` assets) let an app carry its
own timbres as package data rather than relying solely on the fixed host bank.
