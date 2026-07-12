# KOTO-0098: KotoMML multi-track playback

- Status: done
- Type: feature
- Priority: P2
- Requirements: FR-AUD-1

## Goal

Let a single KotoMML score play several simultaneous voices (lead + bass + drums)
so KotoBlocks BGM sounds like game music rather than a single-voice line. The
host-owned synth (KOTO-0095) already has waveforms, ADSR, an instrument bank, loop
points, and a BGM/SFX mix balance; the remaining expressiveness ceiling is the
single voice.

## Acceptance Criteria

- [x] Multiple `MmlTrack`s play simultaneously through the mixer.
- [x] Each track has independent instrument / octave / tempo / loop.
- [x] BGM supports up to 3-4 melodic voices; SFX voices stay at 2.
- [x] `#TRACK <name>` markers separate voices in one `.kmml`-style score
  (single-track text stays valid — a strict superset).
- [x] The mix stays within `i16` headroom (sub-unity per-voice BGM gain; clamp).
- [x] Headless `--audio` capture remains deterministic.
- [x] KotoBlocks BGM updated to lead (square) + bass (triangle) + drum (noise).
- [x] `python harness\check_all.py` passes.

## Resolution

`parse_mml_multi` / `parse_mml_multi_strict` split a score on `#TRACK` marker lines
and parse each section with the existing single-track parser, so every track keeps
its own `T`/`O`/`@`/`L` and `[ ]` loop. `SimAudio` now holds a `Vec<MmlPlayer>` for
the BGM (capped at `MAX_BGM_VOICES = 4`) and sums every voice at the sub-unity BGM
gain, leaving comfortable headroom (measured peak well under the `i16` ceiling for
the KotoBlocks lead+bass+drum score). The KotoBlocks package score has three
voices sharing a 4-beat loop body so they stay locked. The
synth remains deterministic, so the `--audio` capture path is unchanged.

Out of scope (follow-up): named instrument aliases (`@lead`) and an
`audio_set_volume` host call (the host `SimAudio::set_gains` already covers the
balance need). Package-local `.kmml` playback later landed as
`play_bgm_asset(path, len)` in host ABI minor 10.

## Notes

Builds directly on [KOTO-0095](KOTO-0095-app-audio-host-call.md). The `#TRACK`
grammar and the 4-voice budget are documented in
[KOTOMML_FORMAT.md](../../spec/KOTOMML_FORMAT.md).
