# KOTO-0220: App-authored KotoUI component Gallery

- Status: in-progress
- Type: feature
- Priority: P1
- Requirements: FR-SDK-1, FR-SDK-2, FR-SDK-5, FR-SDK-9, FR-SIM-1, FR-SIM-2, FR-SIM-5, NFR-PERF-1, NFR-DRAW-1, NFR-MEM-2, NFR-I18N-1, NFR-I18N-2, NFR-I18N-3
- Related: KOTO-0052, KOTO-0215, KOTO-0218, KOTO-0219
- Roadmap: [KotoUI App ABI Roadmap](../../planning/KOTOUI_APP_ABI_ROADMAP.md)
- Architecture: [RP2040 Shell / Code-Window Resident Overlay](../../architecture/RP2040_SHELL_CODE_RESIDENT_OVERLAY.md)

## Goal

Create a Koto bytecode sample that exercises every v1 component exclusively
through the app-facing SDK, proving that the ABI is usable and visually
equivalent on KotoSim and PicoCalc.

## Implementation Progress

- Started the source-authored `dev.koto.samples.koto-ui-gallery` application
  descriptor and bounded-memory package. Its first scene uses only the KotoUI
  SDK to mount every v1 component kind; interaction, locale, golden, budget,
  and device validation remain tracked by the acceptance criteria below.
- Added a 9-node, 201-byte data-arena scene with initial focus, a deliberately
  disabled control, list-row descriptors, editable text capacity, and a closed
  modal dialog. The event loop opens/closes the dialog through KUP1 updates and
  never calls `draw_*`.
- Kept the deeply nested SDK expansion within the VM's 45 user-local limit by
  isolating capability validation from mount construction and parsing locale
  assets into one bounded heap allocation. The standard build emitted the
  committed KBC, manifest, icon, and KPA under `sdcard_mock`.
- Added visible status updates for Button activation, Checkbox value changes,
  List selection/activation, TextField edit/submission, capacity rejection, and
  Dialog close/cancel. Dialog visibility and focus now change atomically in one
  two-record KUP1 packet, moving focus into the modal child and restoring the
  launch Button when the modal closes.
- Extended deterministic app scripts with `activate`/`confirm`, which maps to
  the normalized KotoUI activation pulse rather than a text intent. The checked-in
  17-frame scenario exercises modal cancellation, focus restoration, the
  disabled-control skip path, Checkbox/List behavior, and a multibyte `あ` edit.
- Added a simulator integration test that launches the committed package and
  pins the yielded result, `Kotoあ` document, declared heap budget, fuel/heap
  bounds, ten-call per-frame ceiling, and zero low-level draw calls.
- Added bounded parsing for translator-authored 22-line UTF-8 locale assets.
  The shared KUP1 builder updates eight component labels and the List row blob
  without keeping per-string offsets in the VM local frame. Initial `ja`/`ja-JP`
  selection and live `LocaleChanged` transitions are covered by retained-state
  assertions.
- Added simulator session hooks for injecting the shared `ConfigSnapshot` at a
  frame boundary and inspecting retained widget text without exposing KUI1
  encoding. English remains the fallback for unrecognized locale tags.
- Fixed reverse locale updates by clearing the reusable KUP1 data arena before
  rebuilding its variable-offset List blob. Without the clear, bytes from a
  shorter prior locale occupied row-reserved fields and the SDK correctly
  rejected the packet. The integration test now pins a live
  `en-US -> ja-JP -> qps-ploc -> en-US` cycle.
- Expanded mount capacities to the maximum locale layout and added deterministic
  ASCII pseudolocale resources for all eight text-bearing component nodes plus
  three List rows. Every component label checked by retained-state tests is
  35–50% longer than English; pixel containment remains for the golden slice.
- Localized all nine transient semantic status messages in the same text assets.
  Integration coverage activates a Japanese Dialog, observes
  its Japanese status, switches the still-open modal to `qps-ploc`, closes it,
  observes the pseudolocalized status, and finally returns every label to English.
- Revalidated all 105 simulator unit tests, 38 simulator integration/golden tests,
  the committed Gallery package, and the project harness after the locale cycle.
  The current 17-frame English interaction peaks at 2,934 VM heap bytes,
  37,281/60,000 fuel, and ten host calls in one frame after loading the
  translator-authored text asset.
- Added App-session UI trace instrumentation for focused widget ID, KUE records
  polled by the VM, present calls that actually painted, and exact submitted
  damage. It resets per frame without changing the retained command image.
- Pinned four 320x320 RGB565 frame hashes: English normal, English modal,
  Japanese modal, and pseudolocale modal. The trace separately pins initial
  full damage, repeated-idle zero work, modal focus trap/restore, activation and
  cancellation response order, modal/status damage, and focus-transition damage.
- Reduced the List viewport to two rows and extended the scenario to select its
  third row. The trace pins selection responses 1 then 2 and proves the retained
  command image drops row 0 while painting rows 1/2. A second scripted run exits
  cleanly from the edited TextField after exercising the same focus path.
- Added the standard SDK include form `include <sdk/koto_ui.koto>;`. The
  compiler resolves it from the workspace root with traversal-safe `sdk/`
  validation; Gallery, the SDK counter sample, LSP completion, and the language
  specification now use the canonical form instead of App-depth-relative paths.
- Moved all Gallery component/status translations out of Koto source into
  `locales/en-US.txt`, `ja-JP.txt`, and `qps-ploc.txt`. The files are packaged
  as read-only data assets, validated as fixed 22-line UTF-8 resources at
  runtime, and loaded on startup/live locale changes. `app.json` now also has a
  reusable generic `assets` path for App-authored text and data.
- Added deterministic capacity coverage at all three ABI boundaries. A
  32-node mount succeeds and a 33rd-node declaration is rejected atomically;
  Gallery fills its TextField's 64-byte capacity, displays the localized
  rejection, edits again after recovery, and keeps the same heap high-water
  mark; a ten-event burst preserves the eight queued responses, reports the two
  drops in one ordered `CapacityRejected`, drains cleanly, and does not grow memory.
- Extended the common App budget report with fixed retained-KotoUI host SRAM,
  retained rectangle/text command high-water, and observed worst host-frame
  microseconds. Gallery pins 6,008 host-session bytes and 70 commands while its
  existing deterministic bounds remain 2,934 VM heap bytes, 37,281/60,000 fuel,
  and ten host calls. Wall-clock latency is reported but not gated; fuel remains
  the reproducible latency bound. `check_budgets.py` now runs the Gallery
  scenario with explicit heap/fuel/host-call/UI-SRAM/command/slot thresholds.
- Completed the deterministic state matrix with an App-session IME composition
  test. It focuses the SDK TextField, renders an uncommitted romaji composition,
  cancels it without changing the App value, and complements the existing
  activation-pulse, disabled, checked, scrolled, modal, locale, capacity-error,
  damage, and idle assertions.
- Fixed composition-only TextField updates in both simulator and device hosts.
  Because an uncommitted composition intentionally emits no App semantic event,
  the host now presents that retained damage directly without adding an idle
  `ui_present` call to bytecode or changing the public ABI.
- Revalidated the host workspace (excluding the cross-compiled firmware crate),
  all App build artifacts, golden frames, runtime budgets, project harness, and
  both RP2040/RP2350A embedded build profiles. KOTO-0220 is ready for PicoCalc
  validation; that device-only criterion is the sole remaining item.
- Added a localized body Label to the modal Dialog so the KotoSim sample shows
  meaningful content instead of an empty center region. The tenth mount record
  keeps the Close action and focus trap unchanged; bounded construction is split
  across two startup frames to remain below the fixed 60,000-instruction budget.
- Aligned retained TextField measurement with the actual M+ 12px font metrics
  (half-width 6px, full-width 12px, line height 13px) on both KotoSim and
  PicoCalc. Kana input now places the caret immediately after the rendered glyph
  instead of accumulating a four-pixel horizontal error per character.
- Changed inline SKK candidate rendering to replace the reading instead of
  appending beside it, and aligned interactive keys with the conventional flow:
  Space converts/cycles, Enter commits, Ctrl+G cancels, and Ctrl alone is inert.
- RP2040 device validation exposed a resident-SRAM regression: the inserted-SD
  boot first stopped after `phase=13`, and a 12-row mitigation reached the shell
  but measured only `free_min=1536` on Gallery's rejected-launch path; KotoBlocks
  then exhausted the stack after `phase=150`. The root fix overlays the mutually
  exclusive 28,308-byte shell state and 32,768-byte app code window in one
  32,776-byte resident slot, saving about 28 KiB while an app runs. The
  performance-sensitive raster/RGB666 strip is restored to 16 rows and the
  RP2040 two-tile PSRAM code cache is retained. The resulting RP2040 release ELF
  uses 210,556 bytes of `.data + .bss` (about 59.8 KiB before the core-0 stack),
  versus 226,568 bytes even with the temporary 8-row mitigation.
- The shell snapshot occupies an aligned reservation at the top of PSRAM while
  the app is active. Runtime audio assets are capped below that address, so code,
  audio, and launcher state cannot overwrite one another.
- Gallery now polls the normalized lifecycle intent once per frame. F10 in
  KotoSim and Shift+F5 (`0x90`) on PicoCalc therefore exit even though EXIT is
  not a retained KotoUI component event; a direct intent regression test covers
  this independently of Cancel/focus behavior.
- Gallery's fully inlined SDK validation code is 208,988 bytes, above the former
  128 KiB device gate. The PSRAM-resident code ceiling is now 256 KiB; the SRAM
  window remains bounded and its RP2040/RP2350A tile counts are unchanged.

## Acceptance Criteria

- [x] Add a descriptor-driven sample under `apps/samples/` with its own
  `app.json`, package, icon, documented app ID, and bounded memory declaration.
- [x] The sample mounts label, button, checkbox, list, text field, panel, and
  dialog controls without calling low-level `draw_*` for component UI.
- [x] Keyboard navigation reaches every enabled control, skips disabled controls,
  traps/restores modal focus, edits multibyte text, scrolls the list, and exits.
- [x] Visible state reflects semantic events for activation, checkbox change,
  list selection, text edit/submission, dialog accept, and cancellation.
- [x] Every user-visible string has `en-US` and `ja-JP` variants selected from
  the host locale; unknown locales fall back to English and a live
  `LocaleChanged` event remounts/updates the visible strings safely.
- [x] A deterministic ASCII `qps-ploc` variant expands labels by 35–50%; clip or
  end-ellipsis contains every single-line control within its declared bounds.
- [x] Deterministic simulator scenarios cover normal, focused, activation-pulse,
  disabled, checked, scrolled, edited/composing, modal, error, English,
  Japanese, unknown-fallback, and pseudolocale states.
- [x] Golden frames and trace assertions pin pixels, focus order, response order,
  exact damage, and repeated-idle zero-redraw behavior at 320x320.
- [x] Capacity scenarios intentionally reach node/string/event limits and show a
  stable recoverable error without panic, memory growth, or state corruption.
- [x] Runtime budget reporting records VM heap, host session SRAM, fuel, host-call
  count, render commands, and worst scripted interaction latency.
- [x] The normal build/package flow compiles the source and includes the sample
  in `sdcard_mock` without special host-only assets or code paths.
- [ ] PicoCalc validation confirms the same focus, activation, editing, dialog,
  exit, and idle-render behavior as the deterministic simulator scenario.
- [x] SDK sample documentation explains how to run, inspect, and adapt the
  Gallery without treating the ABI encoding as an application API.
- [x] Workspace tests, `harness/build_apps.py --check`, golden checks, budget
  gates, Pico build, and `python harness/check_project.py` pass.

## Notes

This complements the native simulator Gallery from KOTO-0215. The two surfaces
should intentionally share visual semantics, while this sample proves that all
state crosses the VM ABI and SDK boundary.
