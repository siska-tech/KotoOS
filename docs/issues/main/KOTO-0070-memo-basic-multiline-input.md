# KOTO-0070: Memo Basic Multiline Input

- Status: done
- Type: bug
- Priority: P0
- Requirements: FR-IME-1, FR-IME-3

## Goal

Fix the most obvious simulator memo regressions so the memo app can accept
space/newline input, move vertically, and render newline-separated text.

## Acceptance Criteria

- [x] Space and newline characters can be inserted through the memo IME path.
- [x] The memo app handles newline, up/down, home, and end edit intents.
- [x] Text drawing displays newline-separated lines instead of one horizontal
  run.
- [x] The committed memo bytecode is regenerated from source.
- [x] `python harness\check_all.py` passes.

## Notes

This is only a baseline usability fix. It does not make the IME or memo editor
feel polished.
