# KotoBlocks KotoAudio bridge

KotoBlocks now uses the same generated native KotoAudio route as every other app.
This replaces the earlier KotoBlocks-only BGM table and hand-authored SFX routing.

The sources are `apps/koto_blocks/audio/*.kmml`. The normal app build scans these
alongside every other app score and emits BGM and SFX entries into the shared
`GENERATED_BGM_ROUTES` and `GENERATED_SFX_ROUTES` arrays. Drums in the BGM use
native KotoAudio aliases (`!bd`, `!sd`, `!hh`, `!oh`).

SIM includes the same generated module as Pico and dispatches route results through
the shared `KotoBlocksAudio` service bridge. Despite the historical module name,
the bridge accepts generic `PolyphonicSequence` and `Sequence` references and is
used by all routed applications.

Regenerate and verify with:

```powershell
python harness/build_apps.py
python harness/build_apps.py --check
cargo test -p koto-audio-gen
cargo test -p koto-sim
```

The runtime does not read or parse these KMML payloads. A missing generated route
is rejected as a build defect rather than falling through to another audio engine.
