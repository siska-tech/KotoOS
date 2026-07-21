# KotoUI single-line text field contract

KOTO-0212 adds an allocation-free `TextField` that edits a caller-owned
`Utf8Buffer`. The component owns only cursor, horizontal viewport, interaction,
and presentation state. KotoIME continues to own romaji conversion, dictionary
access, candidate selection, and sticky-shift behavior.

## Buffer and editing

`Utf8Buffer<'a>` borrows a byte slice and tracks the initialized UTF-8 length.
`new`, `from_str`, and `from_initialized` make capacity and validation explicit.
Insertion first checks a character boundary and available bytes; a full buffer
is unchanged and produces `CapacityRejected`. Removal accepts only ordered UTF-8
boundaries. TextField cursor indices and `TextChanged(usize)` lengths are byte
indices/lengths, not Unicode scalar or display-cell counts.

The focused, enabled field accepts committed `Text(char)`, left/right,
home/end, backspace, delete, submit, and cancel events. Navigation and deletion
may repeat; committed text, submit, and cancel require the initial press.
`TextChanged` means the caller-owned value changed, `Submitted` and `Cancelled`
are semantic requests and do not mutate it, and `CapacityRejected` means an
otherwise valid insertion did not fit. Released, disabled, and unfocused input
is ignored.

IME commit remains an adapter concern: the adapter sends each committed scalar
as `UiAction::Text`. IME cancel clears its own snapshot and may also forward
`UiAction::Cancel` when the application should observe cancellation.

## Painting and damage

`Painter` extends `TextMetrics`; the backend measures the active font in pixels.
TextField advances its UTF-8 viewport until the measured prefix leaves the
cursor visible, so it never assumes an ASCII cell width. The value, placeholder,
focus/cursor, disabled state, composition, and candidate use distinct theme
tokens. `ImeComposition` borrows both strings for one paint call and is never
retained.

Value edits and cursor movement damage the field. A rejected full-buffer edit,
an unchanged cursor, or ignored input produces no damage. Since the IME snapshot
is externally owned, its owner calls `invalidate_composition` when that snapshot
changes. External value replacement similarly calls `invalidate_value`.

The shell's reserved IME line remains available for global conversion UI and
does not become TextField storage. An application may paint the borrowed
composition inline in the field, in the reserved line, or both; layout policy
belongs to the application/shell. The field always clips inline output to its
bounds.

## Fixed memory cost

On the 64-bit host used by tests, `Utf8Buffer` is 24 bytes, `TextField` is 80
bytes, and `ImeComposition` is 32 bytes. Their corresponding 32-bit ARM layouts
are 12, 48, and 16 bytes. The caller-selected byte slice is the only value
storage; none of these types allocates, caches glyphs, or copies IME snapshots.

Coverage includes ASCII and kana insertion, multibyte deletion, invalid
boundaries, full and empty buffers, cursor edges, submit/cancel, repeat/release,
disabled and focus-loss behavior, measured horizontal scrolling, placeholder,
composition/candidate styling, and explicit composition damage.
