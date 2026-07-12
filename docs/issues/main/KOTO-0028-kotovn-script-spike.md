# KOTO-0028: KotoVN Script and Image Pipeline Spike

- Status: done
- Type: research
- Priority: P2
- Requirements: FR-VN-1, FR-VN-2

## Goal

Sketch the first visual novel script and image asset pipeline.

## Acceptance Criteria

- [x] Define a tiny script fixture with background and text commands.
- [x] Define expected image format inputs for RLE/indexed-color assets.
- [x] Identify which pieces run in VM versus host engine code.

## Notes

This should remain a spike until KotoRuntime direction is clearer.

See [../KOTOVN_PIPELINE.md](../../spec/KOTOVN_PIPELINE.md) for the script fixture,
image input contract, and VM/host boundary notes.
