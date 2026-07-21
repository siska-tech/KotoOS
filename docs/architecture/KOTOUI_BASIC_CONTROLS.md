# KotoUI Basic Control Contract

KOTO-0210 adds the first controls on the KOTO-0208 painter and KOTO-0209 event
contracts. They remain `no_std`, allocation-free, and independent of fonts and
render backends.

## Components

- `Label` is non-focusable and forwards start, center, or end alignment to the
  painter. Its assigned rectangle is the deterministic clipping boundary.
- `Button` paints normal, focused, pressed, and disabled states and emits one
  `Activated` response for an accepted `Pressed` pulse. Repeat pulses and a
  duplicate press before release do not reactivate it.
- `Checkbox` uses the same interaction states, adds an explicit inner shape when
  checked, and emits `ValueChanged(0|1)` once per accepted press.

Button and Checkbox clear pressed state on `Released`, focus loss, or disable.
Focus is also shown through `draw_focus_mark`, so it is not communicated only by
color. Checked state changes shape as well as color.

## Ownership and painting

Visible and optional semantic labels are borrowed `&str` values. There is no
control-side byte or character limit and no string copy: the caller owns the
text for the control lifetime, and the painter clips it to the assigned bounds.
Controls retain no font, painter, framebuffer, callback, or application model.

Paint order is stable:

1. control background;
2. control border;
3. text and checkbox shape;
4. non-color-only focus mark, when focused.

No Painter call is made when control bounds and the supplied clip do not
intersect. Empty or narrow content regions are omitted rather than forwarded as
invalid geometry.

## Damage

Bounds changes damage the representable old/new union. Label, value, enabled,
and pressed changes damage the control bounds. Focus changes damage only that
control's focus/border bounds. Setters with an unchanged value and ignored or
repeated activation pulses add no damage.

## Memory measurements

The structures use `repr(C)` because their bounded state size is part of the
embedded contract.

| Type | x86_64 host | thumbv6m-none-eabi | Variable text storage |
| :--- | ----------: | -----------------: | :-------------------- |
| `Label<'_>` | 48 B | 32 B | borrowed only |
| `Button<'_>` | 64 B | 40 B | visible + optional semantic `&str` |
| `Checkbox<'_>` | 64 B | 40 B | visible + optional semantic `&str` |

Host sizes are asserted in unit tests; embedded layout is checked by the ARM
cross-build. Icons, images, radio groups, toggle switches, and animation remain
outside these controls.
