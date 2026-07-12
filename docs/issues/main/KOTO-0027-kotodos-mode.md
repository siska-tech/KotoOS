# KOTO-0027: KotoDOS 320x200 Mode Model

- Status: done
- Type: feature
- Priority: P2
- Requirements: FR-DOS-1, FR-DOS-2, NFR-DRAW-3

## Goal

Model the 320x200 game region and static 320x120 UI region used by DOS-style apps.

## Acceptance Criteria

- [x] Core constants describe the KotoDOS regions.
- [x] Render commands can target only the game region.
- [x] Tests verify region bounds within 320x320.

## Notes

Implementation can wait until base rendering is stable.
