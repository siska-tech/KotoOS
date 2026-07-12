# KOTO-0088: Memo Light Theme And Colored Text

- Status: done
- Type: feature
- Priority: P1
- Requirements: FR-IME-3, FR-SIM-1, FR-SIM-2, FR-SDK-4

## Goal

Bring Koto Memo close to the PicoCalc reference design: a light document
("白背景に黒文字") with a navy title bar carrying a save-state badge, a white
document area with a scrollbar, a pale-blue IME conversion panel showing the
reading and candidate, and a Japanese command bar.

The previous shell could not express this because bytecode `draw_text` carried no
colour — the host painted all app text in one near-white colour, which only suits
a dark theme. This issue adds per-call text colour to the runtime ABI and uses it
to repaint the memo shell.

## Acceptance Criteria

- [x] A `draw_text_color(x, y, buf, len, rgb565)` host call exists end to end:
  `koto-core` ABI/verifier/VM, `kbc-asm`, the `koto-compiler` prelude, and the
  KotoSim host + window renderer.
- [x] The colourless `draw_text` still works (default trait impl + a sentinel that
  cannot collide with a real colour such as white).
- [x] The memo app renders a light theme: navy title/command bars with light text,
  a white document area with near-black body text, and a pale-blue IME panel.
- [x] The title bar shows a save badge: green `保存済` when clean, amber `未保存`
  when dirty; dirty state is tracked across edits and cleared on save.
- [x] The IME panel shows `入力中`, the input mode, the reading (underlined), and
  the candidate in a boxed area while composition is active, and disappears on
  cancel/commit.
- [x] The command bar uses Japanese labels and switches between editing and
  composing actions; the cursor `Ln N Col M` status remains.
- [x] Tests cover the new chrome and the scrollbar geometry; the local gate
  (build sync, memo validation, golden frames) passes.

## Notes

The candidate area keeps the single-candidate model; a candidate count/page
indicator (`1/5` in the reference) belongs with candidate-list navigation
([KOTO-0078](KOTO-0078-ime-candidate-list-navigation.md)). `開く` is shown dimmed
because the open dialog is still tracked by
[KOTO-0080](KOTO-0080-memo-open-save-dialog-baseline.md).

App colours arrive as sign-extended `i16` (the VM `push_i16`), so the host
recovers them with `as u16` at paint time; this revises the visual baselines from
[KOTO-0074](KOTO-0074-memo-visual-shell.md) and
[KOTO-0077](KOTO-0077-ime-candidate-popup-ux.md).
