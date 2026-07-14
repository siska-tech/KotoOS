# KOTO-0207: SD SPI initialization and transfer throughput

- Status: done
- Type: bug
- Priority: P1
- Requirements: HC-6, FR-FS-1, FR-FS-3, NFR-REL-2
- Related: KOTO-0068, KOTO-0118, KOTO-0121, KOTO-0205

## Goal

Stop treating a failed 12 MHz card-acquisition attempt as proof that the SD
card must run permanently at 1 MHz. Initialize SPI-mode cards at the required
low clock, promote the already initialized card to a validated data-transfer
clock, retain a conservative fallback ladder, and measure sequential
throughput so storage stalls can be distinguished from VM, audio, and display
costs on RP2040 and RP2350 hardware.

## Acceptance Criteria

- [x] Product firmware sends the required idle clocks with chip select high,
  attempts standards-compliant acquisition at 400 kHz, and retains the
  device-proven 1 MHz acquisition fallback before attempting a faster data
  clock.
- [x] Acquisition includes an explicit power-stabilization delay, 160 idle
  clocks, an application-level 500 ms retry deadline, and at most two attempts
  per acquisition clock; diagnostics include attempt number and elapsed time.
- [x] After low-speed acquisition, firmware validates data access up to the
  normal-speed SD SPI ceiling of 25 MHz and falls back through documented lower
  transfer clocks without unnecessarily reacquiring the card.
- [x] Boot diagnostics report acquisition and active transfer clocks
  separately, and report each rejected transfer-clock candidate.
- [x] `probe_sd` follows the same clock policy as product firmware instead of
  maintaining a divergent initialization sequence.
- [x] `probe_sd` reports bytes, elapsed time, and sequential KiB/s for a binary
  package read without including UART payload output in the timed interval.
- [x] Existing FAT mount, package enumeration, preference writes, app launch,
  and streamed package assets continue to use the selected transfer clock.
- [x] RP2040 and RP2350A product firmware and SD probe builds pass.
- [x] A PicoCalc hardware capture records the selected clock and measured
  sequential throughput.
- [x] Product firmware confirms that streamed SLD4 audio remains continuous
  while the full-color image sample loads at the promoted transfer clock.
- [x] Single-sector `CMD17` overhead was evaluated after clock promotion. At
  915 KiB/s with correct product behavior it is not currently material, so no
  speculative multi-block `CMD18` follow-up was filed.

## Notes

- `embedded-sdmmc` 0.9 documents card acquisition at 400 kHz. The previous
  product firmware instead constructed SPI at 12 MHz, retried acquisition at
  1 MHz after `CardNotFound`, and then leaves the bus at 1 MHz.
- The existing RP2040 and RP2350A captures therefore validate the compatibility
  fallback, not the card's maximum post-acquisition transfer clock.
- `embedded-sdmmc`'s `VolumeManager` uses a one-sector block cache, so ordinary
  file reads currently issue single-block transfers. Clock promotion is the
  smallest independently verifiable fix; multi-block filesystem work needs
  its own measurements and corruption/recovery scope.
- Implementation validation on 2026-07-14: `cargo fmt --check`, the repository
  harness, RP2040 release `--bins`, and the RP2350A validation-bundle release
  build all passed. Product hardware throughput/audio validation subsequently
  passed at the selected 25 MHz transfer clock.
- First RP2350A hardware run rejected 400 kHz acquisition with the original
  coarse `sd-card-init-error` diagnostic. The compatibility acquisition was
  therefore restored at 1 MHz while keeping post-acquisition promotion; the
  revised diagnostic includes the concrete `embedded-sdmmc` error.
- RP2350A hardware capture on 2026-07-14: 400 kHz acquisition returned
  `CardNotFound`, 1 MHz compatibility acquisition succeeded, and the initialized
  card validated at a 12 MHz transfer clock. A 33,848-byte `kotomines.kpa`
  sequential read completed in 53,296 us at 620 KiB/s with the expected FNV-1a
  checksum `8f8aeb97`; FAT mounted and all 19 package files were enumerated.
- The next validation build tests 25, 20, 16, 12, 8, 4, and 1 MHz in descending
  order. A candidate must pass both CSD access and a full CRC-checked 512-byte
  boot-sector read; a short register response alone cannot select a marginal
  high clock.
- Follow-up RP2350A capture on 2026-07-14 validated the hardened sequence:
  400 kHz attempt 1 returned `CardNotFound` after 11 ms, attempt 2 acquired the
  card after 43 ms, and the CRC-gated transfer ladder selected 25 MHz. The same
  33,848-byte package read completed in 36,113 us at 915 KiB/s with checksum
  `8f8aeb97`, a 47.6% throughput improvement over the earlier 12 MHz capture
  (620 KiB/s). FAT mount and all 19 package entries remained correct.
- The full-color RGB565 gallery and its looping SLD4 background music passed on
  product firmware with the promoted SD clock; image loading no longer causes
  the earlier audible starvation.
