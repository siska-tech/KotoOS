# KOTO-0123: Pico Shell Actions And Preferences

- Status: done
- Type: feature
- Priority: P1
- Requirements: FR-SHELL-2, FR-SDK-2, FR-SDK-4, NFR-REL-1

## Goal

Wire the shared KotoShell command bar behavior and persistent launcher
preferences into the PicoCalc firmware.

## Acceptance Criteria

- [x] PicoCalc keys trigger launch, favorite, sort, category, and details-pane
  actions using the shared shell state machine.
- [x] Favorites, sort mode, category filter, and details-pane preference
  survive reboot.
- [x] Preference writes use a bounded, recoverable file format and do not
  corrupt the package catalog on interruption.
- [x] Command availability and visible labels match KotoSim for equivalent
  shell state.

## Notes

Depends on KOTO-0121. Persistence should use the same logical preference model
as KotoSim while keeping the device storage adapter separate.

Increment 1 hardware validation confirmed F2=`0x82`, F3=`0x83`, and F4=`0x84`;
favorite, sort, and category changes were visible on the PicoCalc. Arrow
selection dirty redraw measured 28 ms.

Increment 2 uses the root-level 8.3 file `SHELLPRF.TXT`, bounded to 2304 bytes.
The file starts with `version=1` and ends with `end=1`; missing, oversized,
invalid UTF-8, or incomplete files are ignored so boot falls back to safe
defaults. Preference I/O never opens or modifies `APPS` or `ICONS`.

Physical validation showed `phase=143 prefs-saved` after favorite changes,
followed by `phase=142 prefs-applied` after reboot. The reboot catalog scan
still accepted all 15 manifests and loaded all 15 icons before applying the
preferences.
