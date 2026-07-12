# SKK Dictionary Index Strategy

This document defines how KotoIME looks up SKK conversion candidates from a
dictionary that lives on the SD card, and the SRAM-resident index that makes
those lookups affordable on RP2040-class hardware. It defines the FR-IME-4
lookup strategy and is shaped by HC-6 (SD is SPI-only with slow random access).

The implementation lives in [`koto_core::skk`](../../src/koto-core/src/skk.rs).
Unit tests run against the tiny fixture
[`harness/fixtures/skk_min.skk`](../../harness/fixtures/skk_min.skk); the shipped
dictionary is the original, CC0-licensed
[`sdcard_mock/dict/skk_koto.skk`](../../sdcard_mock/dict/skk_koto.skk)
(KOTO-0089, ~71KB), which both the sim and the firmware open as
`dict/skk_koto.skk` (8.3 name `SKK_KOTO.SKK`).

## Why an Index

A real SKK dictionary (`SKK-JISYO.L` and friends) is several megabytes of sorted
text. Two facts make naive access unworkable on the target:

- The RP2040 has 264KB of SRAM (HC-1), so the dictionary cannot be resident.
- The SD card is SPI-only; random seeks are expensive (HC-6).

A pure file-backed binary search would issue `O(log n)` random seeks per
keystroke-driven lookup, each paying SD latency. Instead KotoIME keeps a small
index in SRAM that maps each reading's **leading character** to the byte offset
where that character's run begins. A lookup then costs at most **one** random
seek followed by a short sequential scan, which is exactly the access pattern SD
storage favors.

## Dictionary File Assumptions

The prototype targets a deliberately narrow subset of the SKK format. These are
the assumptions a packaging tool must enforce when it ships a dictionary onto the
SD card.

| Assumption | Prototype rule | Rationale |
| :--------- | :------------- | :-------- |
| Encoding | UTF-8 | Traditional SKK dictionaries are EUC-JP; converting to UTF-8 at packaging time keeps the `no_std` core free of a transcoding table. |
| Sections | Okuri-nasi (送り仮名なし) only | Okuri-ari handling and its descending sort are deferred (see Future Work). |
| Sort order | Ascending by reading, byte-lexicographic | UTF-8 byte order equals Unicode scalar order, so `str` comparison is a valid sort key with no collation table. |
| Line format | `reading<space>/cand1/cand2;annotation/.../` | Reading up to the first ASCII space; candidate list is `/`-delimited and starts with `/`. |
| Comments / blanks | Lines starting with `;`, and empty lines, are ignored | Matches SKK's `;;` section headers. |
| Annotations | Text after `;` inside a candidate field is metadata | Stripped by [`DictEntry::candidates`]; preserved in [`DictEntry::raw_candidates`]. |
| Line endings | `\n` or `\r\n` | A single trailing `\r` is stripped per line. |

Malformed lines (non-UTF-8, missing the reading/candidate separator, or a
candidate body that does not start with `/`) are skipped rather than rejected, so
a single bad line cannot break index construction.

A fragment of the fixture:

```text
;; okuri-nasi entries.
あい /愛/相/藍/
かい /回/会/貝/
かさ /傘/笠/
かんじ /漢字/感じ/幹事/
きょう /今日;today/京/強/
```

## Index Table Structure

The index is one entry per **distinct leading character** of a reading. Because
the dictionary is sorted, equal leading characters are contiguous, so each entry
marks the start of a contiguous bucket.

```rust
struct IndexEntry {
    key: [u8; 4], // UTF-8 bytes of one leading scalar value (行頭文字)
    key_len: u8,  // 1..=4
    offset: u32,  // byte offset of the bucket's first line in the dictionary
}

struct SkkIndex<const N: usize> {
    entries: [IndexEntry; N], // sorted ascending by key
    count: usize,
    dict_len: u32,
}
```

The index is fixed-capacity and allocation-free. `N` bounds the number of
distinct leading characters; the default alias `SkkLeadingIndex` uses
`DEFAULT_INDEX_CAPACITY = 192`, which comfortably covers hiragana, katakana, and
ASCII reading initials. `SkkIndex::build` returns `SkkError::IndexFull` if a
dictionary exceeds the capacity, and `SkkError::DictionaryTooLarge` if it will
not fit the 32-bit offset space (4GB, far beyond any SD-resident dictionary).

### Memory Budget

`IndexEntry` is currently 12 bytes on the host build (4 key + 1 length +
padding + 4 offset); use `core::mem::size_of::<IndexEntry>()` as the source of
truth if the layout changes. The leading character of a Japanese reading is
usually hiragana for the okuri-nasi dictionary subset, so the practical live
index size is bounded by the hiragana inventory:

| Leading-character set | Distinct entries | SRAM (≈12 B/entry) |
| :-------------------- | :--------------- | :----------------- |
| Fixture (`skk_min.skk`) | 6 | ~72 B |
| Shipped (`skk_koto.skk`) | 64 | ~768 B |
| Practical hiragana initials | ~80 | ~1.0 KB |
| Default capacity (`N = 192`) | reserved | ~2.3 KB static |

This matches the research target of a "few hundred bytes to ~1KB" live resident
table ([docs/Research.md](../planning/Research.md), section 4) while leaving the
multi-megabyte dictionary body on the SD card. `SkkIndex::sram_bytes` reports
the live entry footprint; the fixed-capacity array reserves the `N = 192` static
capacity shown above.

The leading-character granularity trades table size against scan length: a
popular initial such as か owns a large bucket, so a lookup may scan many lines.
The Future Work section covers finer indexing for production dictionaries.

## Lookup Algorithm

`SkkIndex::lookup(dict, reading)` borrows the dictionary slice the index was built
from and returns a borrowed `DictEntry`:

1. Take the leading character of `reading` (return `None` if empty).
2. **Binary-search** the index `entries` for that leading character. A miss means
   no reading starts with it, so return `None`.
3. The matched entry gives the bucket's start offset; the next entry's offset (or
   end-of-file for the last bucket) gives the end. In a hardware reader, this
   maps to the single random **seek** before scanning.
4. **Sequentially scan** lines from the start offset:
   - reading `<` target: keep scanning.
   - reading `==` target: return the entry.
   - reading `>` target: the dictionary is sorted, so the target is absent —
     return `None`.

On hardware, step 3 is the only backward-capable access; step 4 is
forward-only streaming, satisfying the HC-6 / KPA sequential-read constraints.

## Windowed SD Reader (KOTO-0089)

Both access modes share the index, parsing, and candidate iterator:

- **Slice mode** (`SkkIndex::build` / `lookup`, `SliceDict`): the dictionary is
  a borrowed `&[u8]`. Used by unit tests and available to any host that keeps
  the file in RAM.
- **Windowed mode** (`SkkIndex::build_from_reader` / `lookup_in_reader`,
  `WindowedDict` over a `SkkRead` source): only a scan window of
  `SKK_LOOKUP_WINDOW_BYTES` (512B) is resident. The index is built at app
  startup with one sequential streaming pass; each conversion re-opens the
  file, seeks once to the bucket, and scans forward window by window. On a hit
  the matched line is re-read into the window and candidates borrow it.

The window size is a **packaging contract**: every dictionary line (newline
included) must fit the window, or the windowed path skips the line like a
malformed one. `skk_koto.skk`'s longest line is 168 bytes; the contract is
guarded by the `koto_dict_lines_fit_the_lookup_window` test.

The firmware (`DeviceHost` in
[`app_host.rs`](../../src/koto-pico/src/firmware/app_host.rs)) uses windowed mode
exclusively — the old 4KiB SRAM dictionary buffer is gone, replaced by the
512-byte window. The sim keeps the file in host RAM but converts through
`WindowedDict` anyway, so every sim IME test exercises the hardware code path.
`memo_ime::MemoIme::convert_with_access` accepts either mode through the
`SkkDictAccess` trait.

## Status

- [`koto_core::skk`](../../src/koto-core/src/skk.rs) implements the index, both
  access modes, line parsing, and the annotation-aware candidate iterator.
- Unit tests cover bucket construction, in-bucket scans, annotation stripping,
  first/last buckets, misses, index overflow, streaming-vs-slice build
  equality, windowed-vs-slice lookup equality (including tiny windows that
  force chunk-boundary re-reads and overlong-line skips), and a full
  round-trip of every `skk_koto.skk` reading through the windowed path.

## Future Work

- **Okuri-ari section**: parse the descending-sorted okuri-ari block and the
  trailing okuri letter, needed for verb/adjective conversion. (Until then,
  `skk_koto.skk` ships common verb/adjective surface forms as okuri-nasi
  entries.)
- **Finer index granularity**: optionally key on the first *two* characters, or
  sample every _k_-th line, to bound bucket scan length on large dictionaries.
  This grows the table but keeps the same algorithm.
- **Precomputed sidecar index**: have the packaging tool emit the index as a
  small `.skkidx` file so boot loads it with one sequential read instead of
  scanning the whole dictionary. `build_from_reader` is the reference
  implementation for that tool.
- **Conversion integration**: feed candidates into the SKK conversion flow
  triggered by Sticky Shift ([KOTO-0016](../issues/main/KOTO-0016-ime-sticky-shift.md))
  and surface them on the dedicated IME line (FR-IME-3).
