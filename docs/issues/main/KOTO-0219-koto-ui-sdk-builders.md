# KOTO-0219: KotoSDK UI builders and event API

- Status: done
- Type: feature
- Priority: P1
- Requirements: FR-SDK-1, FR-SDK-2, FR-SDK-5, FR-SDK-9, FR-RT-4, NFR-MEM-2, NFR-PORT-1, NFR-REL-1, NFR-I18N-1, NFR-I18N-2, NFR-I18N-3
- Related: KOTO-0047, KOTO-0052, KOTO-0193, KOTO-0217, KOTO-0218
- Roadmap: [KotoUI App ABI Roadmap](../../planning/KOTOUI_APP_ABI_ROADMAP.md)

## Goal

Expose the KOTO-0217/0218 component ABI through named Koto language SDK APIs so
apps can build bounded UI descriptions and consume events without hard-coded
host-call IDs or hand-encoding binary records.

## Implementation Progress

- Added compiler-prelude lifecycle intrinsics for `ui_capabilities`, `ui_mount`,
  `ui_update`, `ui_present`, `ui_poll_event`, and `ui_reset`. Value-producing
  calls use the existing value-or-negative-status projection; status-only calls
  retain the fixed status convention, and compiler tests pin every canonical
  host-call mnemonic.
- Added predefined v1 constants for the ABI/Host ABI versions, seven node kinds,
  visible/enabled/explicit-LTR/ellipsis flags, three alignments, ten response
  kinds, stable UI-relevant errors, all advertised capacities, and the IME
  capability bit. Capacity values now originate in `koto-core`, are reused by
  KUC1 encoding and mount validation, and are imported by the compiler so the
  SDK cannot drift from the runtime.
- Added the checked-in `sdk/koto_ui.koto` standard source with atomic KUI1
  header begin/finish helpers and named caller-buffer builders for Label,
  Button, Checkbox, List, TextField, Panel, and Dialog. Common bounds, IDs,
  geometry, flags, per-field capacity, and kind-specific arguments are checked
  before a 48-byte record is changed. Compiler/VM tests pin all seven kind bytes
  and prove an invalid Label argument leaves the destination record untouched.
- Synchronized `kbc-asm`'s symbolic host-call resolver with the six UI calls;
  an assembler regression test pins each mnemonic to the canonical VM ID.
- Added KUE1 validation plus named response, widget ID, value/index, aux, text
  pointer, and text length accessors. A compiled VM test decodes a complete
  text-bearing event and rejects a non-zero reserved field.
- Upgraded `ui_mount_finish` to run bounded multi-pass packet validation before
  sealing the KUI1 lengths. The passes cover header/table layout, duplicate IDs,
  prior container references, containment geometry, text/value ranges, canonical
  UTF-8 and TextField cursor boundaries, aggregate arena/TextField/List/Dialog
  capacities, kind-specific arguments, and Dialog action references. Splitting
  validation keeps each inlined pass below the VM's 45 user-local ceiling.
- Added a compiled VM regression that corrupts duplicate IDs, hierarchy, a
  multibyte UTF-8 sequence, TextField capacity, and a Dialog action in turn.
  Every failure is deterministic and leaves the unsealed header unchanged;
  repairing the same caller buffer then produces one successful mount.
- Added KUC1 validation and named locale pointer/length, generation, direction,
  flags, and capability accessors, plus matching `LocaleChanged` accessors.
  `ui_locale_fallback_rank` implements exact (0), language (1), `en-US` (2),
  and no-match (-1), after which the app uses its embedded default. A VM test
  covers `en-US`, `ja-JP`, unknown `fr-CA`, and test-only `qps-ploc`.
- Added an executable lifecycle convention test covering all six calls against
  both a supporting host and an older host. It pins value-call failures to `-1`,
  status failures to `-UNSUPPORTED`, and verifies the fixed verifier/VM stack
  shape remains executable across the complete sequence.
- Added bounded KUP1 begin/finish helpers and named builders for all ten v1
  properties: text, enabled, visible, checked, selection, TextField value,
  bounds, List rows, Dialog open, and focus request. Exact-byte tests cover all
  property IDs, multibyte payload data, and invalid-record atomicity.
- Added `sdk/examples/koto_ui_counter.koto`. The compiled VM regression feeds an
  Activated event, proves the scene mounts once, observes one targeted KUP1
  label update, then reaches idle `yield_frame`; the source also demonstrates
  capability/older-host handling, event draining, locale notification, errors,
  present, reset, and exit.
- Added shared compiler metadata for the six lifecycle intrinsics and LSP
  `textDocument/completion`. Completion combines those canonical intrinsics with
  functions/constants from the root and every include, so all KotoUI builders
  and accessors appear after including `koto_ui.koto`. Removed the obsolete
  oversized validator so full SDK language analysis is diagnostic-free.
- Added full-capacity VM coverage for 32 mount nodes, the complete 2,048-byte
  retained data arena, and 16 targeted update records. Counts 33/17 fail before
  clearing the caller buffer; the exact maxima seal and reach their host calls.
- Completed host validation with `cargo test --workspace --exclude koto-pico`,
  `python harness/build_apps.py --check`, and
  `python harness/check_project.py`. Pico is excluded from the Windows-host
  workspace run because its ARM-only instructions require the target toolchain.

## Acceptance Criteria

- [x] Add compiler-sourced SDK constants for ABI version, node types, flags,
  alignments, response kinds, error codes, capacities, and capability bits.
- [x] Provide named lifecycle wrappers for capability query, mount, update,
  present, event poll, and reset with the existing fixed-result/status convention.
- [x] Provide bounded builder helpers for label, button, checkbox, list, text
  field, panel, and dialog records using caller-provided `buf` storage.
- [x] Builders validate IDs, geometry, indices, UTF-8 byte ranges, hierarchy,
  and capacity before submission and return deterministic negative status on
  failure without corrupting the existing description.
- [x] Provide readable event accessors for widget ID, response kind, value/index,
  and text metadata so normal app code does not use raw record offsets.
- [x] Provide locale tag, locale generation, direction, and `LocaleChanged`
  accessors plus a documented exact/language/`en-US` resource fallback helper;
  app code selects resolved strings before passing them to UI builders.
- [x] Expose named inherited/LTR and clip/ellipsis flags without presenting the
  reserved RTL/wrap values as supported v1 features.
- [x] Document buffer ownership: apps retain business values, builders serialize
  snapshots, and host updates/events are the only retained interaction channel.
- [x] Include a minimal source example demonstrating mount-once, event loop,
  targeted updates, idle yield, error handling, and reset/exit.
- [x] Compiler tests prove wrappers use the canonical host-call constants and
  verify correct stack effects on both success and failure paths.
- [x] Tests cover exact encoded bytes, full buffers, multibyte text, duplicate
  IDs, invalid parent/action references, event decoding, `en-US`/`ja-JP`,
  unknown-locale fallback, `qps-ploc`, and older-host fallback.
- [x] Update `KOTO_SDK.md`, language diagnostics, intrinsic listings, and editor
  tooling metadata/completion sources that enumerate SDK functions.
- [x] `harness/build_apps.py --check`, compiler/LSP tests, workspace tests, and
  `python harness/check_project.py` pass.

## Notes

The Koto language currently exposes `int` and heap-backed `buf`; the SDK should
fit that model rather than requiring a new general object system. Raw encoding
helpers may exist for tests, but documented app code uses named builders.
