# KOTO-0013: Bitmap Font Glyph Model

- Status: done
- Type: feature
- Priority: P0
- Requirements: FR-FONT-1, FR-FONT-2, FR-FONT-3

## Goal

Define the core representation for bitmap font metadata and glyph bitmaps, and
provide a real font asset so the simulator can draw text.

## Acceptance Criteria

- [x] Core can describe fixed-size bitmap glyphs.
- [x] Glyph lookup can report missing glyphs without panicking.
- [x] Tests cover ASCII and Japanese (kana + kanji) codepoints.

## Notes

Adopted font: **M+ BITMAP FONTS** (free, redistributable; see
`assets/fonts/LICENSE_E` / `LICENSE_J`). FR-FONT-1 lists 美咲/k8x12 as examples
("等"); M+ is the same lightweight bitmap class.

Pipeline: `harness/mplus_to_kfont.py` converts the M+ BDF files offline into a
compact fixed-cell `.kfont` blob (header + sorted codepoint index + 1bpp
bitmaps), isolating BDF parsing and the JIS X 0208 → Unicode mapping (decoded via
EUC-JP). The runtime reader [`koto_core::font::BitmapFont`](../../../src/koto-core/src/font.rs)
is `no_std`, allocation-free, and binary-searches the index. Both 10-dot and
12-dot sets are vendored (`assets/fonts/mplus10.kfont`, `mplus12.kfont`), full JIS
X 0208 (~7000 glyphs, half-width Latin + full-width kanji).

### FR-FONT-2 compliance

FR-FONT-2 asks for a JIS level 1+2 font, 16×16, ~300KB, resident in PSRAM.
Status by axis:

- **JIS level 1+2 coverage — met.** Each `.kfont` holds the full JIS X 0208
  (7059 glyphs: ASCII + kana + level 1/2 kanji).
- **~300KB size budget — within budget.** Each blob is 251,657 bytes (~245.8 KiB:
  16B header + ~68.9 KiB index at 10B/glyph + ~176.8 KiB bitmaps), comfortably
  under 300KB. Note this is because the cell is smaller, not a tighter encoding:
  the same format at 16×16 would be ~296KB, i.e. the requirement's ~300KB figure
  essentially *is* a 16×16 full-JIS bitmap. **Caveat:** keeping both 10- and
  12-dot blobs resident at once is ~492 KiB and exceeds the budget — assume only
  one size is resident on-device.
- **16×16 dimensions — intentional deviation.** Adopted M+ sizes are 10-dot and
  12-dot (cell height 13) to suit the 16px shell rows. The 16×16 figure should be
  revisited (or relaxed) given M+ was chosen.
- **PSRAM residency — not yet done.** Only host file loading exists today; the
  on-device PSRAM-resident path and per-glyph PSRAM→SRAM readout (FR-FONT-3,
  depends on KOTO-0022) are future work. The 10B/glyph index (~69 KiB) is a
  candidate for compaction if SRAM/PSRAM gets tight.
