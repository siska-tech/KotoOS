# KOTO-0019: Runtime Host API Boundary

- Status: done
- Type: feature
- Priority: P0
- Requirements: FR-RT-4, FR-SDK-1, FR-SDK-2, FR-SDK-3, FR-SDK-4

## Goal

Define how VM-hosted apps call KotoSDK services without accessing OS internals directly.

## Acceptance Criteria

- [x] Host calls cover draw, input, audio, file, and exit.
- [x] Invalid host calls return errors rather than panicking.
- [x] Sandbox identity is available during file calls.

## Notes

KOTO-0018 chose a custom stack VM first. The VM-neutral host-call ABI is recorded
in [RUNTIME_BYTECODE_ABI.md](../../spec/RUNTIME_BYTECODE_ABI.md).
