# KOTO-0223: KotoConfig foundation and language settings

- Status: DONE 2026-07-18
- Type: feature
- Priority: P1
- Requirements: FR-CONFIG-1, FR-CONFIG-2, FR-CONFIG-3, FR-SHELL-6, FR-SDK-9, NFR-I18N-1, NFR-I18N-2, NFR-MEM-2, NFR-PORT-1, NFR-PORT-3, NFR-REL-1
- Related: KOTO-0123, KOTO-0217, KOTO-0218, KOTO-0222
- Roadmap: [KotoConfig Roadmap](../../planning/KOTOCONFIG_ROADMAP.md)

## Goal

Implement the native KotoConfig settings application and shared bounded
ConfigService, delivering English/Japanese selection as the first setting while
leaving a capability-gated page boundary for future system features.

## Acceptance Criteria

- [x] Add a `no_std`-compatible ConfigService model with fixed-capacity public
  settings, stable keys, validators, nonzero change generations, and read-only
  snapshots; it contains no UI, filesystem, VM, or board dependencies.
- [x] Define a versioned, bounded, checksummed public settings format and
  simulator/Pico storage adapters. Missing, incomplete, oversized, corrupt, or
  unsupported records load safe defaults without partial application.
- [x] Implement KotoConfig as an OS-owned native KotoUI surface with a fixed
  page registry, keyboard focus, semantic controls, damage-only repainting, and
  clean return to KotoShell.
- [x] Register a Language page with self-identifying `English` and `日本語`
  choices; a fresh/invalid configuration selects `en-US`.
- [x] Applying a language persists it, immediately relocalizes KotoConfig,
  increments the locale generation, and exposes the same snapshot to Shell and
  runtime UI capabilities without requiring a reboot.
- [x] A mounted app receives one coalesced `LocaleChanged` event through the
  KOTO-0218 contract; an app that starts afterward reads the current `KUC1` tag.
- [x] Shell favorites, category, sort, and pane preferences remain logically
  separate and survive migration unchanged; global locale has one source of truth.
- [x] The page registry filters on explicit composite capabilities, is allocation
  free, and omits unsupported pages without empty headings or dangling focus.
- [x] Public app/runtime access is read-only and allowlisted; ordinary KPA apps
  cannot mutate ConfigService or enumerate backing storage.
- [x] Tests cover default, both languages, live switch, generation wrap,
  persistence, torn/corrupt/unknown records, unknown locale fallback,
  `qps-ploc` injection, unsupported page filtering, exit, and relaunch.
- [x] Record static flash, ConfigService/KotoConfig SRAM, maximum render
  commands, dirty traces, and write sizes against RP2040 budgets.
- [x] Workspace tests, simulator golden/semantic fixtures, Pico build, storage
  fixtures, and `python harness/check_project.py` pass.

## Notes

Follow [KotoConfig Architecture](../../architecture/KOTOCONFIG_ARCHITECTURE.md).
Secret storage and NetworkService are extension interfaces only in this issue;
no Wi-Fi driver, scanning, credential entry, or network connection is added.

## Implementation Progress

- Increment 1 added `koto_core::config`: a roughly 240-byte fixed-capacity
  ConfigService, separate configuration/locale generations, strict `KCF1`
  codec, compatible unknown-key preservation, and composite capability page
  filtering. Nine focused unit tests pass.
- The portable native `KotoConfigUi` now provides keyboard focus, semantic
  English/Japanese selection, immediate shared-service mutation, pseudolocale
  labels, bounded damage, and an exit action. Four focused tests and pinned
  English/Japanese 320x320 framebuffer hashes cover the initial surface.
- The simulator now persists complete snapshots in alternating checked slots
  and falls back to the older valid slot or English defaults after corruption,
  truncation, or oversize input. Two adapter tests pass.
- Increment 2 adds matching Pico root-FAT slots (`KCFGA.BIN`/`KCFGB.BIN`),
  bounded startup restore, alternate-slot save, and diagnostics. RP2040 and
  Pico 2 W firmware profiles cross-check without enabling Wi-Fi.
- KotoShell now advertises semantic Settings/F1 and both simulator and firmware
  enter the native KotoConfig language screen, save actual changes, repaint only
  reported damage, and return cleanly to Shell.
- KOTO-0218 now publishes the boot/current ConfigService snapshot through
  `KUC1`; mounted sessions coalesce locale-generation changes into one
  `LocaleChanged` event. Gallery and File Note integration tests exercise live
  English, Japanese, and pseudolocale transitions.
- KOTO-0222 completed the shared-snapshot Shell localization and preference
  migration. Simulator and firmware apply the same snapshot immediately after
  KotoConfig changes it, and PicoCalc interaction plus reboot persistence were
  device-confirmed on 2026-07-18.
- A focused host regression pins `ConfigService` at 236 bytes,
  `KotoConfigUi` at 608 bytes, a maximum 280-byte KCF1 write, 13 high-level
  render commands, zero idle dirty rectangles, and one 304x304 panel damage
  rectangle for an applied locale change. The asserted ceilings are 256 bytes,
  1 KiB, 280 bytes, and 16 commands respectively.
- The A/B slot selection and replacement policy now lives in portable
  `koto-core` and is shared by simulator and Pico adapters. Its fault matrix
  covers both slots missing, either slot unreadable, a corrupt replacement,
  stale/newer generations, and the corresponding safe write destination. The
  simulator persistence tests pass and both firmware profiles compile against
  the same policy.
- The 2026-07-18 RP2040 release ELF records `.text` at 430,720 bytes,
  `.rodata` at 415,936 bytes, and `.data` at 66,244 bytes. Including boot2 and
  the vector table, the static flash payload is 913,348 bytes.
- Final validation on 2026-07-18 passed all host-runnable workspace tests,
  simulator fixtures and KotoConfig goldens, RP2040 and Pico 2 W release builds,
  formatting, storage fault fixtures, and `python harness/check_project.py`.
