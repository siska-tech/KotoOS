# KotoUI List Contract

KOTO-0211 adds a fixed-state, keyboard-first list viewport over caller-owned
data. The control copies no collection and allocates no row storage.

## Model and ownership

`ListModel::len` reports the current item count and `ListModel::row` borrows a
`ListRow` for one index. A row contains borrowed visible text, an optional
borrowed semantic label, and an enabled flag. Missing rows are treated as
invalid/disabled and are never selected or painted.

The application calls `sync_model` after structural or enabled-state changes.
The list caches only the last length, selected index, and viewport origin. Text
or other presentation changes with unchanged structure are reported through
`invalidate_row`; the model remains authoritative.

## Selection and viewport

- Initial synchronization selects the first enabled item.
- Up/down selects the next enabled item without wrapping. Page-up/page-down
  moves by the number of fully visible rows, then resolves disabled gaps in the
  requested direction.
- Home/end select the first/last enabled item.
- Pressed activation emits `SelectionActivated(index)`; repeat and release do
  not activate.
- Selection changes emit `SelectionChanged(index)`. When a model shrinks, the
  old index is clamped and the nearest enabled item is selected, preferring the
  forward direction. Empty and all-disabled models have no selection.
- Selection is kept visible. A movement inside the viewport damages only the
  old/new row rectangles; a viewport shift damages the list bounds.

Rows use a fixed positive height (non-positive constructor/setter input becomes
one pixel). Variable-height rows, wrapping, nested lists, spatial grids, and
drag scrolling remain outside this contract.

## Painting

The list paints through the KOTO-0208 `Painter` contract. Disabled rows use the
disabled style; selected rows use the selected/focused style; keyboard focus
adds the non-color-only focus mark. When the model exceeds the viewport, a
three-pixel track and proportional thumb are painted inside the right edge. The
scrollbar is not a registered or focusable child.

All row and scrollbar operations receive the list/paint clip intersection.
Empty or too-narrow text regions are omitted.

## Capacity and memory

Item indices and counts use `usize`; the practical maximum is therefore the
caller model's addressable item count. List state does not grow with that count.

| Type | x86_64 host | thumbv6m-none-eabi | Variable row storage |
| :--- | ----------: | -----------------: | :------------------- |
| `List` | 56 B | 40 B | none |
| `ListRow<'_>` | 40 B | 20 B | borrowed text only |

The host `List` size is asserted in unit tests and the 32-bit layout is guarded
by the ARM cross-build.
