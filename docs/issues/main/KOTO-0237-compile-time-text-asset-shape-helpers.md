# KOTO-0237: Compile-time text asset shape helpers

- Status: done
- Type: feature
- Priority: P3
- Requirements: FR-PKG-3, NFR-MEM-2, NFR-DEV-3, NFR-DEV-4, NFR-REL-1
- Related: KOTO-0183, KOTO-0191, KOTO-0220, KOTO-0230, KOTO-0232, KOTO-0233, KOTO-0235, KOTO-0236, KOTO-0238

## Goal

Remove hand-maintained line counts and line-range payload capacities from
Apps that load manifest-declared UTF-8 text assets by adding two compile-time
helpers:

```koto
asset_text_line_count("path", ...)
asset_text_max_range_bytes(first_line, line_count, "path", ...)
```

The helpers inspect the exact verbatim package assets accepted by `asset_load`
and use the same text shape as `TextResource::parse`. They extend the
KOTO-0236 compile-time asset facility without changing the VM, KBC, host-call,
or package ABI.

## Motivation

KOTO-0236 made raw and parsed storage follow the largest packaged locale asset,
but the Gallery still carries three facts derived directly from locale file
contents:

- `GALLERY_RESOURCE_LINES = 22` must match every locale file and the
  `TextResource::parse` line capacity.
- `GALLERY_LIST_LABEL_BYTES = 30` is the largest total UTF-8 byte length of
  locale lines 9 through 11, used as the List row-label arena.
- The locale-update packet's 195-byte data arena includes 129 bytes for lines
  0 through 8 plus the 66-byte encoded List rows. The 129-byte component is
  documented but hand-summed.

These values drift when a locale gains a line, loses a line, or changes the
UTF-8 size of a string. A raw `asset_len` bound remains memory-safe but cannot
prove that locale files have the same indexed shape, and it over-allocates
subsets such as List labels and relabeling updates.

## Proposed Contract

### `asset_text_line_count(...)`

- Takes one or more string-literal package asset paths.
- Parses every asset with the text rules below.
- Returns the common line count when all assets have the same count.
- Produces a compile diagnostic naming the mismatching path, expected count,
  and actual count when locale shapes differ. It deliberately does not return
  the maximum: doing so would hide missing indexed strings.

### `asset_text_max_range_bytes(...)`

- Takes non-negative compile-time integer `first_line`, positive compile-time
  integer `line_count`, then one or more string-literal package asset paths.
- Requires `first_line + line_count` to be within every asset's line count;
  an out-of-range asset is a compile diagnostic naming the path and bounds.
- For each asset, sums the UTF-8 byte lengths of all lines in the half-open
  range `[first_line, first_line + line_count)`, excluding delimiters.
- Returns the maximum of those per-asset sums. It does not return the longest
  individual line or sum independent per-line maxima.
- Rejects arithmetic outside the signed 32-bit Koto integer domain.

Both helpers are valid exactly where `asset_len` is valid: a top-level `const`
initializer, a local `buf` size, a struct buffer-field size, and as an argument
to another compile-time helper. They have no runtime form. Integer arguments
may use the same literals, prior integer constants, and nested compile-time
forms accepted by capacity helpers.

### Text parsing and path identity

The compiler interpretation must match `TextResource::parse`:

- input must be valid UTF-8;
- LF and CRLF terminate lines; a bare CR is invalid;
- an internal empty line exists and contributes zero payload bytes;
- a trailing delimiter does not synthesize an additional empty line;
- empty input contains zero lines; and
- line lengths count UTF-8 bytes, not Unicode scalar values or glyphs.

Paths use the KOTO-0236 identity guarantee: package `output` paths declared in
the nearest `app.json` `assets` block, independent of the source/include file
that names them. V1 supports verbatim assets only. Undeclared, transformed,
unreadable, invalid-UTF-8, and malformed-line-ending assets receive focused
diagnostics at the offending literal.

## Intended Gallery Adoption

```koto
const GALLERY_RESOURCE_LINES = asset_text_line_count(
    "locales/en-US.txt", "locales/ja-JP.txt", "locales/qps-ploc.txt");
const GALLERY_LIST_LABEL_BYTES = asset_text_max_range_bytes(
    9, 3, "locales/en-US.txt", "locales/ja-JP.txt", "locales/qps-ploc.txt");
const GALLERY_APPLY_TEXT_BYTES = asset_text_max_range_bytes(
    0, 9, "locales/en-US.txt", "locales/ja-JP.txt", "locales/qps-ploc.txt");
const GALLERY_APPLY_DATA_BYTES = ui_list_rows_capacity(
  GALLERY_LIST_ROWS,
  asset_text_max_range_bytes(
    0, 12, "locales/en-US.txt", "locales/ja-JP.txt", "locales/qps-ploc.txt"));
```

With the current assets the first three values fold to 22, 30, and 129. The
apply data capacity folds to 195 by treating lines 0 through 11 as the
selected locale payload and adding the three-row List table through
`ui_list_rows_capacity`; the apply packet uses
`ui_update_capacity(10, GALLERY_APPLY_DATA_BYTES)`. This stays within the
existing narrow compile-time grammar, which deliberately does not evaluate
general `+` expressions in helper arguments.

## Implementation Scope

- Extend `AssetResolver` with verbatim asset-byte loading while keeping
  `asset_len` on its metadata-only path so size queries do not read large
  binary assets unnecessarily.
- Centralize manifest path validation and diagnostics shared by byte-size and
  byte-content lookups; cache text bytes or parsed line ranges during one
  compilation so repeated helpers do not reread a locale file.
- Implement one compiler-side text scanner whose accepted/rejected inputs and
  line boundaries mirror `TextResource::parse` exactly.
- Add both helpers to const, buffer-size, buffer-field, nested helper-argument,
  and compile-time-only expression diagnostics.
- Keep injected resolver support hermetic for compiler and editor tests; add
  LSP coverage showing folded const hover and mapped asset diagnostics.
- Document the forms, path identity, text rules, and multi-asset behavior in
  `KOTO_APP_LANGUAGE.md`; cross-reference `TextResource` in `KOTO_SDK.md`.
- Adopt the helpers in the KotoUI Gallery and update comments without changing
  retained UI behavior, golden frames, or semantic display capacities.

## Acceptance Criteria

- [x] `asset_text_line_count` folds one or more LF/CRLF text assets and rejects
  differing line counts with a diagnostic that identifies the mismatching
  asset and both counts.
- [x] `asset_text_max_range_bytes` returns the maximum per-asset range sum and
  is tested against a fixture where that differs from both the longest single
  line and the sum of independent per-line maxima.
- [x] Empty input, internal empty lines, trailing delimiters, missing trailing
  delimiters, mixed LF/CRLF, non-ASCII UTF-8, invalid UTF-8, bare CR, zero or
  negative range sizes, overflow, and out-of-range spans have explicit tests.
- [x] Both helpers work in `const`, local `buf`, struct buffer-field, and nested
  capacity-helper positions, while ordinary-expression use reports that they
  are compile-time only.
- [x] The helpers accept exactly the KOTO-0236 verbatim `asset_load` namespace;
  undeclared, transformed, and unreadable paths retain focused diagnostics.
- [x] Repeated helper calls do not reread the same text asset during one
  compilation, and `asset_len` remains metadata-only.
- [x] Gallery derives its resource line count, List label arena, and apply
  text arena from locale assets; current values fold to 22, 30, and 129.
- [x] Compiler, LSP, formatting, affected-crate Clippy, App build/package
  synchronization, Gallery scenarios, golden frames, budget checks, and
  project consistency checks pass.

## Notes

Filed 2026-07-17 from the KOTO-0236 Gallery follow-up audit.

Returning a maximum from `asset_text_line_count` was considered and rejected:
indexed localization requires every locale to expose the same semantic lines,
so a mismatch is a source-shape error rather than a capacity-sizing problem.

The range helper intentionally computes `max(sum(asset range))`, not
`sum(max(each line))`. Only one locale is resident at runtime, so independent
per-line maxima would reserve a combination that can never occur and would not
describe the actual payload copied by `UiListRowsBuilder` or an update packet.
Landed 2026-07-17. `AssetResolver::asset_bytes` caches verbatim bytes while
`asset_len` remains metadata-only. One compiler scanner mirrors
`TextResource::parse`; both forms fold through const, buffer, field, and nested
helper positions, with LSP hover and mapped diagnostics using the same path.
Gallery now folds 22 lines, 30 List-label bytes, 129 component-text bytes, and
the complete 195-byte apply arena; scenarios, golden frames, budgets, package
synchronization, compiler/LSP tests, Clippy, formatting, and project checks
pass.

The complementary retained-slot question (the largest *single* line in a
range) and general additive sizing remain follow-up KOTO-0238. KOTO-0237 does
not need general `+` evaluation for Gallery: its apply arena composes lines
0 through 11 with the three-row table through `ui_list_rows_capacity`.
