# KotoUI Foundation Contract

KOTO-0208 introduces `koto-ui` as a dependency-free, `no_std`, allocation-free
crate. It owns component-neutral contracts only. KotoCore/KotoGFX adapters,
input routing, controls, and application models remain outside this foundation.

## Ownership and coordinates

- `WidgetId` is assigned by the caller and is stable only while that caller
  keeps it registered.
- `UiRect` uses signed absolute surface coordinates and positive signed sizes.
  Empty or negative sizes contain no pixels. Rectangle math uses widened `i64`
  edges before returning a representable `i32` rectangle.
- `Painter` receives an explicit clip on every operation. It borrows text or
  glyph runs only for the duration of the call and owns no surface or font.
- `UiResponse` reports semantic outcomes; the UI layer does not retain callbacks
  or application references.

## Damage policy

`DamageSet<N>` clips every request to its surface, removes contained regions,
and merges intersecting or edge-touching regions. The default capacity is eight
independent rectangles. On capacity overflow, it yields the complete surface as
one conservative region. A zero-capacity set is valid and immediately uses the
same full-surface fallback.

Changing component bounds damages their representable union. If an extreme
coordinate range cannot be represented as one `UiRect`, each old/new rectangle
is clipped and retained separately. Identical old/new bounds produce no damage.

## Memory measurements

The fixed-capacity data layout is explicit with `repr(C)` where measurements are
part of this contract.

| Type | x86_64 host | thumbv6m-none-eabi | Notes |
| :--- | ----------: | -----------------: | :---- |
| `WidgetId` | 2 B | 2 B | Transparent `u16` |
| `UiRect` | 16 B | 16 B | Four `i32` fields |
| `Theme` | 32 B | 32 B | Four state styles plus compact tokens |
| `DamageSet<8>` | 160 B | 152 B | Surface, eight rectangles, `usize`, fallback flag/padding |
| `UiContext<8>` | 192 B | 184 B | Theme plus damage set |

Host sizes are asserted in unit tests. The embedded values follow the same
`repr(C)` fields with a 32-bit `usize` and are guarded by the required embedded
cross-build. Component-specific state is measured in its implementing issue.
