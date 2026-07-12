# KOTO-0022: PSRAM Block API Prototype

- Status: done
- Type: feature
- Priority: P1
- Requirements: FR-RT-5, NFR-MEM-4, NFR-MEM-5

## Goal

Prototype the core-facing PSRAM block API without exposing dereferenceable memory.

## Acceptance Criteria

- [x] Trait usage demonstrates read/write into SRAM buffers.
- [x] Tests use an in-memory mock PSRAM backend.
- [x] API rejects out-of-range transfers.

## Notes

Device-specific PIO implementation belongs later in the embedded backend.
