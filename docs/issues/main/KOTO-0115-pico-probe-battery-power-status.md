# KOTO-0115: Pico Probe: Battery And Power Status

- Status: done
- Type: feature
- Priority: P1
- Requirements: FR-SHELL-5, FR-SDK-7, NFR-REL-4

## Goal

Read available battery and external-power information from the PicoCalc STM32
bridge and map it into the existing optional power-status model.

## Acceptance Criteria

- [x] The probe reads the supported STM32 power/battery registers over I2C1 at
  100 kHz and logs the raw register values.
- [x] Available percentage, voltage, and charging/external-power fields map
  into the KOTO-0024 power-status states.
- [x] Unsupported or firmware-dependent fields report `available: false`
  without blocking boot or input polling.
- [x] Bring-up notes record STM32 firmware information when available,
  power-source conditions, and observed values.

## Notes

This is probe 7 from `docs/RP2040_BRINGUP.md` and builds on the 100 kHz STM32
I2C path proven by KOTO-0067.

The ClockworkPi battery register behavior has varied across firmware revisions,
so raw evidence must be retained and unknown values must not be presented as a
valid percentage. Use UART0/GP0 at 115200 8N1 for diagnostics through the
PicoCalc Type-C connection.

Implementation started on 2026-06-20. The `battery_power` probe reads only the
safe, documented STM32 registers: BIOS version `0x01` and battery percentage
`0x0B`. The current protocol does not expose voltage, charging state, or
external-power state, so those capabilities are explicitly logged as
unsupported rather than inferred.

The first physical run at a 250 us register-settle interval returned version
`[0x00,0x00]` and battery `[0x0B,0x00]`. Because the official firmware version
is nonzero (the reference source uses `0x16`), the zero version is treated as
an unprepared response. The follow-up uses the official RP2040 reader's 16 ms
settle interval and no longer accepts version 0.0 as valid evidence.

Reviewing the STM32 update path showed that battery register bit 7 is the
charging flag and bits 0-6 are percentage. A zero register value is the
firmware's default for no battery/unavailable and is also rejected by the
official RP2040 reader; it must not be mapped to a real 0% state. The observed
`[0x0B,0x00]` result is therefore recorded as unavailable.

Physical validation completed on 2026-06-20 with no battery installed. The
STM32 returned `[0x0B,0x00]`, correctly mapping to unavailable rather than a
real 0% state. The probe continued sampling without blocking boot. Charging is
decoded from bit 7 whenever a nonzero battery value is available; voltage and
external-power state remain unsupported by the current register protocol.
