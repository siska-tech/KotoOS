# KOTO-0221: File Note app pilot migration to KotoUI

- Status: done
- Type: feature
- Priority: P1
- Requirements: FR-SDK-1, FR-SDK-2, FR-SDK-4, FR-SDK-5, FR-SDK-9, FR-RT-4, NFR-PERF-1, NFR-DRAW-1, NFR-MEM-2, NFR-REL-1, NFR-I18N-1, NFR-I18N-2
- Related: KOTO-0052, KOTO-0218, KOTO-0219, KOTO-0220
- Roadmap: [KotoUI App ABI Roadmap](../../planning/KOTOUI_APP_ABI_ROADMAP.md)

## Goal

Migrate the existing SDK File Note sample from per-frame immediate drawing to a
small interactive KotoUI form, preserving sandboxed file behavior and proving
the app-facing ABI on an existing application rather than only a conformance
Gallery.

## Implementation Progress

- Rewrote `apps/samples/file_note` as a five-node KotoUI form (panel, status
  Label, 64-byte note TextField, Save/Reload Buttons) that never calls
  `draw_*`. The app ID, package, sandbox path, first-run creation of
  `note.txt` with the original sample bytes, read-back, and the F10/Shift+F5
  exit route are unchanged. All app state (locale asset, parsed resource
  table, note bytes, dirty/status flags, event storage, mount packet) lives in
  one heap block so the deeply inlined SDK validators fit the 45-user-slot
  frame exactly, matching the Gallery's ceiling; every SDK status is checked
  directly in `if` conditions so no scratch local stays live across them.
- Save mounts visible-but-disabled and is enabled by the first `TextChanged`;
  a successful save or reload disables it again in the same atomic KUP1
  packet. Because the host rejects an update whose final focus lands on a
  disabled widget, the Save-button path adds a `RequestFocus` record homing
  focus to the field; submission (focus already on the field) and reload
  (focus on Reload) need none. Reload keeps the field untouched on missing,
  oversized (>64 B), invalid-UTF-8, and read-error outcomes so unsaved text is
  never lost, and refreshes it through `TextValue` only on a normal read.
- Moved all user-visible text into 13-line translator assets
  (`locales/en-US.txt`, `ja-JP.txt`, 35–50% expanded `qps-ploc.txt`) packaged
  as read-only data, parsed into a bounded length table at startup and on live
  `LocaleChanged` events; locale updates re-label the five components plus the
  current status in one packet and never touch the note bytes. Startup spreads
  capability/locale parsing, the sandbox round trip, and mount validation over
  three deterministic frames.
- Added the `koto_ui_file_note` integration suite (14 tests): first run,
  existing note, edit/save/reload byte-exact round trip, disabled-Save focus
  skip, capacity rejection/recovery with a pinned heap high-water mark, a
  deterministic save failure (directory in place of `note.txt`) that preserves
  text, the reload outcome matrix, cancel/exit paths, relaunch persistence
  with a fresh session, an en→ja→qps→en locale cycle, bytecode-rodata
  exclusion of translations, three 320x320 locale goldens, and a trace pinning
  mount damage, repeated-idle zero work, exact per-component edit/status/
  focus damage, and the `TextChanged`/`FocusChanged` response order. The
  KOTO-0178 sweep now documents why KotoUI-authored samples live in their own
  suites instead.
- Extended `check_budgets.py` with a committed edit-then-reload scenario that
  leaves the note bytes unchanged for repeatable gate runs. Measured against
  the immediate-drawing sample: source 996 B → 15.8 KB; KBC 1,428 B →
  148,898 B (inlined SDK validators, PSRAM-resident code on device); VM heap
  106 B → 2,247 B within the unchanged 16,384 B budget; frame fuel 182 →
  22,636 peak (mount frame) under the 60,000 cap; host calls/frame 8 → 9
  peak; retained host session 6,008 B (fixed) with 31 retained commands
  replacing a full-screen rect plus two text draws submitted every frame; the
  idle form now presents nothing and interaction damage is bounded to the
  affected components.
- Documented the pilot as the first application-facing adoption and listed the
  remaining immediate-drawing samples in `docs/spec/SDK_SAMPLES.md` and
  `apps/samples/file_note/README.md`; Memo migration stays a separate later
  decision. PicoCalc validation is the remaining device-only criterion.
- Re-greened the full local gate alongside the pilot: fixed five pre-existing
  `koto-core` clippy failures introduced by toolchain lint drift (redundant
  `drop`, two elidable lifetimes, a `Default` field reassignment, a
  constant assertion) and refreshed the shell golden trace whose committed
  package count (19) predated the Gallery package (20). `check_all.py`,
  `check_project.py`, and the RP2040/RP2350A release firmware builds all pass;
  the golden diff is exactly the intentional one-line package count.
- PicoCalc device validation confirmed 2026-07-17: text entry, Save, Reload,
  persistence across relaunch, error display, exit, and clean return to
  KotoShell behave as in the deterministic simulator scenarios.

## Acceptance Criteria

- [x] Preserve the existing `dev.koto.samples.file-note` identity, sandbox path,
  first-run note creation, read-back behavior, package metadata, and exit route.
- [x] Replace the repeated full-screen `draw_rect`/`draw_text` loop with a KotoUI
  panel containing a text field, Save button, Reload button, and status label.
- [x] Editing and submission update app-owned bounded text; Save writes exactly
  the current valid UTF-8 bytes and reports success/failure without losing text.
- [x] Reload reads the sandboxed file, updates the field through the ABI, and
  reports missing, oversized, invalid, and normal content deterministically.
- [x] Focus order, disabled Save state, activation, cancellation, and exit use
  semantic component events with the same KotoSim/PicoCalc key mapping.
- [x] Localize the form, status, and file-error messages for `en-US` and
  `ja-JP`; unknown locales fall back to English and `qps-ploc` verifies bounded
  labels without changing the saved note bytes.
- [x] An unchanged form yields without UI host calls that repaint; editing,
  status, focus, and button changes damage no more than their component bounds.
- [x] Scripted tests cover first run, existing note, edit/save/reload, capacity
  rejection, file failure, app exit, relaunch, and session reset.
- [x] Compare source/bytecode size, app heap, host session SRAM, frame fuel,
  host-call count, render commands, and dirty traces against the current sample.
- [x] Simulator golden output changes only for the intentional form UI and is
  reviewed alongside the deterministic event/damage trace.
- [x] PicoCalc validation covers text entry, Save, Reload, persistence across
  relaunch, error display, exit, and clean return to KotoShell.
- [x] Document File Note as the first app-facing adoption and list remaining
  immediate-drawing apps; Memo migration remains a separate later decision.
- [x] Workspace tests, `harness/build_apps.py --check`, simulator fixtures,
  budget/golden gates, Pico build, and `python harness/check_project.py` pass.

## Notes

The pilot deliberately adds a useful bounded form while keeping the original
file sandbox demonstration. It must not depend on the host-owned multiline Memo
editor service or expand KotoUI v1 into a general document editor.
