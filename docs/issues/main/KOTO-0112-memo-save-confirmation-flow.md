# KOTO-0112: Memo save confirmation flow

- Status: done
- Type: UX
- Priority: P2
- Requirements: FR-TEXT-?, FR-FS-?

## Goal

Keep ordinary saves lightweight while making overwrite versus Save As explicit.

## Acceptance Criteria

- [x] F2 on a named document asks `上書き保存しますか? (y/n)` in the command/status bar.
- [x] `y` overwrites the current sandbox file.
- [x] `n` opens the filename prompt for Save As.
- [x] F2 on an unnamed new document opens the filename prompt directly.
- [x] F3 wrap appears in the normal command hints and ON/OFF remains visible.
- [x] Regression tests cover overwrite, Save As, and unnamed-document save.
- [x] `python harness/check_all.py` passes.

## Notes

The confirmation uses the command bar's upper line (`y = 286`) without replacing
the editor view. Save As requires a non-empty filename. F5 creates `(新規)`
immediately; naming is deferred until F2.

