# KOTO-0212: KotoUI single-line text field and IME composition

- Status: done
- Type: feature
- Priority: P1
- Requirements: FR-SDK-2, FR-IME-1, FR-IME-3, NFR-MEM-2, NFR-PORT-1
- Related: KOTO-0014, KOTO-0015, KOTO-0038, KOTO-0208, KOTO-0209
- Roadmap: [KotoUI GUI Component Roadmap](../../planning/KOTOUI_ROADMAP.md)

## Goal

Implement a bounded single-line text field that edits caller-owned UTF-8 data,
displays KotoIME composition, and keeps cursor and horizontal viewport behavior
correct without taking ownership of the IME.

## Acceptance Criteria

- [x] Define a caller-provided bounded UTF-8 buffer contract with explicit
  capacity/full behavior and no heap allocation.
- [x] Insert committed text and handle left/right, home/end, backspace, delete,
  submit, and cancel only at valid UTF-8 boundaries.
- [x] Render value, placeholder, cursor, focused/disabled state, and a borrowed
  IME composition/candidate snapshot with visibly distinct composition styling.
- [x] KotoIME conversion, dictionary access, and sticky-shift logic remain
  outside the component; the field consumes their existing output/intents.
- [x] Horizontal scrolling keeps the cursor visible and uses font measurements
  supplied through an interface rather than assuming ASCII cell width.
- [x] Editing returns value-changed, submitted, cancelled, or capacity-rejected
  responses with documented semantics.
- [x] Edits and composition changes damage the field bounds; an unchanged cursor
  and unchanged input produce no damage.
- [x] Tests cover ASCII, kana, multibyte deletion, full buffers, empty strings,
  cursor edges, long-value scrolling, composition commit/cancel, disabled state,
  and focus loss.
- [x] State/buffer memory costs and the relationship to the reserved IME line are
  documented.
- [x] Workspace tests and `python harness/check_project.py` pass.

## Notes

Multiline document editing, selection ranges, clipboard, undo, password masking,
and IME implementation are separate concerns.

`python harness/check_all.py` passes after synchronizing the committed KPA
packages on 2026-07-15.
