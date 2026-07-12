# KOTO-0109: Romaji/kana punctuation and long-vowel input coverage

- Status: done
- Type: bug
- Priority: P1
- Requirements: FR-IME-?

## Goal

Fill missing Japanese punctuation and long-vowel input cases in the memo IME.

## Acceptance Criteria

- [x] Long vowel mark `ー` can be entered.
- [x] Common Japanese punctuation can be entered: `、` `。` `？` `！` `・` where supported by the input scheme.
- [x] Full-width symbol mappings are documented.
- [x] Existing romaji/kana mappings remain unchanged.
- [x] Invalid/incomplete sequence tests remain meaningful.
- [x] Regression tests cover the added symbols.
- [x] `python harness/check_all.py` passes.

## Notes

IME-on symbol input maps `-` to the katakana long-vowel mark `ー` and `/` to
the Japanese middle dot `・`. Existing `,`/`.` mappings produce `、`/`。`, and
`?`/`!` continue through the full-width ASCII mapping as `？`/`！`.
Inside an active Shift/SKK reading, `-` appends `ー` to the reading itself
(`konpyu-ta` -> `こんぴゅーた`) so `q` can commit `コンピュータ`.
`docs/KOTO_SDK.md` documents the complete behavior. The existing romaji table
was not changed, and the full IME test suite (including invalid/incomplete
sequence coverage) passes.
