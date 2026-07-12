# Keyboard Matrix Validation Plan

This plan defines the PicoCalc hardware check that chooses the default KotoSDK
game-button mapping. It extends bring-up probe 3 from
[RP2040_BRINGUP.md](RP2040_BRINGUP.md) and satisfies the FR-SDK-6 /
NFR-PERF-6 / HC-8 requirement set.

Status: hardware validated. The final default mapping is `arrow-zxas`.

## Goals

- Verify that direction plus A/B/X/Y chords do not ghost or block on the STM32
  keyboard matrix exposed over I2C address `0x1F`.
- Record raw held keys and normalized detected buttons in a format that can be
  compared across firmware revisions and bus speeds.
- Pick a default mapping only after every required two-key and three-key game
  chord passes on hardware.

## Candidate Mappings

The direction source is tested separately from the action cluster because the
SDK exposes a gamepad-like `Buttons` model.

| Candidate | Direction keys | A | B | X | Y | Rationale |
| :-------- | :------------- | :- | :- | :- | :- | :-------- |
| `arrow-zxas` | Arrow cluster | `Z` | `X` | `A` | `S` | Natural for two-thumb handheld play if the arrow cluster is reliable. |
| `wasd-jkui` | `W`/`A`/`S`/`D` | `J` | `K` | `U` | `I` | PC-style fallback with both hands separated on the QWERTY matrix. |
| `ijkl-zxas` | `I`/`J`/`K`/`L` | `Z` | `X` | `A` | `S` | Alternates the direction side if the arrow cluster shares blocking paths with actions. |

`confirm`, `cancel`, and text input mappings are not decided by this probe.
They may reuse the selected A/B keys later, but KotoIME text composition must
remain separable from game-button polling.

## Required Chords

For each candidate, test at 100kHz and 400kHz I2C if the firmware accepts both
rates. If 400kHz fails, record the failure and continue at the highest stable
rate.

Required direction chords:

| Group | Chords |
| :---- | :----- |
| Cardinal | `up`, `down`, `left`, `right` |
| Diagonal | `up+left`, `up+right`, `down+left`, `down+right` |

Required action chords:

| Group | Chords |
| :---- | :----- |
| Single action | `A`, `B`, `X`, `Y` |
| Common pairs | `A+B`, `A+X`, `A+Y`, `B+X`, `B+Y`, `X+Y` |

Required game chords:

| Group | Chords |
| :---- | :----- |
| Direction + action | Every cardinal direction plus each action button. |
| Diagonal + action | Every diagonal direction plus `A`, then every diagonal direction plus `B`. |
| Menu-safe | `up+down`, `left+right`, and any physically impossible opposite direction pair must not create action buttons. |

## Probe Procedure

1. Boot the keyboard probe with USB-CDC logging enabled.
2. Print the probe banner with firmware label, board label, bus rate, debounce
   window, candidate name, and build git revision.
3. For each candidate mapping, hold the requested keys until the log reports at
   least five consecutive stable samples.
4. Release all keys and wait for an all-clear sample between chords.
5. Repeat any failed chord three times to separate matrix blocking from operator
   timing.
6. Mark a candidate as passing only if every required chord has no missing
   expected button and no unexpected button.

## Logging Format

The permanent capture format is newline-delimited JSON so logs can be appended
directly from USB-CDC and parsed without keeping a full file in memory.

```json
{"kind":"keyboard_matrix_v1","board":"picocalc-rp2040","firmware":"unknown","bus_hz":100000,"candidate":"arrow-zxas","git":"unknown"}
{"kind":"sample","t_ms":1234,"seq":42,"candidate":"arrow-zxas","chord":"up+A","held_keys":["ArrowUp","Z"],"raw_codes":[82,29],"detected":["up","action_a"],"unexpected":[],"missing":[],"stable":true}
{"kind":"result","candidate":"arrow-zxas","chord":"up+A","bus_hz":100000,"attempt":1,"status":"pass","samples":5}
```

Field rules:

| Field | Meaning |
| :---- | :------ |
| `held_keys` | Human-entered keys the operator is instructed to hold. |
| `raw_codes` | Raw keycodes or scan tokens returned by the STM32 bridge before SDK mapping. |
| `detected` | Normalized `Buttons` names emitted by the candidate mapping. |
| `unexpected` | Detected buttons not expected for the chord. |
| `missing` | Expected buttons absent from the stable sample. |
| `stable` | True after the debounce window sees unchanged raw and detected state. |

For quick manual runs, the probe may also print CSV with the same columns:

```text
kind,t_ms,seq,candidate,chord,held_keys,raw_codes,detected,unexpected,missing,stable
sample,1234,42,arrow-zxas,up+A,ArrowUp+Z,82+29,up+action_a,,,true
```

JSONL is the source of truth; CSV is only a terminal-friendly view.

## Pass Criteria

A candidate is valid when all of the following are true:

- Each required chord produces all expected normalized buttons for five
  consecutive samples.
- No required chord reports an unexpected normalized button.
- Opposite-direction holds do not synthesize action buttons.
- Polling does not block the frame loop budget; the probe must report an input
  poll sample at least once per 16.6ms frame target, or record the achieved rate
  as a failure.
- The result is reproduced at least once after a full reboot.

If multiple candidates pass, prefer them in this order:

1. `arrow-zxas`
2. `wasd-jkui`
3. `ijkl-zxas`

The first passing candidate becomes the default KotoSDK mapping. If no candidate
passes, the embedded backend must keep game actions configurable and ship with a
reduced default of directions plus `A`/`B` only until a better hardware-safe
mapping is measured.

## Default Mapping Decision

Current decision: `arrow-zxas`.

Decision record to fill after the probe:

| Field | Value |
| :---- | :---- |
| Date | 2026-06-21 |
| Board | PicoCalc RP2040 |
| STM32 firmware | Not retained in the submitted final record |
| I2C rate | 100 kHz |
| Selected candidate | `arrow-zxas` |
| Evidence log | KOTO-0067 final `selection` JSONL record |
| Notes | The guided firmware emits selection success only after all 44 required cases pass. `arrow-zxas` was the first passing candidate. |

Final selection record:

```json
{"kind":"selection","status":"pass","selected_candidate":"arrow-zxas","reason":"first_passing_candidate"}
```
