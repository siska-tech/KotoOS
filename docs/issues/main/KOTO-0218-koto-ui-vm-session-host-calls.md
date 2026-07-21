# KOTO-0218: KotoUI VM session and host calls

- Status: done
- Type: feature
- Priority: P1
- Requirements: FR-SDK-1, FR-SDK-2, FR-SDK-5, FR-SDK-9, FR-RT-3, FR-RT-4, NFR-PERF-1, NFR-DRAW-1, NFR-MEM-2, NFR-PORT-1, NFR-PORT-3, NFR-REL-1, NFR-I18N-1, NFR-I18N-2, NFR-I18N-3
- Related: KOTO-0034, KOTO-0035, KOTO-0214, KOTO-0217, KOTO-0223
- Roadmap: [KotoUI App ABI Roadmap](../../planning/KOTOUI_APP_ABI_ROADMAP.md)

## Goal

Implement the frozen KOTO-0217 contract as one bounded per-app KotoUI session
shared by KotoSim and Pico, with verified host calls, semantic event delivery,
and dirty-rectangle rendering.

## Implementation Progress

- Added the allocation-free KUC1 v1.0 encoder in `koto-core`, backed only by a
  copied `ConfigSnapshot`, and checked its exact 64 bytes against the canonical
  `valid_en_us_capabilities.hex` fixture.
- Added `ui_capabilities` (`0x50`) naming, verifier stack effect `(2, 2)`, VM
  heap-range dispatch, and a default `VmHost` seam. KotoSim and Pico use the
  same encoder and return `NO_MEMORY` for destinations shorter than 64 bytes.
- KotoSim loads the newest persisted KotoConfig snapshot when launching an app;
  Pico passes the boot-loaded snapshot into each device session. A simulator
  VM integration test covers `ja-JP` and locale generation 2 end to end.
- Added the fixed-capacity `UiSession` and strict KUI1 v1.0 mount validator.
  It validates the complete header/node/data graph before mutation, copies text
  and values into a 2 KiB host arena, rejects the canonical malformed fixture
  mutations with stable error classes, and preserves the prior live session on
  every validation failure. Its total size is asserted below the 8 KiB ceiling.
- Added `ui_mount` (`0x51`) naming, verifier stack effect `(2, 1)`, heap-range
  dispatch, and KotoSim/Pico adapters. Pico owns the retained session in static
  storage and clears it at every app launch, so no VM pointer or prior-app text
  survives the session boundary.
- Added the KUP1 validator and `ui_update` (`0x52`) on the same VM/KotoSim/Pico
  path. It rejects duplicate or mismatched properties and invalid final
  hierarchy/modal/list/focus states before mutation, updates only fixed-capacity
  host slots, and records bounded old/new component damage. Byte-identical
  updates remain damage-free.
- Added `ui_present` (`0x53`) and a full retained-tree painter for every v1 node
  kind. KotoSim atomically replaces a dedicated retained rect/text layer; Pico
  builds into the existing immediate scratch and commits only a successful image
  into the retained static layer. The existing Pico old/new command diff decides
  LCD damage, while an idle present performs no rebuild and preserves the prior
  layer. Failed scheduling keeps both KotoUI damage and the prior command image.
- Added the bounded eight-record semantic queue, deterministic KUE1 encoder,
  same-frame input dispatch, locale-generation observation, newest-locale
  deduplication, and the saturating queue-overflow latch.
- Added `ui_poll_event` (`0x54`) and `ui_reset` (`0x55`) to verifier, dispatch,
  simulator, and Pico. Short poll buffers do not dequeue; reset and VM exit/trap
  clear the session and retained backend layer.
- The complete `0x50..0x55` suite is now advertised as Host ABI minor 18; older
  minor-17 bytecode remains accepted.
- Added the common single-line text policy ahead of retained command creation:
  clip stays inside component bounds, end-ellipsis uses U+2026 when advertised
  by the painter and otherwise the longest fitting ASCII `...`, and truncation
  never splits UTF-8. Existing validation rejects RTL/wrap as `UNSUPPORTED`.
- KotoGFX raster text now resolves a missing glyph through U+FFFD and then `?`,
  keeping simulator and device raster output on the same deterministic path.
- Measured `UiSession` at 6,000 bytes on the 32-bit firmware target and 6,008
  bytes on the 64-bit host after adding bounded IME presentation state, with a
  compile-time 8 KiB ceiling assertion.
- Connected IME-enabled TextFields to the existing host `KotoMemoIme` and SKK
  conversion path in both simulator and Pico. Composition/candidate text is
  copied into bounded session storage for painting, committed UTF-8 becomes the
  normal TextChanged event, and focus loss, remount, reset, exit, or trap clears
  ownership and composition without retaining host pointers.
- Added a reproducible Panel + focused Button trace. Its first present has one
  `320x320` dirty rectangle and 8 high-level Painter calls; the shared
  simulator/device stroke expansion retains 15 rectangles plus 2 text commands
  (17 commands total). A subsequent unchanged present has zero commands/damage,
  Button activation adds no paint damage, and a text update damages only the
  declared Button bounds `(10,40 80x20)`. The semantic activation record is
  pollable immediately after `ui_frame_begin`, so representative delivery adds
  zero VM frames of latency.
- On 2026-07-16, all host-runnable workspace tests passed with
  `cargo test --workspace --exclude koto-pico --no-fail-fast`; both product
  firmware profiles passed their native ARM-target `cargo check` and release
  build, and `python harness/check_project.py` passed. An unqualified host
  `cargo test --workspace` still attempts to assemble the ARM-only Pico
  `embassy-rp` executor for Windows (`sev`) and is therefore not itself a valid
  Pico test invocation.
- Added one canonical composite scene covering all seven node kinds. Tests now
  verify shared-adapter painting, open-Dialog focus trapping and action routing,
  disabled-control focus skipping, List activation with its preserved
  `app_value`, and Checkbox value-change damage limited to its own bounds.
- Canonical mutations now cover unsupported format, invalid UTF-8, node kind,
  hierarchy, and zero geometry while proving the prior session stays unchanged.
  KotoSim maps these failures to stable `BAD_ARGUMENT`, `UNSUPPORTED`, and
  `NO_MEMORY` results without replacing its retained scene. KotoVM also has an
  explicit verifier regression proving Host ABI minor 17 remains accepted.
- Replaced the session's parallel control behavior with KotoUI event routing.
  Each frame snapshot is normalized once into ordered `UiAction` values;
  Button, Checkbox, List, and TextField receive those through their
  `handle_event` methods, while traversal, disabled/hidden filtering, and Dialog
  scope trapping use `FocusManager`. The returned `UiResponse` and component
  damage are translated back into the fixed KUE1/session representation. A
  dedicated UTF-8 regression covers Japanese insertion, cursor movement, and
  backspace through the TextField implementation. Simulator and Pico continue
  to call the same `UiSession::frame_begin` path.
- Final validation on 2026-07-16 passed all host-runnable workspace tests,
  simulator fixtures/goldens, both native firmware target checks, formatting,
  and the 320-document/229-issue project harness. VM lifecycle tests now assert
  one UI teardown for exit and one for trap, and overflow rendering explicitly
  covers long English plus the `qps-ploc` locale-change path.

## KOTO-0216 Baseline Comparison

Release measurements below use the same optimized product-firmware feature set
as KOTO-0216: `ram_interpreter,ram_audio_mixer`. The RP2040 image uses
`board-picocalc-pico` / `thumbv6m-none-eabi`; Pico 2 W uses
`board-picocalc-pico2w` / `thumbv8m.main-none-eabihf`. The KOTO-0216 comparison
predates the later Host ABI, KotoConfig, and IME work, so its delta represents
the current integrated tree rather than KotoUI code in isolation.

| Measure | KOTO-0216 Pico | KOTO-0218 Pico | Delta | Pico 2 W current |
| :-- | --: | --: | --: | --: |
| Release `.text` | 347,060 B | 419,320 B | +72,260 B | 413,436 B |
| Release `.rodata` | 411,776 B | 414,040 B | +2,264 B | 414,000 B |
| Release `.data` | 54,588 B | 61,760 B | +7,172 B | 61,080 B |
| Release `.bss` | 175,032 B | 177,080 B | +2,048 B | 193,472 B |
| Static SRAM (`.data + .bss`) | 229,620 B | 238,840 B | +9,220 B | 254,552 B |
| `UiSession` (32-bit target) | -- | 6,000 B | +6,000 B | 6,000 B |
| Representative retained commands | 16 | 17 | +1 | 17 |
| Idle commands / dirty rectangles | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |
| Event delivery, additional VM frames | -- | 0 | -- | 0 |

The ABI renderer uses the existing 80-command `AppStaticLayer` ceiling; it does
not enlarge that static layer. KotoShell's separate 16-command render-list
capacity remains unchanged. `UiSession` remains below its frozen 8 KiB limit.

## Acceptance Criteria

- [x] Implement ABI constants, node/event codecs, and validation in a `no_std`
  compatible owner that does not introduce a dependency from `koto-ui` to the VM.
- [x] Add the selected host-call IDs to KotoVM naming, verifier stack effects,
  dispatch, diagnostics, and the documented Host ABI minor.
- [x] Mounting copies validated descriptors/text into fixed-capacity host-owned
  session storage; invalid input leaves the prior session unchanged.
- [x] Updates address stable widget IDs, reject type/shape mismatches, preserve
  valid focus/edit state, and damage only declared old/new component bounds.
- [x] Present routes through the existing KotoUI painter/damage adapter and the
  established simulator/Pico render paths; an unchanged session emits no render
  commands or LCD damage.
- [x] Frame input is normalized once and dispatched through KotoUI focus/control
  events without a simulator- or device-specific component path.
- [x] Event polling returns deterministic fixed records, preserves documented
  ordering, and applies the specified bounded-queue overflow policy.
- [x] `ui_capabilities` returns the current bounded locale, LTR direction, and
  generation identically on simulator/device; a generation change queues one
  `LocaleChanged` event and consumes only the read-only ConfigService snapshot,
  without retaining pointers into configuration storage.
- [x] Validate and implement inherited/explicit LTR plus clip/end-ellipsis;
  reject reserved RTL/wrap modes as `UNSUPPORTED`, and use the specified
  U+FFFD-then-`?` missing-glyph fallback.
- [x] App exit, VM trap, relaunch, and switching packages clear all session,
  focus, modal, text, and queued-event state before another app can observe it.
- [x] Heap range, UTF-8, node, hierarchy, geometry, capability, and version
  failures use stable error codes without panic or partial mutation.
- [x] Existing ABI-minor-17 bytecode and all low-level drawing/Game2D calls retain
  behavior and require no package rebuild solely for this feature.
- [x] Tests cover every node type, malformed canonical fixtures, update rollback,
  input/event order, disabled controls, modal focus, idle damage, event overflow,
  app teardown, locale fallback/change, long English/pseudolocale text, and
  simulator/Pico parity boundaries.
- [x] Record firmware code size, session/static SRAM, maximum render commands,
  idle/interaction dirty traces, and representative event latency against the
  KOTO-0216 baseline.
- [x] Workspace tests, `cargo check -p koto-pico --target thumbv6m-none-eabi`,
  simulator fixtures, and `python harness/check_project.py` pass.

## Notes

The session is OS-owned because the host must retain focus and pressed/editing
state between VM frames. Its capacities are ABI-visible limits, not growable
collections selected independently by each backend.
