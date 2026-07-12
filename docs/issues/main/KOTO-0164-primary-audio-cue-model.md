# KOTO-0164: Primary audio asset / cue model — route table cleanup (SIM + Pico)

- Status: done (design + first slice; still KotoBlocks-only, no second app wired)
- Type: design / refactor
- Priority: P2
- Related: KOTO-0163, KOTO-0162, KOTO-0161, KOTO-0160, KOTO-0146, DIAG-0001

Policy: [AUDIO_DEPRECATION_POLICY.md](../../architecture/AUDIO_DEPRECATION_POLICY.md).
Cue model: [PRIMARY_AUDIO_CUE_MODEL.md](../../architecture/PRIMARY_AUDIO_CUE_MODEL.md).
Bridge reference: [KOTO_KOTOBLOCKS_AUDIO_BRIDGE.md](../../devlog/KOTO_KOTOBLOCKS_AUDIO_BRIDGE.md).

## Goal

Move the primary KotoAudio path from a KotoBlocks-special-cased set of asset-path
`if`/`match` branches toward a small, future-extensible **route table** so a new app
can use the primary path without depending on the legacy parser — while keeping the
KotoBlocks device/SIM behavior byte-for-byte and staying design-cleanup-sized.

Explicitly **not** in scope (unchanged non-goals): generic dynamic audio loader,
`.kmml`/`.kwt` loader or runtime parser, old KotoMML compatibility, stable ABI,
deleting legacy code, voice stealing, cumulative SFX diagnostics.

## What changed

### 1. Primary audio asset / cue model doc

New [PRIMARY_AUDIO_CUE_MODEL.md](../../architecture/PRIMARY_AUDIO_CUE_MODEL.md): the asset path is a
**routing key**, not a parseable format; new apps use generated compact sequence /
authored SFX / KACL clip / built-in cues; the runtime never parses `.mml`/`.kmml`;
`.kmml`/`.kwt` are legacy/deprecated. Includes the SIM↔Pico route table, the
extension-to-a-second-app plan, and the diagnostics/routing-miss semantics.

### 2. Route table replaces magic-string branching

The exact-path routing is now a `PrimaryCue` (`Bgm` | `Sfx(cue)`) route table plus a
single `primary_audio_route(path)` lookup, colocated with the cue definitions:

- **SIM** `src/koto-sim/src/koto_blocks_audio.rs`: `KOTO_BLOCKS_*_ASSET` path
  constants, `PrimaryCue`, `KOTO_BLOCKS_ROUTES`, `primary_audio_route`.
  `runtime/host.rs` dropped its `KOTO_BLOCKS_BGM_ASSET` const and
  `koto_blocks_sfx_kind` match; `play_bgm_asset` / `play_sfx_asset` now call
  `primary_audio_route` and hold **no** `.kmml` magic strings.
- **Pico** `src/koto-pico/src/firmware/audio.rs`: the same path constants,
  `PicoPrimaryCue`, `KOTO_BLOCKS_ROUTES`, `primary_audio_route`. `app_host.rs` dropped
  its `KOTO_BLOCKS_BGM_ASSET` const and `koto_blocks_sfx_cue` match and calls the
  route function instead.

Still KotoBlocks-only, but shaped as `&[(path_key, cue)]` — one entry of a future
per-app registry. The magic strings are localised to the two `audio.rs` modules.

### 3. SIM / Pico representation aligned

Identical `KOTO_BLOCKS_*_ASSET` path constant names and an identically named
`primary_audio_route` on both sides route the same keys. Cue enums stay distinct
(SIM `PrimaryCue`/`SeqSfx` descriptive; Pico `PicoPrimaryCue`/`KotoBlocksSfx` terse) —
renaming the shipped `SeqSfx`/`KotoBlocksSfx` variants was judged higher-risk than the
cleanup warranted, so the divergence (`HardDrop`↔`Lock`, `LineClear`↔`Clear`,
`GameOver`↔`Over`) is documented in the cue-model table and both enum doc-comments
instead. The doc routing table and the code constants now correspond 1:1.

### 4. Diagnostics boundary clarified

`seq-bgm`/`seq-sfx` are the **primary** labels; `legacy-mml`/`legacy-pcm`/`legacy-tone`
are deprecated fallbacks. A routing miss (path not in the table) falls through to
`legacy-*` — safe, but a defect for a primary-path app and the signature to watch on
device. Documented in the cue-model doc and cross-linked from the deprecation policy.

### 5. Regression maintained

- SIM non-silent + primary-path tests in `koto_blocks_audio.rs` and
  `runtime/tests.rs` unchanged and passing (route table returns the same cues the old
  branches did; `koto_blocks_app_drives_primary_seq_audio_not_legacy` still proves the
  app routes to the sequence bridge, not legacy).
- Pico `thumbv6m-none-eabi` build green.

## Files

- `docs/PRIMARY_AUDIO_CUE_MODEL.md` (new)
- `docs/AUDIO_DEPRECATION_POLICY.md` (routing-miss cross-link)
- `docs/KOTO_KOTOBLOCKS_AUDIO_BRIDGE.md` (routing table → route-table constants)
- `src/koto-sim/src/koto_blocks_audio.rs` (route table + constants + `PrimaryCue`)
- `src/koto-sim/src/runtime/host.rs` (use `primary_audio_route`)
- `src/koto-pico/src/firmware/audio.rs` (route table + constants + `PicoPrimaryCue`)
- `src/koto-pico/src/firmware/app_host.rs` (use `primary_audio_route`)

## Validation

- `cargo fmt --all`
- `cargo test` (koto-sim lib + fixtures)
- `cargo check -p koto-sim --features window`
- `cargo build -p koto-pico --target thumbv6m-none-eabi --bins`

## Legacy boundary

Unrouted `audio/*.kmml` paths still take the legacy SD-load + MML / `.kwt` / PCM / tone
chain unchanged; nothing legacy was deleted or reimplemented. The primary path is the
route table; the legacy path is the fallthrough.

## Next

- Wire a **second** primary-path app to prove the route table generalises (the point
  where a per-app registry keyed by the launching manifest earns its keep over the
  single KotoBlocks `const` table).
- Run the still-pending Pico UART capture (KOTO-0163 open item): confirm `seq-bgm`/
  `seq-sfx` served, no `legacy-*` on `koto_blocks_*` paths.
- Optional: unify the `SeqSfx`/`KotoBlocksSfx` variant spellings if a shared cue
  vocabulary becomes worth the cross-crate rename.
