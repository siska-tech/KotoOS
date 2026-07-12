# KotoSnake retained Game2D migration

KotoSnake was the last game still rendering steady gameplay entirely through the
immediate `draw_*` path: every frame it re-emitted the page background, the
playfield, the faint cell grid, the header/HUD bars and their fixed labels, and the
live score/length/best values — on top of the animated snake, apple, and particle
burst. This follows the KotoBlocks playbook (KOTO-0136 static layer, KOTO-0141
retained text) to move the *unchanging* chrome and the *id-keyed* HUD values onto the
host's retained Game2D layers, while leaving the deliberately over-the-top animated
entities on the immediate path.

## What moved where

| Element | Path | Why |
| :------ | :--- | :-- |
| Page background, header/HUD bars + accents, playfield fill, 19+15 grid lines, fixed labels (`スコア`/`ながさ`/`ベスト`/controls hint) | **Retained static layer** (`game2d_static_begin`/`_end`, built once on the title→play transition) | Identical every gameplay frame and never moves. Capturing it once drops ~43 immediate draw commands per frame and gives the device a layout-stable full-screen base. |
| Score, length, best values | **Retained text** (`game2d_text_set`, ids `T_SCORE`/`T_LEN`/`T_BEST`) | Change only on a bite. Diffed by stable id, so an unchanged value costs nothing and they never shift the (churning) immediate command list. |
| Flowing-rainbow snake body + head/eyes, breathing/pulsing apple + sparkle, 24-spark particle burst, eat-flash frame, "+10" / "SPEED UP!" / music banners, the game-over slam panel | **Immediate `draw_*`** | Per-frame animated (rainbow flow by `tick`, sub-cell smooth interpolation, breathing radius, particle physics) or transient overlays — exactly the cases the architecture keeps immediate. |

## Why the snake board is not a retained tilemap or sprite layer

The retained tilemap (KOTO-0135) and sprite stamps (KOTO-0140) are fixed **16×16**
cells anchored at KotoBlocks' **10×20 well at origin (8, 0)**. KotoSnake's field is
**18×14 at origin (16, 44)**; routing it through the tilemap would mean changing that
shared device/host geometry, which would alter KotoBlocks. More fundamentally the
snake body cannot be a fixed retained tile/sprite at all: each segment's colour cycles
every frame (`rainbow((i + tick/2) % 12)`) and every segment eases sub-cell toward its
next cell each frame (the "segments ease smoothly between cells" look), so a retained
cell — which only pays when it *changes* — would change every frame and save nothing
while losing the animation that is the point of the app. The apple breathes
(10→18 px) and its sparkle orbits, so it is not a fixed tile either. These stay
immediate by design.

## Effect on the device draw budget

The Pico keeps two retained immediate command lists (current + previous frame) for the
KOTO-0128 delta, capped at `MAX_APP_DRAW_COMMANDS = 96`. Before this change a single
steady frame emitted the ~43 chrome commands **plus** the snake/apple/particles, so a
long snake (len ≳ 20) overflowed the cap — dropping tail commands and forcing
full-screen repaints. Moving the chrome to the static layer (its own `APP_STATIC`
cell, *not* counted against the 96) roughly doubles the snake-length headroom and
keeps the per-frame immediate list well under the cap in normal play (the fixture
profile shows steady frames at ~22 host calls / ~18 `draw_rect`). The static layer
also supplies the full-screen base, so the delta presenter no longer re-rasters the
whole surface every frame; and pulling the HUD numbers into retained text removes
three commands that the positional command diff would otherwise have to keep aligned
against the snake/particle-driven list-length churn (KOTO-0143 `CommandCountShift`).

## Remaining bottleneck — now addressed (length-aware budget)

The flowing-rainbow snake body is the per-frame cost: with smooth interpolation every
visible segment moves each frame, so each segment's rects are genuine dirty rects. At
the time of this migration a very long snake (len ≳ 40) plus a particle burst still
overran the 96-command immediate cap (`used=96/96 ... full_reason=AreaExceeded`).

That is now fixed by a **length-aware render budget**: the head and the nearest
`RICH_N` body cells keep the full smooth/glinted/rainbow look, while the older tail is
coalesced into a bounded number of collinear *run* rects, so the snake's whole immediate
cost is a constant (≤ 46 rects) regardless of length and APP_DRAW can no longer overflow.
See [KOTO_KOTOSNAKE_LONG_SNAKE_BUDGET.md](KOTO_KOTOSNAKE_LONG_SNAKE_BUDGET.md).

## Behavioural notes

- **Screen shake** now jolts only the snake/apple/effects (`c, v` offsets), not the
  playfield/grid, since the field lives in the fixed static layer. The shake is a few
  frames after a bite/death; entities jittering over a steady field reads the same.
- **Returning to the title** (game over → F1) clears the retained text and empties the
  static layer, because the simulator composites the retained text/static-label layers
  *above* the title's immediate background fill — without the clear the old
  score/labels would show through. (On the device the title's immediate full-screen
  fill is composited last and already covers them.) Re-entering play rebuilds both.

## Verification

`cargo test -p koto-sim --test fixture_runner kotosnake_play_uses_retained_static_and_text_layers`
scripts the F1 start intent to cross into play and asserts the static layer is built
(`game2d_static_begin`/`_end`), the HUD values render through retained text
(`game2d_text_set` ≥ 3/frame), and the app drives **no** `game2d_set_tile` and **no**
`game2d_present` (it uses neither the retained tilemap nor sprites).
