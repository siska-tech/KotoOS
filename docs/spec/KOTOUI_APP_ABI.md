# KotoUI Application ABI

- Status: v1 design frozen by KOTO-0217; implementation begins in KOTO-0218
- Host ABI: major 1, minor 18
- Byte order: little-endian

## Decision

KotoUI applications use a bounded retained description. An app mounts a flat,
versioned node table once, sends atomic property patches when business state
changes, asks the host to present pending damage, and polls fixed semantic event
records. Rust component layouts, callbacks, and VM pointers are not ABI.

| Approach | Advantages | Rejected cost |
| :-- | :-- | :-- |
| One host call per widget/property | Easy first prototype | Large call surface, partial construction states, ABI churn for every property |
| Immediate UI command stream every frame | Stateless host, simple app ownership | Rebuild/validation/call cost every frame; loses focus/edit state and zero-idle redraw |
| Versioned retained description (selected) | Atomic mount, stable IDs, retained interaction, targeted damage, compact host-call surface | Requires a validator, bounded host session, and explicit wire versioning |

The selected format is semantic rather than a dump of `koto-ui` structs. A host
may change its internal KotoUI representation while continuing to accept format
1.0.

## Common rules

- All multi-byte integers are little-endian. Signed values use two's complement.
- Packet offsets are byte offsets and must be naturally valid for their field;
  the decoder must not rely on aligned VM memory access.
- Widget IDs `1..=65534` are app-owned. `0` is the host/system event ID and
  `0xffff` means no widget.
- Coordinates are signed `i16`; widths/heights are `u16` in `1..=32767`.
  Painting clips to the 320x320 app surface. Overflow-safe widened arithmetic is
  required before conversion to KotoUI coordinates.
- Node order is paint order. A parent must precede every child, making cycles
  impossible after validation; keyboard focus order is defined separately below.
- Exactly one root `Panel` has parent `0xffff`. Other nodes name a prior Panel or
  Dialog parent. Dialog nodes are direct children of the root.
- The root Panel is `(0, 0, 320, 320)`. Every other node's clipped bounds must be
  contained by its parent bounds so repainting the retained root can erase a
  hidden, moved, or closed child without relying on app-issued background draws.
- Strings are UTF-8 byte ranges. Length zero is valid. Embedded NUL has no
  special meaning. Ranges may not overlap packet headers or node records.
- Reserved fields and unknown flag bits must be zero in format 1.0.
- Mount and update are atomic: validation completes before any live session
  mutation. Failure leaves the prior session and its pending events unchanged.
- KotoUI receives resolved UTF-8 display text. Translation catalogs and message
  selection belong to Shell/apps, not to the component ABI or KotoUI core.
- Locale tags use canonical ASCII BCP 47 form and are at most 23 bytes. Language
  is lowercase; script and region use canonical Titlecase/uppercase form when
  present; registered variant subtags are permitted. v1 product locales are
  `en-US` and `ja-JP`; unsupported or invalid preferences resolve to `en-US`.

## Required limits

`ui_capabilities` reports these v1 minima and the implementation must not expose
smaller values on either KotoSim or PicoCalc.

| Resource | v1 limit | Policy |
| :-- | --: | :-- |
| Mount packet | 4,096 B | Includes header, 32 nodes, and source data |
| Nodes | 32 | Flat table, maximum hierarchy depth 4 |
| Focusable visible nodes | 16 | Matches KotoUI default focus capacity |
| Host data arena | 2,048 B | Sum of every node's text/value capacity |
| Text fields | 4 | Each at most 256 B; aggregate value capacity at most 512 B |
| List rows | 32 | Aggregate across all lists |
| Dialog descriptors | 2 | At most one dialog open/modal |
| Children/actions per Dialog | 8 / 4 | Matches KotoUI default Dialog capacities |
| Update packet | 2,048 B | At most 16 property records |
| Event queue | 8 | Fixed internal records plus an overflow latch |
| Damage rectangles | 8 | Existing KotoUI full-surface fallback on overflow |
| Host UI session SRAM | 8,192 B | Includes nodes, data, focus, events, and damage; excludes framebuffer/font |

An implementation may advertise larger values in a future compatible minor
format. Apps must query capabilities rather than assuming more than the table.

## Capability record (`KUC1`, 64 bytes)

`ui_capabilities(dst, max_len)` writes this complete dynamic record when
`max_len >= 64`.

| Offset | Size | Field | v1 value |
| --: | --: | :-- | :-- |
| 0 | 4 | magic | ASCII `KUC1` |
| 4 | 2 | format_major | 1 |
| 6 | 2 | format_minor | 0 |
| 8 | 4 | node_kind_mask | bits 1 through 7 set |
| 12 | 2 | max_nodes | 32 |
| 14 | 1 | max_depth | 4 |
| 15 | 1 | event_capacity | 8 |
| 16 | 2 | max_mount_bytes | 4096 |
| 18 | 2 | max_update_bytes | 2048 |
| 20 | 2 | max_data_capacity | 2048 |
| 22 | 2 | max_text_field_bytes | 256 |
| 24 | 2 | max_list_rows | 32 |
| 26 | 1 | damage_capacity | 8 |
| 27 | 1 | max_open_modals | 1 |
| 28 | 2 | host_abi_minor | 18 |
| 30 | 2 | flags | bit 0: IME-capable text fields |
| 32 | 1 | locale_len | 1..23 bytes |
| 33 | 1 | text_direction | 0 LTR; 1 RTL reserved |
| 34 | 2 | reserved | zero |
| 36 | 4 | locale_generation | Nonzero wrapping generation |
| 40 | 24 | locale | Canonical BCP 47 bytes followed by zero padding |

The locale fields may change while an app is running, so this is a capability
snapshot rather than immutable host metadata. Generation zero is reserved. A
locale change increments the generation with wrapping `0xffff_ffff -> 1` and
queues the `LocaleChanged` response described below. `qps-ploc` is accepted by
test/simulator profiles as an expanded LTR pseudolocale, but is not a product
locale advertised on a normal device.

## Mount packet (`KUI1`)

### Header (40 bytes)

| Offset | Size | Field | Rule |
| --: | --: | :-- | :-- |
| 0 | 4 | magic | ASCII `KUI1` |
| 4 | 2 | format_major | 1 |
| 6 | 2 | format_minor | 0 |
| 8 | 4 | total_len | Exact host-call `len` |
| 12 | 2 | node_count | 1..32 |
| 14 | 2 | node_stride | 48 |
| 16 | 4 | nodes_offset | 40 in v1 |
| 20 | 4 | data_offset | At or after the node table |
| 24 | 4 | data_len | Source bytes present in this packet |
| 28 | 2 | root_id | ID of the root Panel |
| 30 | 2 | initial_focus_id | Focusable ID or `0xffff` for first eligible |
| 32 | 4 | flags | zero in v1 |
| 36 | 4 | reserved | zero |

`nodes_offset + node_count * node_stride`, `data_offset + data_len`, and all
subranges use checked arithmetic and must not exceed `total_len`. Padding between
sections must be zero.

### Node record (48 bytes)

| Offset | Size | Field | Meaning |
| --: | --: | :-- | :-- |
| 0 | 2 | id | Stable widget ID |
| 2 | 2 | parent_id | Prior Panel/Dialog ID or `0xffff` for root |
| 4 | 1 | kind | Node kind below |
| 5 | 1 | flags | visibility, enablement, direction, overflow below |
| 6 | 2 | state_flags | Kind-specific flags |
| 8 | 2 | x | signed absolute x |
| 10 | 2 | y | signed absolute y |
| 12 | 2 | width | positive width |
| 14 | 2 | height | positive height |
| 16 | 4 | text_offset | Relative to mount data section |
| 20 | 2 | text_len | Initial UTF-8 bytes |
| 22 | 2 | text_capacity | Reserved mutable text bytes, `>= text_len` |
| 24 | 4 | value_offset | Relative to mount data section |
| 28 | 2 | value_len | Initial kind-specific blob bytes |
| 30 | 2 | value_capacity | Reserved mutable blob bytes, `>= value_len` |
| 32 | 4 | arg0 | Kind-specific signed value |
| 36 | 4 | arg1 | Kind-specific signed value |
| 40 | 4 | arg2 | Kind-specific signed value |
| 44 | 4 | arg3 | Kind-specific signed value |

The host allocates fixed text/value slots from its 2,048-byte arena according to
the declared capacities, then copies the initial ranges. It does not preserve
packet offsets or retain the app pointer. Sum-of-capacities validation happens
before copying.

Node `flags` use bit 0 for visible and bit 1 for enabled. Bits 2–3 are text
direction: 0 inherits the current locale, 1 explicitly selects LTR, and 2
reserves RTL. Bits 4–5 are single-line overflow: 0 clips, 1 uses end-ellipsis,
and 2 reserves wrapping. Values 3 and bits 6–7 are reserved. Format 1.0 accepts
inherited/LTR and clip/end-ellipsis; it returns `UNSUPPORTED` for RTL or wrap.
TextField values use their existing horizontal cursor scrolling rather than an
ellipsis. When ellipsis is requested, the host draws U+2026 if supported by the
active font, otherwise the longest suffix of ASCII `...` that fits.

### Node kinds

| ID | Kind | Text/value and arguments |
| --: | :-- | :-- |
| 1 | Label | text is display/semantic label; `arg0` alignment 0 start, 1 center, 2 end |
| 2 | Button | text is display/semantic label; state bit 0 marks a Dialog-closing action |
| 3 | Checkbox | text is label; state bit 0 is checked; `arg0`/`arg1` offset the square mark from its left-aligned, vertically centered position; `arg2`/`arg3` are zero |
| 4 | List | value is the row blob; `arg0` row count, `arg1` selected index or -1, `arg2` row height; state bit 0 shows scrollbar |
| 5 | TextField | text is placeholder/semantic label; value is UTF-8 content; `arg0` cursor byte offset or -1 for end; state bit 0 enables host IME integration |
| 6 | Panel | text is optional title; `arg0` padding 0..32, `arg1` title height 0..64 |
| 7 | Dialog | text is title; `arg0` padding, `arg1` title height, `arg2` default action ID or -1, `arg3` cancel action ID or -1; state bit 0 is open |

Display-only nodes ignore enabled but it must still be encoded consistently.
Focusable kinds are Button, Checkbox, List, and TextField. Dialog action IDs must
name enabled child Buttons carrying the action state flag. Only one Dialog may
have its open bit set. An initial focus ID must be eligible in the active scope;
when a Dialog starts open, its default action or first eligible child is used.

### List row blob

A List value blob begins with `arg0` fixed 12-byte row records followed by their
UTF-8 labels. Row offsets are relative to the start of the List value blob.

| Offset | Size | Field |
| --: | --: | :-- |
| 0 | 2 | label_offset |
| 2 | 2 | label_len |
| 4 | 2 | flags: bit 0 enabled |
| 6 | 2 | reserved zero |
| 8 | 4 | app_value returned as event `aux` |

Row labels are also their semantic labels in v1. Selection uses a zero-based
index; disabled rows cannot be selected or activated.

## Update packet (`KUP1`)

Updates change existing fixed-capacity slots or scalar state. Node kind, parent,
and declared capacities cannot change without a new mount.

### Header (32 bytes)

| Offset | Size | Field | Rule |
| --: | --: | :-- | :-- |
| 0 | 4 | magic | ASCII `KUP1` |
| 4 | 2 | format_major | 1 |
| 6 | 2 | format_minor | 0 |
| 8 | 4 | total_len | Exact host-call length |
| 12 | 2 | update_count | 1..16 |
| 14 | 2 | update_stride | 32 |
| 16 | 4 | records_offset | 32 in v1 |
| 20 | 4 | data_offset | At or after records |
| 24 | 4 | data_len | Patch source bytes |
| 28 | 4 | reserved | zero |

### Property record (32 bytes)

| Offset | Size | Field |
| --: | --: | :-- |
| 0 | 2 | widget_id |
| 2 | 1 | property kind |
| 3 | 1 | flags, zero in v1 |
| 4 | 4 | data_offset relative to update data section |
| 8 | 2 | data_len |
| 10 | 2 | reserved zero |
| 12 | 4 | arg0 |
| 16 | 4 | arg1 |
| 20 | 4 | arg2 |
| 24 | 4 | arg3 |
| 28 | 4 | reserved zero |

| Property | ID | Payload |
| :-- | --: | :-- |
| SetText | 1 | UTF-8 data within node text capacity |
| SetEnabled | 2 | `arg0` 0/1 |
| SetVisible | 3 | `arg0` 0/1 |
| SetChecked | 4 | Checkbox `arg0` 0/1 |
| SetSelection | 5 | List `arg0` index or -1 |
| SetTextValue | 6 | TextField UTF-8 data; `arg0` cursor or -1 |
| SetBounds | 7 | `arg0..3` x, y, width, height |
| SetListRows | 8 | Complete List row blob; `arg0` row count, `arg1` selection or -1 |
| SetDialogOpen | 9 | Dialog `arg0` 0/1 |
| RequestFocus | 10 | no data/args; target must be eligible |

A packet may update multiple widgets, but the same `(widget, property)` pair may
appear only once. Validation checks every record and aggregate modal/focus/list
limit before applying any record. Programmatic changes use the same KotoUI
setters and damage rules as input-driven changes. The root cannot be hidden or
disabled; closing a Dialog damages its complete backdrop/dialog repaint region.

## Semantic event (`KUE1`)

`ui_poll_event(dst, max_len)` writes one 32-byte header plus optional UTF-8 text.
It returns zero bytes with success when the queue and overflow latch are empty.
If the destination is too small, it returns `NO_MEMORY` without dequeuing.

| Offset | Size | Field |
| --: | --: | :-- |
| 0 | 4 | magic `KUE1` |
| 4 | 2 | format_major 1 |
| 6 | 2 | format_minor 0 |
| 8 | 4 | total_len |
| 12 | 1 | response kind |
| 13 | 1 | flags, zero in v1 |
| 14 | 2 | widget_id |
| 16 | 4 | value |
| 20 | 4 | aux |
| 24 | 4 | text_offset, 32 when text exists else 0 |
| 28 | 2 | text_len |
| 30 | 2 | reserved zero |

| ID | Response | `value` / `aux` / text |
| --: | :-- | :-- |
| 1 | Activated | zero / zero |
| 2 | ValueChanged | checkbox 0/1 / zero |
| 3 | TextChanged | byte length / cursor byte offset / current UTF-8 text |
| 4 | SelectionChanged | selected index / row `app_value` |
| 5 | SelectionActivated | selected index / row `app_value` |
| 6 | Submitted | byte length / cursor / current TextField UTF-8 text when applicable |
| 7 | Cancelled | zero / zero |
| 8 | FocusChanged | previous widget ID or `0xffff` / zero; header widget is new focus |
| 9 | CapacityRejected | resource ID / cumulative dropped count; widget 0 for queue overflow |
| 10 | LocaleChanged | locale generation / text direction; widget 0, text is canonical locale tag |

Events preserve input dispatch order. When the eight-record queue is full, the
new response is dropped, a dropped-count latch increments saturating at
`i32::MAX`, and no existing event is reordered. After queued records drain, one
CapacityRejected event reports the latch and clears it.

Only one pending `LocaleChanged` response is required: if the locale changes
again before delivery, the queued response is replaced with the newest
generation/tag without reordering earlier non-locale responses. After receiving
it, an app selects localized resources using exact tag, language, `en-US`, then
its embedded default, and remounts or atomically updates every affected string.

## Host calls

Host ABI minor 18 reserves the previously unused `0x50..0x55` range.

| ID | Name | Stack arguments | Success result |
| --: | :-- | :-- | :-- |
| 0x50 | `ui_capabilities` | `dst_ptr dst_max` | `bytes_written` |
| 0x51 | `ui_mount` | `src_ptr len` | none |
| 0x52 | `ui_update` | `src_ptr len` | none |
| 0x53 | `ui_present` | none | none |
| 0x54 | `ui_poll_event` | `dst_ptr dst_max` | `bytes_written` |
| 0x55 | `ui_reset` | none | none |

Every call pushes its result slots followed by status on success and failure,
matching the existing Host ABI convention. The verifier stack effects are
respectively `(2,2)`, `(2,1)`, `(2,1)`, `(0,1)`, `(2,2)`, and `(0,1)`.

### Frame lifecycle

1. `ui_capabilities` may be called before mounting and returns the current
   locale snapshot as well as fixed capacities.
2. `ui_mount` atomically replaces any prior session and marks the mounted scene
   damaged. Mount is idempotent only in effect; it resets interaction state.
3. At the start of each app VM frame, the host dispatches that frame's normalized
   input to the mounted session before bytecode resumes. Responses are therefore
   available to `ui_poll_event` in the same VM frame.
4. The app drains events, changes business state, and sends at most one atomic
   update packet for the frame.
5. `ui_present` converts pending KotoUI damage into the existing render path.
   With no damage it succeeds without commands or transfer. Damage clears only
   after scheduling succeeds.
6. The app calls `yield_frame`. Low-level input/drawing calls remain available
   for compatibility but must not be used to double-handle mounted UI controls.
7. `ui_reset`, normal exit, trap, forced termination, or launch of another app
   clears descriptors, focus/modal/IME state, data, damage, and queued events.

`ui_reset` is idempotent and succeeds without a mounted session.

Changing locale does not mutate app-owned text automatically. KotoConfig writes
the shared ConfigService; the host queues `LocaleChanged`, and the app chooses
translated strings and updates/remounts them. Shell reads the same locale source
and generation, preventing Shell/app drift.

## Input, focus, and IME

The host uses the same `VmInputSnapshot` mapping on simulator and device. Bits
0..5 of `pressed_bits` map to up, down, left, right, activate, and cancel.
Editing intents supply backspace, delete, home, end, and submit; `text_codepoint`
is last. Duplicate direction/cancel intent bits produced from the same physical
key are coalesced. Dispatch order is up, down, left, right, focus-next, activate,
cancel, backspace, delete, home, end, submit, then Unicode text.

Cardinal directions move focus to the nearest eligible control wholly beyond the
focused control's corresponding edge. Candidates are ranked by center-to-center
distance, then cross-axis distance, primary-axis distance, and widget ID. This
spatial traversal stays inside the active focus scope and does not wrap. When no
candidate exists, the direction is offered to the focused composite control.
A focused List is the exception: Up/Down changes its selected row first, and
spatial focus movement is attempted only when the selection is already at the
corresponding first/last enabled row. An unfocused List uses disabled theme
colors without changing its enabled or focusable semantics. Tab is carried as
the current `CONVERT` intent and cycles eligible controls by ascending widget
ID, wrapping after the largest ID; Previous uses descending ID order. Hidden
and disabled controls are skipped. A visible open Dialog owns the sole modal
focus scope and restores the prior root focus when closed.

Enabled, focused, and editing are separate states. The mounted `enabled` flag
controls whether a component can receive focus or input. Focus identifies the
component selected by spatial/Tab navigation. A focused TextField enters the
host-owned editing state only after Activate (Enter); before activation it shows
the focus mark but does not accept text or move its caret. While editing,
Left/Right always moves the caret instead of changing focus. Cancel first leaves
editing, while Tab or spatial Up/Down leaves the field and clears editing.
Submit keeps the field in editing state. The caret is horizontally positioned
from measured glyph advances and vertically centered to the backend font's line
height rather than stretched across the control's content height.

Only one focused TextField may own composition. A field with its IME state flag
uses the existing host KotoIME service: integration code supplies composition
snapshots to KotoUI and commits resulting UTF-8 through normal text events.
An active candidate replaces the reading at the same inline position rather than
being appended to its right. Space converts/cycles, Enter commits before Submit
is dispatched, and Ctrl+G cancels the composition; Ctrl alone has no IME action.
Changing focus, hiding/disabling the field, closing its Dialog, reset, or app
termination cancels composition. KotoUI itself still does not implement kana or
SKK conversion.

Format 1.0 lays out text left-to-right. Japanese and English share this path.
Missing glyphs resolve to U+FFFD when the active font contains it and otherwise
to ASCII `?`; missing glyphs never change node geometry. RTL direction and
complex-script shaping are reserved and rejected as `UNSUPPORTED` in v1.

## Error mapping and safety

| Condition | Host status |
| :-- | :-- |
| Bad range/length, UTF-8, duplicate ID, hierarchy, geometry, state/reference, nonzero reserved | `BAD_ARGUMENT` (-2) |
| Unsupported format version, node kind, property, or capability | `UNSUPPORTED` (-5) |
| No mounted session for update/present/poll | `NOT_FOUND` (-4) |
| Node/data/focus/dialog/event destination capacity exceeded | `NO_MEMORY` (-8) |

Empty event polling is successful with zero bytes, not `WOULD_BLOCK`. The host
validates app-heap ranges immediately before reading/writing and never retains a
pointer. Mount/update input bytes may be changed by the app after the call with
no effect. Event output is written only after the complete record fits.

Validation precedence is deterministic: inaccessible/truncated heap range and
bad magic/length are `BAD_ARGUMENT`; unsupported format is checked next;
advertised capacity limits are checked before derived table ranges and return
`NO_MEMORY`; remaining structural/semantic checks return `BAD_ARGUMENT` unless
the table above assigns `UNSUPPORTED`. `ui_capabilities` and `ui_poll_event`
return `NO_MEMORY` for a destination too small for the complete next record and
write nothing.

## Compatibility and presentation

- Host ABI major remains 1 and minor becomes 18. Older KBCs continue to verify
  and run unchanged. A minor-18 KBC is rejected cleanly by an older host.
- Existing `draw_*`, Game2D, editor, input, file, and audio calls are unchanged.
- Format major changes only for an incompatible packet interpretation. New node
  kinds/properties require capability bits and a compatible format minor.
- v1 uses the host KotoUI theme and font. `en-US` is the deterministic locale
  fallback, `ja-JP` is equally supported, and `qps-ploc` provides 35–50% ASCII
  expansion for tests. App-supplied arbitrary themes, images,
  custom painting, accessibility trees, and pointer interaction are deferred.
- The maximum retained UI session is 8 KiB SRAM. KOTO-0218 must measure actual
  static/session size and reduce capacities or explain any excess before merge.

## Canonical fixtures

Text fixtures live in `harness/fixtures/koto_ui_abi/`. Hex is whitespace-free,
lowercase, and represents exact packet bytes. Runtime and compiler tests should
decode the valid fixture and derive malformed cases using the documented byte
mutations rather than maintaining divergent hand-built examples.
