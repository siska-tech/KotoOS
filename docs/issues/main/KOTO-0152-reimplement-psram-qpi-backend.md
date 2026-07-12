# KOTO-0152: Reimplement PicoCalc PSRAM QPI backend as a mode-owned backend

- Status: done (the vendored `koto-psram` crate is the default firmware PSRAM backend)

## Background

KOTO-0151 showed that Picoware-style PSRAM QPI transfer primitives work on KotoOS/Rust:

- QPI read stress can pass
- QPI write diagnostics can pass
- `qspi_4wire_write` works up to `write_clkdiv=2`
- `qspi_psram_rw` read works at `read_clkdiv=6/8`

However, firmware integration is unstable:

- app staging verify returns repeated `0x11` / `0x33` / `0xff`
- serial_safe verify can return all zero
- CodeWindow execution fails with `BadInstruction` / `StackUnderflow`
- legacy SPI and QPI paths appear to fight over PSRAM mode, PIO SM state, pin directions, and CS/SCK state

This suggests that the raw transfer primitives are valid, but the backend integration/state management is not.

## Goal

Create a new PSRAM QPI backend that owns all PSRAM bus state and avoids ad-hoc mixing of legacy SPI and QPI access paths.

## Design

Add a new backend, separate from the current legacy backend:

```rust
struct PicoCalcPsramQpiV2 {
    mode: PsramMode,
    // PIO/SM/program offsets/config state
}

enum PsramMode {
    Unknown,
    QpiRw,
    QpiWriteOnly,
    RecoverSerial,
}
````

All public read/write operations must go through this backend.

Do not let app_runtime, CodeWindow verify, or diag code directly reconfigure PIO/SM/pin state.

## Mode transition rules

Only these methods may switch bus state:

```rust
ensure_qpi_rw()
ensure_qpi_write_only()
recover_to_serial()
enter_qpi()
```

Each transition must explicitly handle:

* SM disable
* FIFO clear
* CS high idle
* pin direction reset
* PIO program/config selection
* shift/autopull/autopush config
* input_sync_bypass policy
* PSRAM command mode
* mode state update

## Initial profiles

Use conservative working profiles first:

```text
read:
  qspi_psram_rw
  read_clkdiv=8
  chunk=120

write:
  qspi_4wire_write
  write_clkdiv=2
  chunk=120 or bounded internal chunks
```

Later candidate:

```text
read_clkdiv=6
```

Do not use:

```text
read_clkdiv=4
```

## Required diagnostics

Add app-size QPI self-tests that match firmware staging:

1. write/read 27004 bytes at addr=0
2. write 16384 bytes at addr=0, then 10620 bytes at addr=0x4000, then read 27004 bytes
3. read CodeWindow tile sizes: 16KiB
4. repeated write/read/read/write transitions
5. KotoBlocks bytecode stage simulation

Each log should include:

```text
phase=338 psram-qpi-v2-stress
step=<name>
addr=0x...
len=<n>
mode_before=<mode>
mode_after=<mode>
read_clkdiv=<n>
write_clkdiv=<n>
chunk=<n>
ok=<0|1>
fail_off=<n>
fail_exp=0x..
fail_got=0x..
first16=<hex>
around_fail=<hex>
```

## Firmware integration

Add feature flag:

```toml
psram_qpi_backend_v2 = []
```

Then use it for:

* app staging write
* CodeWindow read
* app-stage-verify
* cw-verify

All through the same backend instance.

No legacy SPI fallback inside normal staging/CodeWindow operation.

## Success criteria

* QPI V2 app-size staging stress passes
* app-stage-verify passes
* cw-verify passes
* cw-map-verify passes
* KotoBlocks boots from PSRAM
* cw_refill_us improves over `pio3v1`