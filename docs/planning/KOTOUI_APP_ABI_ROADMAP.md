# KotoUI App ABI Roadmap

- Status: planned

## Purpose

Make the native KotoUI component model available to sandboxed KotoRuntime apps
without exposing Rust layouts, pointers, callbacks, or device-specific input and
rendering paths as ABI. This follows the native Shell pilot in KOTO-0216 and
turns KotoUI into an application-visible SDK capability.

The roadmap traces primarily to FR-SHELL-6, FR-SDK-1, FR-SDK-2, FR-SDK-5,
FR-SDK-9, FR-RT-3, FR-RT-4, NFR-PERF-1, NFR-DRAW-1, NFR-MEM-2,
NFR-PORT-1, NFR-PORT-3, NFR-REL-1, and NFR-I18N-1 through NFR-I18N-3.

## Direction and constraints

- Prefer a versioned, bounded retained UI description over one host call per
  widget property or immediate-mode reconstruction every frame. KOTO-0217 must
  validate and freeze this choice before implementation.
- App memory contains serialized descriptors and app-owned values. The host
  validates byte ranges and copies only bounded data; it never retains raw VM
  pointers across a host call.
- Stable numeric widget IDs connect descriptor nodes, retained interaction
  state, and semantic events.
- The host owns focus routing, normalized input dispatch, component painting,
  damage tracking, and modal scope behavior. Apps own business data and react
  to events such as activation, value change, submission, and cancellation.
- Every collection, string, event queue, component session, and damage list has
  a documented fixed capacity and deterministic overflow behavior.
- Simulator and Pico use the same parser, session model, KotoUI controls, and
  event semantics. Only their established HAL/present paths differ.
- Existing ABI-minor-17 applications and low-level drawing APIs remain valid.
- KotoUI consumes already-resolved UTF-8 display strings; translation catalogs
  stay owned by Shell/apps. The host exposes a bounded BCP 47 locale, generation,
  and text direction, with `en-US` as the deterministic fallback.
- v1 product locales are `en-US` and `ja-JP`; `qps-ploc` is a test-only expanded
  LTR locale. RTL and shaping are reserved compatibility work, not claimed v1 support.

## Candidate lifecycle

The design issue will finalize names and wire details, but the intended
lifecycle is mount a versioned description, apply bounded updates, present
damage, poll semantic events, and unmount/reset the session. Capability/version
querying must let an app reject an older host cleanly.

## Milestones

| Milestone | Issue | Outcome |
| :-- | :-- | :-- |
| M1 ABI design | KOTO-0217 | Versioned wire format, ownership, lifecycle, capacities, and compatibility contract |
| M2 Runtime | KOTO-0218 | KotoVM/KotoCore host calls and bounded UI session on simulator and Pico |
| M3 SDK | KOTO-0219 | Koto language builders, wrappers, constants, and event decoding |
| M4 Sample | KOTO-0220 | App-authored KotoUI component Gallery with deterministic regression coverage |
| M5 Pilot | KOTO-0221 | Existing File Note sample migrated to an interactive KotoUI form |
| M6 System configuration | KOTO-0223 | Native KotoConfig owns persisted language selection and publishes the shared locale generation |
| M7 Product localization | KOTO-0222 | Current KotoShell strings and simulator/device checks support English and Japanese from ConfigService |
| M8 Builder ergonomics | KOTO-0229 | App-owned stateful mount/update builders remove manual record, data-cursor, and status plumbing without changing the ABI |
| M9 Resource ergonomics | KOTO-0230 | Indexed text resources and List row builders remove App-local line scans and wire-layout arithmetic without changing the ABI |
| M10 SDK completion | KOTO-0231 | Update submission, resource-text bridging, locale matching, and focused SDK sources remove the remaining common App boilerplate |
| M11 Capacity ergonomics | KOTO-0232 | Compile-time SDK helpers derive bounded packet storage from semantic record/data capacities without exposing wire-layout arithmetic |
| M12 Capacity locality | KOTO-0233 | Helper-sized local buffers and compile-time `len(buf)` move one-use packet sizing from file-scope constants to builder call sites |

KOTO-0218 begins only after KOTO-0217 freezes the v1 contract. KOTO-0219 may
start once the runtime constants and binary fixtures are stable. The Gallery is
the SDK conformance surface; the File Note migration is the application-visible
adoption gate. KOTO-0222 makes the capability visible before third-party apps
adopt it. KOTO-0223 owns locale selection and may proceed once the locale
contract in KOTO-0218 is stable; KOTO-0222 then consumes that single source.
KOTO-0229 builds only on the frozen KOTO-0219 wire encoders and KOTO-0228 static
records, so it may proceed independently without reopening the host ABI.
KOTO-0230 builds on KOTO-0229 and keeps package asset loading and locale fallback
App-owned; it adds only caller-owned SDK parsing and List-blob orchestration.
KOTO-0231 builds on KOTO-0230, keeps presentation and locale fallback explicit,
and preserves the public SDK include while completing and separating the common
App-facing orchestration paths. KOTO-0232 records the follow-up capacity-helper
contract: Apps retain semantic data sizing while the SDK/compiler own packet
layout arithmetic and compile-time protocol-bound checks. KOTO-0233 follows by
making helper-sized buffers and their compile-time capacity available directly
at the builder call site without changing the pointer-only runtime model.

## Validation strategy

- Golden binary fixtures pin valid and malformed UI descriptions and events.
- Parser tests cover hostile offsets, lengths, UTF-8, duplicate IDs, invalid
  hierarchy, unsupported versions, and every capacity boundary.
- Recording-painter and framebuffer tests compare native and ABI-authored
  components, including exact damage and repeated-idle behavior.
- Scripted simulator tests cover focus, activation, editing, modal behavior,
  event overflow, app exit, and session reset between apps.
- Release measurements record firmware code size, static/session SRAM, app heap
  cost, maximum render commands, and representative interaction latency.
- PicoCalc validation covers the Gallery and File Note pilot with the same key
  mappings and visible results as KotoSim.
- Locale validation covers `en-US`, `ja-JP`, fallback from an unknown tag, live
  locale generation changes, and `qps-ploc` overflow/ellipsis at 320x320.

## Non-goals

- Exposing KotoUI Rust structs or trait objects directly in the VM ABI.
- CSS, DOM, recursive unbounded trees, callbacks into a running host call,
  pointer/touch interaction, or arbitrary custom native widgets in v1.
- Replacing Game2D or low-level drawing APIs; games may continue using them.
- Migrating the multiline Memo editor before the app UI ABI and text-field
  lifecycle have passed the bounded pilot.
- General message-format/pluralization engines, runtime translation downloads,
  RTL layout, complex-script shaping, or app-supplied fonts in v1.

## Completion gate

The roadmap is complete when KOTO-0217 through KOTO-0223 and KOTO-0229 through
KOTO-0233 are
`done`, an app can
build and interact with KotoUI controls through named SDK APIs on KotoSim and
PicoCalc, Shell and official samples are usable in English and Japanese,
unknown locales fall back to English, malformed app data cannot escape its
VM/session bounds, idle UI emits no redraw, existing KBC applications remain
compatible, and the full project harness plus device checklist pass.
