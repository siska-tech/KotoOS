# KOTO-0015: Romaji-to-Kana Input Core

- Status: done
- Type: feature
- Priority: P0
- Requirements: FR-IME-1

## Goal

Implement the first romaji-to-kana composition state machine.

## Acceptance Criteria

- [x] Core converts common syllables such as `ka`, `shi`, and `n`.
- [x] Incomplete composition remains buffered.
- [x] Tests cover normal input and cancellation/reset.

## Notes

Implemented in `koto_core::ime` as an allocation-free `RomajiKanaInput` state
machine. The initial table covers vowels, basic unvoiced kana rows, representative
youon such as `sha`/`cha`/`nya`, `n` boundary commit, and small `っ` before
repeated consonants. Dakuten, handakuten, punctuation, editing commands, and SKK
conversion remain future incremental work.
