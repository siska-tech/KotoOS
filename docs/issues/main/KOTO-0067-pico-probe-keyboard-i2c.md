# KOTO-0067: Pico Probe: Keyboard I2C

- Status: done
- Type: feature
- Priority: P0
- Requirements: HC-5, HC-8, FR-SDK-6, NFR-PERF-4, NFR-PERF-5, NFR-PERF-6

## Goal

Read PicoCalc keyboard data over I2C with frame-friendly latency, then capture
the evidence needed to choose a default game-button mapping.

## Acceptance Criteria

- [x] The probe polls the STM32 keyboard bridge at 100kHz or records why it
  cannot.
- [x] Raw key data is logged without blocking the frame budget.
- [x] The keyboard matrix validation procedure can emit the JSONL records
  described in `docs/KEYBOARD_MATRIX.md`.
- [x] The selected default mapping remains pending until evidence is recorded.

## Notes

This combines bring-up probes 2 and 3 because the matrix validation depends on
the basic I2C keyboard reader.

The `keyboard_i2c` probe follows the ClockworkPi STM32 bridge protocol by
selecting FIFO register `0x09` and reading `[state, key]` events. It maintains a
bounded held-key set, polls on a 16 ms frame cadence at 100 kHz, reports poll
duration, and emits JSONL samples for all three candidate mappings. Physical
matrix evidence and the default mapping decision remain pending.

Physical validation on 2026-06-20 confirmed 100 kHz operation when the
PicoCalc mainboard is separately powered over Type-C. A `6` key press produced
raw code 54 with measured poll times around 2.1-4.4 ms, below the 16.667 ms
frame budget. Each event is intentionally emitted once per candidate mapping;
unmapped text keys therefore produce identical records with an empty
`detected` array.

The `arrow-zxas` `up+A` chord was then observed as raw codes `[122, 181]`
(`z`, `ArrowUp`) and stabilized with `detected=["up","action_a"]`. Poll time
was approximately 2.1-4.4 ms. This proves the capture path, but is only one of
the required matrix chords.

An `up+left+A` burst was decoded correctly as
`detected=["up","left","action_a"]`, but an unbounded FIFO drain took 21.952 ms
because repeated HOLD events were consumed in the same frame. The probe now
limits FIFO processing to four events per frame and leaves the remainder queued
for subsequent frames.

The bounded follow-up captured the same three-key state in 4.477 ms, within the
16.667 ms frame budget. A subsequent five-sample stable observation completed
in 2.126 ms with all three normalized buttons present, so this individual chord
is passed. Releasing the chord returned to a stable empty state.

Paused on 2026-06-20 while the remaining matrix chords and default mapping
decision are still pending. The proven 100 kHz reader remains available for
resumption.

The dedicated `keyboard_matrix` firmware now guides the operator through all
44 required chord cases for each candidate. It records the STM32 firmware
version when available, prompts for the physical keys, requires five stable
samples, retries failures up to three times, checks the 16,667 us frame budget,
and emits JSONL result, candidate-summary, and final-selection records. Testing
stops at the first fully passing candidate, matching the preference order in
`docs/KEYBOARD_MATRIX.md`.

Final hardware validation on 2026-06-21 emitted:

`{"kind":"selection","status":"pass","selected_candidate":"arrow-zxas","reason":"first_passing_candidate"}`

Because the firmware emits this record only after every required case for the
candidate passes, this confirms all 44 `arrow-zxas` chord cases completed with
no terminal failure. The selected default game-button mapping is
`arrow-zxas`.

## Resolution

The STM32 keyboard bridge is validated at 100 kHz within the frame budget, the
bounded FIFO reader captures stable multi-key states, and the full matrix
procedure selected `arrow-zxas` as the first passing default mapping.
