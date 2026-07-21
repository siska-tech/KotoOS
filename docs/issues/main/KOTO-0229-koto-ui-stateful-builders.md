# KOTO-0229: KotoUI stateful App builders

- Status: done
- Type: feature
- Priority: P1
- Requirements: FR-SDK-1, FR-SDK-2, FR-SDK-5, FR-SDK-9, FR-RT-4, NFR-PERF-1, NFR-MEM-2, NFR-DEV-3, NFR-DEV-4, NFR-REL-1, NFR-I18N-1, NFR-I18N-2
- Related: KOTO-0104, KOTO-0193, KOTO-0219, KOTO-0220, KOTO-0221, KOTO-0228
- Roadmap: [KotoUI App ABI Roadmap](../../planning/KOTOUI_APP_ABI_ROADMAP.md)

## Goal

Add an application-facing stateful builder layer above the existing bounded
KUI1/KUP1 record encoders. Let Koto Apps construct mounts and updates without
repeating the packet pointer, capacity, record index, data cursor, and
per-operation status plumbing, while preserving caller-owned storage, exact
wire compatibility, deterministic failure, and existing runtime budgets.

## Motivation

KOTO-0219 removed hard-coded wire offsets from normal App code, but its named
functions remain low-level record encoders. The current Gallery performs 12
mount-builder calls, 20 update-builder calls, and 46 explicit status checks;
File Note performs 7, 13, and 53 respectively. This obscures component intent,
makes record/data cursor mistakes easy, and leaves boolean ABI fields exposed
as positional integers.

KOTO-0228 now provides static typed records and receiver methods suitable for
App-owned builder state. The abstraction must remain honest about that model:
builders are bounded App-lifetime values, not dynamically allocated objects,
and their methods still participate in the compiler's inline code/slot cost.

## Proposed Contract

- Provide separate `UiMountBuilder` and `UiUpdateBuilder` structs in
  `sdk/koto_ui.koto`. An App declares as many top-level static instances as it
  needs; the SDK must not hide one mutable global singleton.
- `begin` binds a caller-owned packet, capacity, record capacity, and the
  mount-specific root/focus values. Record capacity fixes the boundary between
  the forward record table and data arena without reserving the protocol-wide
  maximum for every App.
- Node/property methods automatically consume the next record index, copy or
  reserve payload bytes through one checked data cursor, and retain the first
  error as a sticky status. Calls after failure do not mutate the packet.
- `finish` validates and seals the actual record count and packet length, then
  returns that length or the retained negative status. It does not call
  `ui_mount`, `ui_update`, or `ui_present`; serialization and host submission
  remain separate observable operations.
- Builder boolean parameters use Koto `bool`; conversion to the KUI1/KUP1
  numeric representation happens inside the SDK boundary.
- A builder may not be re-entered or carried across `yield_frame` between
  `begin` and `finish`. This restriction and the reset/reuse behavior must be
  explicit and covered by examples.
- Existing `ui_mount_begin`, `ui_mount_add_*`, `ui_mount_finish`,
  `ui_update_begin`, `ui_update_set_*`, and `ui_update_finish` functions remain
  supported for compatibility and exact-byte conformance tests.

## Acceptance Criteria

- [x] Add App-owned `UiMountBuilder` and `UiUpdateBuilder` static record types
  and receiver methods without adding a VM opcode, host call, allocator,
  runtime type, KBC format change, or KUI1/KUP1 ABI revision.
- [x] `begin` validates packet/record capacities before mutation and initializes
  automatic record and data cursors; independent static builder instances do
  not share state.
- [x] Mount methods cover Label, Button, Checkbox, List, TextField, Panel, and
  Dialog, including checked payload copy/reservation and typed boolean options.
- [x] Update methods cover Text, Enabled, Visible, Checked, Selection,
  TextField value, Bounds, List rows, Dialog open, and focus request, including
  typed boolean options and automatic record indexing.
- [x] The first builder failure is sticky, later calls leave the packet
  unchanged, and `finish` returns that failure without sealing or submitting
  the packet. A successful `finish` returns the exact host-call length.
- [x] Builder output is byte-for-byte identical to the existing low-level
  encoders for representative packets and every v1 node/property kind; tests
  also cover full capacities, record/data overlap, UTF-8 payloads, repeated
  reuse, two independent builders, and every cursor overflow boundary.
- [x] Migrate `sdk/examples/koto_ui_counter.koto` to the new layer, then replace
  the common mount/update construction paths in KotoUI Gallery and File Note
  without changing their visible behavior, localization, events, app IDs,
  sandbox behavior, or committed package contents beyond rebuilt bytecode.
- [x] The migrated examples contain no manually maintained successful-path
  record indices and no per-record success checks; one final builder result is
  checked before each mount/update submission.
- [x] Record before/after KBC size, App heap request, user-slot peak, fuel peak,
  and host-call count for Counter, Gallery, and File Note. All existing runtime
  limits remain satisfied, Gallery/File Note do not exceed the 45-user-slot
  ceiling, and any size increase is explained rather than hidden by a budget
  increase.
- [x] Document caller ownership, record capacity versus actual count, sticky
  errors, reuse, non-reentrancy/no-yield rules, separation from submission,
  inline code cost, and continued availability of the low-level encoders.
- [x] Compiler/LSP analysis and completion cover the included structs and
  methods, and compiler/VM tests exercise successful and failed builders across
  root and included sources.
- [x] Format, Clippy, host-workspace tests, App build/package synchronization,
  Gallery/File Note simulator regressions, runtime budgets, and
  `python harness/check_project.py` pass.

## Notes

A fluent chained API is intentionally out of scope because KOTO-0228 does not
support storing or returning struct aliases. Receiver methods mutate the
explicit top-level static builder and return only scalar status/length values.

The existing low-level functions are the compatibility and conformance layer;
the new records are an ergonomic orchestration layer. Implementations must be
measured because all Koto functions and methods inline at call sites. If a
wrapper-over-wrapper design increases code or slot pressure materially, the
builder should share lower-level encoding helpers or encode directly rather
than weakening runtime budgets.

## Implementation Notes (2026-07-17)

- `sdk/koto_ui.koto` now defines App-owned `UiMountBuilder` and
  `UiUpdateBuilder` static records. Their receiver methods own record/data
  cursors, typed Boolean conversion, sticky status, exact-length sealing, and
  explicit reset through the next `begin`; submission remains separate.
- Counter, KotoUI Gallery, and File Note use the stateful layer on their common
  mount/update paths. Gallery prepares its localized row blob and yields before
  `begin`, then completes the non-reentrant builder transaction in one frame.
- Compiler VM tests compare stateful and compatibility mount bytes across every
  v1 node kind, exercise every update method, UTF-8, exact/full capacities,
  record/data cursor overflow, sticky failure, reuse, and two simultaneous
  independent builders. The checked-in Counter is also compiled and executed
  through its first mounted frame. Koto LSP tests resolve the included builder
  type and receiver completion.

Measured release artifacts and deterministic scenarios:

| App | KBC before → after | Heap request before → after | User slots before → after | Fuel peak before/last-recorded → after | Host calls peak |
| :-- | --: | --: | --: | --: | --: |
| Counter | 77,237 → 82,660 B | 426 → 490 B | 43 → 43 | not previously gated → 15,044 | 5 → 5 |
| Gallery | 266,166 → 308,024 B | 2,944 → 3,560 B | 45 → 44 | 37,281 → 58,273 | 10 → 10 |
| File Note | 148,898 → 156,170 B | 2,247 → 2,311 B | 45 → 44 | 22,636 → 29,975 | 9 → 9 |

The Counter/File Note heap increases are the two explicit 32-byte builder
records. Gallery additionally keeps a 384-byte parsed mount resource and a
168-byte locale-update row scratch region so builder payload copies never
alias the packet being written. KBC growth is the visible cost of call-site
inline cursor/common-argument checks; no runtime budget or protocol limit was
raised. Gallery remains below its existing 59,000 fuel gate and both migrated
Apps remain below the 45-user-slot ceiling.
