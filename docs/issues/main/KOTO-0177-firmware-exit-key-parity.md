# KOTO-0177: firmware exit keys diverge from KotoSim (X/Esc exit; F10 should be the only exit)

- Status: DONE 2026-07-11 — device-confirmed. X types `x` / X-and-Esc cancel
  in KotoShogi / Esc does not exit / **Shift+F5 (F10 = `0x90`) exits** /
  Home/End/Del legends work in memo. The F10 hunt took three slices: the 0x8a
  assumption was falsified on device, the Shift+F5 chord guess was also
  wrong, and the `phase=180 key` capture then measured the truth — the bridge
  shift-translates the whole shifted plane itself and Shift+F5 arrives as
  `0x90`. The audit also surfaced and fixed a HOME/END/DELETE parity gap.
  Mapping lives host-tested in `koto-core/src/keymap.rs`; the `phase=180 key`
  diag stays as permanent equipment.
- Type: bug
- Priority: P1 (every app loses the X key, and any Esc/CANCEL use exits the app)
- Related: KOTO-0042 (input/IME host calls), KOTO-0025 (keyboard matrix — the
  0x8a F10 scan code this fix must trust).

## Symptom

On hardware, pressing **X** or **Escape** exits the running app. KotoSim maps
only **F10** to `text_intent::EXIT` (`src/koto-sim/src/window.rs`, the
`Key::F10` block) — Esc there is CANCEL and X is a typed character / game
button. Firmware and sim disagree, so apps behave differently per target.

## Root cause (already located)

`app_input_snapshot` in `src/koto-pico/src/firmware/app_runtime.rs`:

```rust
// F10 follows the validated 0x81-based function-key block. X/Escape
// are also accepted so the first hardware slice always has an exit.
0x8a | b'x' | 0xb1 => text_intent::EXIT,
```

This was a deliberate bring-up shim ("so the first hardware slice always has
an exit") that was never removed. Two knock-on effects in the same function:

1. `0xb1` (Esc) never reaches apps as CANCEL-equivalent input.
2. The typed-character path explicitly suppresses `'x'`
   (`... && event.key != b'x'`), so apps cannot receive a typed `x` at all —
   text entry and any game reading codepoints loses the key.

Note `held_key_bits` mapping X/Esc to game button bit 5 is **correct** (it
matches the sim's game-pad mapping) and must be preserved.

## Fix

- `0x8a => text_intent::EXIT` only.
- Drop the `event.key != b'x'` exclusion from the typed-codepoint admission.
- Decide what Esc (0xb1) should deliver on firmware (sim: LeftCtrl → CANCEL;
  the PicoCalc Esc key is the natural CANCEL carrier) — align with sim intent
  semantics rather than dropping it silently.

## Implementation (2026-07-11)

The scan-code → intent/codepoint mapping moved out of `app_input_snapshot`
into **`koto-core/src/keymap.rs`** (`intent_for_key` /
`typed_codepoint_for_key`) so the contract is host-testable — `koto-pico` is
excluded from the workspace default members and cannot run host tests (same
reasoning as DiagProfile living in koto-gfx). `app_input_snapshot` now calls
the shared functions; `held_key_bits` (X/Esc → game button bit 5) is
unchanged.

Decisions:

- **Esc (0xb1) → CANCEL**, joining Ctrl (0xa5). The sim's Esc closes the
  simulator window itself so LeftCtrl carries CANCEL there; on the PicoCalc
  keyboard Esc is the natural CANCEL carrier. Esc keeps doubling as game
  button B via `held_key_bits`, exactly like Enter (NEWLINE intent +
  button A).
- Parity tests in `koto_core::keymap::tests`:
  `exit_is_delivered_only_by_f10` sweeps all 256 codes and pins EXIT to 0x8a
  alone (mirroring the sim's single `Key::F10 → EXIT`), plus Esc-is-CANCEL,
  x-is-a-typed-character, and typed-admission coverage. `poll_app_input` in
  `koto-sim/src/window.rs` carries a pointer comment to the contract.

## Device findings (2026-07-11, first flash)

- ✅ X types `x`; ✅ X/Esc cancel works in KotoShogi; ✅ Esc no longer exits.
- ❌ **Shift+F5 did not exit** — the PicoCalc keyboard has no dedicated F10
  key (F10 is the shifted legend on the F5 keycap) and the bridge does not
  synthesize `0x8a` for it. KOTO-0025 never validated any code above F5; the
  0x81-block F10 assumption was wrong on real hardware — exactly why the shim
  existed.

Second slice, same day:

- `intent_for_key(key, shift_held)`: Shift+F5 (`0x85` with a shift key held)
  → EXIT chord, on the guess that the bridge reports the F5 code with shift
  as a separate modifier. **Wrong** — see the second-flash capture below.
- Permanent **`phase=180 key`** UART diag: every press/release bridge event
  while an app runs logs `state= key= shift=` (HOLD repeats skipped, so it is
  human-rate).

## Device findings (2026-07-11, second flash)

The chord did not fire. The `phase=180` capture for one Shift+F5 press:

```
phase=180 key frame=916 state=1 key=0xa2 shift=0   ; Shift pressed
phase=180 key frame=932 state=3 key=0x90 shift=1   ; orphan RELEASE (bridge quirk)
phase=180 key frame=936 state=1 key=0x90 shift=1   ; F10 PRESSED
phase=180 key frame=950 state=3 key=0x90 shift=1   ; F10 released
phase=180 key frame=956 state=3 key=0xa2 shift=1   ; Shift released
```

- **The bridge shift-translates the function row itself: Shift+F5 arrives as
  `0x90`** — neither the extrapolated `0x8a` nor `0x85`+shift ever occurs.
  The `0x81`-block F1–F5 codes are real, but the block does not extend
  linearly to the shifted legends.
- Bridge quirk noted for the record: a stray RELEASED `0x90` precedes the
  PRESSED event. Harmless — intents fire on PRESSED only — but a
  press-detection scheme keyed on release order would misread it.

Third slice: `intent_for_key` back to a single argument; **EXIT ⇔
{`0x90` (measured F10), `0x8a` (unobserved protocol alias, kept)}**; plain F5
stays NEW; the chord is gone. `exit_is_delivered_only_by_f10` pins exactly
that pair.

## Shifted-plane audit (2026-07-11, user-prompted)

The keyboard's other shift legends arrive the same way — as dedicated bridge
codes, not modifier+base combinations:

| Keys        | Legend | Bridge code | Intent                              |
| :---------- | :----- | :---------- | :---------------------------------- |
| Shift+F1..F4 | F6–F9 | `0x86..=0x89` (protocol) | none — the sim assigns F6–F9 nothing either; "no intent" is the parity |
| Shift+F5    | F10    | **`0x90` (measured)** | EXIT                    |
| Shift+Esc   | Break  | `0xd0` (protocol) | none — no such intent exists |
| Shift+Tab   | Home   | `0xd2` (protocol) | **HOME** (new)          |
| Shift+Del   | End    | `0xd5` (protocol) | **END** (new)           |
| Del         | Del    | `0xd4` (protocol) | **DELETE** (new)        |

HOME/END/DELETE were a real parity gap this issue's audit surfaced: the sim
delivers all three from PC Home/End/Delete keys and memo implements the
cursor moves, but the firmware dropped the codes on the floor. The `0xd?`
values are ClockworkPi keyboard-firmware protocol constants,
device-unverified (the `0x90` measurement validated the numbering scheme);
`phase=180 key` verifies them for free during the final exit retest.
`shifted_legends_match_sim_intents` pins the whole table.

## Acceptance Criteria

- [x] On hardware, X types `x` in memo and acts as game button B in games; it
      does not exit the app. — device-confirmed 2026-07-11 (typing + KotoShogi
      cancel).
- [x] Esc does not exit the app. — device-confirmed 2026-07-11.
- [x] F10 exits every app (verify the 0x8a scan code on the device before
      removing the shim — that is the reason the shim existed). — 0x8a
      falsified, `phase=180` measured Shift+F5 = `0x90`; exit
      device-confirmed 2026-07-11, along with the Home/End/Del legends and
      plain F5 = NEW.
- [x] KotoSim behavior unchanged (parity test or scenario documenting the
      shared mapping). — sim untouched; `koto_core::keymap::tests` documents
      and enforces the shared mapping (4 tests green).
