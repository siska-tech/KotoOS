# KOTO-0180: vendor the koto-audio repository into the KotoOS workspace

- Status: DONE (2026-07-12, device-confirmed). Vendored under `src/koto-audio`;
  sim + firmware + tools build standalone; device audio smoke passed; the
  standalone koto-audio repo is archived (read-only mirror).
- Type: harness
- Priority: P1 (blocks publication — a sibling-path dependency cannot be
  cloned; also blocks KOTO-0186, the P1 audio buzz-on-exit fix)
- Related: KOTO-0165 (koto-audio device runtime), KOTO-0179 (publication
  cleanup), KOTO-0186 (core1 worker stack overflow — its durable fix edits
  koto-audio's `reset()` and waits for this vendoring).

## Goal

KotoAudio lives in a separate sibling repository today; three crates depend on
it via fragile relative paths that escape the repo root:

- `src/koto-sim/Cargo.toml` → `../../../koto-audio/crates/koto-audio`
- `src/koto-pico/Cargo.toml` → `../../../koto-audio/crates/koto-audio`
- `tools/koto-audio-gen/Cargo.toml` → `koto-audio` + `koto-audio-tools`

A public KotoOS clone must build standalone, so bring the koto-audio crates
into the KotoOS tree (e.g. `src/koto-audio/`, `src/koto-audio-tools/` or a
`vendor/` prefix) and repoint the path dependencies.

## Notes / risks

- Preserve the koto-audio git history if practical (subtree merge) — it holds
  the KOTO-0164/0165 design rationale; a plain file copy is the fallback.
- The koto-audio repo's own license/README must survive the move.
- Watch the workspace `[profile.release]` (KOTO-0176 fat LTO + cu=1): the
  vendored crates join the workspace profile, which is desired but changes
  their codegen vs. the external checkout — device audio needs one smoke pass.
- Decide whether the standalone koto-audio repo is archived or kept as a
  read-only mirror after vendoring; two sources of truth is the failure mode.

## Acceptance Criteria

- [x] Fresh clone of KotoOS alone builds sim + firmware + tools with no
      references outside the repo root. — vendored via subtree merge to
      `src/koto-audio/` (history preserved). `cargo build` (default-members),
      `cargo build -p koto-pico --target thumbv6m-none-eabi --release`, and
      `koto-audio-gen` all build; the only koto-audio path deps are
      `../koto-audio/...` (koto-sim, koto-pico) and `../../src/koto-audio/...`
      (koto-audio-gen). No `../../../koto-audio` sibling escape remains.
- [x] `koto-audio-gen` regenerates identical cue tables
      (`audio_cues_generated.rs` diff-clean). — sha256 unchanged
      (`0eedc1e1…`), 42 cues (9 BGM / 33 SFX).
- [x] Device audio smoke (BGM + one-shots) after the vendored build. —
      device-confirmed 2026-07-12; drums sound via SLDPCM4 as before.

## Implementation notes

- **Layout.** Kept as its own nested workspace under `src/koto-audio/`
  (`exclude`d from the root workspace). This is deliberate: as a non-member
  path dependency cargo caps its lints, so the `check_all` clippy `-D warnings`
  / fmt gate does not police vendored upstream code — the same reason firmware
  escapes the host gate. The crate still inherits the root `[profile.release]`
  at build time (profiles are read from the build root, not the dep).
- **Line endings.** `core.autocrlf=true` rewrote the koto-audio-tools test
  fixtures to CRLF, breaking `practical_mml_example_..._matches_checked_in`
  (generator emits LF). Pinned `src/koto-audio/examples/**` to `eol=lf` via a
  scoped `.gitattributes`.
- **Sibling drift.** The external repo had an uncommitted PCM16
  `builtin_drums_generated.rs` edit, but both sim and firmware build with
  `sldpcm4-drums`, under which that module is `cfg`'d out — irrelevant to the
  vendored build. Vendored the clean committed state.
- **Two sources of truth.** Resolved: the standalone `koto-audio` repo is
  archived (read-only) — `ARCHIVED.md` + README banner point at KotoOS
  `src/koto-audio/` as the source of truth.
