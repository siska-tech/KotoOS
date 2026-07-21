# KOTO-0228: Koto static records and inline methods

- Status: done
- Type: feature
- Priority: P1
- Requirements: FR-PKG-3, FR-RT-4, NFR-MEM-2, NFR-DEV-3, NFR-DEV-4, NFR-REL-1
- Related: KOTO-0045, KOTO-0046, KOTO-0092, KOTO-0104, KOTO-0139, KOTO-0183, KOTO-0193, KOTO-0225

## Goal

Add a deliberately small structured-data model to the Koto App language:
top-level statically allocated heap records, typed field access, and methods
that lower to the existing statically resolved inline-function model. Improve
App state organization without adding a runtime allocator, garbage collection,
dynamic dispatch, runtime type metadata, or a VM / KBC / host ABI change.

The syntax must expose the static storage lifetime. V1 does not make
`Player { ... }` a general expression because that spelling normally implies a
new value or object on each evaluation, while Koto reserves one heap region per
declaration for the lifetime of the App.

## Proposed Language Contract

```koto
struct Player {
    x: int,
    y: int,
    alive: bool,
}

static player: Player = {
    x: 10,
    y: 20,
    alive: true,
};

impl Player {
    fn move_by(self, dx: int, dy: int) {
        self.x = self.x + dx;
        self.y = self.y + dy;
    }

    fn is_alive(self) -> bool {
        return self.alive;
    }
}

fn draw_player(value: Player) {
    if value.is_alive() {
        draw_rect(value.x, value.y, 8, 8, 65535);
    }
}

fn main() {
    player.move_by(2, -1);
    draw_player(player);
}
```

### Storage and initialization

- `struct` defines a compile-time field layout; it does not define a VM object
  or introduce a runtime type descriptor.
- `static name: Type = { ... };` is a top-level declaration. It reserves exactly
  one mutable record in the App heap for the whole App lifetime. A `static`
  declaration inside a function, loop, or conditional is a compile error.
- The initializer accepts literals, `const` values, enum members, and the same
  bounded compile-time integer expressions accepted by other constant data.
  It is materialized in the initial heap image rather than executed each time
  control reaches a source location. An omitted initializer, if supported,
  zero-initializes every field and must be documented explicitly.
- Every field initializer is named. Missing, duplicate, and unknown fields are
  compile errors; source order of initializer entries does not affect layout.
- V1 fields are `int` or `bool`, each stored as one little-endian 32-bit VM word.
  Layout follows declaration order with checked 4-byte offsets and size. Packed
  `u8` / `i16` / `u16` ABI records remain SDK-owned helpers such as `ActorArray`.
- Each static identifier is a compiler-tracked typed heap reference. Passing it
  to a parameter of the same struct type passes its base offset; it does not copy
  the record.

### Fields and methods

- `value.field` emits the existing `load32`; `value.field = expression` emits
  the existing `store32`. The VM's normal whole-heap bounds check remains the
  runtime safety boundary, while the compiler validates the receiver and field.
- `impl Type` may contain receiver methods with an explicit `self` parameter.
  `value.method(args...)` resolves statically and lowers to the same inline
  expansion as a free function whose first parameter is the typed base offset.
- Methods may return no value, `int`, or `bool`. There are no virtual methods,
  function values, traits, inheritance, visibility rules, or associated
  functions in V1.
- Multiple `impl` blocks may be accepted across textual includes, but method
  names for one struct must be unique and diagnostics must identify the second
  definition's actual source file.
- Inline expansion retains the current recursion rejection, local-slot reuse,
  debug mapping, and code-window cost model. Method syntax does not hide code
  duplication at call sites.

### Deliberate V1 restrictions

The following are rejected rather than assigned surprising reference, copy, or
allocation semantics:

```koto
let a = Player { x: 1, y: 2, alive: true }; // no constructor expression
let b = player;                              // no stored aliases or record copy
player = other;                              // no reference assignment/copy
return player;                               // no struct return values
```

Struct parameters and method receivers are the only V1 reference-passing
positions. Nested struct fields, local/static struct arrays, dynamic allocation,
record equality, destructuring, payload enums, exhaustive `match`, enum `impl`,
generics, traits, ownership, borrowing, and destruction are out of scope.
Keeping `Type { ... }` unused as an expression preserves it for a later true
value-construction or allocator-backed design with explicit semantics.

## Implementation Scope

- Extend lexer/parser/AST and compiler symbols with `struct`, `static`, `impl`,
  dot field access, dot field assignment, and receiver method calls.
- Add a small semantic layer that tracks `int`, `bool`, and struct-reference
  types for statics, parameters, receivers, fields, calls, assignments, and
  returns. Do not treat the currently ignored optional type annotation as
  sufficient without enforcing the relevant type relations.
- Compute deterministic static layouts and heap offsets before code emission,
  append constant initial bytes to the KBC heap image, and include record bytes
  in the exact per-App heap request and budget diagnostics.
- Lower field and method operations through existing KBC instructions and
  inline machinery. No new opcode, verifier rule, VM value kind, allocator, host
  call, package field, or ABI version is introduced.
- Expose struct, static, field, impl, method, and typed-parameter symbols through
  the compiler library and Koto LSP; add definition, hover, document symbols,
  receiver-aware completion after `.`, and included-file diagnostics.
- Update VS Code syntax highlighting and the language/SDK documentation. Add a
  focused checked-in sample or fixture that demonstrates persistent structured
  state without implying dynamic construction. Broad App migration is separate
  follow-up work.

## Acceptance Criteria

- [x] The documented `struct`, top-level `static`, `impl`, field read/write, and
  receiver-call syntax parses across root and included sources with stable
  source locations.
- [x] Static initializers are compile-time-only, allocate one deterministic
  heap record per declaration, initialize the KBC heap image, and increase the
  header heap request by the checked layout size without consuming one local
  slot per field.
- [x] `int` and `bool` fields round-trip through `load32` / `store32`; equivalent
  named-field and hand-written heap-access fixtures produce the same observable
  VM behavior and resource bounds.
- [x] Method calls resolve by receiver type and lower to the existing inline
  function machinery with no new VM opcode, runtime metadata, dynamic dispatch,
  recursion behavior, or ABI change.
- [x] Compiler diagnostics reject block-local `static`, `Type { ... }`
  expressions, missing/duplicate/unknown fields, duplicate types/statics/
  methods, unknown impl targets, invalid receivers, struct/int/bool mismatches,
  stored aliases, struct assignment/return, unsupported field types, layout
  overflow, and App heap-limit overflow.
- [x] Included definitions retain correct collision and error attribution;
  method expansion retains correct KDBG source mappings and slot-map reporting.
- [x] LSP definition/hover/document-symbol/completion tests cover structs,
  statics, fields, methods, parameters, SDK/local symbol collisions, and unsaved
  include overlays; VS Code highlighting recognizes the new declarations.
- [x] A focused sample or compiler runtime fixture proves that the same static
  record persists and can be explicitly mutated/reset across frames. Tests also
  prove that no loop-local constructor-like spelling is accepted.
- [x] `KOTO_APP_LANGUAGE.md`, relevant SDK guidance, compiler/LSP documentation,
  and sample guidance explain static lifetime, reference passing, initialization,
  inline code-size cost, heap cost, and all V1 non-goals.
- [x] Compiler, LSP, VS Code extension, workspace, App build/package, runtime
  budget, and `python harness/check_project.py` checks pass.

## Notes

This feature generalizes the static heap/accessor pattern already proven by
`ActorArray`, but does not replace its compact 12-byte packed representation in
V1. The first adoption target should be a small cohesive `int`/`bool` App-state
record; packed protocol packets, large actor pools, and arithmetic tables should
remain in their existing bounded representations.

The design intentionally separates type layout (`struct`), storage (`static`),
and behavior (`impl`). That separation makes the one-instance lifetime visible
in source and avoids teaching constructor syntax whose apparent allocation
behavior the current VM cannot provide.

## Implementation Notes (2026-07-17)

- The compiler carries scalar and struct-reference types through parameters,
  locals, calls, assignments, returns, fields, and receiver calls. Static record
  bytes share the KBC initial heap image; fields use the existing `load32` /
  `store32` instructions and methods reuse call-site inline slots.
- Compiler-library symbols and Koto LSP expose structs, statics, fields, methods,
  and struct-typed parameters. Receiver completion resolves top-level statics and
  struct parameters. The VS Code grammar recognizes declarations and members.
- `sdk/examples/static_record.koto` is the focused fixture. Tests cover layout,
  initialization, mutation, methods, rejection cases, include attribution, and
  persistence across `yield_frame`.
- The existing sample suite was audited in
  `docs/planning/KOTO-0228-SAMPLE-AUDIT.md`. Seven samples now use typed static
  records for cohesive App-lifetime state; byte strings, packed SDK/ABI data,
  and stateless samples intentionally retain their existing representations.
- Format, host-workspace tests (`--exclude koto-pico`), Clippy, App
  build/package synchronization, runtime-budget, and project-harness checks
  pass. The unqualified Windows-host workspace command remains inapplicable
  because it attempts to assemble ARM-only Pico firmware; the documented host
  exclusion was used.
