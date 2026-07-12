# KOTO-0040: Memo Simulator Validation

- Status: done
- Type: harness
- Priority: P0
- Requirements: FR-SIM-2, FR-SIM-3, FR-SIM-5, FR-IME-1, FR-FS-2, NFR-DEV-4

## Goal

Add deterministic simulator and harness coverage proving that the memo app can
be launched, edited through IME, saved, exited, relaunched, and reloaded without
escaping its sandbox or corrupting shell state. This is the end-to-end operation
confirmation gate for both the memo app and IME in KotoSim.

## Acceptance Criteria

- [x] A scripted KotoSim scenario launches `dev.koto.memo`, enters text, saves,
      exits, relaunches, and observes the saved content.
- [x] The scenario covers ASCII input, romaji/kana composition, Sticky Shift,
      and at least one SKK-style Japanese conversion path.
- [x] The validation output records the IME line state before commit and the
      memo document state after commit.
- [x] Save files are written only under the app sandbox namespace.
- [x] The project harness checks the memo fixture and reports clear failures for
      missing assets, invalid runtime, or failed scripted validation.
- [x] `python harness\check_all.py` includes the memo validation path or clearly
      documents when to run it separately.

## Notes

This is the "メモ帳アプリの動作確認と IME の動作確認まで" gate on the PC
simulator. Real PicoCalc confirmation should become a later device issue once
the embedded HAL can run the same package and input flow.
