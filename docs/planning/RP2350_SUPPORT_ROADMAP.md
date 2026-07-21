# RP2350 / Pico 2 Support Roadmap

This roadmap promotes RP2350 from an unspecified future extension to an active
compatibility target for KotoOS. It covers Raspberry Pi Pico 2 / Pico 2 W and
Pimoroni Pico Plus 2 / Pico Plus 2 W modules installed in a ClockworkPi
PicoCalc.

The immediate implementation target is **RP2350A on a Raspberry Pi Pico 2 W**,
because that is the hardware currently available for build/flash/test loops.
The first milestone is feature parity with the RP2040 firmware on that module.
RP2350B and Pico Plus 2(W) onboard PSRAM follow after the RP2350A path is
hardware-validated. RP2040 remains supported and remains the lower-bound memory
profile; RP2350 support must not turn its larger SRAM or optional module PSRAM
into a requirement for portable core code or Koto apps.

## Why This Is Now P0

PicoCalc users commonly replace the original Pico with an RP2350 module. A
firmware distributed only for RP2040 therefore asks those users to swap back to
slower hardware. In addition, Pimoroni Pico Plus 2 variants include 8 MiB of
PSRAM that KotoOS currently cannot discover or use.

The original implementation gap was:

- `koto-pico` enables `embassy-rp/rp2040` unconditionally;
- `.cargo/config.toml` and `memory.x` only describe `thumbv6m-none-eabi`, 2 MiB
  flash, and 264 KiB SRAM;
- the default `koto-psram` dependency enabled an RP2040-only Embassy backend;
- product firmware always initializes the PicoCalc baseboard PSRAM on
  GP2–5/GP20/GP21 through PIO1;
- several PAC-level DMA and PIO diagnostics encode RP2040 assumptions.

KOTO-0204/KOTO-0205 addressed these items. The vendored `koto-psram` backend
now selects RP2040 or RP2350A explicitly, including the MCU-specific DMA
transfer-count register form and divider-2 fast CodeWindow QPI/RX-DMA path.

## Supported Board Matrix

| Module | MCU / build | Module PSRAM | KotoOS PSRAM policy |
| :----- | :---------- | :----------- | :------------------ |
| Pico / Pico W | RP2040, `thumbv6m-none-eabi` | None | PicoCalc 8 MiB PIO/QPI backend |
| Pico 2 / Pico 2 W | RP2350A, `thumbv8m.main-none-eabihf` | None | PicoCalc 8 MiB PIO/QPI backend |
| Pico Plus 2 / 2 W | RP2350B, `thumbv8m.main-none-eabihf` | 8 MiB on QMI CS1 (PSRAM CS on GP47) | Prefer module PSRAM; fall back to PicoCalc PSRAM |

Implementation priority is the order shown: retain RP2040, bring up RP2350A on
the available Pico 2 W, then extend the proven RP2350 structure to RP2350B and
its module PSRAM. RP2350B is not part of the first RP2350A release gate.

The PicoCalc baseboard PSRAM and Pico Plus 2 module PSRAM are separate 8 MiB
devices. The first implementation selects one primary `PsramHal`; combining
them into a 16 MiB address space is intentionally deferred until there is a
measured application need and a region allocator that can represent different
latency and failure characteristics.

## Architecture Decisions

The feature and source boundary is specified in
[`BOARD_PROFILES.md`](../architecture/BOARD_PROFILES.md).

1. **One firmware crate, explicit board features.** `koto-pico` exposes
   mutually exclusive `board-picocalc-pico` and `board-picocalc-pico2w`
   profiles. Each profile owns its internal MCU feature, physical wiring,
   capacities, and clocks. The same structure later gains Pico Plus 2(W) with
   KOTO-0206. Each artifact name and startup banner identifies its profile.
2. **Keep the portable contract.** QMI-mapped PSRAM is exposed to the runtime
   through `PsramHal`/`PsramBlocks`. Raw mapped pointers do not escape the
   RP2350 adapter. This preserves bounds checks, fallback behavior, and the
   simulator/runtime architecture. A future RP2350-only allocator can be
   proposed separately with measurements.
3. **Prefer module PSRAM on Pico Plus 2.** It avoids consuming a PIO state
   machine and the PicoCalc PSRAM pins for the primary store. If module PSRAM
   detection or verification fails, firmware attempts the already supported
   PicoCalc PSRAM and logs the selected backend.
4. **Do not require Wi-Fi.** Pico 2 W and Pico Plus 2 W are supported as compute
   modules first. Network functionality remains out of scope, and the radio
   must not be required to boot KotoOS. KOTO-0224 separately designs the
   capability-gated NetworkService/KotoConfig boundary so a later Wi-Fi build
   can expose settings without changing this offline boot guarantee.
5. **Keep conservative defaults until measured.** RP2040 timing constants,
   DMA request IDs, SRAM hot-section choices, and stack canary thresholds are
   not copied blindly to RP2350. Each is either made MCU-specific or revalidated.

## Delivery Sequence

| Phase | Issue | Outcome |
| :---- | :---- | :------ |
| 1 | [KOTO-0204](../issues/main/KOTO-0204-rp2350-build-board-profiles.md) | RP2350A Pico 2 W firmware and probes compile with an explicit linker/board profile. |
| 2 | [KOTO-0205](../issues/main/KOTO-0205-rp2350-picocalc-peripheral-parity.md) | The available Pico 2 W passes LCD, keyboard, SD, audio, power, USB logging, and PicoCalc PSRAM checks. |
| 3 | [KOTO-0206](../issues/main/KOTO-0206-pico-plus-2-onboard-psram.md) | RP2350B support is added and Pico Plus 2(W) module PSRAM is verified and used through the runtime PSRAM contract. |

Phase 1 and Phase 2 form the first release milestone and can be completed using
the Pico 2 W currently on hand. Phase 3 begins only after that RP2350A path is
stable; its hardware acceptance criteria remain open until a Pico Plus 2(W) is
available.

## Validation Matrix

Every supported artifact must pass a compile gate. Hardware claims require a
recorded device run; a successful cross-build alone is not a support claim.
The first active gate is the RP2350A column. The RP2350B column is the next
milestone and does not block declaring the tested Pico 2 W profile supported.

| Gate | RP2040 Pico | RP2350A Pico 2(W) | RP2350B Pico Plus 2(W) |
| :--- | :---------- | :---------------- | :---------------------- |
| Release compile + UF2 generation | Required | Required | Required |
| Boot banner identifies MCU/board | Required | Required | Required |
| LCD / keyboard / SD / audio / power smoke | Existing baseline | Required | Required |
| PicoCalc PSRAM round trip | Existing baseline | Required | Fallback path required |
| Module QMI PSRAM ID, boundary, and soak tests | N/A | N/A | Required |
| Shell/app/audio session and stack canary | Existing baseline | Required | Required |

## Upstream Facts Used By This Plan

- Pimoroni specifies RP2350B, 16 MiB flash, and 8 MiB PSRAM for both
  [Pico Plus 2](https://shop.pimoroni.com/products/pimoroni-pico-plus-2) and
  [Pico Plus 2 W](https://shop.pimoroni.com/products/pimoroni-pico-plus-2-w).
- The official Pico SDK includes `pimoroni_pico_plus2_rp2350` and
  `pimoroni_pico_plus2_w_rp2350` board definitions:
  <https://github.com/raspberrypi/pico-sdk/releases>.
- `embassy-rp` 0.10 supports RP2350A/RP2350B and provides a QMI CS1 PSRAM
  driver for APS6404L-compatible devices:
  <https://docs.embassy.dev/embassy-rp/0.10.0/rp235xb/embassy_rp/psram/index.html>.
