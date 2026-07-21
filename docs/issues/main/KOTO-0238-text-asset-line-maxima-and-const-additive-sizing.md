# KOTO-0238: Text asset line maxima and compile-time additive sizing

- Status: done
- Type: feature
- Priority: P3
- Requirements: FR-PKG-3, NFR-MEM-2, NFR-DEV-3, NFR-DEV-4, NFR-REL-1
- Related: KOTO-0221, KOTO-0232, KOTO-0233, KOTO-0235, KOTO-0236, KOTO-0237

## Goal

Complete the KOTO-0236/KOTO-0237 compile-time sizing facility so File Note can
derive every locale-dependent capacity from its packaged assets, by adding:

```koto
asset_text_max_line_bytes(first_line, line_count, "path", ...)
```

and additive folding (`+`/`-`) over the already-accepted compile-time integer
atoms in const initializers, buffer sizes, and helper arguments. Both are
compile-time-only extensions with no VM, KBC, host-call, or package ABI change.

## Motivation

The KOTO-0237 audit of File Note found that the two landed text asset helpers
cover the resource line count and the single-line component capacities for
lines 0 through 3, but two facts remain inexpressible:

- The Status label slot holds *one of* the nine status lines (4 through 12) at
  a time, so its retained capacity is the maximum single-line byte length in
  that range — 19 bytes today (`qps-ploc` `[ Note too large! ]`).
  `asset_text_max_range_bytes(4, 9, ...)` deliberately returns the maximum
  per-asset range *sum* (150 bytes), a 7x over-reservation for this slot.
- Arena totals are sums of independently derived capacities: the mount data
  arena is five component capacities plus the 64-byte note document, and the
  reaction packet arena is the status maximum plus the document. With no `+`
  in compile-time positions these stay hand-computed literals
  (`NOTE_MOUNT_DATA_CAPACITY = 176`, `ui_update_capacity(4, 83)`,
  `ui_update_capacity(5, 57)`) with prose comments restating the arithmetic.

The same gap blocks KOTO-0237's own intended Gallery adoption, whose apply
packet example `ui_update_capacity(10, GALLERY_APPLY_TEXT_BYTES +
GALLERY_LIST_BYTES)` already assumes additive helper arguments.

Deriving these removes the last drift class: a translation that grows a line
re-sizes the retained slot, the mount arena, and the update packets on the
next build instead of failing at load, and the hand-maintained
`note_line_cap` table disappears.

## Proposed Contract

### `asset_text_max_line_bytes(...)`

- Takes non-negative compile-time integer `first_line`, positive compile-time
  integer `line_count`, then one or more string-literal package asset paths —
  the same argument shape, text rules, and KOTO-0236 path identity as
  `asset_text_max_range_bytes`.
- Requires `first_line + line_count` to be within every asset's line count;
  an out-of-range asset is a compile diagnostic naming the path and bounds.
- Returns the maximum UTF-8 byte length of any *single* line in the half-open
  range `[first_line, first_line + line_count)` across all listed assets,
  excluding delimiters.
- Rejects arithmetic outside the signed 32-bit Koto integer domain.

This does not revisit the KOTO-0237 decision that the range helper returns
`max(sum(asset range))`: that rationale describes payload actually copied by
one packet, where per-line maxima would reserve an impossible combination. A
retained component slot is the opposite case — its capacity is fixed at mount
and must fit whichever indexed line a later `text_resource` update selects, so
the one-of-N line maximum is the exact bound. The two helpers answer the two
distinct sizing questions.

### Additive compile-time integer expressions

In the three compile-time integer positions — a top-level `const` initializer,
a local or struct-field `buf` size, and an integer helper argument — an
expression may be one or more of the currently accepted atoms (integer
literal, prior integer const, capacity helper call, `asset_len`, text asset
helper call) joined left-to-right by `+` or `-`:

- Folding checks the signed 32-bit Koto integer domain at every step.
- `buf` sizes must still fold positive; capacity-helper argument bounds are
  checked on the folded value as today.
- No parentheses, multiplication, or runtime spill: an expression that mixes
  a runtime value remains an error with the existing diagnostics. Ordinary
  runtime expressions are unaffected.

## Intended File Note Adoption

```koto
const NOTE_RESOURCE_LINES = asset_text_line_count(
    "locales/en-US.txt", "locales/ja-JP.txt", "locales/qps-ploc.txt");
const NOTE_STATUS_FIRST_LINE = 4;
const NOTE_DOC_BYTES = 64;
const NOTE_TITLE_BYTES = asset_text_max_line_bytes(0, 1, ...);        // 18
const NOTE_FIELD_LABEL_BYTES = asset_text_max_line_bytes(1, 1, ...);  //  6
const NOTE_SAVE_BYTES = asset_text_max_line_bytes(2, 1, ...);         //  6
const NOTE_RELOAD_BYTES = asset_text_max_line_bytes(3, 1, ...);       //  9
const NOTE_STATUS_BYTES = asset_text_max_line_bytes(NOTE_STATUS_FIRST_LINE,
    NOTE_RESOURCE_LINES - NOTE_STATUS_FIRST_LINE, ...);               // 19
const NOTE_MOUNT_DATA_CAPACITY = NOTE_TITLE_BYTES + NOTE_STATUS_BYTES
    + NOTE_FIELD_LABEL_BYTES + NOTE_DOC_BYTES + NOTE_SAVE_BYTES
    + NOTE_RELOAD_BYTES;                                              // 122
```

with `ui_update_capacity(4, NOTE_STATUS_BYTES + NOTE_DOC_BYTES)` (folds to
the current 83) for the reaction packet and
`ui_update_capacity(5, asset_text_max_range_bytes(0, 4, ...) +
NOTE_STATUS_BYTES)` (58, one byte conservative over the hand-derived 57) for
the locale apply packet. The mount builder and `note_line_cap` consume the
derived consts, so the runtime per-line validation loop keeps guarding stale
SD-card assets against exactly the capacities the build derived, with no
second statement of the numbers. `NOTE_STATUS_FIRST_LINE` remains app-declared:
the 4-component/9-status layout is `NoteWidget`/`NoteStatus` structure the
asset schema itself does not encode.

With current assets the mount arena shrinks 176 to 122 bytes, so the File
Note KBC changes; budgets are re-verified and golden frames stay identical
(all displayed strings are unchanged).

## Implementation Scope

- Implement `asset_text_max_line_bytes` on the KOTO-0237 `asset_bytes` /
  line-scanner path, sharing its caching, path diagnostics, and range checks.
- Extend the shared compile-time integer folding (const initializers,
  `buf_size`, `const_int_argument`) with left-associative `+`/`-` chains and
  per-step domain diagnostics, keeping every existing single-atom diagnostic
  unchanged.
- Add both features to const, buffer-size, buffer-field, nested
  helper-argument, and compile-time-only expression diagnostics; add LSP
  folded-hover coverage.
- Document the helper and additive expressions in `KOTO_APP_LANGUAGE.md`
  beside the KOTO-0237 helpers; note the slot-capacity vs copied-payload
  distinction.
- Unblock and fold the KOTO-0237 Gallery apply example as written; adopt the
  derived sizing in File Note per the sketch above.

## Acceptance Criteria

- [x] `asset_text_max_line_bytes` folds the maximum single-line byte length
  over a range across assets, tested on a fixture where that differs from
  both the range-sum maximum and every per-asset longest line's asset.
- [x] Range validation, empty lines, CRLF, non-ASCII UTF-8, invalid UTF-8,
  bare CR, zero or negative counts, overflow, and out-of-range spans have
  explicit tests with focused diagnostics.
- [x] `+`/`-` chains fold in const initializers, local `buf` sizes, struct
  buffer-field sizes, and helper integer arguments; overflow and non-positive
  `buf` results are compile diagnostics; runtime expressions are unaffected.
- [x] The KOTO-0237 Gallery apply-packet example folds as written.
- [x] File Note derives its resource line count, per-component capacities,
  status capacity, mount arena, and both update arenas from locale assets;
  the `note_line_cap` values exist only as derived consts; with current
  assets they fold to 18/6/6/9/19, the mount arena to 122, and the reaction
  arena to the current 83.
- [x] File Note KBC rebuilt; budgets re-verified; golden frames, retained UI
  behavior, and semantic display capacities unchanged.
- [x] Compiler, LSP, formatting, affected-crate Clippy, App build/package
  synchronization, File Note scenarios, and project consistency checks pass.

## Notes

Filed 2026-07-17 as the follow-up to the KOTO-0237 File Note capacity audit.

Widening the KOTO-0237 range helper to return per-line maxima was rejected —
its `max(sum)` contract is correct for copied payload and is kept; the new
helper serves the separate retained-slot question. General const expression
evaluation (parentheses, `*`, comparison, `max()`) is deliberately out of
scope: the two known sizing patterns need only addition and subtraction, and
a smaller folding surface keeps diagnostics focused.

KOTO-0237 can land independently with a literal in its Gallery apply packet;
this issue then replaces that literal. Only the File Note adoption depends on
both helpers and additive folding together.

Landed 2026-07-17. The helper shares the KOTO-0237 scanner, byte cache, and
range checks (the bounds diagnostic is parameterized over the helper name and
stays byte-identical for `asset_text_max_range_bytes`); additive folding is
one shared left-associative tail (`const_additive_tail`) over the existing
atom grammar, with per-step 32-bit domain diagnostics and a folded-total
positivity diagnostic for chained `buf` sizes while every single-atom
diagnostic is unchanged. File Note folds 18/6/6/9/19, the mount arena
176→122, the reaction arena stays 83, and the locale apply arena grows 57→58
(one byte conservative, lines 0..3 as one copied range); `note_line_cap` and
the mount builder consume the derived consts, and the oversize-detection
`tmp[NOTE_DOC_BYTES + 4]` buffers demonstrate additive local sizing. The
Gallery apply packet now composes `GALLERY_APPLY_TEXT_BYTES +
GALLERY_LIST_BYTES` (129 + 66 = 195, value-identical to the replaced
`GALLERY_APPLY_DATA_BYTES`). KBCs rebuilt; the interaction scenario peaks at
1,884 heap bytes / 28,625 fuel with slots unchanged at 44/45; File Note and
Gallery scenario suites, locale pixel goldens, budget gate, compiler (139) and
LSP tests, Clippy, formatting, build sync, and project checks all pass.

Follow-up in the same landing: File Note's fixed diagnostic exit codes moved
into a `NoteError` enum (the KOTO-0234 pattern, deferred there because it is
not byte-neutral), preserving every established code value across the 18
sites. Device smoke confirmed 2026-07-17.
