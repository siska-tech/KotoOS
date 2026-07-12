# KOTO-0124: Pico Shell System Status

- Status: done
- Type: feature
- Priority: P1
- Requirements: FR-SHELL-5, FR-SDK-7, NFR-REL-4

## Goal

Connect real PicoCalc storage, battery/power, save-health, and available clock
data to the shared KotoShell header and status strip.

## Acceptance Criteria

- [x] SD present/absent/error state maps to the shared storage indicator.
- [x] Supported battery percentage and charging state map to `PowerState`.
- [x] Unsupported battery or clock data remains visibly unknown without
  blocking boot.
- [x] Low-battery state can be surfaced before write operations.
- [x] Only status pixels whose values change are retransmitted.

## Notes

Builds on KOTO-0115 and KOTO-0120. The current STM32 protocol may leave voltage,
external power, or time unavailable; parity means matching shared fallback
behavior, not inventing data.

Increment 1 integrates the validated STM32 battery register into the product
firmware at a five-second poll interval. The clock remains unset because the
available protocol exposes no clock source. Live power changes repaint only the
right-hand system-status cluster in the header.

Physical testing with a connected battery still reports an I2C read abort after
the register-select write. The product firmware now uses the blocking path from
the validated KOTO-0115 probe, retries three times, logs the concrete
`embassy-rp` abort reason, and preserves the previous displayed state across a
transient failure.

The observed abort is `NoAcknowledge`. Follow-up now mirrors the successful
probe's complete transaction order by reading STM32 firmware version register
`0x01` before battery register `0x0B`, with bounded 100 ms retry spacing. A
failed retry sequence no longer decodes an untouched zero buffer as real data.

The product firmware also uses the active-low SD detect signal on GP22. Removal
immediately changes the shared indicator to `SD×` and disables launches and
writes. Reinsertion changes the indicator to `SD?`; automatic FAT remount is
deliberately unsupported in this increment, so reboot is required before
storage access resumes.

Physical validation confirmed removal and reinsertion transitions. Each status
change retransmitted only the 120x20 header cluster (`dirty_px=2400`) in 4 ms.
Before the BIOS update, the installed STM32 firmware returned version
`[0x00,0x00]` and NACKed battery register `0x0B`. After updating to BIOS v1.6,
the product firmware reads `[0x00,0x16]` as version 1.6 and receives a valid
register response `[0x0B,0x00]`. The zero payload maps to `Unknown`, not a false
0% battery reading. Connecting the battery then produced `[0x0B,0x80]`, which
correctly maps to `Charging { percent: Some(0) }` and renders a charging 0%
indicator. The PicoCalc's adjacent hardware LED also blinked while charging.
