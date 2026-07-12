# KOTO-0078: IME Candidate List Navigation

- Status: in-progress
- Type: feature
- Priority: P1
- Requirements: FR-IME-1, FR-IME-4, FR-SIM-2

## Goal

Support multiple SKK candidates and make candidate navigation visible in the IME
popup.

## Acceptance Criteria

- [x] The SKK lookup path can expose more than the first candidate for a reading.
- [x] The IME state tracks current candidate index and candidate count.
- [x] The popup displays candidate position such as `1/5`.
- [~] Simulator/app input can move to next and previous candidates using
  documented keys. (Next implemented: re-running `ime_convert`/Tab cycles forward
  with wrap; a dedicated previous-candidate key is still pending.)
- [x] Commit inserts the selected candidate, not always the first candidate.
- [x] Tests cover readings with multiple candidates and candidate cycling.

## Notes

This may require extending `SkkSession`, `MemoImeLine`, and the bytecode IME line
serialization format. Keep ABI changes explicit and documented.

Progress: `MemoImeLine` now carries `candidate_index`/`candidate_count`, the
`ime_query_line` record appends those two bytes, `MemoIme::convert_with` cycles to
the next candidate on repeat (wrapping), and the memo app shows `n/m` and commits
the shown candidate. Remaining: a backward (previous-candidate) key, which needs a
window/app intent binding.
