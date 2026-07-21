# KOTO-0232: KotoUI compile-time packet capacity helpers

- Status: done
- Type: feature
- Priority: P1
- Requirements: FR-PKG-3, FR-SDK-1, FR-SDK-2, FR-SDK-5, FR-RT-4, NFR-MEM-2, NFR-DEV-3, NFR-DEV-4, NFR-REL-1
- Related: KOTO-0217, KOTO-0219, KOTO-0229, KOTO-0231
- Follow-up: [KOTO-0233](KOTO-0233-koto-buffer-capacity-at-ui-builder-call-sites.md)
- Roadmap: [KotoUI App ABI Roadmap](../../planning/KOTOUI_APP_ABI_ROADMAP.md)

## Goal

Let an App declare a KotoUI packet from its semantic record count and data-arena
capacity while the SDK/compiler own the KUI1/KUP1 header, record-stride, and
packet-limit arithmetic. Packet storage must remain statically sized and all
invalid capacities must fail before execution.

## Motivation

KOTO-0231 left Apps with constants such as `GALLERY_MOUNT_BYTES = 787`. The
number was correct, but its relationship to `record_capacity`, the builder's
derived `data_offset`, and the reserved data arena was not visible at the call
site. Repeating `header + records * stride + data` in every App would expose
wire-layout details already owned by the SDK.

Apps should instead state only the transaction-specific facts:

```koto
const GALLERY_MOUNT_RECORDS = 10;
const GALLERY_MOUNT_DATA_CAPACITY = 267;
const GALLERY_MOUNT_BYTES = ui_mount_capacity(
    GALLERY_MOUNT_RECORDS, GALLERY_MOUNT_DATA_CAPACITY);
buf gallery_mount_packet[GALLERY_MOUNT_BYTES];
```

The data capacity is still App-owned because it follows the App's strings,
field values, and list rows. Official samples document the byte-level semantic
sum beside each capacity instead of retaining unexplained safety padding.

## Contract

- `ui_mount_capacity(records, data_capacity)` computes
  `40 + records * 48 + data_capacity`.
- `ui_update_capacity(records, data_capacity)` computes
  `32 + records * 32 + data_capacity`.
- Mount accepts 1..32 records, 0..2048 data bytes, and at most 4096 total bytes.
- Update accepts 1..16 records, 0..2048 data bytes, and at most 2048 total bytes.
- A top-level `const` may call either helper with integer literals or prior
  integer constants. A `buf` size may name a prior positive integer constant.
- Compile-time calls outside the bounds above produce a source diagnostic.
- Ordinary runtime calls use the checked SDK functions and return
  `UI_SDK_BAD_ARGUMENT` for an invalid capacity.
- Compiler folding reads the canonical `koto-core` KotoUI layout limits. It is
  a narrow SDK-backed facility, not general const-function evaluation.
- The helpers change neither the KUI1/KUP1 wire format nor builder ownership,
  submission, presentation, or failure semantics.

## Acceptance Criteria

- [x] Both capacity helpers return the exact KotoUI v1 packet size for valid
  record/data capacities.
- [x] Invalid record, data, or total packet capacities are rejected at compile
  time and by the runtime SDK path.
- [x] Helper results can initialize a top-level constant and size a local
  `buf` without runtime arithmetic.
- [x] Compiler limits are sourced from `koto-core` rather than a second set of
  host-layout literals.
- [x] Gallery, File Note, and the SDK Counter example use semantic record/data
  constants and contain no App-authored packet-layout formulas.
- [x] Official App data capacities record their derivation and remove obsolete
  slack without changing visible behavior.
- [x] Compiler, LSP, simulator, budget, formatting, and project consistency
  checks pass without raising a VM, UI-session, or protocol limit.

## Completion Evidence

- Compiler coverage folds both helpers through prior constants, sizes buffers,
  executes the runtime forms, and pins invalid-boundary diagnostics.
- LSP definition coverage resolves the helper to `sdk/koto_ui/abi.koto` through
  the public `sdk/koto_ui.koto` aggregator.
- Gallery's largest locale arena is the 195-byte qps-ploc payload; File Note's
  largest sync and locale arenas are 83 and 57 bytes. Their deterministic heap
  requests fall from 2,888 to 2,735 bytes and from 2,343 to 2,259 bytes.
- Gallery's 11 and File Note's 15 simulator tests pass with unchanged golden
  frames. Deterministic fuel, host-call, and user-slot peaks remain unchanged.
- `cargo fmt --all -- --check`, targeted warning-clean Clippy, core/compiler/LSP
  tests, `python harness/check_budgets.py`, and
  `python harness/check_project.py` pass. Workspace-wide Clippy remains blocked
  by two unrelated pre-existing `koto-psram` warnings.

## Notes

Filed retrospectively on 2026-07-17 from review of KOTO-0231. The implementation
landed with that issue's final ergonomics pass; this issue preserves the helper
contract and its separate rationale for future language/SDK maintenance.
