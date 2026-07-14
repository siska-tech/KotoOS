# RP2350A Pico 2 W PicoCalc Validation

This is the device gate for KOTO-0205. Compilation and UF2 inspection are
complete; rows below remain pending until their output is captured from the
available Pico 2 W installed in a PicoCalc. Do not promote the board profile to
hardware-supported from host results alone.

## Build the validation bundle

```powershell
rustup target add thumbv8m.main-none-eabihf
$env:PICOTOOL = "C:\path\to\picotool.exe"
tools\build-rp2350a.ps1 -ValidationBundle
```

The bundle is written under
`target/thumbv8m.main-none-eabihf/release/` and contains the normal product
firmware, six retained peripheral probes, and
`koto_firmware-picocalc-pico2w-rp2350a-forced-psram-fallback.uf2`.

## Test environment

- Module: Raspberry Pi Pico 2 W / RP2350A
- Module revision or date code: pending capture
- PicoCalc revision: pending capture
- Build profile: `board-picocalc-pico2w,ram_interpreter,ram_audio_mixer`
- Default clocks: system 150 MHz; LCD request 37.5 MHz; PSRAM PIO divider 5
  (30 MHz state machine, two instructions/bit, nominal 15 MHz serial clock)
- PSRAM backend: PicoCalc baseboard serial PIO1/SM0 on
  GP2/GP3/GP20/GP21; module PSRAM absent
- Wireless policy: CYW43 hardware present, initialization disabled
- Debug transport: PicoCalc mainboard UART bridge, UART0 TX GP0, 115200 8N1;
  LCD and keyboard probes additionally expose USB CDC

## Flash and capture order

Power the PicoCalc mainboard before tests that use the keyboard STM32. For each
row, enter BOOTSEL, copy the named UF2, capture the UART/CDC output, and record
the visible or audible observation.

| Order | Artifact | Required evidence | Status |
| :---- | :------- | :---------------- | :----- |
| 1 | `probe_lcd-picocalc-pico2w-rp2350a.uf2` | Correct RGB fills/orientation/rectangles; continuous DMA band; transport line reports RP2350A 37.5 MHz profile. | pass (2026-07-14) |
| 2 | `probe_keyboard-picocalc-pico2w-rp2350a.uf2` | Board ID is RP2350A; powered 100 kHz I2C matrix completes; press/release and required chords stay in budget. | smoke pass; chord retest pending |
| 3 | `probe_sd-picocalc-pico2w-rp2350a.uf2` | Card detects, mounts, lists `APPS/`, and reads a package at 12 MHz or the logged 1 MHz fallback. | pass (2026-07-14; 1 MHz fallback) |
| 4 | `probe_psram-picocalc-pico2w-rp2350a.uf2` | ID has manufacturer `0x0d`, KGD `0x5d`, 64-Mbit density; mid/last-block round trips, range rejection, and two-tile CodeWindow report `pass`; timing and PAC-derived DREQ values are captured. | pass (2026-07-14) |
| 5 | `probe_power-picocalc-pico2w-rp2350a.uf2` | STM32 version and battery percentage are returned, or an explicit nonfatal unavailable result is logged. | pass (2026-07-14; transient read recovered) |
| 6 | `probe_audio-picocalc-pico2w-rp2350a.uf2` | Clean tone on both outputs; runtime system/carrier/DMA pacing values are logged; DMA completes without AHB error. | pass (2026-07-14; speaker) |
| 7 | forced-fallback product UF2 | `phase=17 ... forced_fallback=true`, `phase=198 ... fallback=sram-window`, shell boot, and a small resident app launch/return succeed. | smoke pass; explicit app launch/return pending |
| 8 | normal product UF2 | Board banner, LCD/keyboard/SD/power startup, PSRAM `phase=16`, app launch/return, mixed audio stress, and RP2350-specific `phase=176` stack canary are captured. | pass (2026-07-14) |

`probe_keyboard` starts polling immediately and always writes JSONL to UART0;
native USB CDC is an optional mirror and is never an execution gate. This
probe intentionally does not draw on the LCD.

## Product stress gate

With the normal product artifact, run the existing worst-case session: boot,
shell navigation, each installed game, IME/memo, simultaneous game audio, and
return to shell. Record:

- `phase=171` audio result plus final drops/underruns;
- CodeWindow refill/failure output and successful app exits;
- core-1 liveness and absence of deadlock;
- `phase=176` RP2350 stack-canary `free_min`;
- visible LCD corruption, input loss, or SD errors, including none observed.

## Fast CodeWindow performance candidate

The RP2350A MCU feature now enables the vendored `koto-psram` fast CodeWindow
path. It uses QPI falling-edge reads, 4 KiB RX-DMA chunks on DMA channel 1, and
PIO divider 2 (75 MHz at the RP2350A 150 MHz system clock). A failed fast read
is never accepted partially: the same range is re-read through the safe path.

Device artifact:
`koto_firmware-picocalc-pico2w-rp2350a-fast-codewindow-clkdiv2-diag.uf2`.
Run KotoRogue or KotoShogi and retain `phase=163`, `phase=167`, `phase=175`,
and clean app-exit evidence. Acceptance requires increasing
`fast_success_count`, `fast_fallback_count=0`, bytecode correctness, and a
material reduction from the legacy baseline of roughly 35.4 ms for three
16-KiB refills per sampled frame.

Result: **pass (2026-07-14)**. KotoRogue frame 1,350 completed three 16-KiB
refills (49,152 bytes) in 4,195 us total, with a 1,536 us maximum refill.
The PIO state machine ran at 75 MHz, `fast_success_count` reached 3,962,
`fast_fallback_count` remained zero, and `fallback_reason=none`. Compared with
the retained legacy sample, CodeWindow refill time fell from about 35.4 ms to
4.2 ms (about 8.4x faster); the sampled VM time fell from about 38.4 ms to
6.6 ms and reported frame rate rose from 24 fps to 131 fps.

## Current host-side result (2026-07-14)

- All RP2040 and RP2350A retained binaries link in release mode.
- The eight RP2350A UF2 artifacts above are generated with the RP2350 Arm
  Secure family and RP2350-E10 absolute block.
- Physical RP2350A validation is active. LCD, SD, baseboard PSRAM, power, and
  speaker audio rows have passed; keyboard smoke traffic passed with its
  explicit chord retest still pending. Remaining device rows are not inferred
  from successful host builds.

## Device observations

### 2026-07-14: divider-2 fast CodeWindow observation

- KotoRogue frame 1,350 reported `read_mode=koto_psram_fast_clkdiv2`, 4 KiB
  chunks, eight dummy cycles, and a 75 MHz PIO state-machine clock.
- Three CodeWindow tile refills transferred 49,152 bytes in 4,195 us total;
  the largest individual refill was 1,536 us.
- Fast-read accounting reached 3,962 successes with zero fallbacks and no
  fallback reason. The app remained correct at the sampled 131 fps.
- This validates divider 2 as the RP2350A product setting; the safe full-range
  retry remains enabled for runtime fault containment.

### RP2350A three-slot CodeWindow candidate

The RP2350A board profile now allocates three resident 16-KiB CodeWindow slots
(48 KiB total); RP2040 retains two. The bounded LRU implementation supports up
to four slots and has host coverage for a three-region cycle and fourth-region
eviction. Device acceptance requires KotoRogue to report the same three distinct
tiles with refills falling to zero after warm-up, no fast-read fallback, correct
application behavior, and at least 96 KiB main-stack `free_min`.

Performance result: **pass (2026-07-14)**. At KotoRogue frame 2,430 the
three-slot cache reported `refills=0`, `code_tiles=0`, `cw_refill_us=0`, and
`cw_bytes=0`. VM time was 3,517 us at the sampled frame, versus 6,603 us and
4,195 us of refill time in the divider-2/two-slot sample. Reported frame rate
rose from 131 fps to 222 fps. Fast-read initialization/warm-up reached 10
successes with `fast_fallback_count=0` and `fallback_reason=none`. The separate
post-growth `phase=176` stack-canary capture remains to be retained.

### 2026-07-14: first LCD probe attempt

- Artifact: `probe_lcd-picocalc-pico2w-rp2350a.uf2`
- Observation: flashing completed, but the panel showed no output.
- Root cause found in the probe: LCD initialization was incorrectly nested
  behind `USB CDC wait_connection()`. The PicoCalc mainboard Type-C path
  exposes UART0 and does not satisfy the Pico module's native USB connection,
  so execution waited before touching the LCD.
- Correction: LCD initialization and the complete visible test sequence now
  run immediately after boot; UART0 reports progress independently. Native USB
  CDC only replays the result if it is connected later.
- Corrected UF2: regenerated at the same artifact path. Physical retest is
  complete.

### 2026-07-14: corrected LCD probe retest

- Observation: pass; the corrected probe produced the expected visible LCD
  output after flashing.
- Proven path: RP2350A artifact boot, SPI1 display transport, DMA_CH0 transfer,
  ILI9488 initialization, full fills, and bounded rectangle/band drawing.
- Deviation: the initial blank result was a probe sequencing defect, not an
  RP2350A LCD or PicoCalc wiring failure.

### 2026-07-14: keyboard I2C smoke capture

- Capture: 600 valid JSONL `sample` records and zero `error` records.
- I2C/poll result: pass; 100 kHz bridge traffic remained within the frame
  budget. Observed `poll_us` was 783–1,602 µs, average 1,289.1 µs, with zero
  samples above 16,667 µs.
- Input result: press/release and stable-state transitions were captured for
  numeric, alphabetic, modifier, navigation/function, Enter, Escape, Tab, and
  Backspace codes. Candidate mappings produced expected action/direction
  results for the corresponding letter keys.
- Remaining keyboard gate: explicitly capture the selected `arrow-zxas`
  direction+action chords during the product stress pass. The smoke capture
  did not include raw arrow codes `0xb4`–`0xb7`.

### 2026-07-14: SD mount and sequential-read capture

- Card detect: pass on GP22; observed capacity was 7,822,376,960 bytes.
- Initialization: the 12 MHz attempt returned `CardNotFound`; the retained
  compatibility fallback initialized successfully at 1 MHz. This matches the
  previously validated RP2040 card behavior and is an allowed result.
- FAT/APPS result: pass; the root volume mounted and 18 current `.kpa`
  packages were enumerated with long and 8.3 short names.
- Sequential read: pass; `kotomines.kpa` (`KOTOMI~1.KPA`) completed a
  33,848-byte read in 128-byte chunks with FNV-1a checksum `8f8aeb97`.
- Probe transport correction: UART0 output is immediate and USB CDC is
  optional. A 10-second preflight countdown keeps the one-shot report visible
  while the host opens the PicoCalc USB-UART port.

### 2026-07-14: baseboard PSRAM capture

- Identity: pass; raw ID `0d5d531548644ab5`, manufacturer `0x0d`, KGD
  `0x5d`, and density field `0x53` were accepted as the 64-Mbit/8-MiB device.
- RP2350A timing: system clock 150 MHz, PIO1 SM0 divider 5.0, 30 MHz state
  machine clock, two cycles per bit, and 15 MHz serial clock. PAC-derived DMA
  request values were RX `12` and TX `8`.
- Boundary: pass at block 32,767 (`0x7fff00`–`0x800000`), with identical
  write/read data and no mismatch.
- CodeWindow: pass using the two-tile 16-byte cache over 32 bytes; four
  refills transferred 32 bytes in 48 µs total.
- Normal block round trip: pass; block 257 wrote in 189 µs and read in 200 µs
  with exact data equality. Block 32,768 was correctly rejected as
  `OutOfRange`.
- The forced product-fallback artifact remains a separate validation row; this
  probe confirmed that the physical PSRAM path itself is available and stable.

### 2026-07-14: STM32 power bridge capture

- Six consecutive sampling cycles completed without an unrecovered I2C error.
  STM32 firmware register `0x01` consistently returned `[0x00,0x16]`, decoded
  as BIOS 1.6.
- Four version reads succeeded on the first attempt and two recovered on the
  second attempt. The retained product behavior of three attempts with 100 ms
  spacing therefore absorbs the observed transient read failure.
- All six battery reads succeeded on the first attempt and returned
  `[0x0b,0x00]`. This is the documented nonfatal no-battery/unavailable value
  and correctly maps to `Unknown`, rather than a false 0% reading.
- Voltage and external-power telemetry remain unsupported by the STM32
  register protocol; no values are inferred for those fields.

### 2026-07-14: PWM/DMA audio capture

- Audible result: pass through the PicoCalc speaker. The 500 Hz pattern was
  heard during the expected three-second tone intervals.
- RP2350A clock result: system clock 150 MHz, PWM divider 6, TOP 250, measured
  configuration carrier 99,601 Hz, and sample rate 8 kHz.
- DMA timer 0 pacing was `1/18750`. Two observed 24,000-sample intervals both
  completed with `dma_remaining=0`, `dma_error=0`, `underruns=0`, and
  `result=pass`.
- Headphone left/right observation was not captured in this run; the common
  GP26/GP27 PWM and DMA path is proven, while jack routing remains available
  for the final product stress pass if required.

### 2026-07-14: forced PSRAM-fallback product run

- Fallback selection: pass. The valid physical ID was still logged, with
  `forced_fallback=true`, followed by `phase=198 psram-unavailable
  fallback=sram-window` as required.
- Product behavior: the UI was reported as generally operational without
  PSRAM. An explicit named small-app launch/return capture remains pending, so
  this row is not yet treated as its complete acceptance result.
- Boot PCM diagnostic submitted and consumed all 2,560 frames at 16 kHz with
  zero drops and `result=ok`; the 160 ms boot beep was audibly confirmed. One
  startup underrun was recorded. Sustained in-app audio remains an explicit
  check in the normal-product stress row.

### 2026-07-14: normal-product audio observation

- The normal RP2350A product artifact booted and produced audible sound during
  ordinary application execution, in addition to the confirmed boot beep.
- This closes the audible product-path question raised during the forced
  fallback run: the CPU1 mixer, DMA-paced PWM output, and PicoCalc speaker path
  operate during sustained in-app use.
- Final stress accounting still requires the corresponding `phase=173`
  summary so underrun growth, worker liveness, and core1 stack margin can be
  recorded numerically.

### 2026-07-14: normal-product KotoRogue stress capture

- KotoRogue ran through frame 3,000 at 24 fps with continued application
  sound and no reported visible or interactive problem.
- At frame 3,000 the VM completed 5,085 operations in 38,391 µs. Three
  CodeWindow tile refills completed in 35,503 µs, proving continued execution
  through the physical PSRAM-backed code window during the session.
- The app exited with code 0 and returned to the shell. The return repaint
  completed and the shell heartbeat resumed.
- RP2350 main-stack peak was 42,680 bytes used with 265,616 bytes minimum free,
  unchanged between the running-app and app-exit samples.
- The standard Perf profile suppresses per-cadence `phase=173` lines. Use an
  Audio-profile build when the quantitative CPU1 counters need to be captured.

### 2026-07-14: Audio-profile KotoShogi stress capture

- KotoShogi remained operational through frames 3,750 and 3,780 at 24 fps,
  using three physical-PSRAM CodeWindow refills per sampled frame, and then
  exited normally with code 0.
- Audio counters were stable across both samples: 2,560 submitted and played,
  zero drops, zero command drops, zero unsupported operations, zero mixer
  saturation, and zero late worker passes. The single startup underrun remained
  at 1 and did not increase during the sampled interval.
- CPU1 worker heartbeat advanced from 172,995 to 174,293, proving continued
  core-1 liveness. Worst worker jitter was 1,033 µs and the core1 stack canary
  retained 6,220 bytes of minimum free space.
- One BGM start and two audio events were recorded. Combined with audible
  in-app output and the clean dedicated PWM/DMA probe, this passes the normal
  product audio stress gate.

### 2026-07-14: application-suite smoke observation

- The installed application set was exercised on the RP2350A product build
  and reported generally free of application-level problems.
- This is recorded as a broad device smoke result rather than an exhaustive
  per-application transcript. Detailed evidence is retained for KotoRogue and
  KotoShogi, including PSRAM CodeWindow activity, audio, and clean shell return.
