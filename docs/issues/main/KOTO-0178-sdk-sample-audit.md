# KOTO-0178: SDK samples broken by runtime spec changes (dirty_rects hangs, cannot exit)

- Status: DONE 2026-07-12 — device-confirmed. The hang was a firmware
  panic-halt: the no-base per-command present path rastered the 17-line
  immediate-text band into the 16-line (`RASTER_STRIP_LINES`) strip, an
  out-of-bounds slice panic. Regressed in 520db8b (strip halved 32→16);
  Dirty Rects is the only app that reaches that path. Fixed by banding the
  text raster into strip-sized slices; dirty_rects runs and F10-exits on
  hardware.
- Type: bug
- Priority: P1 (the samples are the SDK's public face; a hanging sample is a
  hard failure)
- Related: KOTO-0052 (SDK samples), KOTO-0177 (exit-key parity — a sample that
  ignores EXIT intent and a firmware whose EXIT mapping is wrong compound each
  other).

## Symptom

After successive runtime/spec changes, some SDK samples no longer run
correctly. Confirmed case: **`apps/samples/dirty_rects`** launches but then
hangs — it cannot even be exited.

The sample source (`apps/samples/dirty_rects/src/main.koto`) looks correct at
face value: it polls `text_intent()`, checks `INTENT_EXIT`, calls
`yield_frame()` every iteration. So the hang is either a host-side regression
(intent delivery, `yield_frame` scheduling for minimal apps) or a stale
`.kbc`/prelude contract — not an obvious source bug. That makes it a runtime
bug until proven otherwise.

## Scope

Audit **all** `apps/samples/*` against the current runtime, sim first, then
device:

- sample_actor_array, sample_app, sample_counter_loop, sample_dirty_rects,
  sample_file_note, sample_hello_text, sample_ime_playground,
  sample_input_echo (manifest set in `sdcard_mock/apps/`).

For each: launches, runs its demonstrated behavior, exits via F10 (sim) /
EXIT intent, and its source still demonstrates current best practice
(host-call names, intent constants, draw budgets).

## Audit findings (2026-07-12, sim)

**Root cause of the dirty_rects "hang": there is no runtime bug and no sample
bug — the 2026-07-11 triage observed the KOTO-0177 exit-key work from its
broken middle slice.** The triage device was flashed with the first/second
KOTO-0177 slice, where EXIT had been narrowed to scan code `0x8a` (later
falsified — the PicoCalc bridge sends F10 as `0x90`, and the old X/Esc shim
was already removed). In that firmware **no key on the keyboard delivers
`INTENT_EXIT`**, and since the samples respond to nothing but EXIT, every
sample presented as "hangs, cannot be exited". The final KOTO-0177 slice
(EXIT ⇔ measured `0x90`, device-confirmed same day) removes the cause.

Evidence that the runtime and samples are healthy:

- `dev.koto.samples.dirty-rects` runs 3,000 scripted frames headless and
  exits cleanly on the EXIT intent (frames=3001, exited(0), fuel_peak=100 of
  60,000 — nowhere near a stall).
- **All 8 committed generations** of `sample_dirty_rects.kbc` (f0a607b →
  current, spanning the KOTO-0140 ABI restamp, the retained text layer, the
  peephole optimizer, and KOTO-0169 Stage 4) run and exit cleanly against the
  current runtime — a stale SD-card `.kbc` alone cannot reproduce the hang
  either.
- All 7 source samples launch, demonstrate their feature (verified visually
  via `--image`: moving band square, frame counter digit, sandbox note
  round-trip, actor bounce, typed-key echo, `comp:k` → `read:か` → `cand:蚊`
  IME states), and exit on `INTENT_EXIT` — the same bit KotoSim's F10 and the
  device's F10 legend deliver (`koto_core::keymap` parity tests).
- `dev.koto.sample` (`bytecode/main.kbc`) is the hand-assembled KOTO-0035
  launch fixture; exiting immediately with code 0 **is** its demo. Pinned by
  test so it is never again mistaken for a broken sample.

Hardening added: `sdk_samples_launch_run_and_exit_on_exit_intent` in
`koto-sim` sweeps every committed sample `.kbc` (launch → 8 drawing frames →
EXIT → exited(0)), so a runtime/prelude spec change can no longer strand the
samples silently.

Sample updated: `hello_text`'s caption said "F10 exits in KotoSim" — stale
since KOTO-0177 made F10 the exit key on both targets. Now "press F10 to
exit" (source + rebuilt `.kbc` + golden frame fixture).

## Device retest (2026-07-12): hang persists — what is now excluded

The retest on current firmware (post-KOTO-0177 final, post-KOTO-0186)
reproduces the hang: dirty_rects launches, then hangs; F10 does not exit. The
KOTO-0177-middle-slice attribution above explains the *triage-day* symptom
but not this one. Code-level exclusion so far:

- **PSRAM/code-window corruption is (almost) excluded for samples.** Every
  sample is single-tile (code 352–3,452 B < 16 KiB tile): exactly one refill
  at launch, immediately followed by the verifier walking every code word
  through that same cached tile. Corruption ⇒ `phase=254
  launch-verify-error` ⇒ bounce to shell (visible), not a hang. Single-tile
  is also not novel: memo (15,344 B) and sokoban (14,208 B) are single-tile
  and device-confirmed working; non-16-multiple refill lengths are exercised
  by koto_blocks (6,892 B tail tile) and kotosnake (15,848 B).
- **Early launch bounces are visible, not hangs.** Every failure path in
  `stage_app_code`/`run_app_session` returns to the firmware main loop, which
  repaints the shell (`phase=30 ready app-return`). A persistent hang means
  control is stuck *inside the app frame loop*.
- Sim exhausts the shared-runtime branches: all samples (all committed
  generations of dirty_rects) launch, run, and exit on INTENT_EXIT.

Remaining branches, all device-only, discriminated by one UART capture:

1. Frame-1 stall in `display_service.present().await` / LCD path (screen
   freezes, keys dead) — capture shows `phase=152` then **no** `phase=160`.
2. VM never yielding (fuel-exhausted loop; `text_intent` never polled, F10
   dead) — `phase=160` frames advance with `fuel=60000`-class values.
3. Key events not reaching the app session — healthy `phase=160`, but no
   `phase=180 key=0x90` on F10 press.
4. App actually exits but the LCD/present path is wedged (exit invisible) —
   `phase=153 app-exited` appears after F10.
5. Stale SD `.kbc` — `phase=156 app-staged ... code_size=` identifies the
   generation: **452 = current**; 480/496/464 = stale (older commits).

### UART capture procedure

With UART attached (Perf profile is enough — all lines below are always-on
or Perf-default):

1. Launch `SDK Dirty Rects` from the shell. Expected launch sequence:
   `phase=150 launch-request` → `phase=156 app-staged backing=psram
   code_size=452` → `phase=151 app-budget heap_request=64` → `phase=152
   app-started` → `phase=160 app-frame ... frame=1` → `phase=160 frame=120,
   240, …` (~every 2 s).
2. Let it run ~5 s, press a few arrows, then F10 (Shift+F5). Every
   press/release logs `phase=180 key state= key= shift=`; F10 must appear as
   `key=0x90`.
3. Note what the screen shows (band + moving square + caption? or black?) —
   distinguishes "presents once then freezes" from "never draws".

Match the log against the branch table above; the failing subsystem is the
one whose expected line is missing (or whose values are pegged).

## Root cause (2026-07-12, from the device UART capture)

The capture showed `phase=152 app-started`, the frame-1 key drain
(`phase=180 frame=0`), **no `phase=160 frame=1`** (nor the always-on frame-1
`phase=168`), and the screen frozen with the band + white square at x=0 —
i.e. frame 1's present drew its two rects and then never returned. That is
branch 1, and it pinpointed the line:

`present_app_commands` (the **no-full-screen-base** per-command present — the
path a first present takes when the app paints no `draw_rect(0,0,320,320,…)`)
rasters an immediate `Text` command as one full-width **17-line** band
(16 glyph rows + descender line, the same 17 the dirty-footprint model uses)
into the RGB565 strip:

```
used = 320 * 17 * 2 = 10,880 B  >  RASTER_STRIP_BYTES = 320 * 16 * 2 = 10,240 B
```

`strip[..used]` is an out-of-bounds slice → panic → `panic-halt` spins the
core forever: screen frozen mid-frame, no input, no further UART. The `Pixels`
arm has an overflow guard (`continue`); the `Text` arm did not.

Why only the samples, and only dirty_rects: every other app (and every other
sample) opens with a full-screen base fill, so their first present takes the
whole-surface pipelined compose, whose bands are capped at `PIPELINE_BAND_PX`
(≤ 8 full-width rows) by construction. Dirty Rects deliberately repaints only
a narrow band — it is the sole app in the fleet that exercises the
per-command no-base arm. And why no sim/host coverage caught it: the strip
budget is firmware-only geometry, and `koto-pico` does not build as a host
test target.

Regression point: **520db8b** (`feat(game2d): add retained text layer`)
halved `RASTER_STRIP_LINES` 32 → 16 (strip 20,480 → 10,240 B); the 17-line
text raster (10,880 B) fit the old strip and overflowed the new one. This
also **retro-explains the original triage observation** — the KOTO-0177
middle-slice exit-mapping story was a red herring for dirty_rects: the panic
predates the triage session and hangs the app regardless of the exit key.

Fix: the Text arm rasters and ships the 17-line band in
`RASTER_STRIP_LINES`-sized slices (the viewport clips glyph rows per slice),
so any band height fits any strip size by construction — same pixels, one
extra 320×1 transfer.

## Acceptance Criteria

- [x] dirty_rects hang reproduced and root-caused (sim scenario or device
      capture), then fixed. — device UART capture 2026-07-12: frame-1 present
      panic-halt in the no-base Text arm (17-line band vs 16-line strip,
      regressed in 520db8b). Fixed by strip-sized text banding;
      device-confirmed 2026-07-12.
- [x] Every sample launches, demonstrates its feature, and exits cleanly in
      KotoSim. — verified headless + visually 2026-07-12; now pinned by the
      `koto-sim` sample sweep test.
- [x] Device smoke for the previously-broken samples. — dirty_rects (the
      confirmed-broken case, and the only app on the no-base present path)
      runs its demo and F10-exits on hardware, 2026-07-12.
- [x] Samples updated where the spec moved under them, so they compile and
      model current API usage. — only `hello_text`'s exit-key caption had
      drifted; host calls, intent constants, and draw budgets are current in
      all samples.
