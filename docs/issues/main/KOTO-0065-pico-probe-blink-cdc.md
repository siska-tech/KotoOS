# KOTO-0065: Pico Probe: Blink And USB CDC

- Status: done
- Type: feature
- Priority: P0
- Requirements: NFR-DEV-1, NFR-DEV-2

## Goal

Prove that the embedded toolchain can build, flash, boot, and print logs before
peripheral bring-up begins.

## Acceptance Criteria

- [x] A probe binary builds for the RP2040 target.
- [x] The probe emits a USB-CDC banner with build or git information.
- [x] The probe toggles a visible LED or equivalent safe output.
- [x] Bring-up notes record the observed result and flashing method.

## Notes

This corresponds to probe 0 in `docs/RP2040_BRINGUP.md`.

The `blink_cdc` firmware now builds for RP2040 and contains the GP25 blink plus
USB CDC banner/heartbeat implementation. Physical flashing, LED observation,
and host CDC observation remain pending and are tracked in
`docs/PICO_HARDWARE_LOG.md`; this issue must not be marked done until those
results are recorded.

`blink_cdc_pico_w` provides the equivalent probe for Pico W / Pico WH modules,
where the onboard LED is CYW43439 GPIO 0 rather than RP2040 GP25. It keeps the
same USB CDC behavior and initializes only enough of CYW43 to drive the LED.

Host-side verification:

- `cargo build -p koto-pico --bin blink_cdc --target thumbv6m-none-eabi --offline`
- `elf2uf2-rs` produced a 237,056-byte `blink_cdc.uf2`.

Hardware observation:

- Tera Term displayed the version banner and repeated `KotoOS probe alive`
  heartbeats.
- The visible LED blink was confirmed on hardware.
- The generated UF2 was flashed by copying it to the BOOTSEL mass-storage
  volume.

## Resolution

The RP2040 probe now has separate standard Pico and Pico W binaries. Physical
validation passed for UF2 flashing, visible LED blinking, USB CDC enumeration,
the version banner, and repeated heartbeat output. KOTO-0066 can use USB CDC as
its primary diagnostic channel.
