# KOTO-0234: Gallery failure codes and storage offsets as named declarations

- Status: done
- Type: cleanup
- Priority: P3
- Requirements: NFR-DEV-3, NFR-DEV-4, NFR-REL-1
- Related: KOTO-0220, KOTO-0233, KOTO-0235

## Goal

Remove Gallery's two remaining magic-number families — literal failure exit
codes (`gallery_fail(-50)`) and hand-computed storage offsets
(`locale_storage + 800`) — by naming them with an enum and separate buffer
declarations, without changing the emitted code (identical assembly modulo
debug line info).

## Motivation

**Failure codes.** Gallery reports every fatal path through
`gallery_fail(code)` with a bare negative literal (-10..-13, -20, -30..-32,
-50..-86). The code is the only diagnostic that reaches the device UART or
simulator output, but reading a call site gives no hint what -64 means, and
writing a new site means scanning the file for the next free number. The
language already supports explicit negative enum members at zero runtime cost
(`GalleryStatus::None = -1` in the same file), so the names are free.

**Storage offsets.** `main()` packs three regions into one
`buf locale_storage[868]` and addresses them with derived literals: the parsed
table at `locale_storage + GALLERY_RESOURCE_RAW_BYTES`, the List rows blob at
`locale_storage + 800` with a free-standing capacity `68`, and the total `868`
(= 384 + 416 + 68) recorded nowhere. The sharing buys nothing today: buffers
are compile-time offsets, not local slots, the helpers already receive each
region as its own pointer parameter, and the peephole folds
`push base; push offset; add` into one constant — the "share one
allocation/base local" comment is a vestige of an older structure. Three named
buffers in the same declaration order occupy the same 868 contiguous bytes at
the same addresses and fold to the same constants.

## Proposed Change

### 1. `GalleryError` enum

- Add a `GalleryError` enum naming each failure site with its **current**
  value, keeping the per-transaction number spacing (for example
  `ResourceLoad = -10`, `Capabilities = -20`, `MountFinish = -30`,
  `DialogBegin = -50`, `StatusSubmit = -64`, `FocusPresent = -86`).
- Replace only the literal `gallery_fail(...)`/`exit(...)` diagnostics.
  Pass-throughs of real SDK/host statuses (`gallery_fail(status)`,
  `gallery_fail(event_len)`, `gallery_fail(UI_SDK_BAD_ARGUMENT)`) stay as they
  are.
- Values are not renumbered: the exit code is the observable failure signal on
  hardware and in simulator logs, and preserved values keep the rebuilt KBC
  code bit-identical (KDBG line shifts only) with no fuel, slot, or heap
  change.

### 2. Named storage declarations in `main()`

- Split `buf locale_storage[868]` into three buffers in the same order:
  `buf locale_raw[GALLERY_RESOURCE_RAW_BYTES]`,
  `buf locale_table[GALLERY_RESOURCE_BYTES]`, and `buf rows_blob[68]`, and
  pass `rows_blob, len(rows_blob)` to `gallery_build_rows`. The literals
  `868`, `800`, and the derived `+ GALLERY_RESOURCE_RAW_BYTES` call-site
  arithmetic disappear; each region's capacity lives on its declaration.
- Finish the KOTO-0233 pattern for the remaining sized buffers:
  `buf event[GALLERY_EVENT_BYTES]` polled with `len(event)` (retiring the
  separate const if it has no other consumer), and `buf capabilities[64]`
  queried with `len(capabilities)` instead of a repeated `64`.
- Replace the vestigial "share one allocation/base local" comment with the
  actual constraint (declaration order fixes the heap layout).
- File Note's single `app` block is explicitly **out of scope**: its one-base
  parameter is deliberate slot engineering (44/45 user slots through deeply
  inlined SDK chains), and restructuring it awaits KOTO-0235's buffer fields.

## Acceptance Criteria

- [x] `GalleryError` members cover every literal failure code in
  `apps/samples/koto_ui_gallery/src/main.koto` with unchanged values, and all
  literal diagnostic sites use the enum.
- [x] SDK/host status pass-through sites are untouched.
- [x] `main()` declares the locale raw/table/rows regions as three named
  buffers in the current layout order, no call site performs manual offset
  arithmetic into them, and the rows/event/capabilities capacities flow
  through `len(...)`.
- [x] Old and new sources compile to identical assembly modulo
  `.loc`/`.debug_file` (KOTO-0233 equivalence method) **for the storage
  split and every `len(...)` migration**; heap request, user slots, stack
  peak, and success-path fuel are unchanged. *Amended at landing:* the enum
  half cannot be bit-identical because the premise that a literal `-10`
  already lowers to one constant push was wrong — see Landing.
- [x] Rebuilt Gallery KBC/KPA passes the simulator suite with unchanged golden
  frames, the budget gate, formatting, and project consistency checks.

## Landing (2026-07-17)

- `GalleryError` names all 19 literal failure sites (`ResourceLoad = -10` …
  `FocusPresent = -86`) with unchanged values; the four pass-through sites
  (`gallery_fail(status)`, `gallery_fail(event_len)` ×2,
  `gallery_fail(UI_SDK_BAD_ARGUMENT)`, plus `exit(capability_len)`) are
  untouched. `main()` declares `locale_raw[GALLERY_RESOURCE_RAW_BYTES]`,
  `locale_table[GALLERY_RESOURCE_BYTES]`, and `rows_blob[68]` in the old
  layout order; `868`, `800`, and the call-site `+ GALLERY_RESOURCE_RAW_BYTES`
  arithmetic are gone. Event polls and the capabilities query use `len(...)`;
  `GALLERY_EVENT_BYTES` had no remaining consumer and was retired.
- Equivalence correction: the compiler lowers a unary-minus literal argument
  as `push_i16 N; push_i16 0; swap; sub_i32`, while an enum member lowers to
  one `push_i16 -N`. The Motivation's "explicit negative enum members at zero
  runtime cost" claim therefore *understated* the change: naming the codes
  removes three instructions at each of the 29 failure branches (assembly
  65,000 → 64,913 lines; KBC 293,830 → 293,378 bytes including KDBG line
  shifts). The storage split and `len(...)` migrations contribute zero diff.
- Every removed instruction sits inside a fatal branch that ends in `exit`,
  so the executed instruction stream of every non-failing frame is identical:
  deterministic budgets are unchanged (stack_peak 7, fuel_peak 58,487,
  heap_request 2,735, user_slots 44). Slot maps differ only in `src=` lines.
- Because KBC words changed, instruction addresses shift and code-window tile
  alignment is not pinned by construction; the simulator gates all pass
  (847 workspace tests, golden frames, budget gate, audio scratch, memo
  validation, project harness, `build_apps.py --check` with every other app's
  bytecode untouched). A device boot + locale-switch smoke is recommended
  before treating the rebuilt KPA as device-verified.

## Notes

SDK generalization of the failure codes was considered and rejected. The exit
codes' value is their per-site uniqueness inside one App; the SDK already owns
the shared status vocabulary (`UI_SDK_OK`, `UI_SDK_BAD_ARGUMENT`, host
`UI_ERROR_*`) that these codes wrap, and an SDK-side enum could only name
generic steps, discarding the site information. The one real duplication — the
three-line `ui_reset(); exit(status);` fail helper shared by Gallery, File
Note, and the Counter example — saves nothing under the full-inlining model
and would add SDK/LSP/spec surface for three lines, so it stays App-owned.

The storage split relies on two verified compiler facts: buffers cost no local
slots (they lower to compile-time offsets), and the peephole constant-folds
`push_i16 base; push_i16 delta; add_i32` (codegen Rule 1) so today's manual
arithmetic and tomorrow's separate declarations emit the same single constants.
The equivalence criterion pins this.

File Note's `note_fail` literals (-10..-73) follow the same enum pattern; if
this lands well the same rename can be applied there as a separate small
change. Its storage layout is tracked by KOTO-0235 instead.
