# KOTO-0230: SDK text resources and List row builders

- Status: done
- Type: feature
- Priority: P1
- Requirements: FR-SDK-1, FR-SDK-2, FR-SDK-5, FR-SDK-9, FR-RT-4, NFR-PERF-1, NFR-MEM-2, NFR-DEV-3, NFR-DEV-4, NFR-REL-1, NFR-I18N-1, NFR-I18N-2
- Related: KOTO-0193, KOTO-0219, KOTO-0220, KOTO-0221, KOTO-0228, KOTO-0229
- Roadmap: [KotoUI App ABI Roadmap](../../planning/KOTOUI_APP_ABI_ROADMAP.md)

## Goal

Add bounded App-owned SDK abstractions for indexed UTF-8 text resources and
KotoUI List row blobs. Remove repeated newline parsing, linear line-offset
scans, 12-byte row-record encoding, label-offset arithmetic, and row-count
plumbing from normal App code without adding allocation, changing package asset
loading, or revising the KUI1/KUP1 ABI.

## Motivation

KOTO-0229 removed packet record/data cursors from Apps, but KotoUI Gallery and
File Note still implement nearly identical locale-resource parsers and pointer
helpers. Gallery also constructs List row records and relative label offsets by
hand in both its mount and locale-update paths. This leaves ABI layout details
in application code and forces scratch-buffer/alias reasoning next to component
intent.

The abstraction must fit the existing Koto execution model. Records are
top-level static values backed by caller-owned heap storage, methods inline at
call sites, and Gallery currently peaks at 44 of 45 user slots and 58,273 of
its 59,000-instruction gate. SDK convenience must therefore reduce or preserve
the measured cost rather than hide it behind a budget increase.

## Proposed Contract

### Text resources

- Provide an App-owned `TextResource` static record in a focused SDK include.
  It binds caller-owned parsed storage; it does not own an allocation or a
  package asset path.
- `parse(raw, raw_len, dst, dst_capacity, line_capacity)` converts bounded
  LF/CRLF UTF-8 input into an SDK-private indexed representation. Define exact
  behavior for an optional final newline, empty lines, bare CR, malformed
  UTF-8, line-count overflow, per-line representable length, and output
  capacity before implementation.
- Store checked offsets or equivalent metadata so `line_ptr(index)` and
  `line_len(index)` are O(1). Expose the actual line count and retain the first
  parse/access error deterministically.
- Keep `asset_load`, manifest path declarations, locale selection, fallback,
  and product-specific line meanings in the App. A generic text resource must
  not become a hidden locale catalog or filesystem API.

### List rows

- Provide an App-owned `UiListRowsBuilder` static record using caller-owned
  blob storage. `begin` fixes row/blob capacities, `row` appends one enabled
  Boolean, UTF-8 label, and signed `app_value`, and `finish` returns the exact
  blob length or sticky negative status.
- Automatically maintain the 12-byte row table, relative label offsets, actual
  row count, and one checked label cursor. Calls after the first failure leave
  the blob unchanged; successful and failed builders may be explicitly reused.
- Add a resource-line convenience that consumes a completed `TextResource`
  without exposing its internal index layout.
- Add typed `UiMountBuilder`/`UiUpdateBuilder` entry points that consume a
  completed row builder and derive its count/length internally. Preserve the
  existing pointer/length `list` and `list_rows` methods as the compatibility
  and conformance layer.

## Acceptance Criteria

- [x] Add App-owned `TextResource` and `UiListRowsBuilder` static record types
  and receiver methods without a VM opcode, host call, allocator, runtime type,
  KBC change, package-format change, or KUI1/KUP1 revision.
- [x] Text parsing validates all arguments before destination mutation and
  deterministically covers LF, CRLF, optional final newline, empty lines, bare
  CR, malformed UTF-8, zero/full capacities, line-count overflow, and every
  index/output boundary.
- [x] Successful text lookup is O(1), returns the exact UTF-8 pointer/length,
  rejects invalid indexes, and supports repeated parse/reuse plus two
  independent resources without shared mutable SDK state.
- [x] List construction automatically encodes every v1 row field, uses Koto
  `bool` for enabled state, preserves signed `app_value`, and rejects invalid
  capacities, row overflow, label/data overlap, malformed UTF-8, and every
  cursor overflow boundary before sealing.
- [x] The first resource/List-builder failure is sticky, later calls leave the
  caller-owned destination unchanged, and `finish` returns the failure without
  exposing a partially valid blob; reset/reuse and active-builder re-entry are
  explicit and tested.
- [x] Builder output is byte-for-byte identical to representative hand-written
  v1 List blobs, including zero rows, maximum rows, UTF-8 labels, disabled rows,
  negative/positive `app_value`, no selection, valid selection, and rejection
  of a disabled selected row by the existing packet finalizer/host validator.
- [x] `UiMountBuilder` and `UiUpdateBuilder` can consume a completed row builder
  without App-maintained row count, blob length, 12-byte offsets, or per-row
  success checks; existing pointer/length methods remain supported.
- [x] Migrate KotoUI Gallery and File Note resource parsing to `TextResource`,
  migrate Gallery mount/update List construction to `UiListRowsBuilder`, and
  remove their App-local line-offset scans and direct List row-record writes
  without changing visible strings, localization fallback, events, app IDs,
  sandbox behavior, or package assets.
- [x] Record before/after KBC size, App heap request, user-slot peak, fuel peak,
  and host-call count for Gallery and File Note. Both remain at or below the
  45-user-slot ceiling and existing fuel/heap/host-call gates; no budget is
  raised to accommodate SDK wrapper cost.
- [x] Document caller ownership, parsed-storage sizing, resource lifetime,
  exact newline/UTF-8 policy, O(1) lookup, sticky errors, reuse, non-reentrancy,
  List blob lifetime, inline code cost, and continued availability of the
  scalar/pointer compatibility APIs.
- [x] Compiler/LSP analysis and receiver completion cover both included record
  types, typed cross-record parameters, resource-line row helpers, and failed
  builders across root and included sources.
- [x] Format, Clippy, host-workspace tests, App build/package synchronization,
  Gallery/File Note simulator regressions, golden frames, runtime budgets, and
  `python harness/check_project.py` pass.

## Non-goals

- A runtime translation catalog, message formatting/pluralization engine,
  dynamic locale downloads, or moving locale fallback/path selection out of
  Apps.
- Dynamic strings, arrays, allocation, ownership/borrowing, or a general
  collection framework for the Koto language.
- An integrated `list_begin`/`list_row` sub-transaction inside an active mount
  or update builder. A separate caller-owned row builder keeps the first change
  measurable; scratch-free packet-arena construction may be evaluated later.
- Removing or changing the existing low-level resource, List blob, KUI1, or
  KUP1 compatibility surfaces.

## Notes

Prefer an indexed representation with checked 16-bit offsets over the current
u8-length table plus linear prefix scan. The exact internal bytes are SDK-private
and may change, but all capacities and failure behavior must remain documented.

Because Koto functions and methods inline, avoid wrapper-over-wrapper encoding
paths that repeat UTF-8 or scalar validation at every nesting level. Measure the
Gallery mount and locale-update frames early; its existing fuel headroom is the
primary feasibility constraint.

## Implementation Notes (2026-07-17)

- `sdk/koto_ui.koto` now provides caller-owned `TextResource` and
  `UiListRowsBuilder` records. Text uses SDK-private u16 offset/length entries
  for O(1) lookup; List rows use the unchanged 12-byte v1 layout. Both retain
  sticky errors and reset only through `parse` / `begin`.
- `UiMountBuilder.list_builder` and
  `UiUpdateBuilder.list_rows_builder` derive sealed row length/count. The
  pointer/length methods remain the conformance layer, and the existing packet
  finalizers still reject malformed UTF-8, malformed blobs, and a disabled
  selected row.
- Gallery and File Note no longer scan preceding line lengths. Gallery also no
  longer writes row offsets/flags directly. It resolves the startup locale
  before mount and prepares resource rows in the parse frame so its mount and
  locale-update frames stay below the unchanged fuel gate.

Measured release artifacts and deterministic scenarios (the “before” column
is the completed KOTO-0229 baseline):

| App | KBC before → after | Heap request before → after | User slots | Fuel peak before → after | Host calls peak |
| :-- | --: | --: | --: | --: | --: |
| Gallery | 308,024 → 285,334 B | 3,560 → 2,888 B | 44 → 44 | 58,273 → 58,487 | 10 → 8 |
| File Note | 156,170 → 174,074 B | 2,311 → 2,343 B | 44 → 44 | 29,975 → 30,095 | 9 → 9 |

Gallery shrinks because startup no longer mounts English and immediately emits
a full locale update; File Note grows because UTF-8 validation and the indexed
two-pass parse inline into both startup and live-locale paths. No heap, fuel,
host-call, slot, or protocol limit was raised.

Validation completed with `cargo fmt --all -- --check`,
`cargo clippy --workspace --exclude koto-pico --all-targets`,
`cargo test --workspace --exclude koto-pico --no-fail-fast`, both App-specific
simulator suites (26 tests), rebuilt KBC/KPA artifacts, deterministic budget
runs, and `python harness/check_project.py`.
