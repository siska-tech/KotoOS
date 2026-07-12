# KOTO-0184: developer support tooling for KotoAudio / KotoGFX app work

- Status: todo
- Type: feature
- Priority: P2
- Related: KOTO-0054 (asset development pipeline), KOTO-0049 (sim app dev
  experience), KOTO-0029 (KotoMML), GFX series (retained layers the tooling
  must teach).

## Goal

There is no support tooling for developing against KotoAudio or KotoGFX from
app code — authors iterate by editing text formats blind (`.kmml`, `.kwt`,
`.kpa`/`.kspr` sprites, tilemaps) and re-running the full app to hear/see the
result.

## Candidate tools (pick and scope in a design note first)

Audio:
- `.kmml` preview player on the host (koto-audio-tools already parses MML —
  wrap it in a "play this file now" CLI with loop/seek).
- Cue-table dry-run: show what `koto-audio-gen` will emit for an app (cue ids,
  sizes, builtin-instrument fallbacks for `.kwt` on Pico — KOTO-0165's
  sim-only wavetable caveat should be *visible*, not tribal knowledge).

Graphics:
- Sprite/tile sheet previewer for `.kpa` image assets and generated tiles.
- Retained-layer inspector in KotoSim: dump static/immediate/sprite/text layer
  contents and budgets per frame (extends the KOTO-0050 runtime inspector).
- Live-reload loop: recompile + relaunch a sim app on source/asset change.

## Acceptance Criteria

- [ ] Design note ranking the candidates by iteration-time saved; top one or
      two implemented, rest filed as follow-ups.
- [ ] The chosen tools are documented in the SDK docs and used to author or
      modify at least one real asset as the proving case.
