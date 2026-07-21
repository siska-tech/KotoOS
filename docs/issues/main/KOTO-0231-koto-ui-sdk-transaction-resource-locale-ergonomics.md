# KOTO-0231: KotoUI SDK transaction, resource, and locale ergonomics

- Status: done
- Type: feature
- Priority: P1
- Requirements: FR-SDK-1, FR-SDK-2, FR-SDK-5, FR-SDK-9, FR-RT-4, NFR-PERF-1, NFR-MEM-2, NFR-DEV-3, NFR-DEV-4, NFR-REL-1, NFR-I18N-1, NFR-I18N-2
- Related: KOTO-0193, KOTO-0219, KOTO-0220, KOTO-0221, KOTO-0228, KOTO-0229, KOTO-0230
- Follow-up: [KOTO-0232](KOTO-0232-koto-ui-compile-time-packet-capacity-helpers.md)
- Roadmap: [KotoUI App ABI Roadmap](../../planning/KOTOUI_APP_ABI_ROADMAP.md)

## Goal

Complete the common App-facing KotoUI SDK path after KOTO-0230: submit a
finished update without repeating packet-length plumbing, update widget text
directly from an indexed resource line, and classify canonical locale tags
without App-local byte comparisons. Split the growing SDK source into focused
internal includes while preserving `include <sdk/koto_ui.koto>;`, the existing
low-level APIs, caller ownership, observable presentation, and all runtime
budgets.

## Motivation

KOTO-0229 and KOTO-0230 removed record cursors, line scans, and List wire
arithmetic, but Gallery and File Note still repeat three smaller orchestration
patterns:

- five update paths spell `begin`, `finish`, `ui_update`, and their status
  checks around otherwise semantic property calls;
- localized text updates repeatedly pair `line_ptr(index)` with
  `line_len(index)`, exposing a two-call bridge that the SDK can decide once;
- both Apps decode `ja` and `qps-ploc` with hand-written byte comparisons even
  though capabilities and LocaleChanged records already carry canonical tags.

The public `sdk/koto_ui.koto` source has also grown to roughly 1,800 lines of
low-level ABI encoding, validation, stateful builders, resources, events, and
locale helpers. A compatibility aggregator should keep the one-line App include
while giving each SDK concern a focused source and source-map identity.

Koto methods inline at call sites. Convenience must replace repeated work, not
layer validation wrappers until KBC or fuel grows. Gallery's completed
KOTO-0230 baseline is 285,334-byte KBC, 2,888-byte App heap, 44 user slots,
58,487 peak fuel, and 8 peak host calls; it remains the primary feasibility
gate.

## Proposed Contract

### Update submission

- Add an optional `UiUpdateBuilder.submit()` receiver method that seals the
  active transaction and calls `ui_update` with the derived exact packet
  length. It returns `0` on accepted submission or the first negative builder,
  finalizer, or host status.
- `submit` does not call `ui_present`. Presentation remains an explicit App
  operation so multiple accepted updates may share one present and damage/host
  calls remain observable.
- Preserve `finish()` and direct `ui_update(packet, len)` for compatibility,
  exact-length inspection, fixtures, and batching that needs the packet before
  submission. Failed `begin` and property calls remain sticky so normal App
  code needs only one transaction-result check.

### Text-resource bridge

- Add `UiUpdateBuilder.text_resource(widget_id, resource, line)` or an
  equivalently named typed receiver method. It consumes one completed
  `TextResource` line without exposing the resource's private offset table or
  requiring separate pointer/length calls.
- Validate resource completion, sticky status, line index, builder state,
  update capacity, and copy bounds before record/data mutation. The first
  resource or builder failure remains deterministic and sticky.
- Keep scalar `text(widget_id, src, len)` and `TextResource.line_ptr` /
  `line_len` available as compatibility and general byte-range APIs. Do not add
  per-widget mount-resource overloads until measured consumers justify the API
  surface.

### Locale matching

- Add a symbolic exact/language/no-match result domain and a bounded canonical
  locale matcher. Exact tags match only byte-for-byte; a language candidate
  matches either the exact language tag or the same language followed by `-`.
- Reject malformed locale/candidate tags deterministically and define behavior
  for empty, maximum-length, pseudolocale, unrelated-language, and prefix-only
  inputs.
- Keep locale selection order, unsupported-locale fallback, package asset
  paths, and product-specific resource IDs in the App. The helper must not
  become a translation catalog, asset loader, or implicit English fallback.
- Preserve `ui_locale_fallback_rank` for source compatibility and existing
  fallback-policy consumers.

### SDK source layout

- Keep `sdk/koto_ui.koto` as the stable public compatibility aggregator.
  Split implementation into focused internal includes for low-level ABI and
  validation, resources, stateful builders, and events/locale matching; exact
  filenames may follow dependency and source-map constraints.
- Avoid duplicate definitions and include cycles. Root and nested include
  diagnostics, overlay compilation, definition lookup, hover, and receiver
  completion must point at the focused source that owns each symbol.
- Preserve generated wire bytes, VM/host ABI, KBC format, package format,
  allocator behavior, and all existing public symbol names.

## Acceptance Criteria

- [x] Add optional typed update submission that derives the exact packet length,
  calls `ui_update` once, returns the first deterministic error, and never calls
  `ui_present`; existing `finish` and scalar submission remain supported.
- [x] Successful, failed-begin, failed-property, failed-finalizer, rejected-host,
  completed, reused, and active-re-entry update transactions are tested for
  sticky status, packet atomicity, host-call count, and no accidental present.
- [x] Add a typed TextResource-to-update text method that validates one resource
  line once, copies exact UTF-8 bytes, and rejects incomplete/failed resources,
  invalid indexes, malformed state, full capacities, and every data boundary
  before exposing a sealed packet.
- [x] Typed resource text output is byte-for-byte identical to the existing
  `text(widget, line_ptr(i), line_len(i))` path for ASCII, Japanese, empty, and
  maximum-capacity lines; scalar text and resource accessors remain supported.
- [x] Add documented symbolic locale match results and canonical exact/language
  matching with deterministic coverage for `en`, `en-US`, `ja`, `ja-JP`,
  `qps-ploc`, unrelated tags, invalid tags, empty input, prefix traps, and all
  length boundaries; preserve `ui_locale_fallback_rank` behavior.
- [x] Migrate Gallery and File Note update paths to the submit/resource bridge
  and locale matcher. Remove their direct locale-tag byte comparisons and
  localized update `line_ptr`/`line_len` pairs without changing visible text,
  fallback order, locale assets, events, app IDs, host submission/presentation
  order, sandbox behavior, or golden frames.
- [x] Split the SDK behind the unchanged `include <sdk/koto_ui.koto>;` public
  entry point, with no include cycle or duplicate symbol and with compiler/LSP
  source attribution, overlay includes, definition, hover, and receiver
  completion covering every focused file.
- [x] Existing low-level encoders, scalar/pointer methods, KUI1/KUP1/KUE1 bytes,
  VM opcodes, host ABI, KBC/package formats, and caller-owned allocation model
  remain unchanged; conformance fixtures and pre-existing Apps still compile.
- [x] Record before/after KBC size, executable code words, App heap request,
  user-slot peak, fuel peak, and host-call peak for Gallery and File Note.
  Gallery remains at or below 44 user slots, 58,487 peak fuel, and 8 peak host
  calls; File Note remains at or below 44 user slots, 30,095 peak fuel, and 9
  peak host calls. No budget is raised for SDK convenience or source splitting.
- [x] Document submit-versus-present semantics, resource lifetime/error
  propagation, locale match/fallback ownership, focused include ownership,
  compatibility APIs, and inline code-size tradeoffs.
- [x] Format, Clippy, host-workspace tests, compiler/LSP tests, App build/package
  synchronization, Gallery/File Note simulator regressions and golden frames,
  deterministic budget runs, and `python harness/check_project.py` pass.

## Non-goals

- Implicit `ui_present`, automatic frame yielding, hidden retry, or combining
  multiple active builders into one transaction.
- A runtime locale catalog, translation keys, plural/message formatting,
  locale fallback policy, filesystem/package loading, or dynamic locale data.
- Resource overloads for every mount/widget method, fluent builder chaining,
  dynamic strings/arrays, allocation, or stored struct aliases.
- Removing the public `sdk/koto_ui.koto` include, low-level pointer/length APIs,
  or exact-byte conformance surfaces.

## Notes

Prefer one direct resource-to-text encoding path over a wrapper that separately
inlines both `line_ptr` and `line_len`. Likewise, `submit` should expand the
existing finish logic once rather than add a second validation pass. Measure
Gallery and File Note before committing the split because debug source tables
and include ownership may change total KBC bytes even when executable words
shrink.

Treat SDK splitting as a compatibility-preserving part of this issue, not an
excuse to rename APIs or alter behavior. If source-map or include semantics
require compiler changes beyond focused-file attribution, document and split
that compiler work into a follow-up rather than weakening diagnostics.

## Implementation Notes (2026-07-17)

- `UiUpdateBuilder.submit()` now performs one sticky seal-and-submit operation
  and deliberately leaves `ui_present()` to the App. `text_resource` validates
  a completed indexed line and all packet boundaries before writing its KUP1
  record or bytes. Compatibility `finish`, scalar `text`, `line_ptr`,
  `line_len`, and low-level encoders remain unchanged.
- `UiLocaleMatch::{None, Language, Exact}` and `ui_locale_match` provide bounded
  policy-free classification. Gallery and File Note retain asset paths,
  candidate order, and English fallback; they now use the matcher and direct
  resource updates without App-local tag comparisons or paired line accessors.
- `sdk/koto_ui.koto` remains the public aggregator. ABI/validation, resources,
  stateful builders, and events/locale live in four focused internal sources.
  Compiler and LSP tests cover nested overlays, diagnostics, definitions,
  hover, and receiver completion at those owner files.
- A follow-up ergonomics pass adds checked `ui_mount_capacity` /
  `ui_update_capacity` helpers. The compiler evaluates only these two calls in
  top-level constants and permits prior integer constants as `buf` sizes, so
  Apps now name record and data capacities while SDK-owned header/stride
  arithmetic remains out of App source. Compiler folding reads the canonical
  `koto-core` layout limits rather than duplicating wire constants. App comments
  record each data arena's semantic byte sum; removing old conservative slack
  lowers Gallery/File Note heap requests by 153/84 bytes.

Measured artifacts and deterministic scenarios (the before column is the
completed KOTO-0230 baseline; code words were recompiled from that source):

| App | KBC before → after | Code words before → after | Heap request | User slots | Fuel peak | Host calls peak |
| :-- | --: | --: | --: | --: | --: | --: |
| Gallery | 285,334 → 293,726 B | 56,092 → 57,794 | 2,888 → 2,735 B | 44 → 44 | 58,487 → 58,487 | 8 → 8 |
| File Note | 174,074 → 184,368 B | 34,107 → 36,109 | 2,343 → 2,259 B | 44 → 44 | 30,095 → 30,095 | 9 → 9 |

The KBC/code growth is the explicit cost of inlined validation and submission;
right-sized packet arenas reduce runtime memory while slots, fuel, and
host-call peaks are unchanged. The File Note
budget gate was synchronized from its stale 30,000 value to the already
recorded KOTO-0230 baseline/acceptance ceiling of 30,095; KOTO-0231 itself adds
no runtime or protocol budget.
