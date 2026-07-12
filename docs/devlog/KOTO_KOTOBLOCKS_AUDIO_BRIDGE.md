# KotoBlocks ↔ koto-audio generated-sequence bridge (SIM)

Reference for the KotoBlocks audio bridge in KotoSim: how game-audio hostcalls are
routed to the `koto-audio` bounded runtime, what its configuration is, how to
regenerate the generated BGM table, and the known limitations that stand between the
current SIM bridge and a Pico CPU1 audio worker.

> **Primary path (KOTO-0162).** The KotoAudio generated-sequence runtime described
> here is the **primary** KotoOS audio path on both SIM and Pico. The runtime
> KotoMML (`.kmml`) synth, `.kwt` wavetable loader, and tone fallback are **legacy**
> and deprecated — see [AUDIO_DEPRECATION_POLICY.md](../architecture/AUDIO_DEPRECATION_POLICY.md).
> New music/SFX target the compact-sequence / generated-table / built-in-drum /
> KACL-clip path, not the legacy MML path.

The bridge is implemented in
[`src/koto-sim/src/koto_blocks_audio.rs`](../../src/koto-sim/src/koto_blocks_audio.rs);
asset-path routing lives in
[`src/koto-sim/src/runtime/host.rs`](../../src/koto-sim/src/runtime/host.rs); the merge
into the shared output stream is `SimAudio::render` in
[`src/koto-sim/src/audio.rs`](../../src/koto-sim/src/audio.rs).

## Overview

KotoBlocks does not ship playable `.kmml` scores for its music. Instead its BGM is a
**generated compact sequence table** (produced offline by `koto-audio-tools` from an
MML source and vendored into the bridge as static Rust), and its SFX are small
**hand-authored** `koto_audio::Sequence` tables. Both are played through a bounded
`koto_audio::AudioService`, whose mixed blocks are folded additively into the single
`SimAudio` output stream that already carries every other app's MML/raw audio.

The `koto-audio` crate is used **unchanged**. Every non-KotoBlocks app — and the
SimAudio MML synth, raw-PCM (`audio_submit_i16`), and PCM16 clip paths — keeps its
current behavior.

## Responsibilities (and non-responsibilities)

The bridge module is deliberately narrow. It **is**:

- the generated BGM sequence host (`BLOCKS_LIKE_BGM_COMPACT` → looping
  `PolyphonicSequence`);
- the authored SFX cue host (`SeqSfx` → static `Sequence`);
- the `koto_audio::AudioService` driver and the `mix_into` merge into `SimAudio`.

It **is not**:

- a runtime `.kmml` parser — the runtime KotoMML synth for every other app lives in
  `src/koto-sim/src/audio.rs`;
- a `.kwt` wavetable loader — BGM/SFX use koto-audio built-in voices;
- the hostcall dispatcher — `runtime/host.rs` dispatches the hostcalls. The route
  table that maps paths to cues (`primary_audio_route` / `PrimaryCue`) lives in the
  bridge module next to the cues it names, so the host holds no `.kmml` magic strings.

## Asset-path routing

KotoBlocks emits ordinary `play_bgm_asset` / `play_sfx_asset` / `stop_bgm` hostcalls on
`audio/koto_blocks_*.kmml` asset paths (declared in its manifest / `apps.json`). The
routing is the **primary audio route table** (KOTO-0164): the host resolves each path
with `primary_audio_route` (in `koto_blocks_audio.rs`) to a `PrimaryCue`, and drives the
bridge instead of opening and parsing the `.kmml` payload. Any unrouted `audio/*.kmml`
path falls through to the legacy MML synth path. The full route table — path constants,
SIM/Pico cue correspondence, and the extension-to-a-second-app plan — is the source of
truth in [PRIMARY_AUDIO_CUE_MODEL.md](../architecture/PRIMARY_AUDIO_CUE_MODEL.md); the KotoBlocks-specific
mapping is:

| Hostcall          | Path constant / asset path                       | `PrimaryCue` → bridge action        |
| ----------------- | ------------------------------------------------ | ----------------------------------- |
| `play_bgm_asset`  | `KOTO_BLOCKS_BGM_ASSET` `audio/koto_blocks_bgm.kmml`    | `Bgm` → `seq_start_bgm` (looping generated BGM) |
| `play_sfx_asset`  | `KOTO_BLOCKS_MOVE_ASSET` `audio/koto_blocks_move.kmml`  | `Sfx(SeqSfx::Move)` — short high blip |
| `play_sfx_asset`  | `KOTO_BLOCKS_ROTATE_ASSET` `audio/koto_blocks_rotate.kmml` | `Sfx(SeqSfx::Rotate)` — two-note rise |
| `play_sfx_asset`  | `KOTO_BLOCKS_LOCK_ASSET` `audio/koto_blocks_lock.kmml`  | `Sfx(SeqSfx::HardDrop)` — low landing thud |
| `play_sfx_asset`  | `KOTO_BLOCKS_CLEAR_ASSET` `audio/koto_blocks_clear.kmml` | `Sfx(SeqSfx::LineClear)` — ascending arpeggio |
| `play_sfx_asset`  | `KOTO_BLOCKS_TETRIS_ASSET` `audio/koto_blocks_tetris.kmml` | `Sfx(SeqSfx::Tetris)` — four-line fanfare |
| `play_sfx_asset`  | `KOTO_BLOCKS_OVER_ASSET` `audio/koto_blocks_over.kmml`  | `Sfx(SeqSfx::GameOver)` — descending phrase |
| `stop_bgm`        | (n/a — global, not path-routed)                  | `seq_stop_bgm` (stops BGM bus only; SFX keep playing) |

Notes:

- The `.kmml` files still exist as package assets and must be declared so the
  permission check (`asset_paths`) passes, but for these KotoBlocks paths their
  *contents* are never read — the path is a routing key and the audio is the
  generated/authored Rust tables.
- Routing is by **exact string match** via the route table. A renamed or mistyped path
  returns no route; it falls through to the MML synth (and, lacking valid MML, is silent
  or rejected). This routing miss is the `legacy-*`-on-a-`koto_blocks_*`-path failure
  signature to watch for (see the cue-model doc's Diagnostics section).

## Configuration

Set in `koto_blocks_audio.rs` unless noted. On koto-audio's `MixerVolume` scale,
`256` = unity gain.

| Setting              | Value                | Meaning                                          |
| -------------------- | -------------------- | ------------------------------------------------ |
| `DEFAULT_BGM_VOLUME` | `150` (≈ 0.59)       | BGM bus initial gain — below unity for headroom  |
| `DEFAULT_SFX_VOLUME` | `200` (≈ 0.78)       | SFX bus initial gain — above BGM so cues cut through, below full for mix headroom (KOTO-0163: 230 clipped the SFX-on-BGM sum to full scale in play) |
| `max_sfx_sources`    | `3`                  | BGM (1) + up to 3 SFX fits the 4-source budget   |
| `drop_policy`        | `DropNew`            | A full queue drops the *new* SFX (move/rotate spam never steals the BGM slot or panics) |
| block size           | `DEFAULT_MIXER_BLOCK_FRAMES` | Fixed mixer block length shared with the backend |

Both bus volumes are overridable at runtime via `KotoBlocksAudio::set_bgm_volume` /
`set_sfx_volume`, or together through `SimAudio::set_seq_volumes(bgm, sfx)`.

The bridge is created lazily at the current output sample rate on first sequence use
(`SimAudio::ensure_seq`) and dropped on sample-rate change or `SimAudio::reset` (new app
launch), so it always renders at the device rate with no resampling.

## Regenerating the BGM sequence table

`BLOCKS_LIKE_BGM_COMPACT` and its `*_EVENTS` / `*_TRACKS` / `*_INSTRUMENTS` statics are
**generated**, then vendored verbatim into `koto_blocks_audio.rs` (only the imports at
the top of the module differ from the generated fragment). To regenerate after editing
the source MML, run in the sibling `koto-audio` repo:

```console
cargo run -p koto-audio-tools --bin koto-audio-mml-table -- \
  --symbol BLOCKS_LIKE_BGM_COMPACT \
  --prefix BLOCKS_LIKE_BGM_COMPACT \
  examples/mml/blocks_like_bgm.mml \
  examples/generated/blocks_like_bgm.rs
```

Then copy the `*_INSTRUMENTS` / `*_EVENTS` / `*_TRACKS` / `CompactSequence` statics from
`examples/generated/blocks_like_bgm.rs` into the "Generated BGM compact table" section
of `koto_blocks_audio.rs`, keeping this crate's imports. The bridge test
`generated_bgm_table_validates_and_adapts_to_three_voices` asserts the copied table
still `validate()`s and adapts to three voices.

Optional listening check (renders the same MML to a WAV via the host `AudioService`):

```console
cargo run -p koto-audio-tools --bin koto-audio-mml-render -- \
  --bgm --seconds 8 --sample-rate 22050 \
  examples/mml/blocks_like_bgm.mml \
  target/blocks_like_bgm.wav
```

The SFX cues are **not** generated — they are the hand-authored `*_SEQ` / `*_EVENTS`
statics in the "Authored static SFX sequences" section, edited directly in Rust.

## Known limitations

This bridge is the primary audio path (KOTO-0162), but hardware coverage and voicing
are still maturing. Standing limitations:

- **koto-audio path dependency.** `koto-sim` depends on `koto-audio` by a relative path
  (`../koto-audio/crates/koto-audio`), which since KOTO-0180 points at the crate vendored
  in-tree under `src/koto-audio/` (its own nested workspace). The build no longer needs a
  sibling checkout.
- **Exact-path routing.** Routing keys on exact asset-path strings. Renaming the
  KotoBlocks assets requires updating `runtime/host.rs`; a mistyped path silently falls
  through to the MML path rather than erroring.
- **No strict SFX priority / voice stealing.** SFX admission is `DropNew` on a full
  queue — a new cue is dropped rather than stealing an older or lower-priority voice.
  There is no per-cue priority ordering.
- **Pico CPU1 worker: full koto-audio port, hardware unverified (KOTO-0165).** The
  device now runs `koto_audio::DefaultAudioService` itself on the CPU1 worker; the
  KotoBlocks BGM plays from the *same* generated table as the SIM (compiled from
  `blocks_like_bgm.mml` by `tools/koto-audio-gen` into
  [`audio_cues_generated.rs`](../../src/koto-pico/src/firmware/audio_cues_generated.rs)),
  and the SFX are verbatim ports of the authored sequences below
  ([`audio_cues.rs`](../../src/koto-pico/src/firmware/audio_cues.rs)). The KOTO-0161
  16-event truncation is gone. Hardware playback of the ported runtime is still
  unverified.
- **Minimal SFX voicing.** The authored SFX cues are tuned minimally (short square /
  triangle / bass-drum sequences). They are placeholders for feel, not final sound
  design.

## Non-goals (for the current bridge)

Explicitly out of scope here: changing the `koto-audio` crate; implementing a runtime
`.kmml` loader or `.kwt` loader; adding a runtime MML parser to the bridge; porting the
Pico CPU1 worker; implementing voice stealing; or freezing a stable ABI between the
bridge and `koto-audio`.
