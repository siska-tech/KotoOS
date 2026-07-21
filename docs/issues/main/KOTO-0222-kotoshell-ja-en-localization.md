# KOTO-0222: KotoShell Japanese/English localization

- Status: DONE 2026-07-18
- Type: feature
- Priority: P1
- Requirements: FR-SHELL-2, FR-SHELL-6, FR-SDK-9, FR-CONFIG-1, FR-CONFIG-2, NFR-I18N-1, NFR-I18N-2, NFR-I18N-3, NFR-MEM-2, NFR-REL-1
- Related: KOTO-0123, KOTO-0216, KOTO-0218, KOTO-0219, KOTO-0223
- Roadmap: [KotoConfig Roadmap](../../planning/KOTOCONFIG_ROADMAP.md)

## Goal

Make the current native KotoShell usable by PicoCalc's English-speaking audience
while retaining Japanese support, using the same locale contract exposed to apps.

## Acceptance Criteria

- [x] Inventory every OS-owned user-visible Shell string, including launcher,
  details pane, command bar, categories/sort, status, empty/error states,
  preferences, launch failure, and app-return messaging.
- [x] Replace inline literals with bounded static `en-US` and `ja-JP` resource
  tables; KotoUI continues to receive resolved UTF-8 and owns no catalog lookup.
- [x] Read locale from ConfigService and remove any duplicated global-locale
  field from Shell preferences while preserving favorite/sort/category values.
- [x] The Shell settings command opens KotoConfig; Shell does not implement a
  second locale selector or write global configuration directly.
- [x] Publish the same locale tag/generation through KotoSim and Pico runtime UI
  capabilities so Shell and apps cannot disagree about the active locale.
- [x] Unknown/corrupt locale values fall back to `en-US`; missing individual
  translations also fall back to the English resource and never display a key.
- [x] Add deterministic `qps-ploc` testing with 35–50% ASCII expansion; all
  single-line Shell text clips or uses end-ellipsis within existing geometry.
- [x] Missing glyphs follow U+FFFD then `?`; v1 remains LTR and does not claim
  RTL or complex-script shaping support.
- [x] Simulator scripts and golden/semantic assertions cover English, Japanese,
  unknown fallback, pseudolocale, live switching, reboot persistence, and
  preference migration.
- [x] PicoCalc validation covers first boot in English, switching to Japanese
  and back, package launch/return, long metadata, and persistence across reboot.
- [x] Record static flash/resource growth and Shell SRAM delta; no per-frame
  allocation or full-screen repaint loop is introduced by locale selection.
- [x] Workspace tests, simulator goldens, Pico build, preference fixtures, and
  `python harness/check_project.py` pass.

## Notes

English is the product fallback because PicoCalc's user base is primarily
international. Japanese remains a first-class locale and KotoIME behavior is
unchanged. KOTO-0223 owns selection/persistence; this issue owns complete Shell
resources and reaction to the shared locale. Message pluralization, downloadable
catalogs, RTL layout, and complex-script shaping remain later work.

## Implementation Progress

- `ShellState` consumes `ConfigSnapshot` and retains only the active locale and
  locale generation. KotoSim and Pico apply the same snapshot used for runtime
  `KUC1` capabilities at boot and immediately after KotoConfig changes it.
- One typed, fixed-shape static table per locale owns launcher title, command
  bar, sort/category/status text, details metadata labels, save state, and the
  system/memory view. App names, descriptions, and category values remain
  app-owned UTF-8. Missing table entries cannot compile, while invalid persisted
  locale records are rejected by ConfigService and therefore load `en-US`.
- Launch failure currently has no rendered Shell message; device failures are
  diagnostic-only UART states. App return similarly repaints the localized
  Shell without rendering a transient message. These paths introduce no hidden
  user-visible literals to inventory.
- `qps-ploc` is a deterministic static table with 35–50% aggregate ASCII
  expansion. Header, status, details, memory rows, and command boundaries clip
  within their existing geometry; locale changes allocate nothing per frame.
- Simulator semantic tests cover distinct English/Japanese/pseudolocale frames,
  immediate snapshot switching, corrupt/default English fallback, persisted
  locale restoration, and ignoring a legacy Shell `locale=` preference while
  retaining sort/category/favorites.
- RP2040 release UF2 measurement against detached `HEAD`, built with the same
  command and toolchain: 1,822,208 -> 1,826,304 bytes (+4,096 bytes). The
  64-bit `ShellState` budget remains 29,672 bytes, so measured Shell SRAM delta
  is 0 bytes. RP2040 release compilation passes; PicoCalc interaction and reboot
  persistence were device-confirmed on 2026-07-18.
- KotoConfig can be dismissed with Cancel, a second F1 press, or F10. KotoSim
  recognizes the PC function keys directly; Pico reuses the device-verified F10
  EXIT carriers (`0x90`, plus the retained `0x8a` alias). A host test sweeps all
  256 scan codes and pins Config exit to F1/F10 only.
