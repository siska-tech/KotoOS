# KOTO-0171: make `psram_fast_code_window` the default firmware build

- Status: DONE 2026-07-07 — decision record; the flip landed together with
  this document. `psram_fast_code_window` is now in koto-pico's **default
  features** (`default = ["ram_interpreter", "psram_fast_code_window"]`);
  opt out with `--no-default-features`.
- Type: firmware performance enablement (decision record)
- Priority: P2
- Requirements: NFR-PERF-1

Source of truth:
[koto-pico Cargo.toml](../../../src/koto-pico/Cargo.toml) (the feature and the
default set), [psram_ext.rs](../../../src/koto-pico/src/psram_ext.rs) (the
`FastFallingClkdiv2` refill override, its safe-read fallback, and the
backend-exclusivity `compile_error`),
[KOTO-0170](KOTO-0170-ram-interpreter-default-on.md) (the hardware session
mix this exact feature combination was soak-tested under).

Relates to: KOTO-0153 (built the fast refill; opt-in at the time pending
hardware confidence), [KOTO-0132](KOTO-0132-profile-and-optimize-pio-psram-read-bandwidth.md)
(the 1.4 MB/s baseline this path was measured against),
[KOTO-0169](KOTO-0169-vm-frame-cost-attribution.md) (identified the fixed
per-frame refill tax as the largest remaining VM-adjacent slice).

## Decision

`psram_fast_code_window` was opt-in only because it was hardware-unverified
at the time it landed. That reservation no longer holds:

- The fast refill path has been exercised extensively on hardware across the
  KOTO-0169/KOTO-0170 investigation builds, including the full KOTO-0170
  worst-case session mix (boot → shell → KotoSnake / KotoBlocks / KotoShogi /
  KotoRogue / KotoRun / KotoMemo + IME, audio active) — which ran on
  `ram_interpreter,psram_fast_code_window`, i.e. **exactly the new default
  set**. The measured stack margin (`phase=176 free_min=7,620 B`) and the
  boot/session soak therefore already cover this combination byte-for-byte.
- Effective refill read bandwidth is ~10 MB/s (vs the 1.4 MB/s safe-profile
  baseline recorded in KOTO-0132) — the refill optimization track is
  considered done; this issue only moves where it is *on*.
- Risk posture is unchanged by design: ordinary PSRAM reads/writes stay on
  the safe profile, and the fast refill falls back to the safe read on any
  unsupported address/length or read error (per-call, in `psram_ext.rs`).
- RAM cost is negligible and already inside the measured budget: `__euninit`
  moves 0x2002_F640 → 0x2002_F650 (16 B) with the feature on; the KOTO-0170
  margin numbers were measured with it enabled.

## Consequences

- The default build is now byte-identical to the soak-tested
  `ram_interpreter,psram_fast_code_window` binary (`__euninit` =
  0x2002_F650, 76,208 B static free).
- `--no-default-features` reproduces the fully-safe legacy behaviour
  (flash-XIP interpreter + safe-profile refills).
- The legacy/qpi/dma PSRAM experiment profiles (`legacy_psram`,
  `psram_dma_read_code_window`, `psram_qpi_safe_read_code_window` and their
  dependents) are mutually exclusive with the fast refill and now require
  `--no-default-features`; the `compile_error` in `psram_ext.rs` says so
  explicitly.

## Acceptance criteria

- [x] `psram_fast_code_window` in koto-pico's default features; opt-out and
      experiment-profile build lines documented.
- [x] Default build verified byte-equivalent (`__euninit`) to the
      hardware-soaked KOTO-0170 combination.
- [x] Exclusivity failure mode verified: enabling a legacy/qpi/dma profile
      without `--no-default-features` fails fast with the descriptive
      `compile_error`, and the profiles still build with it.
