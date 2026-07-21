# KOTO-0246: Bounded JSON data decoder for Koto apps

- Status: in progress
- Type: feature
- Priority: P1
- Requirements: FR-SDK-5, FR-RT-3, FR-RT-4, FR-PKG-3, NFR-MEM-2, NFR-PORT-1, NFR-PORT-3, NFR-REL-1, NFR-DEV-3, NFR-DEV-4
- Related: KOTO-0019, KOTO-0047, KOTO-0230, KOTO-0245, KOTO-0247, KOTO-0249

## Goal

Provide Koto applications with one allocation-free, incremental JSON decoder
for bounded Internet and telemetry payloads. The decoder must consume partial
caller-owned buffers and expose the same deterministic token API in KotoSim
and on device without building an unbounded in-memory document tree.

## Acceptance Criteria

- [x] Define a portable `no_std` decoder with fixed maximum nesting, token
  length, numeric length, and parser-state size. Limits and their exact failure
  results are public SDK constants rather than hidden heap behavior.
- [x] Accept JSON split at every byte boundary across successive input chunks
  and emit stable tokens for object/array boundaries, keys, strings, numbers,
  booleans, and null. A caller can pause after any token and resume without
  retaining a pointer into a previous VM buffer.
- [x] Validate UTF-8, escapes, surrogate pairs, number grammar, separators,
  nesting, and trailing data. Malformed, truncated, over-deep, and oversized
  input fails deterministically with a bounded byte offset and fixed error enum.
- [x] Add allocation-free KotoSDK wrappers and examples for selecting named
  fields while safely skipping unknown objects and arrays. Missing, duplicate,
  and wrong-type fields remain distinguishable to application code.
  (Core selection helpers: `JsonValueSkip` chunk-safe skipper,
  `JsonValueKind`/`JsonEvent::value_kind` for wrong-type checks. Prelude
  wrappers `json_reset`/`json_next`/`json_finish`/`json_token`/
  `json_error_code`/`json_error_offset`/`json_consumed`/`json_depth` over host
  calls `0x4A`–`0x4F` (host ABI minor 20); packaged `dev.koto.samples.json-weather`
  sample proves depth-1 selection, unknown-subtree skip, and
  missing/duplicate/wrong-type distinguishability end to end in KotoSim.)
- [x] Keep manifest/host tooling JSON parsing separate; no host parser or
  dynamically allocated DOM is linked into device application execution.
- [x] Add conformance, adversarial, chunk-boundary, maximum-limit, and recovery
  tests shared by KotoSim and the portable core, including representative
  Weather and MQTT payload fixtures.
- [ ] Record code size, parser-state size, and worst-case per-token work. Parsing
  a bounded chunk does not starve UI frames, audio service, or VM instruction
  budgeting. (Parser-state 320 bytes; host session ~324 bytes overlaid in the
  manifest-scratch union, device `.bss` delta exactly 0. Release RP2040 ELF
  delta vs `16ca552`: `.text` +3,416 B, `.rodata` +152 B, `.data` +24 B ≈
  +3.6 KiB flash total. Worst-case per-token work is O(consumed bytes) with
  constant per-byte state-machine cost plus one ≤256-byte token copy — the app
  bounds it by its chunk size, and the sample's one-≤128-byte-chunk-per-frame
  loop is proven frame-yielding in sim. Device-side frame/audio validation
  remains open.)

## Non-goals

- A mutable JSON DOM, arbitrary object allocation, JSONPath, or schema compiler
- JSON serialization or modification
- CBOR, MessagePack, XML, or provider-specific data models

## Implementation Progress

### 2026-07-19: portable incremental decoder core

The portable, allocation-free tokenizer now lives in `koto_core::json`
(`JsonDecoder`), mirroring how KOTO-0245 landed its Fetch boundary as a core
slice first. The decoder is a fixed-size struct (320 bytes on the host layout:
a 256-byte token scratch buffer, a 16-entry container stack, and the byte-at-a-
time lexer/parser state) that holds no reference to caller input. `next` consumes
a caller-owned chunk and returns `(consumed, JsonEvent)`; `finish` flushes a
trailing bare number and validates document completion. Completed key/string/
number bytes live in the decoder's scratch buffer and are read via `token()`, so
a caller can pause after any token and resume without retaining a pointer into a
previous VM buffer.

Limits are public constants with deterministic failures: `MAX_JSON_DEPTH` (16) →
`DepthExceeded`, `MAX_JSON_TOKEN_BYTES` (256, applied to decoded UTF-8) →
`TokenTooLong`, `MAX_JSON_NUMBER_BYTES` (40) → `NumberTooLong`. The fixed
`#[repr(u8)]` `JsonError` set (1..=11) covers structural, number-grammar, string,
escape, `\u`/surrogate, UTF-8, depth, size, trailing-data, and unexpected-end
faults, each reported with a bounded byte offset. Strings decode escapes and
surrogate pairs into UTF-8 and validate raw multibyte sequences; numbers are
validated against the JSON grammar and surfaced as raw literal text. The failed
state is sticky until `reset`, which reuses the buffer for a fresh document.

Tests (14, host-green, no host network/clock) cover conformance, all
number-grammar variants, escape/`\u`/surrogate/raw-UTF-8 strings, an
every-byte-boundary chunk sweep (`chunk = 1..=len` all agree), maximum-depth
accept-then-reject, oversized token/number, an adversarial malformed-input matrix
with bounded offsets, error stickiness + recovery, and representative Weather and
MQTT payload fixtures. The Weather fixture demonstrates the allocation-free
named-field-selection-with-skip pattern over `depth()`.

The decoder links no dynamically allocated DOM and stays separate from host
manifest/tooling JSON. Still open: allocation-free KotoSDK (`.koto` prelude)
wrappers and a packaged sample for named-field selection; Runtime ABI exposure;
KotoSim/device wiring on top of KOTO-0245's Fetch response bytes; release-ELF
code-size and worst-case per-token work measurement; and the UI/audio/VM-budget
non-starvation validation.

### 2026-07-19: allocation-free selection helpers

Added the portable, allocation-free selection primitives that the future SDK
wrappers build on, entirely within `koto_core::json`. `JsonValueSkip` (with
`SkipProgress`) consumes exactly one value — a scalar or a whole object/array
subtree — across chunk boundaries by counting container events, so application
code skips an unknown or unwanted field without allocating or recursing.
`JsonValueKind` and `JsonEvent::value_kind`/`is_value` let code check a selected
field's type, making a wrong-type field distinguishable from a correct one.
Three new tests cover skipping every value shape (including nested containers
and a `chunk = 1..=len` boundary sweep) and a reusable top-level field selector
that distinguishes Ok / wrong-type / missing / duplicate outcomes. 17 host tests
green, clippy clean. The `.koto` prelude wrappers, Runtime ABI, sim/device
wiring, packaged sample, and code-size/per-token-work measurement remain open.

### 2026-07-21: Runtime ABI, SDK wrappers, sim/device wiring, packaged sample

Host ABI minor 20 exposes the decoder to applications as host calls
`0x4A`–`0x4F` (`json_reset` / `json_next` / `json_finish` / `json_token` /
`json_error` / `json_status`), filling exactly the ID gap between the Fetch
block and the KotoUI block. The design keys off one constraint: `json_next` is
not idempotent, and the compiler's paired-result aliases (`Value2First/Second`)
re-execute their host call, so `json_next` returns only the stable event code
and the companion `(consumed, depth)` pair is read through the idempotent
`json_status` call (`json_consumed()` / `json_depth()` wrappers). Event codes
0–12 (`NeedMore` … `EndDocument`, `Error`; booleans split into `FALSE`/`TRUE`
so no token read is needed) are frozen in `koto_core::json::event_code` with a
guard test, and the compiler prelude sources every `JSON_*` constant — events,
`JSON_ERR_*` 1–11, and the three limits — from those modules so nothing can
drift. `json_token` never truncates: a short destination fails with
`BAD_ARGUMENT`, so `buf tok[256]` (= `JSON_MAX_TOKEN`) always succeeds.

Both runtimes share one implementation, `koto_core::JsonHostSession` (decoder +
last-consumed count, ~324 bytes): KotoSim holds it as a host field, and the
device host tucks it into `DeviceFetchSession`, which lives inside the
manifest-scratch union (`ManifestFetchResident`) — a new compile-time assert
proves the block still fits `MAX_MANIFEST_BYTES` (2304), so the device decoder
costs **zero additional static SRAM**. `begin_app` resets the session so no
token bytes leak across app launches. Verifier support (`known_host_call` +
exact stack effects) means minor-19 programs keep verifying unchanged;
rebuilding the committed apps only moved the KBC header's `host_abi_minor`
byte from 19 to 20 (bytecode byte-identical, verified on `sample_hello_text`).

The packaged `dev.koto.samples.json-weather` sample streams the Fetch sample's
response (enriched with an unknown nested `station` object, an array, `null`,
and a trailing bool) through the decoder at one ≤128-byte chunk per frame,
selecting `location`/`temperature_c` by handling `JSON_KEY` only at depth 1 and
treating the next value event as that key's value — unknown subtrees are
skipped by construction. Missing/duplicate/wrong-type outcomes render as
distinct states. A new sim integration test drives the packaged sample
end-to-end (extracted `Tokyo` / `21`, no error states, clean exit); 21
koto-core json host tests and the full local gate run green (the two failures
on this branch — firmware rustfmt drift and the SKK gallery candidate test —
reproduce identically on the committed HEAD and predate this work). The
shell's golden frame trace was regenerated for the new package count (20→22;
21 was already-committed pre-existing drift). `KOTO_SDK.md`,
`RUNTIME_BYTECODE_ABI.md`, and `SDK_SAMPLES.md` document the wrappers, ABI
rows, minor-20 note, and sample.

Measured cost (release RP2040 ELF, this tree vs committed `16ca552` built
identically): `.text` 469,332 → 472,748 (+3,416 B), `.rodata` +152 B, `.data`
+24 B, `.bss` unchanged — ~3.6 KiB flash for the decoder core, host session,
VM dispatch/verifier rows, and both hosts' glue; zero static SRAM. Still open:
on-device smoke of the packaged sample (fetch backend availability permitting —
without a live backend the sample renders its `fetch unavailable` state, and
the decoder path itself can be exercised the moment fetch bytes exist), and
the device-side non-starvation observation above.
