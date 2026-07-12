# KOTO-0172: shrink the embassy main-task poll stack frame

- Status: DONE 2026-07-07 — hardware-confirmed. Poll frame 39,916 → 15,564 B;
  main-future `POOL` 84,128 → 22,744 B; static free RAM 76,208 → 110,000 B;
  device `phase=176` peak **68,588 → 26,616 B** (`free_min` 7,620 →
  **83,384 B**), monotonic across the boot → shell → apps mix.
- Type: firmware RAM budget / stack-peak reduction
- Priority: P2
- Requirements: NFR-PERF-1

Source of truth:
[koto_firmware.rs](../../../src/koto-pico/src/bin/koto_firmware.rs) (the
`ConstStaticCell` working-set cells + the static `SHELL`),
[shell.rs](../../../src/koto-core/src/shell.rs) (`ShellState::empty` /
`reload_packages`), [KOTO-0170](KOTO-0170-ram-interpreter-default-on.md)
(the `phase=176` canary that exposed the problem and will confirm the fix).

Relates to: KOTO-0134 (first noted the oversized main-task future),
[KOTO-0132](KOTO-0132-profile-and-optimize-pio-psram-read-bandwidth.md) /
KOTO-0134's 2-tile cache (candidates this freed budget can fund).

## Problem — where the KOTO-0170 stack peak actually came from

The KOTO-0170 hardware canary measured a 68,588 B core-0 stack peak, with
~40 KiB already consumed *before* `paint()` at the top of `main`. Static
attribution (no hardware needed) broke that down:

1. **The main task's poll function reserved a 39,916 B stack frame in its
   prologue** (disassembly: `ldr r6, =-39916; add sp, r6`), re-reserved on
   *every* poll — not a boot-only transient. Pre-paint depth = this frame +
   ~130 B of executor entry.
2. Bisection (removing `run_device_app(...).await` moved the frame by only
   8 B; a type-size probe burned into `.rodata` gave the culprit sizes):
   `ShellState` = 28,240 B, `PackageList` = 28,036 B (32 × 876 B
   `PackageInfo` — the 876-byte stride is visible in the frame's literal
   pool). `let packages = PackageList::new()` + `ShellState::new(packages)`
   materialized a ~28 KiB slot in the poll frame.
3. The main-task future (`__embassy_main::POOL`, .bss) was 84,128 B and held
   `shell` (28,240) **and** the `packages` local (28,036) **and** the 5 KiB
   `pcm_diag` boot-tone buffer across awaits — ~61 KiB of duplicated
   working-set.

`StaticCell::init(value)` has the same failure shape: the value argument is
built in the caller's frame before being copied into the cell, so every
`init([0; N])` / `init([DeviceRuntimeHost::new(), ..])` call was another
poll-frame slot (LLVM shared some of those slots; converting them alone only
saved ~0.8 KiB, which is why the bisection was needed).

## Fix (landed)

- **koto-core:** `ShellState::empty()` (const constructor; `new` now
  delegates to it), `ShellState::reload_packages(fill)` (clears and refills
  the catalog *in place*, rebuilds the view, passes the closure's return
  value through), `PackageList::clear()` (slot-by-slot, deliberately not
  `*self = PackageList::new()`). Host tests cover `empty()` ≡
  `new(PackageList::new())` and in-place reload equivalence.
- **koto-pico:** all working-set cells are `ConstStaticCell` (taken with
  `take()`, zero runtime init); the shell lives in a new
  `static SHELL: ConstStaticCell<ShellState>` and the SD catalog is loaded
  directly into it via `reload_packages`; the boot PCM diagnostic tone is a
  compile-time `static` table in flash.

## Measured (static, `rust-nm`/`rust-objdump`, default build)

| Metric | Before | After | Δ |
| --- | --- | --- | --- |
| Main-task poll frame | 39,916 B | 15,564 B | **−24,352 B** |
| `__embassy_main::POOL` (.bss) | 84,128 B | 22,744 B | **−61,384 B** |
| Static free RAM above `.bss` | 76,208 B | 110,000 B | **+33,792 B** |
| `.data` | 0 | 0 | — (SHELL is all-zero → stays `.bss`) |

Expected hardware effect: stack peak ≈ 68,588 − 24,352 ≈ **~44 KiB**, so
`phase=176 free_min` should land around **~65 KiB** (vs 7,620 B before) —
enough to fund a CODE_WINDOW expansion or the KOTO-0134 2-tile cache with
room to spare. Verified host-side: koto-core 136/136 tests, sim `cargo
check`, all koto-pico bins + opt-out build, clippy/rustfmt clean on the
touched files.

## Hardware confirmation (2026-07-07, default build, session mix)

```
phase=176 stack-canary bottom=0x20027250 painted_top=0x2003e290 stack_top=0x20042000 painted=94272
phase=176 stack-peak at=boot used=26616 free_min=83384 lw=0x2003b808
```

- **Peak `used` = 26,616 B** (was 68,588), **`free_min` = 83,384 B** (was
  7,620) — better than the ~44 KiB model because the boot callee depth below
  the frame also shrank (28.5 → 10.9 KiB; the nested shell-paint/init paths
  shared the same materialization pattern).
- Pre-paint depth is now 15,728 B (`stack_top − painted_top`), matching the
  15,564 B frame + executor entry — the static model and the device agree.
- The peak is still established at boot and never deepens through shell/apps
  (`lw` identical at every emit site), same shape as KOTO-0170 observed.

## Remaining / follow-ups

- [x] Hardware confirmation — recorded above.
- [ ] The remaining 15,564 B frame has one ~15 KiB slot (literal-pool gap
      analysis suggests an RGB666-strip-sized object on some shell-paint
      path). Chase it only if a future budget needs it — the current margin
      does not.
- [ ] Spending the freed budget (CODE_WINDOW 16→32 KiB, or the 2-tile cache
      that KOTO-0134 reverted for stack reasons) is deliberately a separate
      issue with its own sign-off, per the deliberate-budget rule.
