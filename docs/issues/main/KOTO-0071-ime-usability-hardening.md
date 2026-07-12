# KOTO-0071: IME Usability Hardening

- Status: done
- Type: feature
- Priority: P0
- Requirements: FR-IME-1, FR-IME-2, FR-IME-3, FR-IME-4

## Goal

Turn the current IME prototype into a predictable simulator input system with
clear behavior for ASCII, romaji composition, SKK conversion, cancellation, and
commit.

## Acceptance Criteria

- [x] A behavior table documents key actions for ASCII input, romaji input,
  Sticky Shift, conversion, commit, cancel, backspace, and newline.
- [x] Tests cover invalid romaji recovery without trapping or losing unrelated
  text.
- [x] Conversion state has visible, deterministic feedback for reading,
  candidate, missing candidate, and pending romaji.
- [x] The simulator window uses discoverable or documented keys for conversion,
  commit, cancel, save, and exit.
- [x] Scripted scenarios cover common failure-prone typing flows, including
  mixed ASCII/Japanese text.

## Notes

This hardening pass keeps the IME small, but makes the simulator behavior
predictable enough for scripted validation and hands-on memo-app testing.
