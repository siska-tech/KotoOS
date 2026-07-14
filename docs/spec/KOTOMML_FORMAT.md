# KotoMML format

KotoMML (`.kmml`) is the text source format for native KotoAudio sequences.
Files need no format or engine declaration.

Application sources live at `apps/<app>/audio/*.kmml`. The normal app build copies
them unchanged to the SD-card tree. `play_bgm_asset` and `play_sfx_asset` select
the bus/role; the filename itself has no engine-selection semantics.

```text
# Native KotoAudio score
@3 T120 V85 O4 L4
C C G G A A G2
F F E E D D C2
```

## Text and commands

- Input is UTF-8; commands are ASCII and case-insensitive.
- Blank lines and lines beginning with `#` are comments, except `#TRACK`.
- Unknown commands, invalid lengths, and invalid instrument IDs are errors.
- Whitespace is insignificant between commands.

| Command | Form | Meaning |
| --- | --- | --- |
| Note | `C D E F G A B`, optional `#`, `+`, or `-` | Pitched note in octave 0–8 |
| Rest | `R` | Silence |
| Tempo | `T32`–`T255` | Quarter-note BPM |
| Volume | `V0`–`V127` | Native KotoAudio note volume |
| Octave | `O0`–`O8`, `<`, `>` | Set or shift octave |
| Length | `L1`, `L2`, `L4`, `L8`, `L16` | Default note/rest length |
| Instrument | `@n` | Native KotoAudio builtin instrument ID |
| Loop | `[ ... ]`, `[ ... ]0` | Loop region; `0` means infinite |
| Track | `#TRACK <name>` | Start another simultaneous BGM track |

Notes and rests may carry a length suffix (`C8`, `F#16`, `R2`) and may be dotted
(`C4.`). Keep every BGM track's loop duration equal so parts remain synchronized.
Both BGM and SFX may be polyphonic within the runtime's bounded voice count.

## Instruments and drums

`@n` refers directly to the builtin instrument table in `koto-audio`; IDs are not
remapped by app name or target. The parser validates the ID against that table.

Drum aliases select native KotoAudio drum instruments and can be mixed with normal
notes:

| Alias | Instrument |
| --- | --- |
| `!bd` | bass drum |
| `!sd` | snare drum |
| `!hh` | closed hi-hat |
| `!oh` | open hi-hat |

Example:

```text
#TRACK melody
@3 T150 V100 O5 L8 [ E G B G ]
#TRACK drums
T150 V90 L8 [ !bd !hh !sd !hh ]
```

## Build and runtime model

Run the normal app build after editing KMML:

```powershell
python harness/build_apps.py
```

The build copies KMML to `sdcard_mock/audio`. At runtime SIM reads that mounted
file directly. Pico reads it from SD, compiles it to a pointer-free KotoAudio cue
image, stores the image in PSRAM, and reuses the PSRAM copy for later plays in the
same app session. `python harness/build_apps.py --check` checks that the SD copy
matches its authored source. The KotoAudio tests also compile every shipped KMML
against the device event limits.

## Audition and bake

`koto-mml` uses the same native KotoAudio conversion and 16 kHz service path:

```powershell
cargo run -p koto-mml -- wav apps/kotosnake/audio/bgm.kmml bgm.wav --loop --seconds 8
cargo run -p koto-mml --features play -- play apps/kotosnake/audio/bgm.kmml --loop
cargo run -p koto-mml -- bake jingle.kmml jingle.kacl --clip-loop whole
```

`--mute N` omits a zero-based track while auditioning. `--loop` applies the BGM
looping policy and `--seconds` bounds playback. Audition, SIM, and Pico all use
native KotoAudio.
