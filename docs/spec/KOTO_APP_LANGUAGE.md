# Koto App Language MVP

This spike selects and specifies the first high-level source language for
KotoRuntime apps so real applications are authored in readable source rather than
hand-written `KBC1` assembly. It freezes a minimal subset before the compiler
(KOTO-0046) is built, and is the decision referenced by the
[bytecode app development roadmap](../planning/BYTECODE_APP_DEV_ROADMAP.md) Milestone 3.

## Decision

Build a small, Koto-specific, ahead-of-time compiled procedural language ("Koto"),
compiled on the host PC to [`KBC1`](RUNTIME_BYTECODE_ABI.md) bytecode. App sources
live at `apps/<id>/src/main.koto`. The
[kbc-asm](../../tools/kbc-asm) assembler stays the low-level IR/debug target; the Koto
compiler may emit assembly text or `KBC1` directly.

This continues the [VM selection](../architecture/RUNTIME_VM_SELECTION.md) reasoning: KotoOS owns a
tiny deterministic integer VM, so it should own a tiny deterministic source
language that maps directly onto it, rather than adopting a dynamic-language
runtime whose memory model fights the RP2040 budget.

## Constraints That Shape The Language

The language is a thin, statically-shaped layer over the existing VM, so its design
is dictated by the runtime contract:

- **Integer-only VM.** The VM stack and locals are `i32`; there is no float, no GC,
  no heap allocator. Koto exposes `int` and `bool`, no dynamic types.
- **48 shared local slots.** `VM_LOCAL_SLOTS = 48` is a flat register file, not a
  per-call frame: locals are shared across all of an app's functions (each gets a
  non-overlapping range), so this is the ceiling on a program's *total* named
  locals, less three codegen scratch slots (45 user slots). The MVP therefore
  forbids recursion and the compiler allocates locals/temporaries statically (see
  [Functions](#functions)). The size is a deliberate, predictable bound for
  RP2040-class SRAM (`[i32; 48]` = 192 B/VM), sized at roughly twice the most
  complex app; see [Runtime ABI: Simulator VM Profile](RUNTIME_BYTECODE_ABI.md#simulator-vm-profile).
- **App heap is the only mutable memory.** `LOAD/STORE8/16/32` address the app
  heap; string/byte data is not readable from the asset's rodata at runtime, so
  constant bytes are materialized into the heap by emitted code (the
  `store_str` pattern). Buffers are fixed-size heap regions with compile-time
  offsets.
- **Cooperative frames.** Apps run inside the per-frame fuel budget and must
  `yield_frame` each frame. State persists across frames in locals and heap, so a
  `loop { ...; yield(); }` is the natural frame loop.
- **Host calls return `(results…, status)`.** Per the ABI return convention, every
  host call pushes a trailing status (`0` ok / `-1` error). Koto surfaces this as
  ordinary `int` returns and explicit checks — there are no exceptions.

## Selected Syntax (MVP)

### Primitive types

- `int` — 32-bit signed integer (the VM word).
- `bool` — `true` / `false`, represented as `1` / `0`.
- `buf` — a fixed-size byte buffer in the app heap: `buf name[N];`.

There are no arrays-of-int, floats, general pointers, or dynamically allocated
objects. The deliberately bounded static-record model is described below. A
`buf` decays to its heap offset (an `int`) when passed to a host/SDK call, paired
with an explicit length.

### Local variables and constants

```koto
let cursor = 0;        // int local, mutable
let done = false;      // bool local
const MAX = 512;       // compile-time constant, no runtime slot
const COPY = MAX;      // a previously declared const is also accepted
buf doc[MAX];          // 512-byte heap buffer
```

`let` binds a mutable local; the compiler maps it to a local slot or a heap-backed
scratch cell. `const` is folded at compile time. Its v1 right-hand side is a
literal, a previously declared const, one of the checked KotoUI capacity
helper calls described in `KOTO_SDK.md`, or a compile-time asset helper call
(below); this is deliberately not a general compile-time function evaluator. `buf` reserves
a heap region with a static offset; its size is a positive integer literal, a
previously declared positive integer `const`, a direct KotoUI capacity-helper
call (`buf packet[ui_update_capacity(1, 19)];`), which keeps one-transaction
sizing facts on the declaration and carries the same compile-time boundary
diagnostics as the `const` form (KOTO-0233), or a direct asset helper call
(KOTO-0236/0237).

**Additive compile-time expressions (KOTO-0238).** In the three compile-time
integer positions — a `const` initializer, a local or struct-field `buf`
size, and an integer helper argument — those atoms may be joined
left-to-right by `+` or `-` (`buf tmp[DOC_BYTES + 4];`,
`ui_update_capacity(4, STATUS_BYTES + DOC_BYTES)`), so arena totals compose
from independently derived capacities instead of hand-computed literals.
Folding checks the signed 32-bit integer domain at every step, and a `buf`
size must still fold positive. There are no parentheses, no multiplication,
and no runtime spill: mixing a runtime value into a compile-time position
stays a compile error, and ordinary runtime expressions are unaffected.

### Compile-time asset sizes: `asset_len` (KOTO-0236)

```koto
const RAW_BYTES =
    asset_len("locales/en-US.txt", "locales/ja-JP.txt", "locales/qps-ploc.txt");
buf raw[RAW_BYTES];
let loaded = asset_load("locales/en-US.txt", 17, raw, len(raw));
```

`asset_len("path/in/package")` folds to the named package asset's byte size at
compile time; with two or more arguments it folds to the **maximum** of the
named assets' sizes (the multi-locale case). It is valid exactly where the
capacity helpers are valid — a top-level `const` initializer, a local `buf`
size, a struct buffer-field size, and as an argument to a compile-time
capacity helper — and nowhere else: there is no runtime form, so naming it in
an ordinary expression is a compile error.

**Path identity.** `asset_len` and `asset_load` share one namespace: the
package asset paths declared as `output` in the app manifest's `assets` block
(`app.json`, discovered as the nearest manifest above the root source file),
which is exactly the set the runtime host permits. The same string literal
means the same asset in both positions, and resolution is
position-independent — any source file in the program, including SDK
includes, sees the same namespace. It is deliberately *not* include-style
source-file-relative resolution. Corollary: any literal `asset_len` accepts
is a valid `asset_load` argument whose runtime permission check cannot fail,
and the folded size is the size of the bytes that ship in the package, so a
translation that grows re-folds its storage on the next build instead of
failing at load time.

**V1 restrictions.** Paths are string literals only (no consts, no
concatenation, no glob). `asset_len` covers manifest entries packaged
verbatim (copy-through `assets` entries); pipeline-transformed outputs
(PNG→`.kim`, KMML→KAQ1) have sizes the compiler cannot see from the source
file, and naming one is a focused diagnostic, as is any path not declared in
the manifest. An asset that folds to 0 bytes cannot size a buffer. Because
asset bytes fold into committed bytecode, packaged text assets pin `eol=lf`
in `.gitattributes` so sizes do not depend on checkout line endings.

### Compile-time text asset shape (KOTO-0237/0238)

```koto
const LINES = asset_text_line_count("locales/en-US.txt", "locales/ja-JP.txt");
const LABEL_BYTES = asset_text_max_range_bytes(
  9, 3, "locales/en-US.txt", "locales/ja-JP.txt");
const STATUS_BYTES = asset_text_max_line_bytes(
  4, LINES - 4, "locales/en-US.txt", "locales/ja-JP.txt");
```

`asset_text_line_count(path, ...)` returns the common line count and rejects
the first asset whose count differs. `asset_text_max_range_bytes(first_line,
line_count, path, ...)` sums the UTF-8 byte lengths in that half-open line
range for each asset and returns the largest per-asset sum. It does not return
the longest single line or sum independent per-line maxima.
`asset_text_max_line_bytes(first_line, line_count, path, ...)` takes the same
arguments and returns the maximum byte length of any *single* line in the
range across all listed assets (KOTO-0238). The first line is non-negative,
the count is positive, and the complete range must exist in every asset.

The two range helpers answer distinct sizing questions: `max_range_bytes`
bounds the payload one packet actually copies (only one locale is resident, so
per-line maxima would reserve an impossible combination), while
`max_line_bytes` bounds a retained component slot whose capacity is fixed at
mount and must fit whichever indexed line a later `text_resource` update
selects — there the one-of-N line maximum is the exact bound.

Both forms share `asset_len`'s path namespace, string-literal V1 restriction,
compile-time-only positions, and focused undeclared/transformed/unreadable
diagnostics. Text must be valid UTF-8. LF and CRLF terminate lines, internal
empty lines count with zero payload bytes, a final delimiter does not create
another empty line, empty input has zero lines, and bare CR is rejected. These
are exactly `TextResource::parse` boundaries; lengths count encoded bytes, not
Unicode scalar values or displayed glyphs.

### Integer enums

```koto
enum GalleryLocale {
    En,          // 0
    Ja,          // 1
    QpsPloc = 8,
    Fallback,    // 9
}

let locale = GalleryLocale::Ja;
```

Enums are top-level compile-time integer domains. The first implicit member is
zero; later implicit members increment the previous value, and an explicit
signed 32-bit integer literal resets the sequence. A trailing comma is allowed,
and duplicate numeric values are valid protocol aliases. Enum and member names
must be unique in their scopes; textual `include` expansion applies normally.

`Name::Member` is an ordinary `int` constant: it can be compared with integers,
passed to `int` parameters, stored in buffers, and folded by the compiler. It
allocates no VM slot or heap object and introduces no opcode, RTTI, or host ABI.
v1 deliberately has no strong enum/int separation, payload variants, `match`,
exhaustiveness checking, or bitflag syntax. Masks and composable flags remain
flat `const` values.

### Static records and inline methods

```koto
struct CounterState {
    value: int,
    enabled: bool,
}

static counter: CounterState = { value: 0, enabled: true, };

impl CounterState {
    fn increment(self) {
        if self.enabled { self.value = self.value + 1; }
    }
}

fn read(state: CounterState) -> int { return state.value; }
```

`struct` declares a compile-time field layout; declaration order assigns each
`int` or `bool` field one little-endian 32-bit word. A top-level `static`
reserves exactly one mutable instance for the App lifetime. Its named scalar
fields must be complete and unique, and literals, `const` values, and enum
members are written into the initial KBC heap image. Initializer order does not
affect the layout. The record's checked byte size contributes directly to the
App heap request and consumes no local slot per field.

Reading `value.field` lowers to `load32`; assigning `value.field = expression`
lowers to `store32`. Passing a static to a parameter of the same struct type
passes its base heap offset without copying. `value.method(args...)` resolves by
receiver type and inlines the matching `impl` method exactly like a free
function whose first parameter is the reference. Calls therefore add no opcode,
runtime type metadata, or dynamic dispatch, but each call site still pays the
method body's bytecode cost and retains the normal recursion rejection.

V1 intentionally rejects block-local `static`, stored aliases (`let copy =
counter`), record assignment/copy, struct returns, constructor-like `Type {
... }` expressions, nested records, arrays of records, allocation, equality,
destructuring, inheritance, traits, and generics. Use a method such as `reset`
for explicit reinitialization; the initializer itself runs only at App load.

#### Fixed buffer fields (KOTO-0235)

```koto
struct NoteStorage {
    doc: buf[64],                           // fixed byte region
    state: buf[4],
    mount: buf[ui_mount_capacity(5, 176)],  // capacity helper as a field size
    dirty: bool,                            // scalars mix freely
}

static note_app: NoteStorage = { dirty: false, };

fn note_sync(app: NoteStorage) {
    if ui_mount(app.mount, len(app.mount)) < 0 { return; }
}
```

A struct field may be a fixed-size byte region, `name: buf[N]`. The size takes
the same forms as a local `buf` declaration — positive integer literal, prior
integer `const`, a KotoUI capacity-helper call, or an `asset_len` call — with
identical boundary diagnostics (KOTO-0232/0233/0236). Layout stays declaration-ordered with checked
offsets: scalars keep one 32-bit word, a buffer field takes its declared
capacity, and the whole record counts into the exact App heap request.

Buffer fields make structure and slot economy compatible: helpers keep
receiving one typed base reference, while every region offset and the total
size become compiler-owned instead of hand-summed `const` offsets.

- **Reads are addresses.** `value.field` lowers to `base + offset` with no
  trailing `load32` — a buffer field decays to its region offset exactly like a
  named `buf`. On a `static` receiver the address folds to one constant; on a
  struct parameter it is one runtime add, identical to hand-written
  `app + REGION_OFFSET`. Passing `value.field` to a host/SDK call passes the
  address; no length pairing is implied by the type.
- **`len(value.field)`** folds to the declared capacity at compile time, on
  statics and struct parameters alike, with no heap read, local slot, or
  runtime instructions. Scalar fields, unknown fields, and non-struct
  receivers are compile-time errors.
- **Not assignable, not initializable.** `value.field = x;` and an initializer
  entry for a buffer field are compile errors. Buffer regions are
  zero-initialized (the VM heap starts zeroed); write through
  `heap_set_u8(value.field + i, x)` as with any buffer.
- **KBC image rule.** Buffer-field regions contribute no bytes to the rodata
  heap image — only scalar fields materialize; the loader zero-fills the heap
  above the image. A buffer field that is *followed* by initialized bytes (a
  later scalar field or a later initialized `static`) leaves a zero run inside
  the image, so declare large buffer-carrying statics (and their buffer
  fields) last to keep the image compact.
- **V1 restrictions.** No indexing sugar (`value.field[i]`; use
  `heap_get_u8(value.field + i)`), no nested structs, and the KOTO-0228
  alias/copy/return rejections are unchanged.

### Expressions and operators

- Arithmetic: `+ - * / %` (map to `add/sub/mul/div_i32`; `/` and `%` by zero trap
  deterministically, as in the VM).
- Bitwise: `& | ^ << >>` and unary `-`.
- Comparison: `== != < <= > >=` yielding `bool`.
- Logical: `&& || !` (short-circuit), on `bool`.
- Indexing a buffer byte: `doc[i]` reads/writes one heap byte (`load8`/`store8`).

### Control flow

```koto
if cond { ... } else if other { ... } else { ... }
while cond { ...; break; ...; continue; }
loop { ... }            // sugar for `while true`
return;                 // or `return expr;`
```

`if`/`while`/`loop`/`break`/`continue`/`return` compile to `br` / `br_if_zero`
against assembler labels. There is no `for`, no `match`, no `goto`.

### Functions

```koto
fn main() { ... }
fn clamp(v: int, lo: int, hi: int) -> int { ... }
```

- `fn main()` is the entry point and owns the frame loop.
- Parameters are `int`, `bool`, or a V1 struct reference. Returns are `int`/`bool`
  only; `buf` is passed as its `int` offset plus an explicit length argument.
- **Non-recursive** in the MVP (the flat 48-slot register file makes a call stack of
  locals impossible without a software frame). The compiler **inlines every function**
  at its call site (there are no `call`/`ret` opcodes; local slots are allocated
  call-site-scoped and reused across disjoint calls); recursion is rejected at compile
  time.
- **Locality note (KOTO-0137).** Because every call is inlined, a helper has no
  standalone bytecode and *reordering `fn` definitions does not change code layout*. A
  helper inlined N times multiplies into the code segment, which matters on the
  PSRAM-backed device: the code window tiles execution into 8 KiB tiles
  (`CODE_WINDOW_BYTES`), and a large inlined body can push a hot `while`-loop across a
  tile boundary, thrashing the window (~3 µs/instruction). Prefer representing a
  constant-return helper or a large branch chain as a **heap table read** (e.g.
  `heap_get_u16(TABLE + k * 2)`) rather than a body that is duplicated inline. See
  `docs/issues/KOTO-0137-koto-blocks-shape-table.md`.

### Source-file splitting (`include`, KOTO-0183)

```koto
include "dungeon.koto";   // path relative to the including file
include <sdk/koto_ui.koto>; // standard SDK path, rooted at the workspace
```

- A whole line of the form `include "relative/path.koto";` or
  `include <sdk/library.koto>;` (optional trailing `//` comment) is replaced
  **before lexing** by the named file's contents.
  The program then compiles exactly as if it were one file: a faithful,
  line-preserving split produces **byte-identical bytecode**. Diagnostics are
  remapped, so errors report the defining file and line.
- Quoted paths use `/`, must be relative, and resolve against the including
  file's directory. Angle-bracket paths are reserved for `sdk/`, resolve from
  the workspace root, and reject `.` or `..` components. Nesting is allowed
  (depth ≤ 16); each file may be included
  **once per program** — a re-include or cycle is a compile error at the
  second include site.
- **Cost warning:** `include` does not change the inlining model above. A
  helper defined in another file still has no standalone bytecode and still
  multiplies into the code segment per call site — splitting files makes that
  bloat easier to *stop seeing*, not cheaper. Sharing one helper file across
  apps re-inlines it into every app; each pays against its own code-window
  budget. Design note: `KOTO_LANGUAGE_INCLUDE.md`.

### Strings and buffers

- String literals are UTF-8 (`"こんにちは"` is allowed; the VM treats bytes opaquely
  and `draw_text` validates UTF-8 host-side).
- A string literal used as a host/SDK argument is placed by the compiler in a heap
  data region and initialized at app start (emitted `store_str`-style byte stores),
  then passed as `(offset, byte_len)`.
- `buf name[N]` is zero-initialized (the VM heap starts zeroed). `len(buf)` is the
  compile-time capacity; runtime content length is whatever the app/host tracks.
  `len` folds to an integer constant with no heap read, local slot, or runtime
  metadata — a buffer stays a raw heap offset. Its operand must name a `buf`
  visible at that point under normal identifier resolution (a `let` or parameter
  of the same name shadows the buffer for its block) or a struct buffer field
  (`len(value.field)`, KOTO-0235); integers, unknown names, not-yet-declared
  buffers, and other functions' buffers are compile-time errors (KOTO-0233).
- Helper intrinsics in the SDK prelude (KOTO-0047) cover copying a literal into a
  buffer and similar bounded operations; the language core has no string objects.

### Host-call error handling

No exceptions. SDK wrappers (KOTO-0047) present each host call as an `int`-returning
function using a uniform convention:

- Calls with a result return it on success (e.g. `file_open` returns a `handle >= 0`,
  `edit_query_text` returns a byte length `>= 0`).
- Failure returns a negative error code (`-(HostErrorCode)`), so `< 0` is the error
  test.
- Status-only calls return `0` on success, `-1` on failure.

```koto
let handle = file_open(path, 8, MODE_WRITE);
if handle < 0 { return; }            // explicit, checked
file_write(handle, doc, n);
file_close(handle);
```

## MVP Coverage Check

The subset must express the full memo loop. Sketch (SDK names finalized in
KOTO-0047):

```koto
fn main() {
    buf path[16];
    buf doc[512];
    str_set(path, "memo.txt");

    let handle = file_open(path, 8, MODE_READ);
    if handle >= 0 {
        let n = file_read(handle, doc, 512);
        file_close(handle);
        edit_load(doc, n);           // hand the document to the host editor
    }

    loop {
        let cp = text_input();       // typed codepoint, 0 if none
        let intent = text_intent();  // edit-intent bitset

        if intent & INTENT_EXIT != 0 {
            let n = edit_query_text(doc, 512);
            let h = file_open(path, 8, MODE_WRITE);
            if h >= 0 { file_write(h, doc, n); file_close(h); }
            exit(0);
        }
        if cp != 0            { ime_feed_char(cp); }
        if intent & INTENT_CONVERT != 0 { ime_convert(); }
        if intent & INTENT_COMMIT  != 0 { ime_feed(IME_COMMIT, 0); }
        if intent & INTENT_BACKSPACE != 0 { edit_delete(DELETE_BACKSPACE); }
        if intent & INTENT_LEFT != 0  { edit_move(DIR_LEFT); }
        if intent & INTENT_RIGHT != 0 { edit_move(DIR_RIGHT); }

        draw_rect(0, 0, 320, 320, COLOR_BG);
        let n = edit_query_text(doc, 512);
        draw_text(0, 0, doc, n);
        ime_query_line(line, 96);
        draw_text(0, 300, line, /* parsed length */);
        yield();
    }
}
```

This exercises every required capability: a **frame loop** (`loop { … yield(); }`),
**drawing** (`draw_rect`/`draw_text`), **typed input** (`text_input`/`text_intent`),
**IME/editor calls** (`ime_feed_char`/`ime_convert`/`edit_*`), **file save/load**
(`file_open`/`read`/`write`/`close`), and **app exit** (`exit`).

## Candidate Comparison

| Approach | Fit to integer VM | Memory predictability | Toolchain cost | Verdict |
| :------- | :---------------- | :-------------------- | :------------- | :------ |
| **Koto-specific AOT language (selected)** | Direct: scalars are `i32`, control flow is branches, buffers are heap offsets. | High: every local/buffer has a static slot/offset; no allocator or GC. | We own a small parser + checker + emitter, reusing the verifier and kbc-asm IR. | **Chosen.** Smallest design that compiles cleanly to today's VM and keeps costs visible. |
| Embed Lua / Rhai / similar dynamic language | Poor: dynamic values, tables, strings, and GC do not map to an `i32` stack VM. | Low: GC heap and dynamic strings make per-app SRAM hard to bound. | Large: port/trim a VM, define sandbox, manage allocator on RP2040. | Rejected for the MVP; revisit only if scripting ergonomics dominate. |
| Author apps in `KBC1` assembly only | Exact, but unreadable for app authors; control flow and strings are manual. | High, but at the cost of authorability. | None beyond kbc-asm (already built). | Kept as the IR/debug layer, not the authoring surface. |
| Compile a subset of an existing static language (e.g. Rust/C) to `KBC1` | Possible, but the frontend and semantics are far larger than needed. | High once trimmed, but the trimming is the hard part. | Very large frontend for little MVP gain. | Rejected for the MVP. |

The VM-selection spike already chose a custom VM over Wasm/Lua/mruby on the same
RP2040 grounds; choosing a matching custom source language keeps the whole stack
small, deterministic, and host-side-tooled.

## Out Of Scope For The MVP

Explicitly excluded from the first language and compiler:

- Dynamic objects, tables, maps, or dynamically-sized collections.
- Garbage collection or any runtime allocator.
- Closures, first-class functions, and function pointers.
- Generics, traits, or type parameters.
- Recursion (rejected at compile time given the flat 48-slot register file).
- Floating point.
- Modules and namespaces. (KOTO-0183 later added flat textual
  `include "file.koto";` splitting — see "Source-file splitting" above — but
  there is still one global namespace and no linkage unit.)
- A large standard library; only the bounded SDK prelude (KOTO-0047) is provided.
- Exceptions / unwinding (errors are explicit `int` results).

These can be revisited after the memo app and a few small apps validate the MVP.

## Memory And Limits

These mirror the simulator VM profile fixed in
[Runtime ABI: Simulator VM Profile](RUNTIME_BYTECODE_ABI.md#simulator-vm-profile);
the compiler's `MIN_STACK`/`HEAP_PROFILE` and scratch-slot constants are paired
with it so compiled apps always load on that VM.

- App locals map onto the 48 VM local slots (45 user + 3 codegen scratch). The
  register file is flat and shared across functions (each function gets a disjoint
  range), but `let`s are block-scoped and their slots are reused once a block ends
  ([KOTO-0092](../issues/main/KOTO-0092-compiler-local-slot-reuse.md)): a `let` is visible
  only to the end of its enclosing `{ }` block, and disjoint blocks (an `if`'s two
  arms, sequential `if`/`while`/`loop` bodies) reuse the same slots. The limit is
  therefore the *peak* number of simultaneously-live locals across a program's call
  chain, not the static total of `let`s; the compiler errors only if that peak
  exceeds the available slots. (`buf` declarations are heap-allocated separately and
  are not block-scoped.)
- All buffers and string literals are placed in the app heap; the compiler sums
  their sizes, plus SDK compile-time heap models such as `ActorArray`, and emits a
  `.heap` request for that exact amount (floored at a small minimum, not a fixed
  profile; per-app heap, KOTO-0096). The runtime gives the app a heap of exactly
  that size, up to the `RuntimeLimits` device ceiling (16 KB) and the package
  manifest's `sram_work_bytes` budget. Long-lived game state should live in these
  heap-backed models rather than consuming additional user local slots.
- `.stack` / `.calls` requests are derived from the deepest expression/call nesting
  the compiler produces, floored at the 16-slot / 4-frame simulator profile.
- The local file, operand stack, and heap are deliberately right-sized bounds, not
  hard ceilings imposed by the bytecode: the `store_local`/`load_local` operand is
  a byte (up to 256 slots) and the header carries 32-bit stack/heap requests. They
  are kept small and predictable for RP2040-class SRAM, and raised deliberately
  (with rationale) when a real app needs it rather than by default.

## Follow-Up Issues

- [KOTO-0046](../issues/main/KOTO-0046-koto-language-compiler-mvp.md): implement the parser,
  semantic checks, and bytecode emission for this subset.
- [KOTO-0047](../issues/main/KOTO-0047-bytecode-sdk-prelude.md): define the SDK prelude
  (`draw_text`, `text_input`, `ime_feed`, `file_*`, `yield`, `exit`, intent/dir/mode
  constants) that these programs call.
- [KOTO-0048](../issues/main/KOTO-0048-app-build-package-loop.md): build `apps/<id>/src`
  sources into `sdcard_mock` bytecode with drift checks.
- [KOTO-0041](../issues/main/KOTO-0041-bytecode-memo-app.md): rewrite Koto Memo as
  `apps/memo/src/main.koto` compiled through this path.
