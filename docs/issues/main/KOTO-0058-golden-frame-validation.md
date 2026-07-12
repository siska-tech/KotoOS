# KOTO-0058: Golden Frame Validation

- Status: done
- Type: harness
- Priority: P1
- Requirements: FR-SIM-1, FR-SIM-5, NFR-DEV-3, NFR-DEV-4

## Goal

Add golden frame or structured frame validation for key simulator screens so UI
and rendering regressions are caught before hardware testing.

## Acceptance Criteria

- [x] The harness can validate at least one shell frame and one app frame using
      either a golden image, stable render-command trace, or structured text/rect
      assertions.
- [x] The validation mode is deterministic across local runs.
- [x] Expected output files are small enough to review and update intentionally.
- [x] Documentation explains when to update golden outputs and how to inspect
      failures.

## Notes

Exact pixel comparison is optional for the first pass. A stable render-command
trace may be a better fit until font and framebuffer details settle.

Implemented as `cargo run -q -p koto-sim -- --golden-frames` plus
`harness/check_golden_frames.py`, comparing against
`harness/fixtures/golden_frames/sim.trace`.
