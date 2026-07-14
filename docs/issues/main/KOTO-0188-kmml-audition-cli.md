# KOTO-0188: .kmml audition — render to WAV and play on the host

- Status: done (2026-07-13), amended by the native-KotoAudio consolidation:
  `tools/koto-mml` now has one render engine, shared with generated SIM/Pico
  cues. The earlier engine-mode split described below is historical.
- Type: feature
- Priority: P2
- Related: KOTO-0184 (audio/gfx dev tooling — the ".kmml preview player"
  candidate), KOTO-0165 (koto-audio runtime, `.kwt` sim-only caveat),
  KOTO-0180 (vendored koto-audio), KotoIDE roadmap Phase 1b
  (`docs/planning/KOTOIDE_ROADMAP.md`).

## Goal

Hearing a `.kmml` edit currently requires wiring it into an app's `audio`
block, regenerating cue tables, and launching the sim. Add a "play this file
now" path:

- **`.kmml` → WAV render** (deterministic, testable), and
- **immediate host playback** of the same render (cpal, behind the existing
  optional-audio feature pattern so headless builds stay dependency-free).

## Design notes

- **Device parity first.** Render through the koto-audio sequence/mixer path
  (it has a sim backend) so what you hear is what the Pico plays — including
  the builtin-instrument fallback for `@6`–`@31`. A separate mode may use the
  KotoSim wavetable synth to hear `.kwt` voices as the sim plays them; the two
  modes make the sim/device gap *audible* instead of tribal knowledge.
- Decide the home during implementation: extend `koto-audio-tools` (already
  owns WAV I/O and the converter reports) vs. a thin new CLI that links both
  synth paths. Prefer whichever keeps the nested-workspace lint cap intact
  (KOTO-0180).
- Loop handling: `[`/`]` loops are infinite by design; the CLI needs a
  `--loops N` / `--max-seconds` bound for WAV output.
- Per-track mute/solo is cheap at parse level and worth including for
  multi-track scores.

## Acceptance Criteria

- [x] CLI renders a `.kmml` (with and without `.kwt` instruments) to WAV and
      plays it immediately on Windows host audio.
      → `koto-mml wav` (twinkle, kotomines SFX, kotosnake `.kwt` BGM all
      verified); `koto-mml play` streamed the KotoSnake BGM through cpal
      (opt-in `play` feature, koto-sim `window` pattern); without the feature
      it fails with a rebuild hint.
- [x] Device-parity mode renders via the koto-audio pipeline; sim-voice mode
      renders `.kwt` timbres; the mode is explicit in output/report text.
      → device mode reuses the cue-table conversion (now the `koto-audio-gen`
      library; regenerated `audio_cues_generated.rs` byte-identical) through
      `DefaultAudioService` + capture backend at 16 kHz, and notes the `.kwt`
      → builtin fallback; sim mode loads `#INST` `.kwt` via ancestor +
      `sdcard_mock` resolution and reports each binding; every report line
      starts `mode=device|sim`.
- [x] A fixture render is covered by a harness check (hash or golden report),
      keeping the render deterministic.
      → `twinkle_fixture_renders_are_deterministic_and_audible` double-renders
      the committed fixture in both modes and pins FNV-1a hashes; runs under
      `check_all.py`'s workspace test gate (koto-mml is a default member).
- [x] `docs/spec/KOTOMML_FORMAT.md` (or SDK docs) documents the audition
      workflow.
      → new "Audition (`koto-mml`, KOTO-0188)" section.
