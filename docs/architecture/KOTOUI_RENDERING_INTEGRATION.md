# KotoUI rendering integration

KOTO-0214 places the concrete adapter in `koto-core`, the existing layer that
already depends on both foundations:

```text
koto-ui (component contracts) ─┐
                               ├─> koto-core::ui_render ─> simulator / Pico
koto-gfx (Canvas, font, Rect) ─┘
```

`koto-ui` remains dependency-free and does not know about KotoCore, KotoGFX,
font blobs, framebuffers, simulator windows, or device HALs. An integration
test reads both manifests and fails if this direction is reversed.

## Shared painter

`CanvasUiPainter` borrows a KotoGFX `Canvas` and `BitmapFont`. It maps RGB565
fills directly, builds borders and focus marks from clipped fills, measures
each bitmap glyph's actual advance, and rasterizes borrowed text/glyph runs
inside the intersection of the operation clip, bounds, and canvas surface.
Missing glyphs use the font's half-width advance, matching `Canvas::draw_text`.
Single-line runs reject newlines rather than introducing implicit layout.

The simulator supplies a full-frame Canvas. Firmware supplies the established
compact viewport Canvas for a dirty strip. Both call the same component paint
closure through `paint_ui_damage`; only storage/present handling outside the
Canvas differs. A parity test paints one component into both forms and compares
every RGB565 byte in the viewport.

`paint_ui_damage` invokes the caller's flat component paint tree once for each
declared damage rectangle and performs zero calls for an idle context. It does
not clear damage implicitly. Component tests remain the source of transition
geometry; integration tests verify that checkbox and idle transitions retain
their declared clips and that recording operations correspond to rendered
border, background, text, and focus pixels.

## Render requests and overflow

`ui_damage_commands` clips every `UiRect` to an RGB565 `RenderSurface` and emits
the existing `RenderCommand::rect` requests. No damage produces an empty list.
If the target command capacity cannot hold every visible rectangle, it emits
one `RenderCommand::Full`; a zero-capacity target reports `CommandCapacity`.
This is separate from `DamageSet` overflow, whose documented fallback has
already collapsed damage to its full surface before conversion.

## Release size record

Measurements used `cargo build --release` on 2026-07-15 and `rust-size -A` on
`target/thumbv6m-none-eabi/release/koto_firmware`.

| Metric | Before adapter | After adapter | Delta |
| :-- | --: | --: | --: |
| Firmware ELF file | 890,868 B | 890,868 B | 0 B |
| `.text` | 345,132 B | 345,132 B | 0 B |
| `.rodata` | 411,744 B | 411,744 B | 0 B |
| `.data` | 54,588 B | 54,588 B | 0 B |
| `.bss` | 175,032 B | 175,032 B | 0 B |
| Release `koto-core` rlib | 1,683,942 B | 1,731,458 B | +47,516 B |

The generic adapter is not yet used by the production firmware pilot, so the
linker removes it and the deployable image is unchanged. The rlib increase is
archive/metadata cost, not shipped code. Existing component state types are
unchanged. `CanvasUiPainter` is two borrowed pointers: 8 bytes on ARM (16 bytes
on the 64-bit host), transient on the paint stack, with zero owned framebuffer,
font, heap, or persistent SRAM. Consequently `.data + .bss` remains 229,620
bytes and the unexplained release/SRAM regression is zero.
