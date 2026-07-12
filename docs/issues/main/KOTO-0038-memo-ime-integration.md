# KOTO-0038: Memo IME Integration

- Status: done
- Type: feature
- Priority: P0
- Requirements: FR-IME-1, FR-IME-2, FR-IME-3, FR-IME-4, NFR-PERF-4

## Goal

Connect KotoIME composition to the memo editor so romaji/kana input, Sticky
Shift, SKK-style conversion triggers, candidate selection, and committed text
all flow into the editor while the fixed IME line displays current composition.
This issue is the feature-level IME behavior gate for the memo app scope.

## Acceptance Criteria

- [x] Raw key input can update romaji/kana composition and commit kana into the
      memo editor.
- [x] Sticky Shift affects exactly one subsequent stroke and can trigger
      SKK-style conversion without requiring simultaneous thumb chords.
- [x] SKK candidate lookup uses the existing index strategy and fails gracefully
      when the dictionary or candidate is missing.
- [x] The IME line exposes deterministic render state for composition,
      candidates, and empty/unsupported states.
- [x] Scripted IME tests cover romaji-to-kana composition without launching the
      full memo app.
- [x] Scripted memo-editor tests enter and convert at least one short Japanese
      phrase, then commit it into the document buffer.

## Notes

Depends on KOTO-0037 and the existing KOTO-0015, KOTO-0016, and KOTO-0017
building blocks. Device keyboard-matrix validation remains separate; this issue
uses the normalized input model.

Implemented in `koto-core::memo_ime` with `KotoMemoIme`, `MemoImeKey`, and a
deterministic `MemoImeLine` render state. Sticky Shift starts conversion mode,
romaji commits kana into either the memo buffer or SKK reading buffer, and
`SkkIndex` lookup can promote a reading candidate before commit.
