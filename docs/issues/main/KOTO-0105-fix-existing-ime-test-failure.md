# KOTO-0105: Fix existing IME test failure

- Status: done
- Type: bug
- Priority: P0
- Requirements: FR-IME-1

## Goal

Restore the full local check suite by reconciling the stale
`ime::tests::reports_incomplete_and_invalid_sequences` expectation with the
current romaji-to-kana table.

## Acceptance Criteria

- [x] `cargo test` reproduces and isolates the IME failure.
- [x] The expected behavior of incomplete and invalid IME sequences is clarified.
- [x] The failing test is fixed without weakening coverage.
- [x] Full `python harness/check_all.py` passes.

## Notes

The composer was correct. The test expected `k` followed by `w` to return
`InvalidSequence`, but the current table supports the foreign-syllable forms
`kwa`, `kwi`, `kwe`, and `kwo`. Therefore `kw` is a valid incomplete prefix and
must remain buffered.

The regression test now proves both sides of the contract:

- `kw` remains pending, `finish()` reports `IncompleteComposition`, and `kwa`
  commits `くぁ`.
- `kq` remains a genuinely invalid sequence, returns `InvalidSequence`, and
  preserves the prior pending `k`.

KOTO-0104's ActorArray/compiler/runtime changes were unrelated; their targeted
compiler, runtime, build, golden-frame, budget, and project checks were already
green before this cleanup.
