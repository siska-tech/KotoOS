# KOTO-0233: Koto buffer capacity at KotoUI builder call sites

- Status: done
- Type: feature
- Priority: P1
- Requirements: FR-PKG-3, FR-SDK-1, FR-SDK-2, FR-SDK-5, FR-RT-4, NFR-MEM-2, NFR-DEV-3, NFR-DEV-4, NFR-REL-1
- Related: KOTO-0219, KOTO-0229, KOTO-0232
- Roadmap: [KotoUI App ABI Roadmap](../../planning/KOTOUI_APP_ABI_ROADMAP.md)

## Goal

Keep one-use KotoUI packet sizing facts beside the builder transaction that
consumes them. Let an App size a local packet buffer directly with the bounded
capacity helpers and pass that buffer's compile-time capacity to `begin`, so
official Apps do not need a block of transaction-specific `*_RECORDS`,
`*_DATA_CAPACITY`, and `*_BYTES` constants at file scope.

## Motivation

KOTO-0232 moved wire-layout arithmetic into `ui_mount_capacity` and
`ui_update_capacity`, but Gallery still opens with four three-constant groups:

```koto
const GALLERY_STATUS_UPDATE_RECORDS = 1;
const GALLERY_STATUS_UPDATE_DATA_CAPACITY = 19;
const GALLERY_STATUS_UPDATE_BYTES = ui_update_capacity(
    GALLERY_STATUS_UPDATE_RECORDS, GALLERY_STATUS_UPDATE_DATA_CAPACITY);
```

These values describe only one builder transaction. Keeping them far from the
buffer and `begin` call makes the App scan a global constant block to understand
local storage, and gives a single concept three names.

The intended call-site form is:

```koto
fn gallery_set_status(line: int) {
    // One text record; the longest localized status is 19 UTF-8 bytes.
    buf update[ui_update_capacity(1, 19)];
    let status = gallery_update_builder.begin(update, len(update), 1);
    // ...
}
```

The record count remains explicit because the one-pass wire builder must reserve
its record table before writing data. The buffer capacity is derived from the
declaration through `len(update)` instead of being repeated as a separately
named argument.

## Proposed Contract

- Permit `ui_mount_capacity(...)` and `ui_update_capacity(...)` directly as a
  local `buf` size. Their arguments remain integer literals or prior top-level
  integer constants and retain KOTO-0232's compile-time bounds.
- Implement the existing language-spec promise that `len(local_buf)` is its
  compile-time byte capacity. It emits an integer constant and creates no
  runtime buffer descriptor, slice, length field, or host ABI change.
- Reject `len` for ordinary integers, unknown names, or values whose capacity
  is not statically known, with a source-mapped diagnostic.
- Preserve lexical buffer scope and shadowing rules when resolving `len(buf)`.
- Keep builder `record_capacity` explicit. Automatically counting records would
  require a two-pass builder, data relocation, or maximum-table reservation and
  is outside this issue.
- Keep general compile-time expression/function evaluation out of scope.

## Acceptance Criteria

- [x] A local buffer can be declared as
  `buf packet[ui_mount_capacity(records, data_capacity)]` or with the update
  equivalent, with the same boundary diagnostics as a top-level helper call.
- [x] `len(buf)` lowers to the declared compile-time capacity without a heap
  read, local slot, runtime instruction sequence, or metadata allocation.
- [x] Invalid, unknown, out-of-scope, and shadowed `len` operands have focused
  compiler diagnostics and regression tests.
- [x] Gallery moves mount/dialog/status/locale transaction sizing beside each
  buffer and removes the corresponding file-scope constant groups.
- [x] File Note and the SDK Counter example adopt the same pattern where a
  capacity belongs to only one builder transaction.
- [x] Data-capacity derivation comments remain adjacent to each declaration;
  shared domain/configuration constants remain at file scope.
- [x] Compiler and LSP tests cover helper-sized buffers, `len(buf)`, definitions,
  diagnostics, and unsaved overlays as applicable.
- [x] Rebuilt Gallery and File Note artifacts preserve golden behavior and do
  not increase KBC words, heap request, user slots, fuel, or host-call peaks.
- [x] Formatting, warning-clean affected-crate Clippy, simulator suites, budget
  gate, and project consistency checks pass.

## Completion Evidence

- The parser folds both capacity helpers directly as a `buf` size through the
  shared `capacity_helper_call` path, so call-site declarations carry the same
  KotoUI v1 boundary diagnostics as `const` initializers.
- `len(buf)` folds in codegen to `push` of the declared capacity — the
  zero-cost test pins the emitted constant and the absence of loads and local
  slots. Diagnostics cover wrong arity, non-identifier operands, integer locals
  (including a `let` that shadows the buffer), use before declaration, other
  functions' buffers, consts, `data` tables, and unknown names; block scoping
  restores the buffer after a shadowing block ends.
- Gallery keeps mount/dialog/status/locale sizing beside each declaration and
  drops four three-constant file-scope groups; File Note migrates the sync and
  locale transactions while the mount capacity stays file-scope beside the
  shared `NOTE_*` heap layout it participates in; the SDK Counter example
  migrates both packets.
- Old-form and new-form sources compile to byte-identical code and rodata for
  all three apps (assembly diff modulo `.loc`/`.debug_file` only), so KBC
  words, heap request, user slots, fuel, and host-call peaks are unchanged by
  construction. Gallery and File Note deterministic heap requests remain 2,735
  and 2,259 bytes.
- Gallery's 11 and File Note's 15 simulator tests pass with unchanged golden
  frames. `python harness/build_apps.py --check` confirms every other app's
  committed bytecode is untouched by the compiler change.
- `cargo fmt --all -- --check`, warning-clean `cargo clippy -p koto-compiler
  -p koto-lsp --all-targets`, 119 compiler and 12 LSP tests,
  `python harness/check_budgets.py`, and `python harness/check_project.py`
  pass.

## Notes

This is deliberately a language/compiler locality improvement, not a stateful
builder redesign. A buffer currently lowers to a raw app-heap pointer, so an SDK
method cannot discover its allocation extent by itself. Compile-time `len(buf)`
keeps that extent explicit at the call while preserving the existing zero-cost,
pointer-only runtime representation.

The language specification already describes `len(buf)` as the compile-time
capacity. Implementation must either make that statement executable as scoped
above or adjust the specification if compiler constraints invalidate the design.
