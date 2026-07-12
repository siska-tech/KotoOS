# KOTO-0023: Software PCM Mixer Core

- Status: done
- Type: feature
- Priority: P1
- Requirements: FR-MML-1, FR-MML-2, FR-MML-3, NFR-REL-3

## Goal

Create the first platform-independent PCM mixer core for BGM and sound effects.

## Acceptance Criteria

- [x] Mixer can sum at least two simple sample streams.
- [x] Output clamps safely to `i16`.
- [x] Tests cover silence, single stream, and mixed stream output.

## Notes

MML parsing can wait; start with raw generated wave/sample sources.
