# KOTO-0225: Koto language enums and SDK constant domains

- Status: done
- Type: feature
- Priority: P1
- Requirements: FR-PKG-3, FR-SDK-5, FR-RT-4, NFR-DEV-3, NFR-DEV-4, NFR-REL-1
- Related: KOTO-0046, KOTO-0047, KOTO-0052, KOTO-0193, KOTO-0194, KOTO-0219, KOTO-0220

## Goal

Add zero-runtime-cost, namespace-qualified integer enums to the Koto App
language, use them to organize SDK constant families, and migrate appropriate
official Apps away from unrelated magic integers without changing the VM or
wire ABIs. Gallery is the proving App; the migration audit covers the rest of
the checked-in Koto sources rather than treating enum support as UI-only.

## Proposed Language Contract

```koto
enum GalleryLocale {
    En = 0,
    Ja = 1,
    QpsPloc = 2,
}

if locale == GalleryLocale::Ja {
    // Enum members are compile-time `int` constants in v1.
}
```

- Enums are top-level declarations and participate in textual `include`
  expansion like `const`, `data`, and functions.
- Members use `EnumName::Member`. The first implicit value is zero and each
  following implicit value increments the prior value; an explicit signed
  integer literal resets the sequence.
- v1 is intentionally integer-backed and not a new runtime type. Enum members
  may be passed to existing `int` parameters, compared with integers, stored in
  buffers, and constant-folded. No bytecode instruction, heap object, RTTI, or
  host ABI change is introduced.
- Enum names and member names must be unique in their respective scopes.
  Unknown namespaces/members, overflow during implicit increment, malformed
  explicit values, and collisions across included files are attributed compile
  errors. Duplicate numeric values are allowed for protocol aliases.
- `match`, exhaustive checking, payload variants, bitflags, and strong
  enum-to-int separation are out of scope. Composable masks such as
  `UI_FLAG_VISIBLE | UI_FLAG_ENABLED` and `INTENT_*` remain integer constants.

## SDK Adoption

The compiler-sourced SDK prelude should expose these runtime-sourced domains:

- `UiNodeKind`, `UiAlignment`, `UiResponse`, and `UiError`;
- `FileMode`, `ImeKey`, `EditDirection`, and `DeleteKind`;
- `UiProperty` from `sdk/koto_ui.koto` for KUP1 builder internals.

Existing flat constants (`UI_RESPONSE_ACTIVATED`, `MODE_READ`, `DIR_LEFT`, and
similar) remain compatibility aliases for existing Apps and KBC source builds.
Capacity constants, ABI versions, byte sizes, capability bits, intent masks,
and UI flags remain flat constants because they are values or bitsets rather
than mutually exclusive domains. SDK documentation uses enum names for new
examples and records the compatibility policy.

## Existing-Source Migration Audit

Classify every grouped integer family before editing it. Initial candidates are:

- KotoUI Gallery locale resource, widget ID, and semantic-status domains;
- Memo interaction/dialog modes;
- KotoBlocks, KotoMines, KotoSnake, KotoRun, KotoRogue, and Sokoban game states,
  directions, hazards, tile/cell kinds, and message kinds;
- retained sprite/text slot IDs only where the values are identities rather
  than arithmetic offsets.

Do not convert RGB565 colors, dimensions, capacities, bit masks, counters,
buffer-layout offsets, array indices used arithmetically, or isolated constants
merely because they are integers. Record the audited groups and rationale so a
large mechanical rewrite does not disguise semantics or increase code size.

## Acceptance Criteria

- [x] Lexer/parser/AST support the documented `enum` declaration,
  auto-increment, explicit signed values, trailing commas, and `Name::Member`
  expressions across root and included sources.
- [x] Name resolution and codegen lower enum members to compile-time integers;
  representative enum and equivalent `const` programs have identical executable
  opcode/rodata output, with no additional VM slots, heap, or host calls.
- [x] Diagnostics cover duplicate enum/member names, unknown enum/member,
  include collisions, implicit-value overflow, and malformed declarations with
  the correct included-file source location.
- [x] Public compiler symbols distinguish enum declarations and members so LSP
  definition, hover, document symbols, and completion after `::` work for both
  local and SDK enums, including unsaved include overlays.
- [x] VS Code syntax highlighting and language documentation cover declarations,
  qualified members, integer-backed v1 semantics, and the explicit non-goals.
- [x] SDK prelude exposes `UiNodeKind`, `UiAlignment`, `UiResponse`, `UiError`,
  `FileMode`, `ImeKey`, `EditDirection`, and `DeleteKind` from the same Rust ABI
  sources as the current flat constants; `sdk/koto_ui.koto` uses `UiProperty`.
- [x] Existing flat SDK constants remain source-compatible aliases, and compiler
  tests prove old and enum-based SDK examples emit the same host-call arguments.
- [x] Audit all checked-in `*.koto` Apps, document accepted/rejected candidates,
  and migrate at least Gallery plus one non-UI App representing a state machine.
- [x] Migrated Apps preserve scenario results, golden frames, budget thresholds,
  package behavior, and executable opcode/rodata content except where a reviewed
  source/debug-map change necessarily changes KDBG metadata.
- [x] Update `KOTO_APP_LANGUAGE.md`, `KOTO_SDK.md`, SDK examples, Gallery docs,
  LSP docs/tests, and scaffold/example guidance to prefer enum domains for new
  mutually exclusive integer families.
- [x] Compiler, LSP, VS Code extension, migrated-App, workspace, build/package,
  budget, and project harness checks pass.

## Completion Evidence

- Added compile-time integer enums, include-aware diagnostics and symbols, SDK
  domains with flat compatibility aliases, LSP enum navigation/completion, and
  VS Code syntax highlighting. Enum/`const` equivalence and SDK host-call
  argument compatibility are pinned by compiler tests.
- Audited all checked-in App integer families in
  `docs/planning/KOTO-0225-ENUM-AUDIT.md`; migrated Gallery and KotoRun while
  retaining values, masks, offsets, and arithmetic identities as flat constants.
- Corrected the Gallery layout and clipping, checkbox placement/activation,
  spatial arrow navigation, Tab ID traversal, list focus behavior, and the
  distinction between component focus and TextField editing/caret state.
- Validation passed for compiler/LSP tests, the host workspace excluding the
  embedded-only `koto-pico` crate, the Pico `thumbv6m-none-eabi` library check,
  simulator Gallery scenarios/goldens, App build/package synchronization,
  runtime budgets, formatting, and the project harness. The VS Code TextMate
  grammar also parses as valid JSON.

## Notes

This is a language/compiler and SDK-source compatibility feature, not a VM ABI
feature. Keeping v1 enum members integer-backed makes adoption possible before
Koto has general structs or a static type system, while namespace qualification
solves the immediate collision/readability problem visible in Gallery. Strong
typing or exhaustive `match` may be proposed later using migration evidence
from this issue.
