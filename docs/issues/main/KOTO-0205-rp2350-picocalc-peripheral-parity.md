# KOTO-0205: RP2350A Pico 2 W PicoCalc peripheral parity

- Status: in-progress
- Type: feature
- Priority: P0
- Requirements: FR-SDK-8, NFR-PORT-4, NFR-PORT-6, NFR-DEV-5
- Related: KOTO-0065, KOTO-0126, KOTO-0204

## Goal

Run the current KotoOS product firmware on the available RP2350A Pico 2 W in a
PicoCalc, with the same LCD, keyboard, SD, audio, power, USB logging, app, and
baseboard PSRAM behavior already validated on RP2040.

## Acceptance Criteria

- [ ] The available Pico 2 W installed in PicoCalc boots the RP2350A artifact and
  reports its board profile over the normal debug transport.
- [ ] LCD initialization, dirty rectangles, keyboard I2C, SD mount/package
  reads, PWM audio, battery/power polling, and app launch/return pass the
  retained device probes or the equivalent product-firmware checks.
- [ ] The PicoCalc baseboard 8 MiB PSRAM on GP2–5/GP20/GP21 passes ID,
  round-trip, boundary, CodeWindow, and fallback tests on RP2350.
- [ ] PIO and DMA timing is measured on RP2350; RP2040-specific clock dividers,
  DREQ values, or PAC register assumptions are not reused without evidence.
- [ ] The two-core audio worker completes the existing mixed app/audio stress
  session with no new underruns, deadlocks, or core stack corruption.
- [ ] RP2350 static-memory and stack-canary results are recorded separately
  from RP2040 and use an RP2350-appropriate linker/RAM boundary.
- [ ] Pico 2 W can boot and operate without initializing the wireless radio;
  networking remains out of scope.
- [ ] The RP2350 hardware log records module model/revision, build profile,
  clocks, PSRAM backend, pass/fail results, and any deviations from the RP2040
  parity baseline.
- [ ] The RP2040 hardware path remains buildable and its existing board
  behavior is not regressed.

## Notes

- Depends on the compile/linker foundation in KOTO-0204.
- RP2350B/Pico Plus 2(W) is outside this issue and does not block completing
  the RP2350A release gate.
- Host-side implementation started 2026-07-14. The RP2350A validation bundle,
  PSRAM identity/fallback gate, board-specific conservative clocks, and device
  procedure are implemented. Physical acceptance remains open because no Pico
  2 W was connected in BOOTSEL or via the PicoCalc UART during the session; see
  [`RP2350A_PICOCALC_VALIDATION.md`](../../hardware/RP2350A_PICOCALC_VALIDATION.md).
- The first LCD attempt produced no panel output because `probe_lcd` waited for
  native USB CDC before initializing the display. The probe now starts LCD and
  UART0 diagnostics immediately. The corrected RP2350A UF2 passed its visible
  LCD retest on 2026-07-14.
- The RP2350A keyboard smoke capture produced 600 samples, zero I2C errors,
  and zero frame-budget overruns (`poll_us` 783–1,602). Explicit
  direction+action chord capture remains part of the final product stress
  pass.
- The RP2350A SD probe passed on 2026-07-14. The test card was detected at
  7,822,376,960 bytes; its 12 MHz initialization returned `CardNotFound`, then
  the retained 1 MHz fallback mounted FAT and enumerated 18 `.kpa` packages.
  A 33,848-byte sequential read of `kotomines.kpa` completed with FNV-1a
  checksum `8f8aeb97`.
- The PicoCalc baseboard PSRAM probe passed on RP2350A on 2026-07-14. The
  8-MiB identity was valid, the last 256-byte block round-tripped, the
  two-tile CodeWindow passed, block 257 matched exactly, and the first invalid
  block was rejected. The measured serial clock was 15 MHz; RP2350 DMA request
  values were PIO1 RX `12` and TX `8`.
- The RP2350A power probe passed six consecutive cycles on 2026-07-14. BIOS
  1.6 was returned every cycle; two transient version-register read failures
  recovered on attempt 2 and the other four succeeded on attempt 1. Battery
  register reads all succeeded on attempt 1 and returned the expected
  no-battery/unavailable value `[0x0b,0x00]`.
- The RP2350A audio probe passed through the PicoCalc speaker on 2026-07-14.
  Two 24,000-sample DMA intervals completed at an 8 kHz sample rate with a
  99,601 Hz PWM carrier, zero remaining transfers, zero AHB errors, and zero
  reported underruns.
- The forced-PSRAM-fallback product run selected `sram-window` and remained
  generally operational on 2026-07-14. Its 160 ms boot PCM diagnostic consumed
  all 2,560 submitted frames with zero drops but logged one startup underrun;
  the short boot beep was audibly confirmed. Sustained in-app audio is retained
  as a check for the normal-product stress run.
- The normal RP2350A product artifact subsequently produced audible sound
  during ordinary in-app operation. This confirms sustained operation of the
  product CPU1 mixer and DMA/PWM speaker path; the final `phase=173` counters
  remain to be captured for quantitative stress acceptance.
- KotoRogue then ran to frame 3,000 at 24 fps using three PSRAM CodeWindow
  refills per sampled frame, exited with code 0, and returned to the shell.
  RP2350 main-stack usage peaked at 42,680 bytes with 265,616 bytes minimum
  free. The Perf diagnostic profile suppresses periodic audio summaries; the
  existing Audio-profile build is used when those counters are required.
- The Audio-profile product run kept KotoShogi operational through frame 3,780
  and exited with code 0. Across the final two samples, underruns stayed at the
  single startup event, all drop/late/saturation counters stayed zero, worker
  heartbeat advanced by 1,298, maximum worker jitter was 1,033 µs, and the
  core1 stack retained 6,220 bytes minimum free. This passes the RP2350A
  two-core product audio stress gate.
- The installed application set was subsequently reported generally free of
  problems on the RP2350A product build. This is a suite-level smoke result;
  KotoRogue and KotoShogi provide the retained detailed stress captures.
- Follow-up performance work vendors `koto-psram` into the KotoOS workspace
  and enables its fast CodeWindow QPI/RX-DMA backend for RP2350A. RP2350 uses
  the structured DMA transfer-count register and divider 2 (75 MHz); the
  existing safe full-range retry remains the failure fallback. Physical A/B
  performance validation passed on 2026-07-14: KotoRogue transferred three
  16-KiB tiles in 4,195 us versus the roughly 35.4 ms legacy baseline,
  `fast_success_count` reached 3,962, and `fast_fallback_count` remained zero.
- Before board-specific SRAM tuning, board selection was separated from MCU
  selection. Product builds now select `board-picocalc-pico` or
  `board-picocalc-pico2w`; each profile owns its MCU, clocks, capacities, and
  PicoCalc carrier mapping under `src/board/`. Product firmware consumes
  semantic peripheral roles instead of GPIO-numbered Embassy fields.
- The RP2350A profile then raised `code_window_tiles` from two to three. The
  core cache was generalized from a fixed two-slot implementation to bounded
  1–4-slot LRU; RP2350A now allocates 48 KiB as three 16-KiB slots while RP2040
  remains at 32 KiB/two slots. Device acceptance remains tied to KotoRogue
  steady-state refill and stack-canary evidence.
- RP2350A three-slot performance passed on device: KotoRogue frame 2,430 had
  zero refills/bytes/refill time, 3,517 us VM time, and a reported 222 fps.
  The fast path had 10 warm-up successes, zero fallbacks, and no fallback
  reason. This removes the two-slot sample's 4,195 us steady-state refill cost;
  the post-growth stack-canary line is the remaining memory-margin capture.
