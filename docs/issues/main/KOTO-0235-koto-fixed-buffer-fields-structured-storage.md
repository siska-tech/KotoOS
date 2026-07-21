# KOTO-0235: Koto fixed buffer fields in static records (structured storage)

- Status: done
- Type: feature
- Priority: P2
- Requirements: FR-PKG-3, FR-RT-4, NFR-MEM-2, NFR-DEV-3, NFR-DEV-4, NFR-REL-1
- Related: KOTO-0221, KOTO-0228, KOTO-0232, KOTO-0233, KOTO-0234, KOTO-0236

## Goal

Let a `struct` declare fixed-size buffer fields (`raw: buf[384]`) so an App's
shared heap block becomes a compiler-derived layout instead of hand-summed
offset constants, while preserving the KOTO-0228 static-record model: no
runtime allocator, no VM/KBC/host ABI change, and the same one-base-reference
parameter cost through deeply inlined SDK call chains.

## Motivation

File Note packs its whole App state into `buf app[1388]` and addresses it with
hand-maintained offsets (`NOTE_RAW = 0` … `NOTE_MOUNT = 932`,
`NOTE_APP_BYTES = 1388`). The single block is deliberate slot engineering: the
helpers receive one `app: int` base so the deeply inlined SDK packet
validators fit the 45-slot local frame. The price is that region sizes, their
running sum, and the total are all separate facts a human must keep
consistent; inserting a region can silently corrupt its neighbors.

KOTO-0228 gave scalar App state named fields; this issue extends the same
layout model to bounded byte regions so structure and slot economy stop being
a trade-off:

```koto
struct NoteStorage {
    raw: buf[384],                          // locale asset text
    resource: buf[384],                     // parsed line table + payload
    doc: buf[64],                           // note bytes
    state: buf[4],                          // len u16 / dirty u8 / status u8
    event: buf[96],
    mount: buf[ui_mount_capacity(5, 176)],  // KOTO-0232 helper as a field size
}

static note_app: NoteStorage = {};

fn note_sync(app: NoteStorage, refresh: int, focus_home: int) {
    if note_update_builder.begin(app.mount, len(app.mount), 4) != 0 { ... }
}
```

## Proposed Language Contract

- A `struct` field may be `name: buf[N]` where `N` is a positive integer
  literal, a prior top-level integer `const`, or a capacity-helper call with
  KOTO-0232/0233's compile-time bounds and diagnostics (reuse
  `capacity_helper_call`).
- Layout stays declaration-ordered with checked offsets and total size; scalar
  `int`/`bool` fields keep their 32-bit representation. The record's size
  (scalars + buffer regions) counts into the exact heap request and budget
  diagnostics, exactly like today's separate `buf` storage.
- Reading a buffer field yields its address: `value.field` lowers to
  `base + offset` with **no** trailing `load32`. On a `static` receiver the
  address folds to a constant; on a struct parameter it is one runtime add —
  identical cost to today's `app + NOTE_MOUNT`.
- Buffer fields are integer-valued in expressions (a `buf` already decays to
  its offset today) but are not assignable: `value.field = x;` and an
  initializer entry for a buffer field are compile errors in the stored-alias
  diagnostic class. Buffer fields are zero-initialized (the VM heap starts
  zeroed).
- `len(value.field)` extends KOTO-0233's compile-time `len` to field operands
  whose receiver is a struct with a buffer field; it folds to the declared
  capacity with no runtime cost, on statics and struct parameters alike.
- KBC image rule: buffer-field regions contribute **no bytes** to the rodata
  heap image — only scalar fields materialize. The loader already zero-fills
  the heap above the image, so semantics are unchanged; a static whose buffer
  fields are followed by later initialized statics still zero-spans correctly.
  Document the convention that large buffer-carrying statics go last so the
  image does not carry embedded zero runs.

### Deliberate V1 restrictions

- No indexing sugar (`value.field[i]`); `heap_get_u8(value.field + i)` matches
  today's base-plus-offset usage. Sugar can follow separately if wanted.
- No nested structs, no buffer-field initializer payloads, no local `static`,
  no change to KOTO-0228's alias/copy/return rejections.
- Passing `value.field` to a host/SDK call passes the address, exactly like a
  named `buf` today; no slice/length pairing is implied by the type.

## Implementation Scope

- Parser: field grammar `name: buf[SIZE]` sharing the buf-size path (literal /
  prior const / capacity helper); struct-layout size accounting.
- Codegen: per-field sizes and offsets in `StructInfo`; address-valued field
  reads for buffer fields; `FieldAssign` and initializer rejection; `emit_len`
  extension to buffer-field operands; static image emission skipping buffer
  regions.
- LSP/tooling: buffer-field variant of the field symbol detail (hover shows
  capacity and offset, not "32-bit field"), definition/completion coverage,
  VS Code grammar if needed.
- Docs: `KOTO_APP_LANGUAGE.md` (struct fields, `len`, image rule),
  `KOTO_SDK.md` cross-reference for helper-sized fields.
- Adoption: migrate File Note's `NOTE_*` offset block to a `NoteStorage`
  static as the proving target; Gallery's `main()` locale storage may follow
  where it clarifies (a plain local-`buf` split is already available in
  today's language and does not depend on this feature).

## Acceptance Criteria

- [x] Buffer fields parse with literal, const, and capacity-helper sizes and
  carry the same boundary diagnostics as KOTO-0232/0233 call sites.
- [x] Field reads yield addresses with no `load32`; static receivers fold to a
  constant; struct-parameter receivers emit exactly one add. Equivalent
  hand-written base-plus-offset fixtures produce identical observable VM
  behavior and resource bounds.
- [x] `len(value.field)` folds to the declared capacity on statics and struct
  parameters with no heap read, local slot, or runtime instructions; invalid
  operands (scalar fields, unknown fields, non-struct receivers) have focused
  diagnostics.
- [x] Buffer-field assignment and buffer-field initializer entries are
  rejected with focused diagnostics; zero-initialization is documented and
  tested.
- [x] Static records with buffer fields increase the heap request by the
  checked layout size while the KBC rodata image materializes scalar bytes
  only; loader zero-fill semantics are proven by a runtime fixture.
- [x] File Note replaces its `NOTE_*` offset constants with a `NoteStorage`
  static at unchanged user-slot peak (44/45), fuel, host-call, and heap
  budgets, passing its simulator suite with unchanged golden frames.
- [x] Compiler, LSP, formatting, affected-crate Clippy, App build/package
  synchronization, runtime budget, and project consistency checks pass.

## Completion (2026-07-17)

Landed as specified; no VM/KBC/host ABI change. Verification evidence:

- Compiler/LSP: 126 + 13 unit tests green, including the new KOTO-0235
  fixtures (size grammar and boundary diagnostics, address folding on static
  receivers with zero `add_i32`/`load32`, one-add struct-parameter access, a
  hand-written base-plus-offset equivalence pair with identical `.stack` and
  `.heap` bounds, `len` folding and its focused rejections,
  assignment/initializer rejections, and a runtime zero-fill fixture proving
  the rodata image materializes scalar bytes only). Affected-crate Clippy and
  rustfmt clean.
- Byte neutrality: a full `build_apps.py` rebuild reproduced every committed
  `.kbc` hash-identical, so existing apps are unaffected by the compiler
  change.
- File Note proving target: `NOTE_*` offsets replaced by the `NoteStorage`
  static; budgets unchanged versus the pre-migration source (stack_peak 6,
  fuel_peak 30095, heap_request 2259, user_slots 44/45), full koto-sim suite
  green including `koto_ui_file_note` locale/320-square golden frames.
- VS Code grammar already highlights `buf` as a keyword; no grammar change was
  needed. Device smoke of the rebuilt File Note package remains a follow-up,
  as with other bytecode-refreshing changes.

## Notes

Filed from the KOTO-0234 review discussion (2026-07-17). A lighter
"named sub-buffer view" design (`buf app[1388] { raw: 384, ... }`) was
considered and rejected: without a type the view cannot cross a function
boundary, and File Note's whole point is handing one typed base to helpers —
it converges to struct buffer fields.

This is why KOTO-0234 deliberately scopes File Note out: restructuring it
without this feature would trade slot budget for readability. With buffer
fields the trade-off disappears — the helper signature cost stays one
reference parameter while every region and the total become compiler-owned.
