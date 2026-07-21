# KotoUI panel and dialog composition

KOTO-0213 provides flat, allocation-free composition without introducing a
layout tree. `Panel` owns frame presentation and deterministic geometry;
`Dialog` owns only stable child/action IDs and the lifecycle of one existing
`FocusManager` modal scope. Applications continue to own labels, lists, text
fields, buttons, models, and buffers.

## Panel and geometry

`Panel` paints an optional opaque dimming backdrop, frame background/border,
borrowed title, title divider, and exposes the remaining content rectangle.
The backend still enforces the explicit clip, so a partly visible panel is safe
on a small surface. An empty frame, over-inset content, zero-height title, or
unrepresentable arithmetic returns `LayoutError` before painting or opening a
dialog.

`PanelLayout::inset`, `row`, and `button` compute absolute rectangles. Rows use
`y + index * (height + gap)` and must remain inside the content rectangle.
Buttons divide one row deterministically; remainder pixels go to earlier
buttons. The helpers allocate no memory, recurse nowhere, and perform no
constraint solving.

## Modal lifecycle and results

`Dialog<CHILDREN, ACTIONS>` stores a fixed array of child IDs and a fixed array
of `DialogAction`. Adding an action also adds its ID to the child list, and both
capacities are checked before either array changes. Duplicate IDs are rejected.
The default capacities are eight children and four actions.

The caller registers focusable children in the dialog's `FocusScopeId` before
opening it. On open, Dialog validates panel geometry, then selects the enabled
configured default action. If that action is disabled, it selects the first
enabled action in insertion order. With no enabled action it lets
`FocusManager` select the first focusable child. Applications must keep
`DialogAction::enabled` and the corresponding `FocusEntry::enabled` synchronized.

Activation closes only when the focused ID is an enabled action. Cancellation
and programmatic close also close the modal. `DialogResult` always identifies
the dialog; accepted results include the selected action, programmatic close
includes a focused action when present, and cancellation has no action. Closing
restores the focus saved by `FocusManager`.

Opening and closing damage the backdrop plus frame, allowing obscured pixels to
be restored. Dialog does not widen ordinary child damage, so selection, cursor,
or button-state updates retain their component-sized rectangles. Nested modal
scopes remain unsupported by the existing focus contract.

## Memo composition examples

These are mappings for later migration, not changes to Memo in this issue:

- Open: a titled Panel contains a `List` child for caller-owned filenames plus
  Open and Cancel actions. The list model remains in Memo; accepting Open
  returns its button ID and Memo reads the selected list index.
- Save confirmation: a label child describes the existing filename, with
  Overwrite and Cancel actions. Overwrite is the default only when saving is
  currently permitted; a disabled action cannot close the dialog.
- Save As: a `TextField` child borrows Memo's bounded filename buffer, with Save
  and Cancel actions. KotoIME continues to own conversion and the field only
  receives committed text and a borrowed composition snapshot.

For each composition, the application obtains `panel.layout(theme)?.content`,
uses `PanelLayout::row` for body controls, and splits the last row with
`PanelLayout::button`. It then constructs controls with those absolute bounds
and registers their IDs in the dialog scope.

## Memory cost

On the 64-bit test host, `Panel` is 64 bytes and the default
`Dialog<8, 4>` is 128 bytes. Their 32-bit ARM layouts are compile-time checked
at 48 and 100 bytes. A child slot is one `WidgetId`; an action slot is one
`DialogAction`. Borrowed titles, caller-owned controls, models, buffers, and
focus registry storage are not copied into the dialog.
