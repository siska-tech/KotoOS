# KOTO-0186: core1 audio worker stack overflow under LTO (buzz-on-exit regression)

- Status: **DONE — device-confirmed 2026-07-12.** Durable in-place-reset fix +
  core1 canary/heartbeat landed on `develop`; the LTO ELF re-measurement below
  confirms the frame collapse, and the on-device KotoShogi/KotoRogue exit now
  yields clean silence with next-app audio re-init. KOTO-0176 Stage-1 audio gate
  cleared. (Was: deferred until KOTO-0180 lands — owner decision 2026-07-11; the
  durable fix touches koto-audio, which was easier once vendored.)
- Type: bug
- Priority: P1 (device audio is unusable after the first BGM-app exit)
- Related: KOTO-0176 (the LTO change that exposed it — its Stage 1 audio gate is
  FAILED on this until fixed), KOTO-0180 (vendor koto-audio — the blocker),
  KOTO-0172 (the same by-value-ctor stack lesson on core0), KOTO-0170 (stack
  canary pattern to extend to core1).

## Symptom

Post KOTO-0176 (fat LTO + codegen-units=1), exiting KotoShogi or KotoRogue
leaves a continuous buzz that never stops; audio never recovers for later app
launches.

## Root cause (confirmed by ELF measurement, 2026-07-11 build)

LTO inlines the koto-audio service into the core1 worker, and the unioned
frames exceed the 8 KiB `AUDIO_CORE1_STACK_BYTES`:

- `PicoAudioWorker::run` frame = 3,772 B (absorbs `tick()`'s `[i64;128]`
  accumulator et al.)
- `PicoAudioWorker::apply_command` frame = **5,060 B** — `StopAll` →
  `DefaultAudioService::reset()` rebuilds `SourceLifecycle::new()` +
  `Mixer::new()` **by value**, materializing both temporaries on the caller
  frame (koto-audio `service.rs::reset`).
- Chain ≈ 8.9 KiB > 8,192 B. `StopAll` first runs at BGM-app exit
  (`app_runtime.rs` `audio.stop()`), so the overflow fires exactly then.

Consequences: ~700 B overflow scribbles `APP_STATIC_SHADOW` (the .bss symbol
directly below the stack; embassy task POOL is next), the worker dies, and the
hardware-paced DMA loops the last 16 ms of the duty ring forever — that loop
*is* the buzz. A live worker overwrites the ring with silence even when idle,
so a sustained buzz is per se proof of worker death. `spawn_core1` runs once at
boot, so nothing recovers.

## Fix plan (after KOTO-0180)

1. **Durable**: make `reset()` (and any other big by-value rebuild in
   koto-audio's service path) construct in place — no whole-struct temporaries
   on the worker stack.
2. **Margin**: revisit `AUDIO_CORE1_STACK_BYTES` with the measured post-fix
   frame sizes; document the sizing rationale in `audio.rs` (the current 8 KiB
   comment predates LTO).
3. **Detection**: core1 stack canary (KOTO-0170 pattern) + a worker heartbeat
   counter surfaced in the `phase=173` stats line, so a dead worker is a
   number, not a mystery sound.

Interim mitigation (stack bump to 12–16 KiB) was considered and **not** taken —
the owner chose to wait for the durable fix. No bump was needed: the in-place
reset alone brought the chain back well under 8 KiB (see re-measurement).

## Fix as landed (2026-07-12)

1. **Durable — in-place reset.** `AudioService::reset` no longer reassigns
   `SourceLifecycle::new(..)` / `Mixer::new(..)`. New `SourceLifecycle::reset`
   (per-element `fill` of records/active + `SourceQueue::clear`) and
   `Mixer::reset` (re-derive default volumes) mutate the existing storage in
   place, so no whole-struct temporary is materialized on the worker frame.
   Behavioural parity is locked by two new koto-audio service tests
   (`reset_clears_sources_events_and_counters_and_stays_playable`,
   `reset_restores_mixer_volumes_to_policy_defaults`); full `koto-audio` suite
   155/155 green.
2. **Detection — canary + heartbeat.** Core0 paints `AUDIO_CORE1_STACK` with the
   `"koto"` canary before `spawn_core1` and scans it read-only; a monotonic
   `worker_heartbeat` is bumped once per worker pass. Both surface on the
   `phase=173 audio-summary` line as `worker_heartbeat=` and
   `core1_stack_free_min=`, so a wedged/dead worker is a frozen number (and a
   thin margin a visible one) instead of an inferred-from-the-buzz mystery.

## ELF re-measurement (thumbv6m release, fat LTO, 2026-07-12 build)

Both worker fns are still distinct symbols; `sub sp` prologue = the frame:

| fn | pre-fix `sub sp` | post-fix `sub sp` |
| --- | --- | --- |
| `PicoAudioWorker::run` | 3,772 B | **3,772 B** (unchanged — `tick()`'s inlined `[i64;128]` accumulator) |
| `apply_command` (StopAll → `reset` inlined) | 5,060 B | **140 B** |

`reset` no longer appears as its own symbol *and* no longer inflates
`apply_command`: the by-value `SourceLifecycle`/`Mixer` temporaries are gone, so
its inlined arm collapsed 5,060 → 140 B. Worst chain `run → drain_commands →
apply_command(StopAll)` ≈ 3,772 + 140 (+ two 20 B pushes) ≈ **~4.0 KiB**, versus
the pre-fix ~8.9 KiB — ~4 KiB of headroom under the unchanged 8 KiB stack. The
`core1_stack_free_min` canary reports the exact live low-water on device.

## Acceptance Criteria

- [x] KotoShogi/KotoRogue exit → clean silence; next app's audio initializes.
      *(device-confirmed 2026-07-12.)*
- [x] ELF frame sizes of `run`/`apply_command` re-measured and recorded here.
- [x] Core1 canary + heartbeat plumbed to UART (`phase=173`
      `worker_heartbeat` / `core1_stack_free_min`); a dead worker freezes the
      heartbeat. *(device-confirmed 2026-07-12.)*
