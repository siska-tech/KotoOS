# KotoUI GUI Component Roadmap

- Status: complete (2026-07-15)

## Purpose

KotoUI is a small, keyboard-first GUI component layer for KotoShell, Memo, and
other PDA-style applications. It sits above the rendering primitives without
turning KotoOS into a general-purpose desktop GUI toolkit.

This roadmap turns Phase 7 of
[`kotogfx-architecture.md`](../architecture/kotogfx-architecture.md) into
implementation-sized work. It traces primarily to FR-SHELL-2, FR-SHELL-3,
FR-SDK-1, FR-SDK-2, FR-SDK-5, NFR-PERF-1, NFR-DRAW-1, NFR-MEM-2, and
NFR-PORT-1.

## Scope and constraints

- Provide label, button, checkbox, list, single-line text field, panel, and
  modal dialog components.
- Use a flat, caller-owned component collection with absolute rectangles.
  Small alignment/inset helpers are allowed; Flexbox, Grid, constraint solving,
  and recursive heap-owned trees are not.
- Keep component state and traversal bounded and allocation-free so the core
  crate remains `no_std` compatible.
- Treat keyboard and gamepad navigation as the primary interaction model.
  Pointer/touch input is outside the first implementation.
- Produce explicit dirty rectangles when visual state changes. Idle frames must
  not repaint unchanged components.
- Keep rendering behind a painter contract. KotoUI must not own the LCD,
  framebuffer, KotoGFX compositor, font cache, or input HAL.
- Keep application data outside widgets where practical. A list reads a model;
  a text field edits a caller-provided bounded buffer.
- Preserve the existing KotoIME ownership boundary. A text field consumes
  committed text and composition snapshots; it does not implement kana/SKK
  conversion itself.

## Public model

The initial API should converge on these concepts:

- `WidgetId`: stable caller-assigned identity.
- `UiRect`: component bounds in surface coordinates.
- `UiEvent`: normalized navigation, activation, cancellation, text, and editing
  events.
- `UiResponse`: observable activation/value/submission changes returned to the
  application.
- `FocusManager`: bounded focus order and modal focus scope.
- `Theme`: colors, borders, spacing, and focus/disabled state tokens.
- `Painter`: clipped rectangles, borders, glyph/text runs, and focus marks.
- `DamageSet`: bounded dirty rectangles with a defined full-region fallback.

Concrete names may change during implementation, but the ownership and bounded
memory rules above are acceptance constraints.

## Milestones and dependency order

| Milestone | Issues | Outcome |
| :-------- | :----- | :------ |
| M1 Core | KOTO-0208, KOTO-0209 | Allocation-free UI model, damage tracking, and keyboard focus/event routing |
| M2 Components | KOTO-0210, KOTO-0211, KOTO-0212, KOTO-0213 | Basic display, selection, editing, and modal composition components |
| M3 Integration | KOTO-0214 | KotoGFX/KotoCore painter integration with simulator/device parity |
| M4 Validation | KOTO-0215 | Interactive component gallery and deterministic regression coverage |
| M5 Adoption | KOTO-0216 | A bounded KotoShell surface uses KotoUI in production |

KOTO-0210 can begin after KOTO-0208. KOTO-0209 can proceed in parallel with
KOTO-0210. KOTO-0211 and KOTO-0212 require both. KOTO-0213 requires the basic
components and focus scopes. Integration and gallery work follow the component
contracts; Shell adoption is the final compatibility gate.

## Validation strategy

- Unit tests use a recording painter and synthetic key/text events.
- Every state transition checks both `UiResponse` and the exact damaged bounds.
- Boundary tests cover empty models, maximum capacities, UTF-8 cursor movement,
  clipping, disabled controls, focus loss, and damage-list overflow.
- A simulator gallery supplies keyboard-driven scenarios and golden-frame
  coverage for default, focused, pressed, disabled, editing, and modal states.
- The integration milestone records component-state SRAM cost and checks
  `no_std`/embedded builds before adoption.
- The Shell pilot compares behavior and repaint scope before and after migration
  and retains existing simulator tests.

## Explicit non-goals

- CSS, DOM, Flexbox, Grid, automatic content measurement, or runtime theme files.
- Arbitrarily deep widget trees or heap allocation in the embedded core.
- Pointer gestures, drag-and-drop, animation framework, rich text, multiline
  document editing, virtual keyboard, or accessibility tree in the first pass.
- Replacing KotoGFX retained layers, KotoFont, KotoIME, or the existing Memo
  editor core.
- Exposing every component as a new VM host call. An app-facing declarative ABI
  is planned separately after native validation in the
  [KotoUI App ABI roadmap](KOTOUI_APP_ABI_ROADMAP.md).

## Completion gate

The roadmap is complete when KOTO-0208 through KOTO-0216 are `done`, all
component states are covered by deterministic tests, an interactive simulator
gallery is available, and one production KotoShell surface uses the components
without increasing idle repaint work or violating the embedded memory budget.
