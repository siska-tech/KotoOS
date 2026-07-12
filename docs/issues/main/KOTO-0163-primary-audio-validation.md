# KOTO-0163: Validate the primary KotoAudio path on KotoBlocks (SIM + Pico) and minimal tuning

- Status: in-progress (SIM validated + regression test + one volume trim; Pico device run still pending)
- Type: validation / tuning
- Priority: P1
- Related: KOTO-0162, KOTO-0161, KOTO-0160, KOTO-0146, KOTO-0133, DIAG-0001

Policy: [AUDIO_DEPRECATION_POLICY.md](../../architecture/AUDIO_DEPRECATION_POLICY.md).
Bridge reference: [KOTO_KOTOBLOCKS_AUDIO_BRIDGE.md](../../devlog/KOTO_KOTOBLOCKS_AUDIO_BRIDGE.md).

## Goal

Confirm the primary path defined in KOTO-0162 — KotoAudio generated sequence BGM,
built-in-drum / authored SFX, KACL clip path — is stable and usable on KotoBlocks,
and apply only minimal tuning. Do **not** reintroduce the legacy `.kmml`/`.kwt`/MML
paths.

## SIM validation (done)

Driven through the real `koto_blocks.kbc` app in KotoSim (not synthetic bridge
calls). The title screen bakes its tile cache for ~28 frames before it honours a
start intent, so the start press must land after the bake — an early press is
silently ignored (root cause of an initial "no audio" false alarm during this
validation).

- **BGM + SFX play end-to-end.** Scripted run (bake → `newline` start → gameplay
  inputs) captured via `koto-sim --audio`: 243 285 samples, ~102 k non-silent.
  BGM-only run (start then idle): ~88 k / 103 k non-silent. The board reaches the
  play state (`実行中`) with NEXT/HOLD/score HUD.
- **Primary path, not legacy — proven by test.** New regression test
  `koto_blocks_app_drives_primary_seq_audio_not_legacy` in
  `src/koto-sim/src/runtime/tests.rs` launches the real app, starts it, and asserts
  the BGM asset routed to the **sequence bridge** (`SimAudio::seq_counter()` is
  `Some` and `bgm_start_count >= 1`; the legacy MML BGM path never creates `seq`),
  that gameplay raises `SfxAsset` hostcalls, that the mixed timeline is non-silent,
  that BGM stays active during play, and that `dropped_source_count == 0` at a normal
  cadence. This guards against the exact-path routing drifting out of sync with the
  app's asset paths (which would silently fall back to legacy).
- **stop_bgm scope.** `stop_bgm_stops_only_bgm_and_keeps_sfx` (existing) confirms the
  bridge stops only the BGM bus; active SFX keep playing. The app's game-over path
  calls `sfx_over()` *then* `stop_bgm()`, so the game-over cue survives the stop.
- **SFX reliability.** `each_sfx_cue_renders_non_silent` renders all six cues
  (incl. GameOver, Tetris) non-silent; `rapid_sfx_spam_never_panics` confirms
  `DropNew` drops excess spam without panicking or stealing the BGM slot. Under
  normal play (SFX spaced by input), the 3-slot SFX budget dropped nothing.
- Full suite: `cargo test -p koto-sim` → 103 lib + 13 fixture tests ok.

### Levels / volume tuning (one change)

Measured captured WAVs (peak / RMS vs full scale):

| Scenario        | peak      | RMS       | near-clip (\|s\|≥32000) |
| --------------- | --------- | --------- | ----------------------- |
| BGM only        | 36.0% FS  | 8.2% FS   | 0                       |
| BGM+SFX @ SFX230 | 100.0% FS | 10.2% FS  | 14                      |
| BGM+SFX @ SFX200 | 91.3% FS  | 12.4% FS  | 0                       |

BGM alone is healthy with headroom; SFX stacking on a BGM peak reached full scale
and clamped 14 samples (~0.006%) at the old `DEFAULT_SFX_VOLUME = 230`. Trimming to
**200** (≈0.78, still well above the 150/≈0.59 BGM bus so cues cut through) holds the
combined peak near 91% with no audible loss of presence and no clip. This is the only
tuning change; BGM volume is unchanged.

Files: `src/koto-sim/src/koto_blocks_audio.rs` (`DEFAULT_SFX_VOLUME` 230→200 + doc),
`docs/KOTO_KOTOBLOCKS_AUDIO_BRIDGE.md` (config table).

### legacy fallback occurrence

**None in SIM.** The KotoBlocks exact asset paths route to the sequence bridge
before any file read; the regression test proves `seq` is created (legacy would leave
it `None`). No `legacy-*` path was exercised for KotoBlocks.

## Representative phase=172/173 lines

phase=172/173 are **Pico-firmware** UART diagnostics (`koto-pico`), not emitted by
KotoSim. They appear only when `DIAG_PROFILE` (in `koto-pico` `config.rs`) enables the
Audio class: `Audio` profile → `phase=173` rollup; `Verbose` → adds the per-call
`phase=172` firehose. The default `Perf` profile emits neither. Expected served-path
lines for KotoBlocks (format from `app_host.rs` / `app_runtime.rs`):

```
phase=172 audio hostcall=0x25 name=play_bgm_asset sample_rate=-1 frames=0 channels=0 bytes=0 backend=<backend> result=seq-bgm
phase=172 audio hostcall=0x26 name=play_sfx_asset sample_rate=<rate> frames=<n> channels=1 bytes=0 backend=<backend> result=seq-sfx
phase=173 audio-summary frame=<f> audio_events=<n> samples_submitted=<n> samples_played=<n> drops=0 underruns=0 unsupported_count=0 buffer_level=<n> buffer_capacity=2048 command_drops=0 bgm_starts=1 bgm_stops=<n> bgm_voices=<n> sfx_voices=<n> mixer_saturations=<n> worker_late=0 worker_max_jitter_us=<n>
```

A capture that instead shows `result=legacy-mml` / `legacy-pcm` / `legacy-tone` on a
`koto_blocks_*` path means the exact-path routing missed (path drift) and fell back —
that is the failure signature to watch for on device.

## Pico hardware (not yet run — no device in this environment)

Device validation still requires a physical Pico + UART capture; it could not be
performed here. `koto-pico` is a `thumbv6m-none-eabi` target with no host-runnable
audio tests, so SIM parity + code inspection are the ceiling without hardware.
Procedure when a device is available:

1. Build/flash `koto-pico` with `DIAG_PROFILE = Verbose` (or `Audio` for the rollup
   only). Refresh the physical SD `.kbc` first (stale-SD is a known "no reaction" trap).
2. Launch KotoBlocks; start the game; play moves/rotations/locks/line-clears; trigger
   game over.
3. In the UART capture, confirm `result=seq-bgm` on the BGM start and `result=seq-sfx`
   on each SFX; confirm **no** `legacy-*` label on any `koto_blocks_*` path.
4. In `phase=173 audio-summary`, confirm `command_drops=0`, `unsupported_count=0`,
   `drops`/`underruns` ≈ 0, and `mixer_saturations` low; `bgm_starts` increments on
   start and `bgm_stops` on `stop_bgm` (BGM only; SFX unaffected).

Note: the firmware SFX voicing is per-note volume in embedded `PicoBgmScore` tables
with its own worker mixer — there is no single SFX-bus knob mirroring the SIM
`DEFAULT_SFX_VOLUME`, so the SIM trim above does not transfer 1:1. If the device
`phase=173` shows non-trivial `mixer_saturations`, retune the firmware SFX note
volumes rather than a bus gain.

## Non-goals (unchanged from KOTO-0162)

`.kmml` loader reimplementation, `.kwt` loader reimplementation, old MML
compatibility, a new parser, a frozen audio ABI, or a large audio rewrite.

## Next minimal fix

- **Run the Pico UART capture** (step 4 above) to close the one open item — confirm
  `seq-bgm`/`seq-sfx` served with `command_drops`/`mixer_saturations` low, no
  `legacy-*` on KotoBlocks paths. This is the only remaining validation gap.
- If device `mixer_saturations` is non-trivial, trim the loudest firmware SFX note
  volume (the BassDrum lock/hard-drop cue at 255) for the same headroom the SIM trim
  gained — smallest possible follow-up.
