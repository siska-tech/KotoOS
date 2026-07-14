# RP2040 / RP2350 Bring-Up Plan

This document chooses the first embedded Rust HAL backend for the PicoCalc and defines the
minimal device bring-up sequence. It is the research output of
[KOTO-0008](../issues/main/KOTO-0008-rp2040-bringup-plan.md) and resolves the open HAL question in
[TRACEABILITY.md](../planning/TRACEABILITY.md).

It assumes the hardware facts and constraints established in
[REQUIREMENTS.md](../planning/REQUIREMENTS.md) (sections 2.1–2.3, HC-1 through HC-8), the layering rules in
[ARCHITECTURE.md](../architecture/ARCHITECTURE.md), the trait contract in [HAL_API.md](../architecture/HAL_API.md), and the
hardware survey in [Research.md](../planning/Research.md). Where this plan and those documents disagree,
the requirements win and this plan should be corrected.

## 1. Decision Summary

**First backend: `embassy-rp`.** The PicoCalc embedded backend (`koto-hal/pico`) is implemented
against the `embassy-rp` HAL, used in mostly-blocking style for the shell frame loop, with async
reserved for overlapped DMA transfers. The Pico C SDK is **not** a build dependency; it is kept
only as a documented escape hatch for individual peripherals if a pure-Rust driver proves
inadequate, and any such use stays behind a Rust FFI adapter inside the HAL (per NFR-PORT-5).

Status: **accepted (2026-06-13).** No embedded backend code has landed yet; the implementation is
tracked by later issues. The bring-up probes in section 4 are the first embedded work this decision
authorizes.

## 2. Backend Comparison

The candidates are the three named in the acceptance criteria: `embassy-rp`, `rp-hal`
(`rp2040-hal` / `rp235x-hal`), and a thin FFI layer over the Pico C SDK.

| Criterion                                          | `embassy-rp`                                 | `rp-hal` (`rp2040-hal` / `rp235x-hal`)              | Pico SDK FFI                                   |
| :------------------------------------------------- | :------------------------------------------- | :-------------------------------------------------- | :--------------------------------------------- |
| Language / toolchain                               | Pure Rust, `cargo` only                      | Pure Rust, `cargo` only                             | Rust + C, CMake + bindgen in the build path    |
| RP2040 + RP2350 from one crate                     | Yes, via feature flags                       | No, two sibling crates with diverging APIs          | Yes (C SDK is unified) but doubles FFI surface |
| Non-blocking SPI / I2C DMA (NFR-DRAW-2, NFR-REL-3) | First-class async DMA                        | Manual DMA wiring, blocking embedded-hal by default | Mature, but driven from C                      |
| PIO access for PSRAM (HC-3)                        | Built-in PIO driver                          | Built-in PIO driver                                 | `rp2040-psram` (C, header-only) is proven      |
| SD / FAT path                                      | `embedded-sdmmc` (pure Rust)                 | `embedded-sdmmc` (pure Rust)                        | FatFs (C), needs FFI                           |
| Maturity on PicoCalc-class boards                  | High, widely used                            | High, the longest-lived option                      | Highest (most forum projects are C)            |
| Fit with Rust-first policy (NFR-PORT-1, NFR-DEV-4) | Best                                         | Best                                                | Weakest; pulls C into the core build           |
| Frame-loop ergonomics (poll-per-frame input model) | Async adds a learning curve; usable blocking | Most direct for a synchronous loop                  | Direct, but in C                               |

### Why `embassy-rp`

1. **One HAL for both MCUs.** The upgrade path to RP2350 is an explicit project goal
   (REQUIREMENTS 3.3). `embassy-rp` covers RP2040 and RP2350 behind feature flags, so the HAL
   does not fork when the Pico module is swapped. `rp-hal` would require maintaining `rp2040-hal`
   and `rp235x-hal` against two APIs.
2. **DMA is the bottleneck strategy, not an afterthought.** HC-2 forces dirty-rectangle and
   scanline DMA (NFR-DRAW-1, NFR-DRAW-2), and audio needs ring-buffer DMA (NFR-REL-3).
   `embassy-rp`'s async DMA lets the CPU rasterize one scanline buffer while another transfers,
   which is exactly the double-buffer model in Research.md section 2.
3. **PIO is available without C.** PSRAM on RP2040 must be PIO-driven and is never memory-mapped
   (HC-3, NFR-MEM-5). `embassy-rp` exposes the PIO blocks directly, so the block-transfer
   `PsramHal` can be written in Rust, referencing the `rp2040-psram` PIO program for timing.
4. **Keeps the build pure Rust.** `cargo fmt` / `clippy` / `test` stay the only toolchain
   (NFR-DEV-4), and the FFI isolation rule (NFR-PORT-5) is honored by *not* needing FFI at all
   for the first pass.

### When the escape hatch applies

Keep the C SDK option documented but dormant. Reach for a small Rust-FFI adapter (inside
`koto-hal/pico`, never in the core) only if a specific peripheral is blocked in pure Rust — the
likely candidates are PSRAM timing edge cases (fall back to wrapping `rp2040-psram`) or an SD card
compatibility problem that `embedded-sdmmc` cannot resolve (fall back to FatFs). Adopting the C
SDK wholesale is explicitly rejected: it contradicts the Rust-first policy and adds a CMake/bindgen
build that every contributor would carry.

### RP2040 vs RP2350 stance

Develop and validate on the standard-kit RP2040 (Pico 1H, 2MB flash, 264KB SRAM) first, because it
is the binding constraint (REQUIREMENTS 3.3, HC-1). RP2350 is now an active compatibility target,
not only future headroom. Its board profiles, PicoCalc peripheral parity, and Pico Plus 2(W)
onboard PSRAM work are defined in the
[RP2350 / Pico 2 Support Roadmap](../planning/RP2350_SUPPORT_ROADMAP.md). RP2040 remains supported
and continues to define the portable lower-bound memory profile.

## 3. HAL Trait Mapping

Each backend peripheral implements the trait already drafted in [HAL_API.md](../architecture/HAL_API.md). The
mapping below records which `embassy-rp` facility backs each trait so bring-up work has a target.

| HAL trait  | PicoCalc peripheral       | `embassy-rp` facility         | Fixed pins (REQUIREMENTS 2.3)                                |
| :--------- | :------------------------ | :---------------------------- | :----------------------------------------------------------- |
| `VideoHal` | ILI9488 / ST7365P LCD     | SPI1 + DMA, GPIO for DC/RESET | SCK=GP10, MOSI=GP11, MISO=GP12, CS=GP13, DC=GP14, RESET=GP15 |
| `InputHal` | STM32 keyboard (I2C 0x1F) | I2C1, 100kHz+                 | SDA=GP6, SCL=GP7                                             |
| `FsHal`    | MicroSD (SPI0)            | SPI0 + `embedded-sdmmc`       | MISO=GP16, CS=GP17, SCK=GP18, MOSI=GP19, DETECT=GP22         |
| `PsramHal` | 8MB SPI PSRAM             | PIO state machine + DMA       | CS=GP20, SCK=GP21, MOSI=GP2, MISO=GP3                        |
| `AudioHal` | Dual PWM speaker          | PWM slice 5 + timer/DMA       | left=GP26, right=GP27                                        |
| `PowerHal` | STM32 battery gauge       | I2C1 (shared with input)      | SDA=GP6, SCL=GP7                                             |

The pin assignment lives entirely inside `koto-hal/pico` so it never leaks to the core
(NFR-PORT-4).

## 4. Bring-Up Sequence and First Probes

Probes run in dependency order: each one is the smallest test that proves a single peripheral and
logs its result over USB-CDC (section 5). They map directly onto Phase 3 of
[VALIDATION_PLAN.md](../planning/VALIDATION_PLAN.md) and should graduate into permanent device checks. Probes
are throwaway `main`s under a `pico-probes` binary set, not part of the shipped HAL, so they can be
crude.

| #   | Probe                   | Goal                                                             | Method                                                                                                                                 | Pass criteria                                                            | Requirements / constraints               |
| :-- | :---------------------- | :--------------------------------------------------------------- | :------------------------------------------------------------------------------------------------------------------------------------- | :----------------------------------------------------------------------- | :--------------------------------------- |
| 0   | Toolchain + blink + CDC | Prove flashing, runtime, and logging before touching peripherals | Build for `thumbv6m-none-eabi`, flash UF2, blink an LED (or LCD backlight), print a banner over USB-CDC                                | UF2 boots; banner appears in the host terminal                           | NFR-DEV-1, NFR-DEV-2                     |
| 1   | LCD init + fill         | Get any stable image on the panel                                | Select an [LCD profile](LCD_INIT_PROFILES.md), drive SPI1 + DC/RESET, run the controller init, fill solid colors, draw one rect, push one scanline band via DMA | Correct colors, correct orientation, only the requested rect changes; profile logged | NFR-DRAW-1, NFR-DRAW-2, NFR-PERF-2, HC-2 |
| 2   | Keyboard I2C poll       | Read keys with acceptable latency                                | Init I2C1 at 100kHz (try 400kHz), poll address 0x1F non-blocking once per frame, log keycodes                                          | Keys register; bus clock is ≥100kHz; no per-frame stall                  | NFR-PERF-4, NFR-PERF-5, HC-5             |
| 3   | Keyboard chord matrix   | Validate the default game mapping                                | Run the [keyboard matrix validation plan](KEYBOARD_MATRIX.md): hold candidate direction + action chords, log raw and normalized states | A candidate D-pad + A/B/X/Y set has no blocking on real hardware         | FR-SDK-6, NFR-PERF-6, HC-8               |
| 4   | SD mount + read         | Read packages from SD over SPI0                                  | Init SPI0, mount FAT via `embedded-sdmmc`, list `apps/`, sequentially read one `.kpa.json`, exercise the SPI-clock fallback on failure | Known-good card mounts; file reads back; low clock fallback logs cleanly | FR-FS-1, FR-FS-3, NFR-REL-2, HC-6        |
| 5   | PSRAM block round-trip  | Prove PIO block transfer, never pointers                         | Load the PIO program, write a known pattern to PSRAM via `PsramHal::write`, read it back into an SRAM buffer                           | Pattern round-trips; no attempt to dereference PSRAM                     | FR-RT-5, NFR-MEM-4, NFR-MEM-5, HC-3      |
| 6   | [Audio PWM tone](../issues/main/KOTO-0114-pico-probe-pwm-audio-output.md) | Prove the software-mixer output path | Fix PWM slice 5 carrier above audible range, feed a single test tone from a ring buffer via timer/DMA | Audible clean tone, no underrun while the idle shell runs | FR-MML-2, NFR-REL-3, HC-4 |
| 7   | [Battery poll](../issues/main/KOTO-0115-pico-probe-battery-power-status.md) | Read power state, degrade cleanly | Poll the STM32 gauge over I2C1, log percent / millivolts / charging | A value is returned, or `available: false` is reported without error | FR-SHELL-5, FR-SDK-7, NFR-REL-4 |

Notes:

- Probe 1 (LCD) is sequenced first after the toolchain because a working panel turns every later
  probe into a visible result and is the highest-value early win (Research.md section 2, phase 2).
- Probe 3 (chord matrix) produces data, not a binary pass: its output feeds
  [KOTO-0025](../issues/main/KOTO-0025-keyboard-matrix-validation.md) and fixes the default mapping.
- Probe 5 must enforce the architecture rule that PSRAM is only ever reached through block
  transfers into SRAM buffers (ARCHITECTURE.md core rule 4).

## 5. Flashing and Debug Workflow

### Primary path: UF2 over BOOTSEL (no extra hardware)

This is the required workflow (NFR-DEV-1) and needs nothing but a USB cable.

1. Build for the target (`thumbv6m-none-eabi` on RP2040, `thumbv8m.main-none-eabihf` on RP2350).
2. Convert the ELF to UF2 with `elf2uf2-rs` or `picotool`, or wire it as the cargo runner so
   `cargo run` produces and offers the UF2.
3. Enter BOOTSEL on the PicoCalc (hold the module's BOOTSEL while powering / resetting); the board
   mounts as a USB mass-storage volume.
4. Copy the UF2 onto that volume; the board reboots into the new firmware.

### Logging: USB-CDC virtual serial

Debug output goes to USB-CDC (NFR-DEV-2). Two options:

- **Plain CDC line logging** via `embassy-usb`, read with any host terminal (Tera Term, CoolTerm,
  `pyserial`). Simplest, no special tooling.
- **`defmt` over USB-CDC** for compact, structured, level-filtered logs. Preferred once the probes
  grow, because it keeps formatting strings off-device and off the slow link.

### Optional fast path: probe-rs over SWD

If the Pico module's SWD pads are wired to a debug probe (e.g. a second Pico running `debugprobe`,
or a CMSIS-DAP probe), `probe-rs` enables a far tighter loop: `cargo embed` / `cargo run` flashes
directly over SWD, gives RTT logging, and allows real debugging and `panic` backtraces. This is
optional convenience; the project must never *require* a debug probe, since the standard kit ships
without one.

### Workspace wiring this implies

When the embedded backend lands (a later issue, not this one), the following is expected and is
recorded here so bring-up does not rediscover it:

- A `koto-hal/pico` crate (or feature) targeting `thumbv6m-none-eabi` with a `memory.x` describing
  the 2MB-flash / 264KB-SRAM layout.
- A `.cargo/config.toml` setting the target and the UF2 (or `probe-rs`) runner.
- Probe binaries kept out of the host (`koto-sim`) build so `cargo test` on the host stays clean
  (NFR-DEV-4).

## 6. Risks and Open Questions

| Risk / question                                                  | Impact                            | Handling                                                                                  |
| :--------------------------------------------------------------- | :-------------------------------- | :---------------------------------------------------------------------------------------- |
| `embassy-rp` async ergonomics vs a poll-per-frame loop           | Slower initial bring-up           | Use blocking APIs for the frame loop; reserve async for DMA overlap only                  |
| PIO PSRAM timing harder in Rust than the C `rp2040-psram`        | PSRAM probe (5) slips             | Port the proven PIO program; FFI-wrap `rp2040-psram` as the documented fallback           |
| LCD controller variants (ILI9488 vs ST7365P) need different init | LCD probe (1) shows garbled image | Profile contract defined in [LCD_INIT_PROFILES.md](LCD_INIT_PROFILES.md); probe 1 selects and logs a profile |
| Keyboard firmware caps I2C at 10kHz                              | Input latency                     | Probe 2 verifies the achieved clock and logs it; document the working firmware revision   |
| Battery reporting varies across PicoCalc firmware                | Power probe (7) flaky             | `PowerHal` already allows `available: false`; never block boot on it                      |
| SDXC / low-quality SD init failures                              | SD probe (4) fails to mount       | SPI-clock fallback in probe 4; publish a known-good card list                             |
```
