# KOTO-0170: make `ram_interpreter` the default firmware build

- Status: DONE 2026-07-07 — Stage 0 measured on hardware (peak `used=68,588 B`
  across the full session mix, both builds; `free_min=7,620 B` with the
  feature on) and Stage 1 outcome (1) taken: `ram_interpreter` is now in
  koto-pico's **default features** (opt out with `--no-default-features`).
  The `phase=176` canary stays as the permanent `.bss`-growth tripwire.
- Type: firmware RAM budget / performance enablement
- Priority: P2
- Requirements: NFR-PERF-1

Source of truth:
[koto-pico Cargo.toml](../../../src/koto-pico/Cargo.toml) (the opt-in
`ram_interpreter` feature and its rationale comment),
[KOTO-0169](KOTO-0169-vm-frame-cost-attribution.md) Stage-2 record (the boot
failure, the reshaped ~2.4 KiB form, and the device-confirmed −16.4% ns/op),
`memory.x` / cortex-m-rt `link.x` (RAM layout: `.data` + `.bss` grow up from
`0x2000_0000`, the main stack grows down from `0x2004_2000` with **no linker
guard**).

Relates to: [KOTO-0169](KOTO-0169-vm-frame-cost-attribution.md) (produced the
lever this issue wants to enable by default), KOTO-0136 (the historical
~78.5 KiB-free boot hang — the first documented collision with this invisible
stack ceiling).

## Problem

KOTO-0169 Stage 2 delivered a device-confirmed **−16.4% interpret+fetch
ns/op** (1,97x → 1,646) by running the VM hot loop from SRAM — but only
behind `--features ram_interpreter`, because the RAM budget almost has no
headroom left:

| Static free RAM above `.bss` | Boot result |
| --- | --- |
| 79.3 KiB (default build today) | boots |
| 76.4 KiB (opt-in `ram_interpreter`, −2.9 KiB) | boots |
| 73.9 KiB (first default-on shape, −5.4 KiB) | **does not boot** |

The real main-stack peak is therefore somewhere in **(73.9, 76.4) KiB** — the
booting opt-in build may be running with **less than 2.5 KiB of margin**,
which is too thin to bet the default firmware on (and too thin to keep
shipping blind, feature or not: any future `.bss` growth of ~2 KiB is a
boot-risking change — see the pico-firmware-stack-headroom history).

## Staged plan

### Stage 0 — measure the real stack peak (observe-only)

Paint-and-scan canary: at boot (before the executor starts), fill the region
between `__ebss`/`__euninit` and the current SP with a pattern; scan for the
low-water mark on a sparse cadence (e.g. on app exit and on a `phase=154`-like
heartbeat) and emit it as a new sparse UART line (`phase=176 stack-peak
used= free_min=` — 176 is unclaimed). Scan cost is a linear read of the
untouched region; gate the cadence under DIAG-0001 if it measures above
trivial. Run it through a worst-case session mix (boot → shell browse →
KotoRun/KotoSnake/KotoBlocks/KotoShogi launches, IME/memo, audio active) on
both the default and `ram_interpreter` builds.

**Gate:** a measured peak with app coverage, giving the true margin numbers
for both builds.

#### Stage 0 record — instrumentation (2026-07-07)

The canary lives in
[stack_canary.rs](../../../src/koto-pico/src/firmware/stack_canary.rs):

- **Paint:** `stack_canary::paint()` is the first statement of `main` — it
  fills `[__euninit, boot-SP − 64)` with the word `0x6F74_6F6B` ("koto")
  before `embassy_rp::init`, so the scans cover the whole boot including
  clock/peripheral bring-up.
- **Scan/emit:** `emit_peak` walks up from `__euninit` to the first
  overwritten word (the low-water mark; monotonic, so any later scan reports
  the session-wide peak). Cost is a linear read of the *untouched* gap only
  (~19k word loads ≈ sub-ms worst case), so every emit site is always-on
  under all DIAG-0001 profiles — this is the permanent `.bss`-growth
  tripwire, not an investigation-only diagnostic.
- **Emit sites (all sparse):** one self-describing
  `phase=176 stack-canary bottom= painted_top= stack_top= painted=` region
  line plus a `at=boot` peak right before `phase=23 shell-loop-enter`;
  `at=shell` every ~30 s of shell heartbeat; `at=app` every 600 in-app frames
  (~10 s); `at=app-exit` right after `run_device_app` returns (the VM +
  present path is the expected deepest user).
- **Line format:**
  `phase=176 stack-peak at=<site> used=<bytes> free_min=<bytes> lw=0x<addr>`
  — `used` is the peak depth below `_stack_start` (0x2004_2000), `free_min`
  the minimum margin left above the statics.
- **Canary RAM cost: zero.** `__euninit` is byte-identical with and without
  the change (default build 0x2002_EA40 → 79,296 B free; `ram_interpreter`
  build 0x2002_F640 → 76,224 B free; the 4-byte `PAINTED_TOP` static is
  absorbed by existing alignment padding). Code lives in flash.
- Both variants build clean (`cargo build -p koto-pico --bin koto_firmware
  --target thumbv6m-none-eabi --release [--features ram_interpreter]`);
  clippy/rustfmt findings in the touched files are the known pre-existing
  drift, none from this change.

#### Stage 0 record — hardware measurement (2026-07-07, gate PASSED)

Both builds (each with `psram_fast_code_window`, the production-leaning
combo) ran the worst-case mix: boot → shell browse → KotoSnake / KotoBlocks /
KotoShogi / KotoRogue / KotoRun / KotoMemo (IME), audio active, each app past
multiple 600-frame cadences.

| Build (+`psram_fast_code_window`) | `used` peak (B) | `free_min` (B) | `lw` |
| --- | --- | --- | --- |
| default (flash-XIP interpreter) | 68,588 | 10,692 | 0x2003_1414 |
| `ram_interpreter` | 68,588 | **7,620** | 0x2003_1414 |

Findings:

- **The peak is a boot-time phenomenon, not a VM one.** `at=boot` already
  reports the final low-water mark and it never moves again — not one word —
  through every app session and shell browse, in both builds. The deepest
  core-0 stack user is the boot bring-up path, so app/VM work adds no
  stack-margin pressure on top of what boot already establishes.
- The two builds peak at the *identical* depth and low-water address, which
  is consistent with the peak living in shared boot code: `ram_interpreter`
  moves the cost entirely to statics (`.data`), exactly the −3,072 B
  difference visible in `free_min`.
- The region lines also show ~40 KiB of stack already consumed *before*
  `paint()` runs at the top of `main` (`stack_top − painted_top ≈ 40,112 B`,
  counted in `used` conservatively since the boot SP genuinely reached
  there): the embassy main-task spawn/entry transient. Together with the
  post-paint 28.5 KiB of boot-init depth this is a possible future shrink
  target (relates to the KOTO-0134 main-future-size note), out of scope here.
- **Historical discrepancy, recorded:** the measured peak (68,588 B) sits
  *below* the (73.9, 76.4) KB bracket inferred from the KOTO-0169 Stage-2
  boot failure — the first default-on shape (73.9 KB free, nominal ~5.3 KiB
  margin against this peak) should have booted if its peak matched today's.
  Either that ~5.4 KiB shape genuinely peaked deeper (different inlining /
  spawn transient), or its hang was not a pure stack-vs-`.bss` collision.
  The flip decision below uses the measured margin of the *current* shape,
  which is also the exact binary the session mix above soak-tested.

Core 1 (`AUDIO_CORE1_STACK`) is *not* covered by this canary; measure it the
same way before shrinking it (Stage-1 outcome (2) candidate — not needed).

### Stage 1 — decide with numbers (one of three outcomes)

1. **Margin is real (≥ ~6 KiB above peak with the feature on):** flip the
   default — move `ram_interpreter` into koto-pico's default features, keep
   the opt-out documented, record before/after `phase=175` ns/op and a boot
   soak. The canary stays as a permanent regression tripwire.
2. **Margin is thin:** fund it first — candidates, each a deliberate budget
   with its own trade-off (own sign-off): `AUDIO_CORE1_STACK_BYTES`
   (8 KiB — measure core-1 peak with the same canary before touching),
   `SKK_DICT_CAP`, `KICON_SCRATCH`, shrinking the 2.4 KiB further (e.g.
   flash-pinning `exec_binary`'s twin copies). Then flip as in (1).
3. **Peak is effectively at the ceiling already:** record that the default
   stays opt-in and why; the canary still lands as the guard for future
   `.bss` growth.

#### Stage 1 record — outcome (1) taken (2026-07-07)

Measured `free_min = 7,620 B` (≈ 7.4 KiB) with the feature on, over the
≥ ~6 KiB bar → **default flipped**: koto-pico now has
`default = ["ram_interpreter"]` ([Cargo.toml](../../../src/koto-pico/Cargo.toml)),
opt-out via `--no-default-features`. Verified post-flip:

- New default build's `__euninit` = 0x2002_F640 (76,224 B free) — **byte-
  identical to the opt-in `ram_interpreter` build**, i.e. the flip is a pure
  alias of the exact feature set the session mix above already soak-tested
  on hardware (boot + all six apps + shell + audio). The
  `--no-default-features` opt-out reproduces the old default exactly
  (`__euninit` = 0x2002_EA40, 79,296 B free). All koto-pico bins build.
- Before/after `phase=175` ns/op is the KOTO-0169 device record for the same
  feature set: interpret+fetch ns/op 1,97x → 1,646 (Stage 2 alone, −16.4%),
  1,975 → 1,418 with Stages 2+3 cumulative.
- The `phase=176` canary stays always-on under every DIAG-0001 profile as
  the regression tripwire: any change that grows `.bss` (or deepens boot)
  must keep `free_min` comfortably positive on hardware — treat a hardware
  capture showing `free_min` under ~4 KiB as a stop-ship regression.

### Non-goals

- No VM/interpreter changes (KOTO-0169 owns those; the feature's content is
  frozen here — this issue only decides *where it's on*).
- No code-window / PSRAM buffer resizing to pay for it (KOTO-0132/0155 own
  those trade-offs).
- No `memory.x` growth tricks (scratch banks are already inside the 264 KiB
  budget; there is no free RAM to "find" at the linker level).

## Acceptance criteria

- [x] Stage 0: `phase=176 stack-peak` (or equivalent) measured on hardware
      across the worst-case session mix, on both default and
      `ram_interpreter` builds; peaks and margins recorded here.
- [x] Stage 1 decision recorded with the numbers: default flipped. The
      before/after ns/op is the KOTO-0169 device record for the identical
      feature set, and the Stage-0 session mix doubles as the boot soak —
      the flipped default is byte-identical to the build it ran on.
- [x] The stack canary remains in the firmware as a regression tripwire for
      future `.bss` growth (always-on under every DIAG-0001 profile; the
      emit cadence is sparse and the scan reads only the untouched gap).
