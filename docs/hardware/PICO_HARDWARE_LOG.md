# PicoCalc Hardware Bring-Up Log

This log records observations from physical PicoCalc probes. A probe binary
building successfully is not treated as proof that the peripheral works on the
device.

> Note: the probe binaries below were later renamed and pruned. The retained
> probes now build as `probe_lcd`, `probe_keyboard`, `probe_sd`, `probe_psram`,
> `probe_power`, and `probe_audio` — see
> [`src/koto-pico/README.md`](../../src/koto-pico/README.md). The historical
> `--bin` names in the entries below are preserved as a record. The obsolete
> `bootstrap`, `blink_cdc`, `blink_cdc_pico_w`, `keyboard_matrix`, and
> `device_probe` experiments moved to `src/koto-pico/bringup/archive/`.

## KOTO-0065: Blink And USB CDC

- Firmware: `blink_cdc`
- Target: standard Pico 1H / RP2040
- Build: `cargo build -p koto-pico --bin blink_cdc --target thumbv6m-none-eabi`
- UF2 generation: verified on host (237,056-byte debug UF2)
- Visible output: GP25 LED, 250 ms on / 750 ms off
- USB identity: `KotoOS RP2040 probe`, serial `KOTO-0065`
- Expected first line: `KotoOS KOTO-0065 blink+cdc v0.1.0`
- Expected heartbeat: `KotoOS probe alive` every two seconds
- Flashing method: copied generated UF2 to the BOOTSEL mass-storage volume
- Observed LED result: pass; visible LED blink confirmed on hardware
- Observed USB CDC result: pass; banner followed by repeated two-second
  heartbeats
- Host terminal: Tera Term
- Observed output:

  ```text
  KotoOS KOTO-0065 blink+cdc v0.1.0
  KotoOS probe alive
  KotoOS probe alive
  KotoOS probe alive
  KotoOS probe alive
  ```

Notes:

- GP25 is the visible LED on the standard Pico 1 / Pico 1H. Pico W modules use
  the separate `blink_cdc_pico_w` firmware below.
- The CDC banner is sent again whenever the host reconnects.

### Pico W / Pico WH Variant

- Firmware: `blink_cdc_pico_w`
- Build: `cargo build -p koto-pico --bin blink_cdc_pico_w --target thumbv6m-none-eabi`
- Visible output: CYW43439 GPIO 0 LED, 250 ms on / 750 ms off
- Radio wiring: power GP23, data GP24, CS GP25, clock GP29
- USB identity: `KotoOS Pico W probe`, serial `KOTO-0065-W`
- Expected first line: `KotoOS KOTO-0065 Pico W blink+cdc v0.1.0`
- Expected heartbeat: `KotoOS Pico W probe alive` every two seconds
- Flashing method: pending physical validation
- Observed LED result: pending physical validation
- Observed USB CDC result: pending physical validation

The CYW43 PIO wiring and GPIO-0 LED path follow Embassy's official Pico W
`wifi_blinky` example. Wi-Fi networking is not enabled by this probe.

## KOTO-0066: LCD Fill

- Firmware: `lcd_fill`
- Status: pass
- Build: `cargo build -p koto-pico --bin lcd_fill --target thumbv6m-none-eabi`
- Selected profile: `ili9488-spi`
- Detection method: manual
- SPI clock: 20 MHz
- Wire format: RGB666 (`COLMOD=0x66`)
- Initial orientation/color order: `MADCTL=0x48` (MX + BGR)
- Expected sequence: full red, green, blue, and black fills; colored corner
  markers; centered yellow rectangle; cyan 8-scanline band at y=200
- Expected USB identity: `KotoOS LCD fill probe`, serial `KOTO-0066`
- Observation date: 2026-06-21
- Observed orientation/color result: pass; landscape orientation and RGB color
  order matched the expected corner-marker pattern
- Observed partial-update result: pass; the corner markers and centered
  rectangle had clean, unclipped boundaries
- Observed DMA result: pass; the cyan band was continuous at y=200 with the
  expected height of eight scanlines
- Visible artifacts: none attributable to drawing or address-window handling
- Controller/profile observation: manual `ili9488-spi` selection validated;
  controller or panel label was not independently read

## KOTO-0067: Keyboard I2C

- Firmware: `keyboard_i2c`
- Status: pass
- Build: `cargo build -p koto-pico --bin keyboard_i2c --target thumbv6m-none-eabi`
- I2C: I2C1, SDA=GP6, SCL=GP7, address `0x1F`, 100 kHz
- Protocol: write FIFO register `0x09`, then read `[state, key]`
- Poll cadence: 16 ms; firmware logs measured `poll_us` against a 16,667 us
  frame budget
- Stable sample threshold: five consecutive unchanged held-key samples
- Output: USB-CDC JSONL for `arrow-zxas`, `wasd-jkui`, and `ijkl-zxas`
- Expected USB identity: `KotoOS keyboard I2C probe`, serial `KOTO-0067`
- Observed STM32 firmware/version: pending
- Power requirement: the Pico USB connection powers the RP2040 probe but did
  not power the keyboard STM32. The PicoCalc mainboard Type-C supply must be
  connected and the PicoCalc power button used before opening the probe COM
  port.
- Initial 100 kHz result while the mainboard was off: `fifo_register_write`
  failed because address `0x1F` did not acknowledge.
- Observed 10 kHz result with the mainboard powered: pass. FIFO press and
  release events were received correctly; pressing `1` produced raw code `49`.
- Observed 10 kHz latency: fail against the frame budget. Stable samples took
  about 22.2 ms and frames draining press/release events took about 44.5 ms,
  exceeding the 16,667 us target.
- Observed 100 kHz powered retest: pass with a 250 us register-to-read
  interval. Pressing `6` produced raw code `54`; stable/changed samples measured
  approximately 2.1-4.4 ms, below the 16,667 us frame budget.
- The three otherwise-identical JSONL sample records are intentional: the same
  held-key state is evaluated independently against `arrow-zxas`, `wasd-jkui`,
  and `ijkl-zxas`. Unmapped text keys have an empty `detected` array.
- `arrow-zxas` chord `up+A`: pass for this individual chord. Raw codes
  `[122, 181]` represented `z+ArrowUp`, stabilized as
  `detected=["up","action_a"]`, and remained within the frame budget at about
  2.1-4.4 ms per poll.
- `arrow-zxas` chord `up+left+A`: all three raw keys and normalized buttons
  were captured, but the initial changed sample took 21.952 ms while draining
  repeated FIFO events. Firmware was revised to process at most four FIFO
  events per frame, leaving excess events queued for the next frame.
- Bounded FIFO retest: `up+left+A` was decoded as
  `detected=["up","left","action_a"]` in 4.477 ms. The changed sample is within
  budget.
- Stable `up+left+A` confirmation: pass. Five unchanged samples produced
  `detected=["up","left","action_a"]` with a 2.126 ms poll, followed by a
  stable empty state after release.
- Observed chord-matrix result: pass on 2026-06-21; the final-capture firmware
  completed all 44 required cases for the first candidate without terminal
  failure
- Selected default mapping: `arrow-zxas`
- Final selection record:
  `{"kind":"selection","status":"pass","selected_candidate":"arrow-zxas","reason":"first_passing_candidate"}`
- Final-capture firmware: `keyboard_matrix`; prompts for all 44 required chord
  cases, requires five stable samples, retries failures three times, and emits
  candidate and selection JSONL summaries

## KOTO-0068: SD Mount And Read

- Firmware: `sd_read`
- Status: pass
- Build: `cargo build -p koto-pico --bin sd_read --target thumbv6m-none-eabi`
- Wiring: SPI0, MISO=GP16, CS=GP17, SCK=GP18, MOSI=GP19, detect=GP22
- Fast SPI attempt: 12 MHz
- Compatibility fallback: 1 MHz after reinitializing the card
- Filesystem: FAT16/FAT32 through `embedded-sdmmc`
- Procedure: list `apps/` with long filenames, select the first
  `*.kpa.json`, and stream it in 128-byte chunks over USB CDC
- Expected USB identity: `KotoOS SD read probe`, serial `KOTO-0068`
- Card model: TOSHIBA 8GB SDHC Memory Card
- Observed capacity: 7,822,376,960 bytes
- Observed clock result: 12 MHz returned `CardNotFound`; 1 MHz fallback passed
- Observed FAT mount: pass
- Observed `apps/` listing: pass; 15 `.kpa.json` long filenames were listed
- Observed sequential read: pass; `kotomines.kpa.json` opened as
  `KOTOMI~1.JSO` and completed a 1,545-byte read in 128-byte chunks

## KOTO-0069: PSRAM Round-Trip

- Firmware: `psram_roundtrip`
- Status: pending physical validation
- Build: `cargo build -p koto-pico --bin psram_roundtrip --target thumbv6m-none-eabi`
- Wiring: PIO1, MOSI=GP2, MISO=GP3, CS=GP20, SCK=GP21
- Capacity model: 8 MiB
- Block API size: 256 bytes
- Initial serial clock: approximately 16.6 MHz
- Transfer implementation: PIO FIFO, split into 16-byte protocol transactions
- Test location: block 257
- Procedure: write deterministic SRAM pattern, read back into a separate SRAM
  buffer, compare every byte, and verify block index 32768 is rejected
- Initial observed write timing: 2.459 ms
- Initial observed read timing: 1.627 ms
- Initial range check: pass
- Initial round-trip result: fail at byte 0
- Follow-up diagnostic: log the first 16 expected and actual bytes to
  distinguish all-zero/all-`FF` reads from bit-order or sampling errors
- Logging transport changed to UART0 TX on GP0, 115200 8N1. The PicoCalc
  mainboard Type-C port can now power the board and carry probe output through
  its USB-UART bridge; the Pico USB port is only needed for UF2 flashing.
- UART diagnostics are repeated every five seconds because the first boot
  report can finish before the host opens the PicoCalc USB-UART COM port.
- Diagnostic bytes showed a one-bit left rotation for every byte:
  expected `5aa3ec35...`, actual `b547d86a...`. This identifies the extra
  high-speed read clock as the cause rather than power or wiring. The follow-up
  firmware uses the non-fudge PIO read phase at 16.6 MHz.
- The non-fudge phase still advanced the stream by one bit. The next firmware
  removes the unsampled clock before the first MISO sample and pre-decrements
  the read-bit count in software.
- Final round-trip result: pass; all 256 bytes matched exactly
- Final write timing: 2.439 ms per 256-byte block
- Final read timing: 1.618 ms per 256-byte block
- Final out-of-range result: pass; block index 32768 returned `OutOfRange`
- Final PIO decision: sample the first MISO bit immediately and provide the PIO
  loop counter as `read_bits - 1`

## KOTO-0115: Battery And Power Status

- Firmware: `battery_power`
- Status: pending physical validation
- Build: `cargo build -p koto-pico --bin battery_power --target thumbv6m-none-eabi`
- Logging: UART0 TX on GP0, 115200 8N1 through PicoCalc Type-C
- I2C: I2C1, SDA=GP6, SCL=GP7, address `0x1F`, 100 kHz
- Safe registers: BIOS version `0x01`, battery percentage `0x0B`
- Unsupported by current STM32 register protocol: battery voltage, charging
  state, and external-power state
- Sampling interval: 5 seconds
- Initial 250 us settle result: version raw `[0x00,0x00]`, battery raw
  `[0x0B,0x00]`. Version 0.0 is inconsistent with the official nonzero BIOS
  version and is treated as an unprepared response, so the apparent 0% battery
  is not yet accepted as valid evidence.
- Follow-up register-settle interval: 16 ms, matching the official RP2040
  reader
- Confirmed firmware result: unavailable; register `0x01` returned
  `[0x00,0x00]`
- Confirmed battery result: unavailable; register `0x0B` returned
  `[0x0B,0x00]`. Zero is the STM32 default for no battery/unavailable and is
  rejected by the official RP2040 reader, so it is not treated as 0%.
- Charging capability: available when battery data is nonzero; bit 7 is the
  charging flag and bits 0-6 are percentage
- Physical power condition: no battery installed; PicoCalc powered through
  mainboard Type-C
- Final mapping result: pass; `[0x0B,0x00]` mapped to unavailable, not 0%

## KOTO-0114: PWM Audio Output

- Firmware: `pwm_audio`
- Status: pending physical validation
- Build: `cargo build -p koto-pico --bin pwm_audio --target thumbv6m-none-eabi`
- Logging: UART0 TX on GP0, 115200 8N1 through PicoCalc Type-C
- Output: PWM slice 5, channel A=GP26, channel B=GP27
- Carrier configuration: divider 6, TOP 250, approximately 83.0 kHz at the
  default 125 MHz system clock
- PCM path: KOTO-0023 software mixer into a 1024-byte-aligned, bounded
  256-sample DMA ring containing exactly 16 cycles of the test tone
- Sample rate: 8 kHz, RP2040 DMA timer 0 pacing
- Test signal: deterministic 500 Hz triangle wave, three seconds on and two
  seconds silent
- Initial observation on 2026-06-21: fail; the async timer feeder reported
  24,000 underruns for 24,000 samples with maximum lateness of 181-183
  microseconds against a 125-microsecond sample period
- Root cause: Embassy async timer wake latency was incorrectly used as the PCM
  sample clock
- Corrective implementation: DMA channel 0 writes packed PWM compare values
  from the ring, paced by DMA timer 0 at 8 kHz; diagnostics report remaining
  transfers and DMA bus errors
- Corrective build physical result: pass on 2026-06-21; repeated 24,000-sample
  intervals completed with `dma_remaining=0`, `dma_error=0`, `underruns=0`,
  and `result=pass`
- Observed speaker/headphone result: pass; the 500 Hz tone was clearly audible
  through the PicoCalc speaker
