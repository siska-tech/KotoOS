# KOTO-0103: KotoBlocks game-feel effects pass

- Status: done
- Type: feature
- Priority: P2
- Requirements: FR-DOS-2

## Goal

Add lightweight, tile-based, audio-synced visual feedback so KotoBlocks reads as a
polished game, while preserving tile-based game logic and the bounded VM budgets
reclaimed in KOTO-0102 (user slots stay under 42 / 45).

## Acceptance Criteria

- [x] Line clears flash before the board collapses.
- [x] Four-line clears trigger a distinct effect: a longer flash, a "4 LINE!"
      banner, and a special fanfare SFX (`SFX_TETRIS`).
- [x] Active-piece ghost / hard-drop trail give drop feedback.
- [x] Active piece falls with smooth pixel interpolation while collision stays
      tile-based; move/rotate give a brief action-flash halo.
- [x] Game over has a board sweep transition (beyond the current dim + panel).
- [x] Level-up shows a brief banner.
- [x] Scripted KotoBlocks scenario still passes; budget diagnostics show frame fuel
      within the KotoBlocks profile and user slots stay under 45 (44 / 45).

## Status

Pre-existing (KOTO-0094): graduated line-clear flash (`flash = 6 + dx * 2`, longer
for more rows, white blink before collapse), hard-drop trail (`dropy` streak), ghost
piece outline, pause overlay.

This pass (the cheap, high-impact "four-line is special" set):

- **Tetris fanfare asset** added to the KotoBlocks package: a longer, higher rising
  fanfare than the normal clear cue, so a four-line clear sounds like an event. Wired through
  `audio_id`, the SDK `sdk_consts`, and `sfx_mml`, and covered by the audio bank
  parse test. Documented in `docs/KOTO_SDK.md`.
- **Four-line special** in `apps/koto_blocks`: a four-line lock plays its package fanfare
  and sets `flash = 24`. A three-line clear tops out at `flash = 12`, so any flash
  above 12 means "tetris" -- the flash count doubles as a **"4 LINE!" banner** timer
  with **no extra local**, keeping user slots at 41 / 45 (KOTO-0102 headroom intact).

Game-over board sweep and level-up banner (this pass):

- **Game-over board sweep**: on game over, a near-black overlay (`C_SWEEP`) descends
  the well one row per frame, sinking the board before the panel draws on top. The
  sweep reuses `flash` (provably 0 and otherwise unused at game-over entry) as its
  0..20 frame counter -- **no extra local**. The "4 LINE!" banner is gated to play
  state so the sweep's `flash` reuse never reveals it.
- **Level-up banner**: a level rise (detected by recomputing the level into the free
  `tr` scratch and comparing) shows a "LEVEL UP!" banner for 30 frames via a new
  `lvlup` local (the single user slot this pass spends: 41 -> 42 / 45, two free).

Smooth fall and action flash (this pass):

- **Smooth fall** (`vy`, 1 slot): the active piece's visual pixel-y eases half the
  remaining gap toward `py * 16` each frame -- smooth under gravity, self-limiting
  under a fast soft drop, never overshooting. Collision, ghost, board, and hard-drop
  trail all stay on the tile grid; only the active-piece blit uses `vy`. Reset to 0
  on spawn and hold-swap.
- **Move/rotate action flash** (`fxhi`, 1 slot): a successful move or rotate sets a
  3-frame timer; while it runs, a bright (`C_PANEL`) outline is drawn one pixel
  *outside* each piece cell. The rect-then-blit compositor hides interior edges
  (covered by neighbour tiles), leaving a clean white silhouette halo.

Score popup and lock flash (added after KOTO-0104 freed budget):

- **Lock flash** (0 slots): a lock with no line clear sets the existing action-flash
  timer (`fxhi`), so the just-landed piece gets a one-frame white silhouette halo;
  the smooth-fall `vy` is snapped to the landed row at lock so the halo sits on the
  piece (a hard drop jumps `py` while `vy` is still easing). Pairs with `SFX_LOCK`
  and the hard-drop trail for a "slam" feel.
- **Score popup** (`pop`, 1 slot): a line clear stores the points it will award
  (mirroring the collapse scoring), and render shows "+N" over the well during the
  flash (leading zeros trimmed). No separate timer -- the flash window is the
  popup's lifetime.

These cost one user slot (`pop`); KOTO-0104 call-site inline slot reuse had brought
koto-blocks to 42 / 45, so this lands at 43 / 45 with two slots still free.

UI polish (same pass): the HOLD box switched to a horizontal layout (label left,
piece centered on the right) so a two-cell-tall spawn shape clears the 50px frame
instead of overflowing the bottom border; the title screen gained a dark showcase
band of all seven tetrominoes, a pulsing "F1 スタート" button (reusing the now-idle
bake cursor `steps` as the blink timer — no new local), and a controls hint.

## Verification

- **Game-over sweep**: screenshot-verified end to end -- a scripted scenario tops out
  and the captured frame shows the dark overlay descended over the well with the
  game-over panel readable on top and a few un-swept board cells still visible.
- **"4 LINE!" and "LEVEL UP!" banners**: draw verified by screenshot via a temporary
  forced render (a four-line clear and a level rise each need many cleared lines,
  impractical to script blindly). Their triggers are small additions to the existing,
  working line-clear/scoring code.
- The package-local fanfare parses through strict KotoMML validation.
- **Smooth fall + action flash**: screenshot-verified -- a scripted soft drop shows
  the piece eased between grid lines (smooth fall), and a move/rotate shows the white
  silhouette halo around the piece. Collision stays tile-based (ghost/board unchanged).
- **Lock flash**: screenshot-verified end to end -- a scripted hard drop to the empty
  floor shows the landed piece with its white halo and the hard-drop trail.
- **Score popup**: draw verified by forced render ("+1200" over the well, leading
  zeros trimmed); the trigger stores the points at lock from the existing scoring.
- Budget scenario passes; user slots 43 / 45, frame fuel within the KotoBlocks
  profile. (Smooth fall intentionally moves the active piece sub-tile, so the normal
  frame is no longer byte-identical to the pre-effects baseline -- verified by
  screenshot instead.)

A deterministic scripted four-line clear / 10-line level rise is impractical to author
blindly; regression tests asserting the banner text and fanfare event are a
worthwhile follow-up if the RNG/piece sequence is pinned.
