# KOTO-0036: Runtime Text And File Host Calls

- Status: done
- Type: feature
- Priority: P0
- Requirements: FR-SDK-1, FR-SDK-4, FR-FS-2, FR-RT-4, NFR-REL-1

## Goal

Extend the bytecode host-call boundary with the text and sandboxed file services
needed by a memo app: `draw_text`, `file_open`, `file_read`, `file_write`, and
`file_close`. Calls must validate app heap buffers and resolve paths through the
app sandbox identity before touching the HAL filesystem.

## Acceptance Criteria

- [x] `draw_text` reads UTF-8 from app memory, rejects invalid ranges or invalid
      text, and records bounded render work without pushing pixels directly.
- [x] File host calls use the implicit app context and sandbox resolver so app
      bytecode cannot escape its save-data namespace.
- [x] Open file handles are capped, invalid handles fail deterministically, and
      all handles are released on app exit.
- [x] Host-call failures use the ABI error convention from
      `docs/RUNTIME_BYTECODE_ABI.md`.
- [x] Tests cover successful text/file calls plus invalid pointer, missing file,
      permission, and path-escape failures.

## Notes

Depends on KOTO-0034 and should be usable by KOTO-0035. Keep storage operations
host-testable through the existing `HostFs` adapter before adding device FAT
integration.

Implemented by extending `VmHost` with `draw_text`, `file_open`, `file_read`,
`file_write`, and `file_close`. `BytecodeVm` validates heap ranges and UTF-8
before dispatching. KotoSim maps app paths into `data/<app_id>/...`, caps open
handles at eight, and releases them on VM exit.
