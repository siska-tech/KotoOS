# KOTO-0217: App-facing KotoUI ABI and locale contract

- Status: done
- Type: research
- Priority: P1
- Requirements: FR-SDK-1, FR-SDK-2, FR-SDK-5, FR-SDK-9, FR-RT-3, FR-RT-4, NFR-DRAW-1, NFR-MEM-2, NFR-PORT-1, NFR-REL-1, NFR-I18N-1, NFR-I18N-2, NFR-I18N-3
- Related: KOTO-0019, KOTO-0047, KOTO-0208, KOTO-0214, KOTO-0216
- Roadmap: [KotoUI App ABI Roadmap](../../planning/KOTOUI_APP_ABI_ROADMAP.md)

## Goal

Freeze a versioned, bounded, VM-safe contract that lets a Koto app describe
KotoUI controls and receive semantic events without exposing KotoUI Rust memory
layouts or creating an immediate per-property host-call API.

## Acceptance Criteria

- [x] Compare per-control host calls, immediate command streams, and a retained
  serialized description; record the selected approach and rejected tradeoffs.
- [x] Add `docs/spec/KOTOUI_APP_ABI.md` defining a little-endian wire format with
  header/version, node records, stable widget IDs, geometry, control properties,
  string/value ranges, hierarchy, and forward-compatible reserved fields.
- [x] Define v1 representation for label, button, checkbox, list, single-line
  text field, panel, and dialog, including which state is app-owned versus
  retained by the host.
- [x] Define mount/update/present/event-poll/reset lifecycle semantics, including
  repeated calls, app exit, trap recovery, launch of a different app, and host
  capability/version discovery.
- [x] Define fixed event records and ordering for activation, value/selection/text
  changes, submission, cancellation, capacity rejection, and focus changes.
- [x] Define validation and error behavior for hostile offsets/lengths, invalid
  UTF-8, duplicate IDs, cycles, unsupported types/versions, impossible geometry,
  disabled focus targets, and every capacity overflow.
- [x] State that the host never retains raw app-heap pointers across a host call;
  document copy/borrow lifetime and time-of-check/time-of-use behavior.
- [x] Select numeric host-call IDs and the next compatible Host ABI minor; give
  every call a fixed verifier stack effect and success/failure result shape.
- [x] Set and justify RP2040 budgets for nodes, strings, list rows, text storage,
  dialogs, queued events, damage rectangles, session SRAM, and app-heap payload.
- [x] Specify how normalized Shell/device input, KotoIME composition, focus,
  modal routing, damage, and idle frames map to the existing KotoUI contracts.
- [x] Publish canonical valid/malformed binary examples for runtime, compiler,
  simulator, and documentation tests.
- [x] Define locale ownership and discovery without placing translation catalogs
  in KotoUI: bounded BCP 47 tag, generation, direction, English fallback, and a
  semantic locale-change event.
- [x] Define v1 text-direction and overflow flags, supported LTR/ellipsis
  behavior, deterministic missing-glyph fallback, and reserved RTL/wrap values.
- [x] Add `ja-JP`, `en-US`, unknown-locale fallback, and expanded `qps-ploc`
  coverage to the implementation, SDK, sample, pilot, and Shell follow-up issues.
- [x] Update runtime ABI/SDK documentation with reserved design entries, verify
  requirement and issue traceability, and pass `python harness/check_project.py`.

## Notes

This issue freezes the boundary but does not implement host calls. The ABI must
describe semantic components, not mirror `repr(C)` Rust structs; KotoUI may
change its internal layout without requiring a Host ABI major bump.

## Decision Record

- Selected a retained semantic wire format (`KUI1` mount, `KUP1` atomic update,
  `KUE1` event, and `KUC1` capabilities) over per-property calls or per-frame
  immediate reconstruction.
- Reserved Host ABI minor 18 and IDs `0x50` through `0x55`; KOTO-0218 subsequently
  implemented the complete call range and advanced the runtime to minor 18.
- Fixed the v1 limits at 32 nodes, 16 focus entries, 2,048 bytes of host data,
  eight queued events, and an 8 KiB total UI-session SRAM ceiling.
- Published the normative specification and canonical hex fixtures in
  [`KOTOUI_APP_ABI.md`](../../spec/KOTOUI_APP_ABI.md) and added fixture integrity
  checks to `harness/check_project.py`.
- Revised `KUC1` to a 64-byte dynamic capability record carrying a bounded BCP
  47 locale, generation, and direction. KotoUI receives resolved UTF-8 and does
  not own translation tables; v1 supports LTR `en-US`/`ja-JP`, with `en-US`
  fallback and `qps-ploc` used for layout validation.
- Reserved node direction/overflow values and `LocaleChanged` event ID 10 so
  later RTL, shaping, and wrapping can evolve without reinterpreting v1 bits.
- `python harness/check_project.py`, Python syntax compilation, and
  `git diff --check` pass on 2026-07-15.
