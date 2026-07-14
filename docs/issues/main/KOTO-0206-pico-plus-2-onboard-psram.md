# KOTO-0206: RP2350B Pico Plus 2(W) and onboard PSRAM backend

- Status: todo
- Type: feature
- Priority: P1
- Requirements: FR-RT-6, NFR-MEM-6, NFR-PORT-6, NFR-DEV-5
- Related: KOTO-0022, KOTO-0069, KOTO-0171, KOTO-0204, KOTO-0205

## Goal

Extend the hardware-validated RP2350A structure to RP2350B, then use the 8 MiB
QMI CS1 PSRAM built into Pimoroni Pico Plus 2 and Pico Plus 2 W as KotoOS's
primary PSRAM store. Preserve the bounded `PsramHal` API and fall back cleanly
to the separate PicoCalc baseboard PSRAM. This is the milestone after the
on-hand Pico 2 W passes KOTO-0204 and KOTO-0205.

## Acceptance Criteria

- [ ] The MCU/board feature structure from KOTO-0204 is extended with an
  RP2350B and Pico Plus 2(W) profile using the correct flash/SRAM linker and
  image definition.
- [ ] The RP2350B board profile describes 8 MiB module PSRAM on QMI CS1 with
  the board-defined chip select (GP47), without leaking that pin into portable
  core code.
- [ ] Firmware initializes the module PSRAM with the `embassy-rp` QMI PSRAM
  driver (or a documented, isolated alternative), verifies a supported device
  and size, and reports the result in the boot log.
- [ ] A bounds-checked adapter implements `PsramHal` for mapped QMI PSRAM;
  raw mapped pointers and unchecked slices do not escape the RP2350 backend.
- [ ] `read`, `write`, and `read_code_window` pass deterministic pattern,
  unaligned/small transfer, end-of-device boundary, and out-of-range tests.
- [ ] The Koto runtime stages bytecode/audio/assets into module PSRAM and runs
  the existing shell/app/audio session with logs proving the module backend
  served CodeWindow and ordinary transfers.
- [ ] Backend selection prefers verified module PSRAM, falls back to PicoCalc
  PSRAM when module initialization fails, and retains the current bounded SRAM
  fallback when neither device is usable.
- [ ] Logs and the memory-status surface distinguish `module-qmi`,
  `picocalc-pio`, and `sram-fallback` and report the selected capacity.
- [ ] A repeated full-range/chunk soak and the existing worst-case app/audio
  session complete without corruption, timeout, or audio regression.
- [ ] Pico Plus 2 and Pico Plus 2 W hardware results are recorded separately
  if their board definitions, radio startup, or flash/PSRAM timing differ.
- [ ] Documentation states that the two physical 8 MiB devices are not yet
  combined into a flat 16 MiB pool and explains the fallback order.
- [ ] RP2040 and ordinary Pico 2 builds do not pull in or reference the QMI
  module-PSRAM backend.

## Notes

- `embassy-rp` 0.10 exposes RP2350 QMI CS1 PSRAM as memory-mapped storage. The
  first KotoOS integration deliberately adapts it to `PsramHal`; changing the
  runtime to allocate or execute from mapped memory is separate future work.
- Coordinate the low-level adapter with the sibling `koto-psram` project only
  if the reusable driver belongs there. KotoOS must not require its RP2040-only
  Embassy feature in an RP2350 build.
- Hardware validation waits for a Pico Plus 2(W); it does not block RP2350A
  support on the Pico 2 W already available.
