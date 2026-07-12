# KOTO-0162: Deprecate legacy KotoOS audio; KotoAudio sequence runtime is primary

- Status: in-progress (first slice: docs + boundary + diagnostics; hardware confirmation pending)
- Type: architecture / deprecation
- Priority: P1
- Related: KOTO-0160, KOTO-0161, KOTO-0146, KOTO-0133, KOTO-0095, KOTO-0098, KOTO-0029

Policy source of truth: [AUDIO_DEPRECATION_POLICY.md](../../architecture/AUDIO_DEPRECATION_POLICY.md).
Bridge reference: [KOTO_KOTOBLOCKS_AUDIO_BRIDGE.md](../../devlog/KOTO_KOTOBLOCKS_AUDIO_BRIDGE.md).

## Background

The original KotoOS audio path (CPU1 worker, KotoMML subset, `.kwt` wavetables,
BGM/SFX mixing) works but is an exploratory, after-the-fact implementation with
limited sound quality and fuzzy design boundaries. The sibling `koto-audio` crate has
since matured into a bounded runtime (PCM16 clips, SLDPCM4, mono/poly sequences,
BGM/SFX bus, built-in instruments, fixed PCM drums, compact-sequence + generated-table
workflow, render-to-WAV, and SIM/Pico bridges).

## Goal

Treat the KotoAudio generated-sequence runtime as the official audio path going
forward. Isolate the old implementation as **legacy/deprecated** and keep only the
knowledge worth retaining. Do **not** preserve compatibility with the old path.

## First slice (this change)

1. **Deprecation policy doc.** New `AUDIO_DEPRECATION_POLICY.md`: legacy
   `.kmml`/`.kwt`/tone paths, primary KotoAudio compact-sequence/generated-table path,
   and an explicit "no compatibility guarantee." Diagnostics `result=` convention
   table lives here.

2. **KotoBlocks path is primary.** The bridge doc and the SIM/Pico Rustdoc now state
   the KotoAudio sequence path is the **primary** audio path on both SIM and Pico, and
   the MML/`.kwt`/tone paths are legacy fallbacks — not the milestone target they read
   as before.

3. **Legacy boundary.** The legacy runtime MML parser, `.kwt` loader, and tone
   fallback are fenced behind clear `LEGACY` module banners / Rustdoc in
   `koto-sim/src/audio.rs` and `koto-pico/src/firmware/audio.rs`, so they do not read
   as the primary path. No large deletion in this slice (see non-goals).

4. **Diagnostics.** `result=seq-bgm` / `result=seq-sfx` are the primary labels. The
   legacy paths that previously logged an ambiguous `result=ok` now log `legacy-mml`,
   `legacy-pcm`, `legacy-tone`, and `legacy-tone-fallback` so a capture makes fallback
   obvious. The `unsupported_count`, `command_drops`, and `mixer_saturations`
   counters are retained unchanged.

## Non-goals

- full deletion of the old audio path;
- reimplementing the `.kmml` loader;
- reimplementing the `.kwt` loader;
- old KotoMML compatibility;
- freezing a stable audio ABI;
- adding dynamic allocation.

## What it does NOT change

VM semantics, opcode values, bytecode ABI, hostcall IDs, `RuntimeLimits`, the CPU1
worker command protocol, and the koto-audio crate are untouched. The legacy paths
keep their panic-free / safe-stop behavior and existing counters; only their
diagnostic `result=` labels and surrounding documentation change.

## Acceptance

- `cargo fmt` — clean.
- `cargo test` — ok.
- `cargo check -p koto-sim --features window` — ok.
- `cargo check`/`build -p koto-pico --target thumbv6m-none-eabi` — ok.
- KotoBlocks BGM/SFX still play in KotoSim.
- Hardware: KotoBlocks BGM/SFX still play on Pico; a UART capture shows `seq-bgm` /
  `seq-sfx` as the served path (and any fallback shows a `legacy-` label).

## Remaining work

- Device UART run confirming the primary `seq-*` path and legacy labeling.
- Later slices may retire legacy code once no shipped app depends on the `.kmml` /
  `.kwt` / tone paths.
