# Primary audio asset / cue model (KOTO-0164)

How a KotoOS app plays sound on the **primary** audio path: the KotoAudio bounded
runtime driving *generated compact sequences*, *authored SFX cues*, and *KACL
clips*. This is the model new apps target. The runtime `.kmml`/`.kwt`/MML/tone paths
are **legacy** and deprecated — see
[AUDIO_DEPRECATION_POLICY.md](AUDIO_DEPRECATION_POLICY.md). KotoBlocks is the first
and (today) only app wired to the primary path; its integration details are in
[KOTO_KOTOBLOCKS_AUDIO_BRIDGE.md](../devlog/KOTO_KOTOBLOCKS_AUDIO_BRIDGE.md).

This document is the source of truth for the **asset → cue routing model**: what an
audio asset path means, how the host resolves it to a primary cue, and where the
route table and constants live in code.

## The model in one paragraph

A primary-path app declares audio assets in its manifest and triggers them with the
ordinary `play_bgm_asset` / `play_sfx_asset` / `stop_bgm` hostcalls. The host does
**not** read or parse the asset payload for a primary-path asset. Instead it treats
the asset path as a **routing key**: it looks the path up in a small per-app route
table that maps the key to a **primary cue** (start the generated BGM loop, or play
a specific authored/embedded SFX). The audio itself is generated compact-sequence /
authored Rust tables compiled into KotoSim and the firmware, played through the
bounded `koto-audio` runtime — on the SIM via the KotoBlocks bridge, on Pico via the
ported runtime on the CPU1 worker (KOTO-0165).

Since KOTO-0165 the two sides differ in **coverage**: on **Pico** the route tables
cover *every shipped app* (the `tools/koto-audio-gen` step compiles each app's
`apps/*/audio/*.kmml` into vendored sequence statics, and the legacy fallback chain
no longer exists), while on **SIM** only KotoBlocks is routed and other apps still
take the deprecated legacy MML path.

## Asset path is a routing key, not a format

- The `.kmml` extension on the KotoBlocks asset paths is **legacy naming**. For a
  routed path the extension is meaningless: the file's *contents* are never read.
- The path still has to be a **declared package asset** so the manifest permission
  check (`asset_paths` on SIM, the SD path check on Pico) passes. A physical file may
  exist, but for a routed path it is a placeholder — the sound is the compiled table.
- New apps should treat the audio asset path purely as a stable identifier for a
  route-table entry. Do **not** ship parseable MML/`.kwt` for a primary-path cue and
  do **not** rely on the runtime parsing it — there is no runtime MML/`.kmml`/`.kwt`
  parser on the primary path, by design (non-goal, KOTO-0162/0164).

## What a new app uses

| Cue kind         | Source of the audio                                    | Runtime parse? |
| ---------------- | ------------------------------------------------------ | -------------- |
| BGM (looping)    | generated compact sequence table (offline `koto-audio-tools`), vendored as Rust | no |
| SFX (one-shot)   | hand-authored `koto_audio::Sequence` / embedded `PicoBgmScore` table | no |
| Clip             | KACL PCM16 clip via the `koto-audio` clip path         | no |
| built-in cue     | `koto-audio` built-in instruments / fixed PCM drums     | no |

Legacy-only (deprecated, do not target): runtime `.kmml` subset parse + synth,
`.kwt` wavetable loader, tone-fallback "music".

## Route table (code ↔ doc correspondence)

The route table maps each asset routing key to a primary cue. SIM and Pico keep
**identical path constants** and an **identically named** `primary_audio_route`
lookup so both route the same keys; the cue enums differ only in spelling (SIM is
descriptive, Pico is terse). Keep this table in sync with the two code tables.

| Path constant (`KOTO_BLOCKS_*_ASSET`) | Asset path (routing key)        | Hostcall          | SIM cue (`PrimaryCue` / `SeqSfx`) | Pico cue (`PicoPrimaryCue` / `KotoBlocksSfx`) | Meaning                    |
| ------------------------------------- | ------------------------------- | ----------------- | --------------------------------- | --------------------------------------------- | -------------------------- |
| `BGM`                                 | `audio/koto_blocks_bgm.kmml`    | `play_bgm_asset`  | `Bgm`                             | `Bgm`                                         | start looping generated BGM |
| `MOVE`                                | `audio/koto_blocks_move.kmml`   | `play_sfx_asset`  | `Sfx(Move)`                       | `Sfx(Move)`                                   | short high blip            |
| `ROTATE`                              | `audio/koto_blocks_rotate.kmml` | `play_sfx_asset`  | `Sfx(Rotate)`                     | `Sfx(Rotate)`                                 | two-note rise              |
| `LOCK`                                | `audio/koto_blocks_lock.kmml`   | `play_sfx_asset`  | `Sfx(HardDrop)`                   | `Sfx(Lock)`                                   | low landing thud           |
| `CLEAR`                               | `audio/koto_blocks_clear.kmml`  | `play_sfx_asset`  | `Sfx(LineClear)`                  | `Sfx(Clear)`                                  | ascending arpeggio         |
| `TETRIS`                              | `audio/koto_blocks_tetris.kmml` | `play_sfx_asset`  | `Sfx(Tetris)`                     | `Sfx(Tetris)`                                 | four-line fanfare          |
| `OVER`                                | `audio/koto_blocks_over.kmml`   | `play_sfx_asset`  | `Sfx(GameOver)`                   | `Sfx(Over)`                                   | descending phrase          |
| (global — not path-routed)            | n/a                             | `stop_bgm`        | stop BGM bus only                 | high-priority `StopBgm`                       | stop BGM; SFX keep playing |

Where the code lives:

- **SIM:** `src/koto-sim/src/koto_blocks_audio.rs` — `KOTO_BLOCKS_*_ASSET` constants,
  `PrimaryCue`, `KOTO_BLOCKS_ROUTES`, and `primary_audio_route`. The dispatch is
  `SimRuntimeHost::play_bgm_asset` / `play_sfx_asset` in
  `src/koto-sim/src/runtime/host.rs`, which calls `primary_audio_route` and no longer
  holds any `.kmml` magic strings of its own.
- **Pico (KOTO-0165):** `src/koto-pico/src/firmware/audio_cues.rs` —
  `PicoPrimaryCue` now carries the sequence itself
  (`Bgm(&PolyphonicSequence)` / `Sfx(&Sequence)`), and `primary_audio_route`
  consults the hand-authored KotoBlocks SFX table plus the generated
  `GENERATED_BGM_ROUTES` / `GENERATED_SFX_ROUTES` arrays in
  `audio_cues_generated.rs` (produced by `tools/koto-audio-gen`, covering every
  shipped app). The dispatch is `play_bgm_asset` / `play_sfx_asset` in
  `src/koto-pico/src/firmware/app_host.rs`.

## Extending to a second app (future)

On **Pico** this is now solved by generation (KOTO-0165): drop `audio/*.kmml`
sources into the new app's directory and regenerate — `tools/koto-audio-gen` scans
`apps/*/audio/*.kmml` and emits the route rows and sequence statics automatically.

On **SIM** the route table is still a set of KotoBlocks `const`s plus one lookup
function, not a per-app registry — the shape (`&[(path_key, PrimaryCue)]`) is
deliberately what one entry of a future per-app registry would hold. To add a second
primary-path app there without a magic-string sprawl:

1. Add its `<APP>_*_ASSET` path constants and cue rows to the same module.
2. Extend `primary_audio_route` (or introduce a per-app table keyed by the launching
   manifest) to consult the new app's rows.
3. Add the new rows to the table above so the doc and code stay in correspondence.

This is intentionally **not** a generic dynamic audio loader (non-goal): routes are
compiled-in tables, resolved by exact path match, with no runtime allocation or
parsing.

## Diagnostics

The audio per-call trace (`phase=172`, DIAG-0001 §3.2) records a `result=` label so a
capture shows which path served each hostcall:

| `result=` label | Path                                   | Status                    |
| --------------- | -------------------------------------- | ------------------------- |
| `seq-bgm`       | primary — routed generated BGM         | **primary**               |
| `seq-sfx`       | primary — routed authored/embedded SFX | **primary**               |
| `unrouted`      | Pico routing miss (plays nothing)      | defect signal (KOTO-0165) |
| `legacy-mml`    | `.kmml` parse + synth                  | SIM-only deprecated fallback |
| `legacy-pcm`    | raw `.kmml`-as-PCM asset               | removed on Pico (KOTO-0165) |
| `legacy-tone`   | tone SFX / `play_sfx`                  | removed on Pico (KOTO-0165) |

A **routing miss** — a path that should have matched a route but does not (a rename,
a typo, or a manifest/route-table drift) — returns `None` from `primary_audio_route`.
On **SIM** it falls through to the **legacy** MML path (the tell is a `legacy-*`
label where `seq-*` was expected). On **Pico** there is no fallback since KOTO-0165:
the call logs `result=unrouted`, bumps `unsupported_count`, returns `UNSUPPORTED`,
and plays nothing. Either way it is safe (bounded, non-panicking) but is a defect
for a primary-path app: treat it as a routing bug to fix, not an accepted state.
On Pico the fix is usually regenerating the tables after adding or renaming an
asset: `cargo run -p koto-audio-gen -- src/koto-pico/src/firmware/audio_cues_generated.rs`.

## Non-goals

Unchanged from KOTO-0162/0163 and reaffirmed here: a generic dynamic audio loader, a
runtime MML/`.kmml`/`.kwt` parser or loader, old KotoMML compatibility, a frozen audio
ABI, deleting the legacy code, voice stealing, and cumulative SFX diagnostics.
