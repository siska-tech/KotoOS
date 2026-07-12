# KOTO-0017: SKK Dictionary Index Strategy

- Status: done
- Type: research
- Priority: P1
- Requirements: FR-IME-4, HC-6

## Goal

Design the SD-card-friendly dictionary lookup strategy and its SRAM-resident index.

## Acceptance Criteria

- [x] Document dictionary file assumptions.
- [x] Define the index table structure and memory budget.
- [x] Prototype lookup against a tiny fixture dictionary.

## Notes

The first prototype should avoid needing a full SKK dictionary.

Designed in [SKK_DICTIONARY.md](../../spec/SKK_DICTIONARY.md): UTF-8 okuri-nasi dictionary
intended for SD-card storage, sorted ascending by reading, with an SRAM-resident
index of one `(leading character -> byte offset)` entry per distinct reading
initial (行頭文字オフセット). The designed lookup binary-searches that index, then
does one seek plus a forward-only sequential scan, matching HC-6.

Prototyped as the allocation-free `koto_core::skk` module
([skk.rs](../../../src/koto-core/src/skk.rs)) with seven tests over the tiny fixture
[`harness/fixtures/skk_min.skk`](../../../harness/fixtures/skk_min.skk). The index is
~1KB live for a practical hiragana initial set; okuri-ari support, finer-grained
indexing, a precomputed `.skkidx` sidecar, conversion integration, and a windowed
SD reader are recorded as future work in the design doc.
