# KOTO-0073: IME Toggle And Status Bar Baseline

- Status: done
- Type: feature
- Priority: P0
- Requirements: FR-IME-1, FR-IME-3

## Goal

Add a minimal IME ON/OFF path and always-visible memo status bar so simulator
users can tell whether typed letters will be inserted as ASCII or composed as
romaji/kana.

## Acceptance Criteria

- [x] The runtime exposes named `IME_TOGGLE` and `INTENT_IME_TOGGLE` constants.
- [x] KotoSim maps a key to the IME toggle intent.
- [x] The memo app starts in ASCII mode and can toggle IME mode.
- [x] The memo app draws a status bar showing `IME:ON` or `IME:OFF`.
- [x] IME-off mode inserts typed ASCII letters directly into the editor.
- [x] `docs/RUNTIME_BYTECODE_ABI.md` and `docs/KOTO_SDK.md` document the new
  constants.
- [x] `python harness\check_all.py` passes.

## Notes

This is intentionally a baseline, not the final text-entry UX. Candidate
selection, richer composition display, and editor polish remain in KOTO-0071 and
KOTO-0072.
