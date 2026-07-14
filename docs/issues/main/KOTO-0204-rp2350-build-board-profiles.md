# KOTO-0204: RP2350A Pico 2 W build foundation

- Status: done
- Type: feature
- Priority: P0
- Requirements: FR-SDK-8, NFR-PORT-6, NFR-DEV-5
- Related: KOTO-0008, KOTO-0064

## Goal

Make the product firmware and retained hardware probes compile for the
available RP2350A Pico 2 W without editing dependency feature lists or linker
files by hand, while retaining RP2040. Give each output an explicit board
identity so an incompatible UF2 cannot be mistaken for another PicoCalc
module. RP2350B is deliberately left to KOTO-0206 after this path is proven.

## Acceptance Criteria

- [x] `koto-pico` defines mutually exclusive MCU features for RP2040 and
  RP2350A; selecting zero or multiple MCU features produces an actionable
  compile error.
- [x] A Pico 2 W board profile documents its MCU, 4 MiB flash, 520 KiB SRAM,
  LED/radio capability, and lack of module PSRAM.
- [x] `embassy-rp`, executor/runtime support, and any MCU-specific dependency
  features are selected by the MCU feature instead of hard-coding `rp2040`.
- [x] RP2350A builds use `thumbv8m.main-none-eabihf` and an RP2350 linker/image
  definition; RP2040 continues to use `thumbv6m-none-eabi` and its existing
  boot2/linker layout.
- [x] Static RAM placement, multicore startup, atomic behavior, interrupts,
  DMA channel ownership, and PAC-only code compile for each MCU, with
  MCU-specific implementations or diagnostics gates where registers differ.
- [x] `koto_firmware` and all retained probes compile in release mode for the
  applicable RP2040 and RP2350A/Pico 2 W profiles.
- [x] Each firmware banner and generated UF2 filename contains an unambiguous
  board profile and MCU family.
- [x] The repository check or CI matrix prevents RP2040 or RP2350A from
  silently bit-rotting while keeping ordinary host tests independent of the
  embedded targets.
- [x] README build instructions document exact commands and required Rust
  targets for every supported board profile.
- [x] Existing RP2040 firmware still builds with unchanged default behavior.

## Validation

Validated on 2026-07-14:

- `python harness/check_embedded.py` passed for every retained RP2040 and
  RP2350A binary.
- Release builds of every retained binary passed for both
  `thumbv6m-none-eabi` and `thumbv8m.main-none-eabihf`.
- `tools/build-rp2350a.ps1 -AllBins` generated
  `koto_firmware-picocalc-pico2w-rp2350a.uf2`; official picotool 2.3.0
  identified it as an RP2350 Arm Secure image with its image-definition block
  at `0x10000114`.
- Zero and multiple MCU selections fail with actionable `rp-pac` diagnostics;
  the crate also carries explicit selection guards.
- `python harness/check_project.py` passed. `python harness/check_all.py`
  reaches Clippy but is currently blocked by pre-existing Rust 1.96 warnings
  in `koto-gfx` (`int_plus_one`, `manual_repeat_n`, and test-only warnings),
  outside this issue's scope.

## Notes

- This issue is a compile/toolchain foundation, not a hardware support claim.
- `koto-psram` remains RP2040-only and is now isolated behind the RP2040 MCU
  feature. Porting the PicoCalc baseboard PSRAM path and validating it on the
  device belongs to KOTO-0205. KOTO-0206 owns RP2350B QMI PSRAM.
- Do not add an untestable RP2350B compile requirement here. KOTO-0206 extends
  the same board-capability pattern when matching hardware is available.
- Prefer board capability constants/types over scattered `cfg` expressions in
  product firmware.
