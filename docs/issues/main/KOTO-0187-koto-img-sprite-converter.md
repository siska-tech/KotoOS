# KOTO-0187: koto-img — PNG ↔ .kspr sprite converter

- Status: done (2026-07-13) — `tools/koto-img` landed (workspace + gate
  member); PNG round-trip and `build_apps.py` byte-parity pinned by crate
  tests; KotoRogue sheet proved the loop (see Acceptance Criteria).
- Type: feature
- Priority: P2
- Related: KOTO-0184 (audio/gfx dev tooling — this is its top-ranked
  candidate), KOTO-0054 (asset pipeline), KotoIDE roadmap Phase 1a
  (`docs/planning/KOTOIDE_ROADMAP.md`).

## Goal

Sprite art is authored as ASCII `.kspr` (one character per pixel) with no
import path from standard pixel-art tools and no preview short of launching an
app with `--image`. Provide a Rust CLI `koto-img` (under `tools/`) with:

- **PNG → `.kspr`**: slice a PNG into 16×16 tiles (vertical strip order),
  build the `color <char> <RRGGBB>` palette from the exact colors present, and
  emit reviewable `.kspr` text. Art gets authored in Aseprite or any paint
  tool; the committed source stays diffable text.
- **`.kspr` / `.kim` → PNG**: render either the text source or the compiled
  `KIM1` strip to PNG for previews, reviews, and README shots.

## Design constraints

- **Exact colors, no quantization surprises.** Pixel art sources have few,
  exact colors. Reject PNGs whose distinct color count exceeds the `.kspr`
  palette-character budget rather than silently quantizing; report the count.
- **Round-trip stability.** PNG → `.kspr` → (build) → `.kim` → PNG must
  reproduce the input pixels exactly (RGB565 conversion is the one lossy step
  and must be applied consistently in both directions — document it).
- **Stable text output.** Emitted `.kspr` must be deterministic (palette
  order, tile order, comments) so regeneration produces no spurious diffs.
- The compiled format and `build_apps.py` `images` block are unchanged; this
  tool only adds paths *into* and *out of* the existing formats.

## Acceptance Criteria

- [x] `koto-img` converts PNG → `.kspr` and `.kspr`/`.kim` → PNG with a
      round-trip test pinning pixel equality (modulo documented RGB565
      truncation).
      → `kotorogue_png_round_trip_reproduces_identical_kim` goes through the
      real PNG codec; `expand565_is_idempotent_over_all_pixels` pins the
      truncate/expand algebra for all 65,536 RGB565 values.
- [x] One shipped app's sprite sheet re-exported and re-imported as the
      proving case, byte-identical `.kim` after `build_apps.py`.
      → KotoRogue `tiles.kspr` (19 tiles): `kspr2png` → `png2kspr` →
      `build_apps.py` left the committed `kotorogue_tiles.kim` untouched
      (verified via `git status`); also pinned in-tree by the round-trip test.
- [x] `docs/guides/ASSET_PIPELINE.md` documents the tool in the asset map.
      → two asset-map rows plus a "PNG import/export (`koto-img`)" section.
- [x] Harness coverage (`check_project.py` or a converter self-test) so a
      regression fails the local gate.
      → `koto-img` is a workspace default member, so its 7 tests (including
      `compile_matches_build_apps_output_for_kotorogue`, which recompiles the
      shipped sheet against the committed `.kim`) run under `check_all.py`'s
      fmt/clippy/test gates.
