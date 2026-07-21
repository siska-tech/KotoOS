# KotoSDK Prelude

The KotoSDK prelude is the app-facing contract for [Koto language](KOTO_APP_LANGUAGE.md)
apps: named functions for drawing, input, IME, the text editor, sandboxed files,
and app lifecycle, plus named constants — so app source never hard-codes numeric
[host-call IDs](RUNTIME_BYTECODE_ABI.md). The prelude is built into the
[koto-compiler](../../tools/koto-compiler): every program may call these functions
and use these constants with no import.

Checked-in sample apps are documented in [KotoSDK Samples](SDK_SAMPLES.md).

## Conventions

- **Values are `int`.** Every SDK function returns an `int`; there are no
  exceptions. Buffers are passed as a `buf` (which evaluates to its heap offset)
  together with an explicit length argument.
- **Error results.** Functions that produce a value return it on success
  (a handle, byte count, codepoint, etc.) and `-1` on failure (`result < 0` is the
  error test). Status-only functions return `0` on success and a negative status
  (`-error_code`) on failure. Every host call pushes a fixed number of values on
  both paths, so failures never desync the operand stack.
- **Buffer ownership.** Buffers are app-owned regions of the app heap declared with
  `buf name[N];`. The host only reads or writes the exact `(buffer, length)` slice
  you pass, and that length must stay within the buffer. Each app is given a heap
  sized to exactly what its buffers and string literals need (the compiler records it
  in the KBC header; per-app profile, KOTO-0096), up to a **16 KB** device ceiling;
  the host editor document holds up to **1024 bytes**.
- **Sandboxing.** File paths are app-virtual and resolved through the app's
  save-data sandbox (`data/<app_id>/...`); a path cannot escape it.

## Lifecycle

| Function | Host call | Returns | Notes |
| :------- | :-------- | :------ | :---- |
| `exit(code)` | `exit` | (never returns) | Terminates the app with `code`. |
| `yield_frame()` | `yield_frame` | status | Ends the current frame; the app resumes next frame with state intact. |

## Bounded Fetch (Host ABI minor 19)

Fetch is a nonblocking, read-only `GET` API. The package manifest v2 exact-origin
allowlist is checked before DNS; absence is default-denied. Apps receive no DNS,
socket, or TLS object and must call `yield_frame()` while pending.

| Function | Host call | Returns | Notes |
| :-- | :-- | :-- | :-- |
| `fetch_start(url, len)` | `fetch_start` (`0x46`) | request ID / `-1` | At most one live request for the app; URL maximum 384 bytes. |
| `fetch_poll_state(id)` | `fetch_poll` (`0x47`) | `FETCH_*` / `-1` | Returns Pending, Headers, Body, Complete, or Failed. |
| `fetch_poll_metadata(id)` | `fetch_poll` (`0x47`) | status/error / `-1` | HTTP status in Headers; fixed error enum in Failed. |
| `fetch_read(id, dst, max)` | `fetch_read` (`0x48`) | bytes / `-1` | `max <= FETCH_MAX_READ` (512); total body maximum 65,536 bytes. |
| `fetch_cancel(id)` | `fetch_cancel` (`0x49`) | status | Cancels only the implicit current app's request. |

`fetch_start`, `fetch_poll_state`, `fetch_poll_metadata`, `fetch_read`, and
`fetch_cancel` are allocation-free compiler-prelude wrappers. State constants
are `FETCH_PENDING=0`, `FETCH_HEADERS=1`, `FETCH_BODY=2`,
`FETCH_COMPLETE=3`, and `FETCH_FAILED=4`. Redirects, cookies, credentials,
request bodies, custom headers, and retries are not part of v1.

## Bounded JSON decoding (Host ABI minor 20)

The `json_*` wrappers (KOTO-0246) expose the host-owned, allocation-free
incremental JSON decoder. The app feeds caller-owned chunks — typically
`fetch_read` output — split at any byte boundary and pulls one event per call;
no document tree is built and completed tokens live in a bounded host scratch
until the next token starts. The decoder needs no permission: it never touches
the network or filesystem.

| Function | Host call | Returns | Notes |
| :-- | :-- | :-- | :-- |
| `json_reset()` | `json_reset` (`0x4A`) | status | Clears all decoder state for a fresh document. |
| `json_next(src, len)` | `json_next` (`0x4B`) | `JSON_*` event / `-1` | Feeds a chunk, returns the next event. Read the consumed byte count with `json_consumed()` and advance `src` by it before the next call. |
| `json_finish()` | `json_finish` (`0x4C`) | `JSON_*` event / `-1` | Call at end of input until `JSON_END_DOC` or `JSON_ERROR`; a flushed trailing bare number is one extra `JSON_NUMBER`. |
| `json_token(dst, max)` | `json_token` (`0x4D`) | bytes / `-1` | Copies the latest `JSON_KEY`/`JSON_STR`/`JSON_NUMBER` token. Never truncates: fails if `max` is short, so `max >= JSON_MAX_TOKEN` always succeeds. |
| `json_error_code()` | `json_error` (`0x4E`) | `JSON_ERR_*` / `0` | Sticky failure code, `0` while healthy. |
| `json_error_offset()` | `json_error` (`0x4E`) | byte offset | Document byte offset of the sticky failure. |
| `json_consumed()` | `json_status` (`0x4F`) | bytes | Bytes consumed from `src` by the most recent `json_next`. |
| `json_depth()` | `json_status` (`0x4F`) | depth | Current container nesting depth; the skip idiom below builds on it. |

Event constants: `JSON_NEED_MORE=0` (chunk drained without an event),
`JSON_BEGIN_OBJECT=1`, `JSON_END_OBJECT=2`, `JSON_BEGIN_ARRAY=3`,
`JSON_END_ARRAY=4`, `JSON_KEY=5`, `JSON_STR=6`, `JSON_NUMBER=7`,
`JSON_FALSE=8`, `JSON_TRUE=9`, `JSON_NULL=10`, `JSON_END_DOC=11`, and
`JSON_ERROR=12`. Booleans split into two events so no token read is needed.
`JSON_NUMBER` tokens are the raw validated literal text; strings and keys are
decoded UTF-8 (escapes and surrogate pairs resolved).

Failure is deterministic and sticky until `json_reset()`: `json_error_code()`
returns one of `JSON_ERR_UNEXPECTED_BYTE=1`, `JSON_ERR_INVALID_NUMBER=2`,
`JSON_ERR_INVALID_STRING=3`, `JSON_ERR_INVALID_ESCAPE=4`,
`JSON_ERR_INVALID_UNICODE=5`, `JSON_ERR_INVALID_UTF8=6`,
`JSON_ERR_DEPTH_EXCEEDED=7`, `JSON_ERR_TOKEN_TOO_LONG=8`,
`JSON_ERR_NUMBER_TOO_LONG=9`, `JSON_ERR_TRAILING_DATA=10`, or
`JSON_ERR_UNEXPECTED_END=11`. The limits behind codes 7–9 are the SDK constants
`JSON_MAX_DEPTH` (16), `JSON_MAX_TOKEN` (256 decoded bytes), and
`JSON_MAX_NUMBER` (40 literal bytes).

**Named-field selection with skip.** Because `json_depth()` reports container
nesting, an app selects fields of the root object by handling `JSON_KEY` only
at depth 1 and treating the very next value event as that key's value; every
other event belongs to an unknown subtree and is skipped by simply ignoring it.
Missing (flag never set), duplicate (flag already set), and wrong-type (value
event differs from the expected one) fields stay distinguishable. The packaged
[JSON Weather sample](SDK_SAMPLES.md) demonstrates the full idiom over Fetch
response bytes, parsing at most one bounded chunk per frame.

## Advisory time (Host ABI minor 21)

`time_query(kind)` (host call `time_query`, `0x56`, KOTO-0247) exposes the
KOTO-0244 advisory clock to applications. It needs no permission and returns
one value per selector:

| Selector | Returns | Notes |
| :-- | :-- | :-- |
| `TIME_UTC_SECONDS` | UTC seconds / `-1` | Synchronized Unix UTC seconds; `-1` while time is unknown (before the first valid SNTP synchronization, after radio/network teardown, or outside the `i32` range). |
| `TIME_OFFSET_MINUTES` | minutes | The user-configured KotoConfig UTC offset (`-720..=840`, 15-minute steps). Always available; local time is `utc + offset * 60`. |
| `TIME_MONOTONIC_MS` | milliseconds | Monotonic frame-clock milliseconds masked to `TIME_MONOTONIC_MASK` (30 bits): always non-negative, wrapping about every 12.4 days. Compute intervals as `(now - then) & TIME_MONOTONIC_MASK`. |

An unknown selector fails with `BAD_ARGUMENT`. The clock is unauthenticated
advisory data for update labels, cache age, and bounded refresh pacing:
unknown time is a normal presentation state (show it explicitly) and must
never disable refresh, authorize an update, or mark downloaded data trusted.
In KotoSim both selectors are driven by the deterministic frame clock and a
scenario-scripted UTC anchor, never the host wall clock.

## Application credential vault (Host ABI minor 22)

`vault_handle` and `fetch_start_authenticated` (host calls `0x57`/`0x58`,
KOTO-0248) let an application use an OS-owned network credential without ever
seeing a secret byte. The application resolves an opaque, generation-tagged
handle for a destination it is granted, then asks the OS to start a request that
injects the credential inside the authenticated transport. Secrets, headers, and
logins never enter VM memory.

| Function | Host call | Returns | Notes |
| :-- | :-- | :-- | :-- |
| `vault_handle(service, url_ptr, url_len)` | `vault_handle` (`0x57`) | handle / `0` | Resolves the running app's opaque credential handle for `url`; `0` means no grant applies (default-denied). `service` is `VAULT_SERVICE_FETCH` or `VAULT_SERVICE_MQTT`. Never exposes a secret. |
| `fetch_start_authenticated(url_ptr, url_len, handle)` | `fetch_start_authenticated` (`0x58`) | request ID / `-1` | Starts one allowlisted GET with the granted credential injected by the OS. A zero, stale, foreign, or off-endpoint handle fails closed with a fixed error before any secret is touched. |

Grants are created, replaced, and revoked out of band through the OS-owned vault
and its consent flow, never through this ABI. A handle is opaque: it cannot be
reversed to bytes, enumerated, or used by another application, and it becomes
stale when its grant is replaced or revoked. In KotoSim the vault is a
deterministic fake with synthetic handles and fake secrets that never reads any
host credential store.

## Bounded MQTT subscribe (Host ABI minor 23)

The MQTT wrappers (host calls `0x59`–`0x5F`, KOTO-0249) let a foreground app
receive bounded live telemetry from an OS-owned MQTT 3.1.1 QoS-0 subscribe
service. The app names a manifest-declared broker and exact topic *by index*,
polls the session, and drains complete messages into its own bounded buffers.
It never sees a socket, TLS state, or credential byte; the OS owns the transport
and, on device, injects any authenticated-broker credential via a KOTO-0248
grant. Handles are generation-tagged and app-context-bound: a stale, foreign, or
unknown handle fails closed.

| Function | Host call | Returns | Notes |
| :-- | :-- | :-- | :-- |
| `mqtt_connect(broker_index)` | `mqtt_connect` (`0x59`) | session / `-1` | Opens a session to the manifest broker at `broker_index`. One session per app and one globally. |
| `mqtt_subscribe(session, topic_index)` | `mqtt_subscribe` (`0x5A`) | `0` / `-1` | Subscribes a connected session to the manifest exact topic filter at `topic_index`. A topic outside the grant fails closed. |
| `mqtt_poll(session)` | `mqtt_poll` (`0x5B`) | `MQTT_*` state | `MQTT_CONNECTING`, `MQTT_CONNECTED`, `MQTT_MESSAGE`, `MQTT_DISCONNECTED`, or `MQTT_FAILED`. |
| `mqtt_peek_topic_len(session)` / `mqtt_peek_payload_len(session)` | `mqtt_peek` (`0x5C`) | bytes | Full lengths of the oldest queued message without consuming it, so buffers can be sized. `(0, 0)` when empty. Idempotent. |
| `mqtt_read(session, t_ptr, t_max, p_ptr, p_max)` | `mqtt_read` (`0x5D`) | `MQTT_READ_*` / `-1` | Copies and consumes the oldest message: `MQTT_READ_NONE`, `MQTT_READ_MESSAGE`, or `MQTT_READ_RETAINED`. A message too large for either buffer is consumed and fails closed, never truncated. |
| `mqtt_disconnect(session)` | `mqtt_disconnect` (`0x5E`) | `0` | Disconnects and zeroizes the session. |
| `mqtt_dropped(session)` | `mqtt_dropped` (`0x5F`) | count | Saturating count of messages the OS queue dropped (drop-oldest). Idempotent. |

`mqtt_read` consumes the oldest message, so — like `json_next` — its companion
lengths are read through the separate idempotent `mqtt_peek_*` calls rather than
paired-result aliases of the same host call. The inbound queue is a fixed
eight-deep drop-oldest ring, so memory never grows with message rate or payload
size. The session is cancelled and zeroized on app exit, capability loss,
network-generation change, permission revocation, or teardown; an inactive app
receives no background messages, and an unsupported/offline build returns a
stable `Unavailable`. In KotoSim the broker is a deterministic fake that delivers
scripted retained and live messages with no host network or wall clock.

## KotoUI application session (Host ABI minor 18)

KOTO-0217 freezes a bounded retained component ABI. KOTO-0218 implements the raw
host calls below. KOTO-0219 exposes these lifecycle calls in the compiler
prelude and adds named SDK builders so applications do not need to construct the
wire records manually.

| Function | Host call | Returns | Purpose |
| :-- | :-- | :-- | :-- |
| `ui_capabilities(dst, max_len)` | `ui_capabilities` (`0x50`) | bytes written / `-1` | Query the 64-byte `KUC1` format/capacity record plus current BCP 47 locale, generation, and LTR direction. |
| `ui_mount(src, len)` | `ui_mount` (`0x51`) | status | Atomically copy and mount a `KUI1` description. |
| `ui_update(src, len)` | `ui_update` (`0x52`) | status | Atomically apply a `KUP1` property batch. |
| `ui_present()` | `ui_present` (`0x53`) | status | Schedule only pending component damage. |
| `ui_poll_event(dst, max_len)` | `ui_poll_event` (`0x54`) | bytes written / `-1` | Read one `KUE1` semantic event; zero means empty. |
| `ui_reset()` | `ui_reset` (`0x55`) | status | Clear the app-owned host UI session. |

Normal app source will use KOTO-0219 named builders for Label, Button, Checkbox,
List, TextField, Panel, and Dialog records rather than hard-coded offsets. The
wire format, event meanings, ownership, overflow behavior, and v1 capacities are
normative in [KotoUI Application ABI](KOTOUI_APP_ABI.md).

### App-owned stateful builders

New App code should normally declare its own top-level `static`
`UiMountBuilder` and `UiUpdateBuilder` records, as shown by
[`sdk/examples/koto_ui_counter.koto`](../../sdk/examples/koto_ui_counter.koto).
The SDK does not hide a mutable singleton, so two builder instances have
independent cursors and sticky status. The record fields are initialized to
zero/`false`; `begin` binds the caller-owned packet and resets them for reuse.

`UiMountBuilder.begin(packet, capacity, record_capacity, root_id, focus_id)` and
`UiUpdateBuilder.begin(packet, capacity, record_capacity)` reserve the forward
record table. `record_capacity` fixes the data-arena boundary; it is not the
eventual record count and need not be the protocol maximum. Node methods
(`label`, `button`, `checkbox`, `checkbox_with_mark_offset`, `list`,
`text_field`, `panel`, and `dialog`) and
property methods (`text`, `enabled`, `visible`, `checked`, `selection`,
`text_value`, `bounds`, `list_rows`, `dialog_open`, and `request_focus`) consume
the next record automatically. Payload methods copy or reserve through one
bounded data cursor. Boolean options use `bool`; the SDK writes their numeric
KUI1/KUP1 representation.

Declare packet storage from semantic record/data capacities instead of copying
wire-layout arithmetic into the App. When the sizing facts belong to one builder
transaction, keep them on the declaration and derive the `begin` capacity from
the buffer itself (KOTO-0233):

```koto
fn gallery_set_status(line: int) {
    // One text record; the longest localized status is 19 UTF-8 bytes.
    buf update[ui_update_capacity(1, 19)];
    let status = gallery_update_builder.begin(update, len(update), 1);
    // ...
}
```

The record count stays explicit at `begin` because the one-pass wire builder
reserves its record table before writing data. Capacities shared by more than
one consumer (for example a packet region inside a shared heap layout) remain
top-level constants:

```koto
const MOUNT_RECORDS = 10;
const MOUNT_DATA_CAPACITY = 267;
const MOUNT_BYTES = ui_mount_capacity(MOUNT_RECORDS, MOUNT_DATA_CAPACITY);
buf mount_packet[MOUNT_BYTES];
```

`ui_mount_capacity(records, data_capacity)` computes `40 + records * 48 +
data_capacity`; `ui_update_capacity` computes `32 + records * 32 +
data_capacity`. Two storage constructors (KOTO-0236) size the caller-owned
regions behind the resource types below the same way:
`ui_text_resource_capacity(line_capacity, payload_capacity)` computes
`line_capacity * 4 + payload_capacity` (the `TextResource::parse`
destination), and `ui_list_rows_capacity(row_capacity, label_capacity)`
computes `row_capacity * 12 + label_capacity` (the `UiListRowsBuilder::begin`
blob), each checked against its consumer's own bounds. In a top-level `const`
declaration, directly as a local `buf` size, or as a struct buffer-field size
(`mount: buf[ui_mount_capacity(5, 176)]`, KOTO-0235 — see
`KOTO_APP_LANGUAGE.md`) the compiler evaluates these calls and reports
out-of-range capacities; a helper argument may itself be a folded asset or
capacity helper call, so `ui_text_resource_capacity(
asset_text_line_count("locales/..."), asset_len("locales/..."))` is a complete
derived sizing (since `parse` strips delimiters,
`payload_len <= raw_len` always holds). In an ordinary expression their
checked SDK implementations return the capacity or `UI_SDK_BAD_ARGUMENT`.
This is a narrowly defined compiler-backed SDK facility, not general
const-function execution. The header/stride numbers remain owned by the SDK,
while the App names only the transaction's semantic record count and reserved
arena.

A packet region inside a shared App-state layout no longer needs the
`MOUNT_BYTES`-style constant block above: a struct buffer field carries the
same declaration-site sizing facts, `len(app.mount)` folds to the declared
capacity for `begin`, and the field read passes the region's address at the
one-add cost of hand-written base-plus-offset.

The first detected error is sticky. Later methods return it without changing
the packet, and `finish` returns it without sealing packet lengths. On success,
`finish` validates the actual count and complete packet and returns the exact
length to pass to `ui_mount` or `ui_update`; it never submits or presents. A
completed success or failure may be followed by `begin` to reuse the instance.
Calling `begin` on an active builder is rejected. Apps must not re-enter a
builder or call `yield_frame` between its successful `begin` and `finish`;
prepare resources and yield before `begin` when work must span frames.

Builders are App-lifetime static heap records, not allocated objects. Their
methods follow the language's normal call-site inline model, so repeated calls
have visible KBC, fuel, and local-slot cost. The low-level encoders below remain
supported for exact-byte fixtures, specialized layouts, and compatibility.

### Indexed text resources and List rows

`TextResource` is an App-owned static record that binds caller-owned parsed
storage. It does not allocate, load an asset, choose a locale, or own a package
path. `parse(raw, raw_len, dst, dst_capacity, line_capacity)` reserves
`line_capacity * 4` bytes for SDK-private u16 offset/length entries and stores
the compact line payload after that table. The caller must therefore provide
at least `ui_text_resource_capacity(line_capacity, payload_bytes)` bytes —
declare the destination with that helper (`asset_text_line_count` for matching
locale shape and `asset_len` for the payload bound, KOTO-0236/0237) rather than
restating the stride — and keep `dst` alive
while any line pointer is used. `count`, `line_ptr(index)`, and
`line_len(index)` expose the completed resource; successful lookup is O(1).

Parsing accepts LF and CRLF, preserves interior empty lines, and treats a final
newline as the terminator of the preceding line rather than an additional empty
line. Empty input has zero lines. Bare CR, malformed UTF-8, input/output beyond
the 16-bit indexed representation, line-count overflow, and every destination
boundary are rejected before destination mutation. A failed parse or access is
sticky. Calling `parse` again is the explicit reset/reuse operation; independent
records share no SDK mutable state. Raw input is needed only during `parse`, but
the parsed destination must remain alive for access.

For an arena that copies a contiguous subset of resource lines, such as List
labels, `asset_text_max_range_bytes(first_line, line_count, paths...)` folds
the largest per-locale payload for that range using these same boundaries.
For a retained slot that holds *one of* the lines in a range at a time, such
as a status Label later retargeted with `text_resource`,
`asset_text_max_line_bytes(first_line, line_count, paths...)` folds the
largest single line across the listed locales instead (KOTO-0238).

`UiListRowsBuilder` similarly binds a caller-owned List blob sized with
`ui_list_rows_capacity(row_capacity, label_capacity)`. `begin(blob,
capacity, row_capacity)` fixes the full `row_capacity * 12` v1 row table;
`row(enabled, label, label_len, app_value)` appends a checked UTF-8 label and
signed application value, while `resource_row` consumes one completed
`TextResource` line. `finish` returns the exact blob length. The builder owns no
allocation: keep the blob alive through mount/update sealing. Its first error is
sticky, later row calls do not change the blob, `finish` never exposes a failed
blob as complete, and the next `begin` explicitly reuses a completed builder.
Re-entering an active builder is rejected, and Apps must not yield between
`begin` and `finish`.

`UiMountBuilder.list_builder` and `UiUpdateBuilder.list_rows_builder` consume a
completed row builder and derive its row count and exact blob length. The mount
method still takes the reserved List payload capacity because KUI1 Text/List
capacity is part of the retained node contract. The scalar `list` / `list_rows`
methods and all pointer/length encoders remain supported as the compatibility
and byte-conformance surface. The row and text methods inline at their call
sites like every Koto function, so Apps should prepare resources/rows once and
reuse them where practical.

`UiUpdateBuilder.text_resource(widget_id, resource, line)` copies one validated
line directly from a completed `TextResource`. It checks resource completion,
line/table bounds, transaction state, record capacity, and packet data capacity
before changing either packet region. The resource remains caller-owned and
must stay alive only through the call; `text(widget_id, src, len)` and the
`line_ptr` / `line_len` accessors remain available for arbitrary byte ranges.

`UiUpdateBuilder.submit()` seals the active KUP1 packet at its derived exact
length and calls `ui_update` once. It returns `0` for an accepted update or the
first sticky builder, finalizer, or host error. It never calls `ui_present`:
presentation remains an explicit App decision, so several accepted updates may
share one presentation and host-call/damage order stays observable. Use
`finish()` plus scalar `ui_update(packet, len)` when inspecting or batching a
sealed packet separately. Like other receiver helpers, both conveniences inline
at call sites; repeated use can increase KBC size even when runtime peaks fall
or remain unchanged.

The stable `sdk/koto_ui.koto` entry point is a compatibility aggregator over
`koto_ui/abi.koto` (wire encoding and validation), `resources.koto`
(TextResource/List rows), `builders.koto` (stateful mount/update transactions),
and `events_locale.koto` (events, capabilities, and locale matching). Apps keep
the single `include <sdk/koto_ui.koto>;` spelling; diagnostics, definitions,
hover, and receiver completion identify the focused owner source.

The checked-in standard source currently provides
`ui_mount_begin`/`ui_mount_finish` and these named record builders:

| Node | Builder |
| :-- | :-- |
| Label | `ui_mount_add_label` |
| Button | `ui_mount_add_button` |
| Checkbox | `ui_mount_add_checkbox` |
| Checkbox with adjusted mark | `ui_mount_add_checkbox_with_mark_offset` |
| List | `ui_mount_add_list` |
| TextField | `ui_mount_add_text_field` |
| Panel | `ui_mount_add_panel` |
| Dialog | `ui_mount_add_dialog` |

Every builder receives a caller-owned destination buffer, its capacity, and a
zero-based node index. It validates all arguments it can decide locally before
changing the 48-byte destination record and returns `0` or
`UI_SDK_BAD_ARGUMENT` (`-2`). Cross-record rules such as duplicate IDs,
hierarchy, aggregate capacity, UTF-8 ranges, and Dialog action references are
checked by `ui_mount_finish` before it seals `total_len` and `data_len`. The
finalizer uses bounded structure, data, kind, and action passes so inlined SDK
code remains below the VM local-slot ceiling. A failed pass does not change the
unsealed header, allowing the caller to repair or discard its buffer. The host
independently repeats normative validation before an atomic mount. Application
business values remain app-owned—the packet is only a serialized snapshot
copied by `ui_mount`.

Targeted changes use `ui_update_begin` and `ui_update_finish` around one or more
named property builders:

| Property | Builder |
| :-- | :-- |
| Text | `ui_update_set_text` |
| Enabled | `ui_update_set_enabled` |
| Visible | `ui_update_set_visible` |
| Checked | `ui_update_set_checked` |
| Selection | `ui_update_set_selection` |
| TextField value/cursor | `ui_update_set_text_value` |
| Bounds | `ui_update_set_bounds` |
| List rows | `ui_update_set_list_rows` |
| Dialog open | `ui_update_set_dialog_open` |
| Focus request | `ui_update_request_focus` |

`ui_packet_copy` copies caller-owned resource bytes into either packet's data
section after checking destination bounds. `ui_update_finish` rejects duplicate
widget/property pairs, invalid data ranges, malformed UTF-8 and row blobs, and
multiple focus requests before sealing the KUP1 lengths. Node-kind, mounted
capacity, effective modal/focus scope, and aggregate geometry are independently
validated by the host against the retained session before the update applies.

A complete mount-once counter is available at
[`sdk/examples/koto_ui_counter.koto`](../../sdk/examples/koto_ui_counter.koto).
It drains semantic events, sends one targeted label update after activation,
handles locale/error/older-host paths, presents damage, and yields when idle.

`koto-lsp` completion reads lifecycle intrinsic metadata from the compiler and
combines it with symbols from included source. After
`include <sdk/koto_ui.koto>;`, lifecycle calls, builders, validators, accessors, and
constants therefore appear together without a separately maintained editor
function list.

After `ui_poll_event` returns a positive byte count, call
`ui_event_validate(event, count)` once, then read the record through
`ui_event_response`, `ui_event_widget_id`, `ui_event_value` (or the semantic
alias `ui_event_index`), `ui_event_aux`, `ui_event_text_ptr`, and
`ui_event_text_len`. A text pointer of zero means the event carries no text.
These accessors keep KUE1 byte offsets out of normal application code.

### Locale and resource fallback

Validate a successful 64-byte capability query with
`ui_capabilities_validate`, then use `ui_capabilities_locale_ptr`,
`ui_capabilities_locale_len`, `ui_capabilities_locale_generation`, and
`ui_capabilities_direction`. `ui_capabilities_has(cap, UI_CAP_IME)` tests the
v1 IME bit without exposing the record flags directly. A
`UI_RESPONSE_LOCALE_CHANGED` event has parallel
`ui_locale_changed_generation`, `ui_locale_changed_direction`,
`ui_locale_changed_tag_ptr`, and `ui_locale_changed_tag_len` accessors after
`ui_locale_changed_validate` succeeds.

For each available resource tag, call
`ui_locale_fallback_rank(locale, locale_len, candidate, candidate_len)` and keep
the candidate with the smallest non-negative rank:

| Rank | Match |
| --: | :-- |
| 0 | Exact canonical tag, including `ja-JP`, `en-US`, or test-only `qps-ploc` |
| 1 | Requested language subtag, for example `ja` for `ja-JP` |
| 2 | Deterministic `en-US` fallback |
| -1 | Not a candidate |

If no resource has a non-negative rank, use the app's embedded default. Resolve
the string before passing its pointer, byte length, and capacity to a mount or
update builder. Locale changes never mutate business state or app buffers; the
app drains the semantic event and explicitly rebuilds the affected snapshot.

For direct classification without fallback policy, use
`ui_locale_match(locale, locale_len, candidate, candidate_len)`. It returns
`UiLocaleMatch::Exact` only for byte-identical valid canonical tags,
`UiLocaleMatch::Language` when a plain language candidate prefixes the locale
and is followed by `-`, and `UiLocaleMatch::None` for unrelated or malformed
tags. Empty tags, leading/trailing or repeated hyphens, non-ASCII-alphanumeric
subtags, and tags longer than 23 bytes do not match. Asset choice, candidate
order, unsupported-locale behavior, and English fallback remain App-owned;
`ui_locale_fallback_rank` is unchanged for compatibility consumers.

An app must treat `ui_capabilities(...) < 0` as an older-host/unsupported path
and must not issue the remaining UI lifecycle calls. It may retain its existing
immediate-mode interface or report that the app requires Host ABI minor 18.
Value-producing lifecycle calls use the standard `-1` failure sentinel;
status-only calls preserve `-error_code`.

### KotoUI constants

The compiler predefines the following constant families from the same
`koto-core` capacities used by KUC1 encoding and host validation:

| Family | Names |
| :-- | :-- |
| Versions | `UI_ABI_MAJOR`, `UI_ABI_MINOR`, `UI_HOST_ABI_MINOR` |
| Nodes | `UI_NODE_LABEL`, `UI_NODE_BUTTON`, `UI_NODE_CHECKBOX`, `UI_NODE_LIST`, `UI_NODE_TEXT_FIELD`, `UI_NODE_PANEL`, `UI_NODE_DIALOG` |
| Sentinels | `UI_PARENT_ROOT`, `UI_FOCUS_FIRST`, `UI_SELECTION_NONE`, `UI_CURSOR_END`, `UI_ACTION_NONE` (all `-1`, with distinct semantic roles) |
| Flags | `UI_FLAG_VISIBLE`, `UI_FLAG_ENABLED`, `UI_FLAG_LTR`, `UI_FLAG_ELLIPSIS` |
| Alignment | `UI_ALIGN_START`, `UI_ALIGN_CENTER`, `UI_ALIGN_END` |
| Responses | `UI_RESPONSE_ACTIVATED`, `UI_RESPONSE_VALUE_CHANGED`, `UI_RESPONSE_TEXT_CHANGED`, `UI_RESPONSE_SELECTION_CHANGED`, `UI_RESPONSE_SELECTION_ACTIVATED`, `UI_RESPONSE_SUBMITTED`, `UI_RESPONSE_CANCELLED`, `UI_RESPONSE_FOCUS_CHANGED`, `UI_RESPONSE_CAPACITY_REJECTED`, `UI_RESPONSE_LOCALE_CHANGED` |
| Errors | `UI_ERROR_BAD_ARGUMENT`, `UI_ERROR_NOT_FOUND`, `UI_ERROR_UNSUPPORTED`, `UI_ERROR_NO_MEMORY` |
| Capacities | `UI_CAPABILITIES_BYTES`, `UI_EVENT_HEADER_BYTES`, `UI_MAX_NODES`, `UI_MAX_MOUNT_BYTES`, `UI_MAX_UPDATE_BYTES`, `UI_MAX_DATA_BYTES`, `UI_EVENT_QUEUE_CAPACITY`, `UI_MAX_TEXT_FIELDS`, `UI_MAX_TEXT_FIELD_BYTES`, `UI_MAX_LIST_ROWS`, `UI_DAMAGE_CAPACITY`, `UI_MAX_OPEN_MODALS` |
| Capability bits | `UI_CAP_IME` |

Inherited direction and clipping are represented by leaving the corresponding
flag bits clear. The SDK intentionally exposes explicit LTR and ellipsis only;
v1 does not define public constants for reserved RTL or wrapping modes. Error
constants are positive ABI codes. Status-only wrappers return their negative
value, while value-producing wrappers retain the standard `-1` failure sentinel.

New code should use the compiler-backed integer enum domains `UiNodeKind`,
`UiAlignment`, `UiResponse`, and `UiError` (for example,
`UiResponse::Activated`). `sdk/koto_ui.koto` also exposes `UiProperty` for its
KUP1 builder tags. The `UI_NODE_*`, `UI_ALIGN_*`, `UI_RESPONSE_*`, `UI_ERROR_*`,
and `UI_PROPERTY_*` spellings remain compatibility aliases and have identical
integer values and generated host-call arguments.

Stateful-builder Boolean arguments use the language values `true` and `false`;
the compatibility encoders continue to accept numeric `1` and `0`. Sentinel constants deliberately remain
flat rather than forming an enum: each is paired with otherwise-arbitrary IDs,
indices, or offsets, and equal numeric values do not make those roles one domain.

## Drawing

| Function | Host call | Returns | Notes |
| :------- | :-------- | :------ | :---- |
| `draw_rect(x, y, w, h, rgb565)` | `draw_rect` | status | Fills a clipped rectangle in RGB565. |
| `draw_text(x, y, buf, len)` | `draw_text` | status | Draws `len` UTF-8 bytes from `buf`. The host chooses the font/colour. |
| `draw_text_color(x, y, buf, len, rgb565)` | `draw_text_color` | status | Like `draw_text`, but draws in the caller-chosen RGB565 colour. |
| `draw_pixels(x, y, w, h, buf, len)` | `draw_pixels_rgb565` | status | Blits a `w`x`h` block of little-endian RGB565 pixels from `buf` (row-major, `len == w*h*2` bytes). The opaque tile/sprite primitive (PicoMings model); for sprite transparency, omit empty cells. |
| `draw_pixels_persistent(x, y, w, h, buf, len)` | `draw_pixels_persistent_rgb565` | status | Commits an RGB565 block as a persistent LCD-GRAM update. Later frames do not erase it; intended for bounded-buffer streaming of full-screen art. |
| `game2d_set_tile(layer, x, y, tile_ref)` | `game2d_set_tile` | status | Writes one cell of a host-retained 16x16-tile layer. `tile_ref` is the byte offset of a 16x16 RGB565 tile in the app heap (as passed to `draw_pixels`), or `< 0` to clear the cell. Write only the cells that change. |
| `game2d_clear_layer(layer)` | `game2d_clear_layer` | status | Clears every cell of a tilemap `layer`. |
| `game2d_configure_tilemap(layer, columns, rows, origin_x, origin_y)` | `game2d_configure_tilemap` | status | Clears and configures layer 0. `columns`/`rows` must each be 1..20; origin is the top-left pixel position. |
| `game2d_present()` | `game2d_present` | status | Composites the retained layers for this frame in fixed z-order (static → tile → sprite → immediate). Call once per frame after updating retained state; on the simulator it must follow every `sprite_set` so the final sprite state is re-emitted. |
| `game2d_static_begin()` | `game2d_static_begin` | status | Clears the retained static/background layer and routes subsequent `draw_rect`/`draw_text`/`draw_text_color`/`draw_pixels` into it until `game2d_static_end`. |
| `game2d_static_end()` | `game2d_static_end` | status | Routes draw calls back to the per-frame immediate list. |
| `game2d_stamp_define(stamp_id, cells_off, count, format)` | `game2d_stamp_define` | status | Registers a reusable cell *stamp*: `count` cells at app-heap byte offset `cells_off`. `format 0` packs each cell as a `(dcol,drow)` nibble (`drow*4 + dcol`). Define once; the cell bytes stay in the heap. |
| `game2d_sprite_set(inst_id, stamp_id, x, y, tile_ref)` | `game2d_sprite_set` | status | Creates/updates retained sprite `inst_id`: draws `stamp_id`'s cells at `(x + dcol*16, y + drow*16)`, each blitting the 16x16 tile at heap offset `tile_ref`. Diffed by stable `inst_id`. |
| `game2d_sprite_hide(inst_id)` | `game2d_sprite_hide` | status | Hides a retained sprite (its footprint is erased next present). |
| `game2d_sprite_clear_all()` | `game2d_sprite_clear_all` | status | Hides every retained sprite. |

The Game2D tilemap (KOTO-0135) is the retained alternative to re-blitting a whole
board with `draw_pixels` every frame: the host keeps the layer and repaints only
cells whose `tile_ref` changed, so a still board costs nothing. Bake tile art in
the app heap exactly as for `draw_pixels`, then name cells by their heap offset.
One allocation-free layer with a 20x20 maximum is supported; layer must be `0`.
Call `game2d_configure_tilemap` once before writing cells to select any active
size up to 20x20 and its pixel origin. Tiles remain 16x16 RGB565. For backward
compatibility an unconfigured layer uses 10x20 at `(8, 0)`. See
[GAME2D_ABI.md](GAME2D_ABI.md).

The Game2D static/background layer (KOTO-0136) is the retained alternative to
re-emitting unchanging chrome — a full-screen background, panel frames, fixed
labels — every frame. Capture it once between `game2d_static_begin()` and
`game2d_static_end()` (e.g. when entering a screen or when the layout changes);
the host composites it beneath the board tilemap and the per-frame immediate
commands, so a draw call per static element is spent once, not every frame. Rebuild
it (another `begin`/`end`) only when the static layout actually changes.

The Game2D sprite/stamp layer (KOTO-0140) is the retained alternative to
re-blitting moving pieces — an active piece, a ghost, previews, a cursor, a small
actor — with `draw_pixels` every frame. A *stamp* is a reusable cell pattern
(`game2d_stamp_define`, defined once, reusing the same packed-nibble cell table you
already use for blits); a *sprite* is a retained placed instance of a stamp drawing
a chosen tile (`game2d_sprite_set`). The host diffs sprites by stable `inst_id`, so
a moving sprite repaints only the union of the cells it left and entered — no
per-frame blit loop, no command-list churn. Hide an instance with
`game2d_sprite_hide` (or all with `game2d_sprite_clear_all`). v1 is cell-stamp-only
(a stamp draws one tile per cell); a sprite is composited above the tile layer and
below the immediate `draw_*` list. See [GAME2D_ABI.md](GAME2D_ABI.md).

## Input

| Function | Host call | Returns | Notes |
| :------- | :-------- | :------ | :---- |
| `text_input()` | `text_input` | codepoint | Typed Unicode scalar this frame, or `0` if none. |
| `text_intent()` | `text_input` | intent bits | Edit-intent bitset this frame (see `INTENT_*`). |
| `input_held()` | `input_snapshot` | held button bits | Frame-stable held buttons. |
| `input_pressed()` | `input_snapshot` | pressed button bits | Buttons newly pressed this frame. |

## Audio

Audio synthesis and decoding are **host-owned**: apps trigger built-in effects
by id and start package-local KotoMML or looping KACL with `play_bgm_asset`.
Waveform data never lives in the VM heap; large clips stream from the package.
`audio_submit` is a low-level escape hatch for apps that generate their own PCM.

| Function | Host call | Returns | Notes |
| :------- | :-------- | :------ | :---- |
| `play_sfx(id)` | `play_sfx` | status | Triggers a one-shot sound effect (`SFX_*`). |
| `play_sfx_asset(path, len)` | `play_sfx_asset` | status | Plays a one-shot Native cue (`.kmml`) or runtime-ready PCM16/SLD4 clip (`.kacl`) from the app KPA. Large KACL payloads stream through a bounded host ring. |
| `play_bgm_asset(path, len)` | `play_bgm_asset` | status | Starts package background music from `audio/*.kmml`, or from a PCM16/SLD4 `.kacl` carrying whole-clip infinite-loop metadata. Large KACL assets stream through bounded host buffers. |
| `stop_bgm()` | `stop_bgm` | status | Stops the looping background-music track. |
| `audio_submit(buf, frames, channels)` | `audio_submit_i16` | frames accepted / `-1` | Submits `frames * channels` interleaved little-endian i16 PCM samples from `buf` (so `buf` holds `frames * channels * 2` bytes). Nonblocking; may accept fewer frames. Stereo is downmixed to mono. |

## Heap Data

The low-level typed heap helpers are bounds-checked by the VM. They are intended
for SDK helpers and focused tests; normal app code should prefer named helpers such
as `ActorArray` accessors instead of open-coded field offsets.

| Function | Emits | Returns | Notes |
| :------- | :---- | :------ | :---- |
| `heap_get_u8(addr)` | `load8` | `0..255` | Reads one byte. |
| `heap_set_u8(addr, value)` | `store8` | no value | Writes the low 8 bits. |
| `heap_get_u16(addr)` | `load16` | `0..65535` | Reads little-endian unsigned 16-bit. |
| `heap_set_u16(addr, value)` | `store16` | no value | Writes the low 16 bits, little-endian. |
| `heap_get_i16(addr)` | `load16` + sign extension | `-32768..32767` | Reads little-endian signed 16-bit. |
| `heap_set_i16(addr, value)` | `store16` | no value | Writes the low 16 bits, little-endian. |

## ActorArray

`ActorArray` is a small heap-backed game-state model. `actor_array_new(count)`
reserves `count * 12` bytes at compile time and returns the base offset. Each actor
stores `x`, `y`, `vx`, and `vy` as signed i16; `state` and `frame` as u8; and
`timer` as u16. Actor count changes the app heap request and addressed heap
high-water, not `user_slots_used`.

| Function | Returns | Notes |
| :------- | :------ | :---- |
| `actor_array_new(count)` | base offset | `count` must be a positive compile-time constant. |
| `actor_set_pos(actors, i, x, y)` | no value | Writes `x`/`y` as signed i16. |
| `actor_set_vel(actors, i, vx, vy)` | no value | Writes `vx`/`vy` as signed i16. |
| `actor_set_state(actors, i, state)` | no value | Writes one byte. |
| `actor_set_frame(actors, i, frame)` | no value | Writes one byte. |
| `actor_set_timer(actors, i, timer)` | no value | Writes unsigned 16-bit. |
| `actor_x(actors, i)`, `actor_y(actors, i)` | signed i16 | Position accessors. |
| `actor_vx(actors, i)`, `actor_vy(actors, i)` | signed i16 | Velocity accessors. |
| `actor_state(actors, i)`, `actor_frame(actors, i)` | u8 | State/frame accessors. |
| `actor_timer(actors, i)` | u16 | Timer accessor. |

## IME

| Function | Host call | Returns | Notes |
| :------- | :-------- | :------ | :---- |
| `ime_feed_key(kind, codepoint)` | `ime_feed_key` | status | Feeds one key (`IME_*` kind; `codepoint` used when `kind == IME_CHARACTER`). `IME_TOGGLE` switches IME on/off. |
| `ime_convert()` | `ime_convert` | status | Runs dictionary conversion on the current reading. |
| `ime_query_line(buf, max)` | `ime_query_line` | bytes written / `-1` | Serializes the structured composition line into `buf` (layout below). |
| `ime_display(buf, max)` | `ime_display` | bytes written | Writes a plain UTF-8 display string for the active composition. Active states are prefixed as `comp:`, `read:`, `cand:`, or `miss:` so apps can draw deterministic feedback directly with `draw_text`. |

The `ime_query_line` record is: `[mode:u8][sticky:u8]` then three length-prefixed
UTF-8 fields (`pending`, `reading`, `candidate`), each `[len:u8][bytes]`, then
`[cand_index:u8][cand_count:u8]` — the shown candidate's zero-based position and
the total candidate count for the current reading (both saturate at `255`; `0`
when there is no candidate). `mode` is `0` empty, `1` composing, `2` converting,
`3` candidate, `4` missing-candidate. Re-running `ime_convert` while a candidate
is shown cycles to the next candidate (wrapping).

### Simulator IME behavior

| Input | IME off | IME on |
| :---- | :------ | :----- |
| ASCII character | Inserts the character directly. | Alphabetic romaji is composed into kana. Unsupported romaji recovers as literal text instead of trapping. |
| Romaji sequence | Inserts each ASCII character directly. | Complete sequences commit kana (`ka` -> `か`); partial input is displayed as `comp:<pending>`. `nn`/`n'` commit a single ん with nothing left pending. |
| Punctuation / symbol | Inserts the ASCII character directly. | Inserts Japanese punctuation (`,`/`.` -> `、`/`。`, `-` -> `ー`, `/` -> `・`) or the full-width form for other printable symbols (`?`/`!` -> `？`/`！`, otherwise U+FF01..U+FF5E). Digits and spaces stay half-width. |
| Sticky Shift | No text change by itself. | Arms the next character as a conversion reading starter; the reading display uses `read:<reading>`. |
| Convert / Space | No text change without a reading; Space remains ordinary text. | Looks up the reading. A hit replaces the inline reading with the candidate; a miss keeps the reading visible. Pressing Space again cycles to the next candidate. |
| `q` while converting | Inserts `q`. | Commits the current reading converted to katakana and clears the composition. |
| Commit | No text change without active composition. | Commits the candidate or reading to the editor and clears the IME line. |
| Cancel | No text change. | Clears pending romaji, reading, candidate, and Sticky Shift state. |
| Backspace | Deletes before the editor cursor. | Deletes before the editor cursor. Apps may instead feed `IME_BACKSPACE` while composing to delete the last reading/pending character (ending the conversion when none remain) rather than editing the document. |
| Enter / Newline | Inserts a newline directly. | Commits an active candidate/reading without submitting the field; with no active composition it remains a normal newline/submit action. |

KotoSim window mode maps F1 to IME on/off, Space to conversion/candidate cycling,
Enter to commit, and Ctrl+G to cancel, matching the conventional SKK flow. F2
saves, F4 opens, F5 creates an unnamed empty memo, and F10 exits the app. Tab is
retained as a compatibility convert key for non-retained editor apps; retained
KotoUI sessions consume it as focus-next in widget-ID order. Named-file saves
ask for `y/n`; `n` opens Save As.
Backspace and Delete repeat while held. App input scripts use
the same intent names (`ime-toggle`, `convert`, `commit`, `cancel`, `save`,
`exit`) plus single-quoted characters such as `'a'` or `'\n'`.

## Text Editor

| Function | Host call | Returns | Notes |
| :------- | :-------- | :------ | :---- |
| `edit_load(buf, len)` | `edit_load` | status | Replaces the document with `len` UTF-8 bytes from `buf`. |
| `edit_query_text(buf, max)` | `edit_query_text` | byte length / `-1` | Copies the document into `buf` (up to `max`); returns its length. |
| `edit_visible_line(buf, max, row)` | `edit_visible_line` | byte length / `-1` | Copies one visible editor row into `buf`. `row` is relative to the current scroll position. |
| `edit_cursor_col()` | `edit_cursor_view` | column / `-1` | Returns the cursor column in the visible editor viewport. |
| `edit_cursor_row()` | `edit_cursor_view` | row / `-1` | Returns the cursor row in the visible editor viewport. |
| `edit_scroll_row()` | `edit_scroll_row` | row / `-1` | Returns the first visible document row, useful for scroll indicators. |
| `edit_cell_width()` | `edit_view_metrics` | pixels / `-1` | Returns the half-width cell advance used by the host editor and renderer. |
| `edit_cell_height()` | `edit_view_metrics` | pixels / `-1` | Returns the editor row pitch used by the host editor and renderer. |
| `edit_cursor_status(buf, max)` | `edit_cursor_status` | bytes written / `-1` | Writes a compact `Ln N Col M` status string for the current cursor. |
| `edit_total_lines()` | `edit_total_lines` | rows / `-1` | Returns the total visual (soft-wrapped) row count for scroll indicators. |
| `edit_wrap()` | `edit_wrap` | `1` / `0` | Returns whether the editor soft-wraps long lines (`0` = horizontal scroll). |
| `edit_hscroll()` | `edit_hscroll_view` | columns / `-1` | Horizontal scroll offset (columns); `0` while wrapping. |
| `edit_line_cols()` | `edit_hscroll_view` | columns / `-1` | Display width of the cursor's logical line, for a horizontal scrollbar. |
| `edit_move(dir)` | `edit_move` | status | Moves the cursor (`DIR_*`). |
| `edit_delete(kind)` | `edit_delete` | removed (`0`/`1`) / `-1` | Deletes around the cursor (`DELETE_*`). |
| `edit_reserve_rows(rows)` | `edit_reserve_rows` | status | Reserves the bottom `rows` rows of the viewport for an overlay (e.g. the IME panel) so the host scrolls the cursor above them; pass `0` to clear. |
| `edit_configure(cols, rows)` | `edit_configure` | status | Configures the app-visible editor viewport before loading document text. |

## Files

| Function | Host call | Returns | Notes |
| :------- | :-------- | :------ | :---- |
| `file_open(path, len, mode)` | `file_open` | handle / `-1` | Opens a sandboxed file (`MODE_*`). |
| `file_read(handle, buf, max)` | `file_read` | bytes read / `-1` | Reads into `buf`. |
| `file_write(handle, buf, len)` | `file_write` | bytes written / `-1` | Writes `len` bytes from `buf`. |
| `file_close(handle)` | `file_close` | status | Releases the handle. |
| `dir_count(buf, max, index)` | `dir_list` | entry count / `-1` | Number of files in the app save-data sandbox (also writes entry `index` into `buf`). |
| `dir_name(buf, max, index)` | `dir_list` | name length / `-1` | Writes the `index`-th sandbox filename (sorted) into `buf`; `0` when out of range. |

## Package Assets

The `file_*` calls above target the per-app **save sandbox** (`data/<app_id>/...`).
Package assets are different: they are the read-only files the manifest ships
inside the `.kpa` (audio, icons, sprite sheets). Audio assets are played by path
(`play_*_asset`); other asset bytes are pulled into the app heap with `asset_load`.
App-authored text or data files are declared in the `app.json` `assets` array as
`source` plus an optional package `output` path; `build_apps.py` copies them
byte-for-byte into the KPA as read-only `data` entries.

| Function | Host call | Returns | Notes |
| :------- | :-------- | :------ | :---- |
| `asset_load(path, len, buf, max)` | `asset_load` | bytes read / `-1` | Copies a manifest-declared package asset (e.g. an `*.kim` tile sheet) into `buf` in one shot, up to `max` bytes. The path must be declared by the manifest `assets` list; it is resolved read-only against the package, never the save sandbox. |

A typical use is loading a [`KIM1`](../guides/ASSET_PIPELINE.md) tile sheet once at startup,
then blitting individual 16×16 tiles each frame with `draw_pixels`:

```koto
buf sheet[9736];                                   // KIM1 header (8) + 19 * 512
asset_load("sprites/tiles.kim", 17, sheet, 9736);  // load once
draw_pixels(x, y, 16, 16, sheet + 8 + tile * 512, 512);  // blit tile `tile`
```

### Text map assets (`.map`)

An `app.json` `maps` block declares an app-relative directory, logical `width`
and `height`, and the allowed `glyphs`. `build_apps.py` validates every `*.map`
file in that directory and adds it to the KPA as a read-only `data` asset at the
same package-local path. Map contents are not generated into Koto source or KBC
rodata.

The source format is UTF-8 text with exactly `height` rows and `width` Unicode
glyphs per row. Rows use LF or CRLF; a final row ending is optional. A bare CR,
blank extra row, invalid UTF-8, undeclared glyph, or a start-marker count other
than exactly one `@` fails the build. Runtime code must skip CR/LF explicitly or
decode into a flat cell buffer before row-major indexing.

For a fixed buffer, reserve at most
`(width * max_utf8_bytes_per_allowed_glyph + 2) * height` bytes. The `+2`
allows CRLF on every row, including the last. Check the `asset_load` result and
the decoded cell count before reading the map:

```koto
buf raw_map[440]; // (20 * 1 + 2) * 20 for an ASCII 20x20 map
let loaded = asset_load("maps/world.map", 14, raw_map, 440);
if loaded < 0 { exit(2); }
// Decode exactly 400 non-CR/LF bytes into a bounded flat cell buffer.
```

## Constants

Mutually exclusive SDK values are also exposed as `FileMode`, `ImeKey`,
`EditDirection`, and `DeleteKind`; new code should prefer spellings such as
`FileMode::Read` and `EditDirection::Left`. The table below records the stable
flat aliases retained for existing source and KBC rebuilds. Capability bits,
intent masks, capacities, ABI versions, and byte sizes remain flat constants.

| Group | Names | Values |
| :---- | :---- | :----- |
| File modes | `MODE_READ`, `MODE_WRITE`, `MODE_READWRITE` | `0`, `1`, `2` |
| IME key kinds | `IME_CHARACTER`, `IME_SHIFT`, `IME_CONVERT`, `IME_COMMIT`, `IME_CANCEL`, `IME_OTHER`, `IME_TOGGLE`, `IME_BACKSPACE` | `0`–`7` |
| Cursor directions | `DIR_LEFT`, `DIR_RIGHT`, `DIR_UP`, `DIR_DOWN`, `DIR_HOME`, `DIR_END` | `0`–`5` |
| Delete kinds | `DELETE_BACKSPACE`, `DELETE_FORWARD` | `0`, `1` |
| Intent bits | `INTENT_SHIFT`, `INTENT_CONVERT`, `INTENT_COMMIT`, `INTENT_CANCEL`, `INTENT_BACKSPACE`, `INTENT_DELETE`, `INTENT_LEFT`, `INTENT_RIGHT`, `INTENT_UP`, `INTENT_DOWN`, `INTENT_HOME`, `INTENT_END`, `INTENT_NEWLINE`, `INTENT_SAVE`, `INTENT_EXIT`, `INTENT_IME_TOGGLE`, `INTENT_OPEN`, `INTENT_NEW` | `1<<0` … `1<<17` |
| Time query | `TIME_UTC_SECONDS`, `TIME_OFFSET_MINUTES`, `TIME_MONOTONIC_MS`, `TIME_MONOTONIC_MASK` | `0`, `1`, `2`, `0x3FFFFFFF` |
| Vault service | `VAULT_SERVICE_FETCH`, `VAULT_SERVICE_MQTT` | `0`, `1` |
| MQTT poll state | `MQTT_CONNECTING`, `MQTT_CONNECTED`, `MQTT_MESSAGE`, `MQTT_DISCONNECTED`, `MQTT_FAILED` | `0`–`4` |
| MQTT read result | `MQTT_READ_NONE`, `MQTT_READ_MESSAGE`, `MQTT_READ_RETAINED` | `0`, `1`, `2` |
| MQTT profile | `MQTT_MAX_BROKERS`, `MQTT_MAX_TOPIC_FILTERS`, `MQTT_MAX_TOPIC_BYTES`, `MQTT_MAX_PAYLOAD_BYTES`, `MQTT_MAX_QUEUE`, `MQTT_KEEPALIVE_SECS` | `2`, `8`, `128`, `192`, `8`, `60` |

A user `const` of the same name overrides the predefined value. The constants are
sourced from the host ABI modules (`koto_core::runtime`), so they cannot drift from
the runtime.

## Examples

### Frame loop skeleton

```koto
fn main() {
    loop {
        let intent = text_intent();
        if intent & INTENT_EXIT != 0 {
            exit(0);
        }
        draw_rect(0, 0, 320, 320, 0);
        yield_frame();
    }
}
```

### Load and save a sandboxed file

```koto
fn main() {
    buf path[8];
    buf doc[512];
    // (path bytes filled elsewhere, e.g. via a string literal copy)

    let h = file_open(path, 8, MODE_READ);
    if h >= 0 {
        let n = file_read(h, doc, 512);
        file_close(h);
        edit_load(doc, n);
    }

    let len = edit_query_text(doc, 512);
    let w = file_open(path, 8, MODE_WRITE);
    if w >= 0 {
        file_write(w, doc, len);
        file_close(w);
    }
    exit(0);
}
```

### IME composition and commit

```koto
fn main() {
    buf line[96];
    loop {
        let cp = text_input();
        let intent = text_intent();
        if cp != 0 {
            ime_feed_key(IME_CHARACTER, cp);
        }
        if intent & INTENT_CONVERT != 0 {
            ime_convert();
        }
        if intent & INTENT_COMMIT != 0 {
            ime_feed_key(IME_COMMIT, 0);
        }
        let clen = ime_display(line, 96);
        draw_text(0, 300, line, clen);
        yield_frame();
    }
}
```

## Stability

This prelude is intentionally small so Koto Memo, KotoVN prototypes, and later PDA
utilities share the same runtime concepts. New host calls should be added here as
named wrappers with documented results rather than exposed as raw IDs.
