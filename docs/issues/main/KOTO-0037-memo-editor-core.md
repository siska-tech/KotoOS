# KOTO-0037: Memo Editor Core

- Status: done
- Type: feature
- Priority: P0
- Requirements: FR-IME-3, FR-SHELL-3, FR-SDK-1, FR-SDK-2, NFR-DRAW-1, NFR-MEM-2

## Goal

Add a portable memo editor state model that can hold a bounded text document,
move a cursor, insert/delete characters, expose visible lines for rendering, and
mark only changed text/IME regions dirty.

## Acceptance Criteria

- [x] The editor model uses fixed or caller-provided bounded storage and has an
      explicit document capacity suitable for RP2040 SRAM budgets.
- [x] Cursor movement, insertion, deletion, newline handling, and scrolling are
      covered by unit tests.
- [x] The editor reports dirty rectangles or text-line invalidations instead of
      requiring full-screen repaint for every key.
- [x] The layout reserves the existing fixed IME line and never overlaps content,
      status, or input composition regions.
- [x] The model is independent of simulator-only crates and can live in
      `koto-core`.

## Notes

This can be implemented as native portable core logic even if the first packaged
memo fixture drives it through bytecode host calls. The important boundary is
that editing policy stays reusable between KotoSim and device builds.

Implemented in `koto-core::memo` with `MemoEditor`, fixed-capacity document
storage, cursor movement, scrolling, visible-line access, and dirty line/IME
rectangle reporting against `TextLayout`.
