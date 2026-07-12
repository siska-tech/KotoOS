# KOTO-0074: Memo Visual Shell

- Status: done
- Type: feature
- Priority: P0
- Requirements: FR-IME-3, FR-SIM-1, FR-SIM-2

## Goal

Make Koto Memo look like a complete PDA-style app rather than raw text painted
onto a black framebuffer. The target baseline is a framed memo view with a title
bar, filename, save state, white document area, and bottom command bar.

## Acceptance Criteria

- [x] The memo app draws a top title bar with an app label, `memo.txt`, and a
  visible save state.
- [x] The document area has a distinct light background separated from chrome.
- [x] The bottom command bar lists the primary keys/actions available in the
  current state.
- [x] The app shows current line and column in the command/status area.
- [x] A scripted or golden-frame check covers the expected chrome elements.

## Notes

This issue is visual structure only. It should not introduce candidate lists,
dialogs, or new editor behavior beyond the state needed to display the chrome.

Completed: the memo app now draws a title bar, filename, save state, framed
document area, bottom command/status bar, and `Ln N Col M` status text. The
existing bytecode-session UI test now asserts the chrome draw/text calls.
