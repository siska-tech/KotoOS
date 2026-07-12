# KOTO-0089: Larger SKK Dictionary For Evaluation

- Status: in-progress
- Type: research
- Priority: P2
- Requirements: FR-IME-1, FR-IME-4

## Goal

The committed IME fixture `harness/fixtures/skk_min.skk` holds only a handful of
readings, which is enough for tests but too small to evaluate real Japanese input
(coverage, candidate ordering, conversion ergonomics). Evaluate adopting a
dictionary around the size of `SKK-JISYO.S` so the memo IME can be exercised with
realistic text.

## Acceptance Criteria

- [x] Decide on a dictionary source/size (e.g. `SKK-JISYO.S`) and confirm its
  license is compatible with bundling or documenting how to fetch it.
  → `SKK-JISYO.S` is GPL; instead an **original** dictionary was compiled from
  scratch (no SKK-JISYO file was consulted), dedicated to the public domain
  (CC0 1.0), so bundling is unencumbered.
- [x] Confirm `SkkLeadingIndex` / `SkkIndex<N>` capacity and the runtime index
  build cost scale to that entry count (or adjust the const generics / build).
  → 64 distinct leading characters, well inside `DEFAULT_INDEX_CAPACITY = 192`;
  guarded by `skk::tests::koto_dict_is_sorted_and_every_reading_is_reachable`.
- [x] Decide where the larger dictionary lives (committed asset vs. fetched into
  `sdcard_mock/dict/` by a tool) and how the host loads it.
  → Committed asset `sdcard_mock/dict/skk_koto.skk` (~71KB, 8.3-safe name).
  Sim and firmware now open `dict/skk_koto.skk` directly; the stale
  `sdcard_mock/dict/skk_min.skk` copy was removed (it was byte-identical to
  the harness fixture, which stays).
- [x] Keep the tiny `skk_min.skk` fixture for deterministic unit tests.

## Progress (2026-07-10)

`sdcard_mock/dict/skk_koto.skk` landed: 2,771 readings / 4,982 candidate words,
okuri-nasi only, UTF-8, ascending byte-lexicographic sort per
`docs/SKK_DICTIONARY.md`. Written from scratch by topic enumeration (time,
people, body, food, nature, places, work, tech, suru-verb nouns, adjectives,
single-kanji on-yomi fans, …) specifically so no SKK-JISYO GPL provenance can
attach; header records the provenance and the CC0 dedication. Because the
engine only does okuri-nasi lookups, common verb/adjective surface forms
(dictionary form, masu-stem, -sa nouns) are included as okuri-nasi entries.

Two regression tests in `koto_core::skk` build the real index over the shipped
file (sortedness, capacity, per-reading lookup round-trip, everyday readings).

## Remaining

- [x] Dictionary selection: sim `SKK_DICT_PATH` and the firmware's name match
  both moved to `dict/skk_koto.skk` (8.3: `SKK_KOTO.SKK`). Old SD cards that
  only carry `skk_min.skk` must be refreshed.
- [x] Windowed SD reader (2026-07-10): `koto_core::skk` gained `SkkRead`,
  `SkkIndex::build_from_reader` (streaming one-pass index build) and
  `SkkIndex::lookup_in_reader` (bucket seek + window-sized forward scans);
  `MemoIme::convert_with_access` runs conversion over either a resident slice
  or a `WindowedDict`. The firmware dropped its 4KiB SRAM dictionary buffer
  for a 512B window (`SKK_LOOKUP_WINDOW_BYTES`, a packaging contract on max
  line length — longest shipped line is 168B) and re-opens the file per
  conversion (one seek + sequential reads, HC-6-friendly, user-paced). The
  sim converts through the same windowed path for hardware parity.
  Hardware smoke test still pending (code is target-built + clippy-clean;
  behavior verified via host tests and the sim's end-to-end memo tests).
- [ ] Coverage/ordering evaluation with real memo typing on the sim.
- [ ] Device smoke test: boot with the new SD layout, confirm `phase=159
  skk-loaded dict_len=71082 buckets=64` and end-to-end memo conversion.

## Notes

Raised during memo IME evaluation: candidate cycling, `q`-to-katakana, and
full-width symbol input now work ([KOTO-0078](KOTO-0078-ime-candidate-list-navigation.md)),
but only against a minimal dictionary. A realistic dictionary is the next thing
needed to judge input quality. Watch repository size and the embedded-target
memory budget when choosing one.
