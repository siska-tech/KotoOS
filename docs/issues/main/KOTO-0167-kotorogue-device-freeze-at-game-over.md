# KOTO-0167: KotoRogue freezes at game over on hardware

- Status: todo (investigation; needs a device capture at the freeze moment)
- Type: bug
- Priority: P1 (hard freeze, hardware only)
- Related: KOTO-0165 (koto-audio device runtime — prime suspect),
  [BUG-GFX-0012](../kotogfx/BUG-GFX-0012-kotosnake-initial-ui-invisible.md) /
  [GFX-0013](../kotogfx/GFX-0013-diffed-static-rebuild-damage.md) (the death
  frame takes a forced full repaint), KOTO-0056 (app failure recovery — did it
  engage?).

## Symptom

On hardware (2026-07-05 session), KotoRogue hard-freezes when the player dies.
The last `phase=160` lines before silence show normal play, then a
`full_reason=CommandCountShift` frame (`lat_ms=205`) in the hurt/death window;
no further app-frame lines are emitted.

## What is already ruled out

The **app-side game-over lifecycle is clean in KotoSim**: a scripted run
(START_HP=1 build, deterministic seed) drives play → lethal retaliation →
death frame (vignette + emptied retained layers) → result screen renders →
Enter restarts a fresh run → play continues. No trap, no fuel runaway
(`fuel_peak` ~26K), no stuck state. The freeze mechanism is device-side or
device-timing-dependent.

## Evidence pointing at the audio service

The same session's `phase=173` lines show the audio worker degrading long
before the freeze: `worker_late` 205 → 896 → 912 (≈1 s late), `underruns`
19 → 150 climbing, `buffer_level=0` throughout. The death turn is the one
moment the app fires `stop_bgm()` and a one-shot (`sfx_over`) back-to-back
while also paying a forced whole-surface repaint (~180 ms) in the same frame —
i.e. the CPU is maximally contended exactly when the (already ~1 s late)
audio worker receives a stop + start.

Secondary suspects: the present path during the death frame (StaticRebuild
forced-full + CommandCountShift interplay), or a device-only VM trap whose
recovery screen never paints.

## Next capture (device)

1. UART tail **at** the freeze: is there any panic / `phase=153` / recovery
   output after the last `phase=160`? Does `phase=173` keep printing (audio
   worker alive) while `phase=160` stops (frame loop dead), or do both stop?
2. When frozen: does BGM/any audio keep sounding? Does F10 still exit?
3. Cross-check: game over KotoMines or resign KotoShogi on device — both run
   the same `stop_bgm()`-plus-static-empty pattern; if they also freeze, the
   app is exonerated entirely and KOTO-0165's stop path is the focus.
4. If reproducible: retry after a fresh boot with a short session (low
   `worker_late`) — if death then survives, the freeze correlates with the
   degraded-worker state, not with game over per se.

## Acceptance criteria

- [ ] Freeze mechanism identified from a capture (frame loop vs audio worker vs
      trap), with the responsible component named.
- [ ] Fix lands in the responsible component; KotoRogue death → result →
      restart verified on hardware.
- [ ] The KotoSim death-lifecycle script is kept as a regression scenario.
