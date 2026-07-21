# KOTO-0252: RP2040 app-session main-stack peak attribution and reduction

- Status: done (hardware-confirmed 2026-07-21: `at=app free_min=9864` and
  `core1_stack_free_min=6556`, both clear their bars; SRAM report re-injected)
- Type: firmware memory
- Priority: P1
- Requirements: NFR-MEM-1, NFR-MEM-2, NFR-MEM-4, NFR-REL-3
- Related: KOTO-0170, KOTO-0172, KOTO-0186, KOTO-0227, KOTO-0251
- Roadmap: [KotoConfig Roadmap](../../planning/KOTOCONFIG_ROADMAP.md)

## Goal

Attribute and reduce the long-standing ~56.9 KiB CPU0 main-stack app-session
peak (`phase=176 stack-peak at=app used=56912 lw=0x200341b0`) so the Pico W
wifi-config product image restores `free_min >= 8 KiB` under its worst
supported workload, unblocking the KOTO-0251 threshold acceptance criterion.

## Background

- The absolute low-water mark `0x200341b0` predates the network wiring:
  KOTO-0170 measured `free_min = 7,620 B` on the offline image against the
  same dip. The KOTO-0251 wifi-config image grows `.data + .bss` by ~4 KiB
  (bounded `NetworkService`/page/credential residents plus the HAL mailbox),
  which moved `__ebss` up to `0x200334ec` and left `free_min = 3,316 B` on a
  session including game launches — below the frozen `>= 8 KiB` bar and the
  KOTO-0170 ~4 KiB stop-ship floor.
- KOTO-0251's boot blocker showed the failure mode this margin protects
  against is real and silent: a transient frame that crosses `_stack_end`
  scribbles the `.bss` tail, and the KOTO-0170 canary cannot see it (paint
  starts at `__ebss`; the overflow lands below it).
- KOTO-0172 (main-future prologue, 68,588 -> 26,616 B boot peak) and the
  KOTO-0251 fix (`initialize_rich_residency` split into `#[inline(never)]`
  frames, ~50 KiB summed transient -> ~18 KiB largest single) are the proven
  playbook: find the by-value temporaries or summed frames on the deep path
  and bound them.

## Acceptance Criteria

- [x] Attribute the `lw=0x200341b0` dip to its call path (which app-session
  frames — VM host, present path, loaders — own the ~56.9 KiB peak), with the
  measurement method recorded.
- [x] Reduce the peak (or the wifi-config static footprint) so the Pico W
  wifi-config image measures `phase=176 free_min >= 8 KiB` on hardware across
  the KOTO-0170 worst session mix plus the KOTO-0251 Wi-Fi-plus-stream
  workload, and re-inject the margins into the machine-readable SRAM report.
  Hardware-confirmed 2026-07-21: `at=app used=50332 free_min=9864
  lw=0x20035b64 guard=ok` (≥ 8 KiB bar met, +1,672 B margin; `lw` moved up
  6,580 B from `0x200341b0`). Measured margin re-injected into
  `audio_residency_memory_picow_wifi_config.json`
  (`cpu0_phase176_free_min = 9864`).
- [x] `core1_stack_free_min >= 4 KiB` (`phase=173`) re-captured on the same
  image and workload. Hardware 2026-07-21: `core1_stack_free_min=6556`
  (+2,460 B over the bar), both while associated and after radio-off, all
  audio counters zero; unchanged vs the KOTO-0227 6,588 B (this issue's
  changes are CPU0/init-path only). Recorded as `cpu1_stack_free_min = 6556`
  in the same SRAM report.
- [x] No behavior change: existing budgets/goldens hold, all-board release
  builds and embedded cross-checks pass, and the offline image's accepted
  behavior is preserved. Hardware: BGM plays after radio-off and SKK
  conversion works on the wifi-config image.
- [x] Consider (and either land or explicitly reject with rationale) a
  permanent guard for below-`__ebss` overflow, e.g. a tripwire word painted
  just under `_stack_end` scanned by the phase=176 pass, so this class of
  corruption reports instead of silently landing in whatever static moved in.
  Landed as the `GUARD_BYTES = 32` floor band; `guard=ok` observed on
  hardware across boot/shell/app/app-exit.

## Non-goals

- Changing the 36 KiB switchable arena, CodeWindow, or raster budgets as a
  memory source (KOTO-0251 non-goals carry over).
- Revisiting the frozen KOTO-0227 thresholds themselves.

## Notes

- Workload separation measured on device (2026-07-20): the Wi-Fi chain plus
  shell alone reports `at=shell used=51740 free_min=8488 lw=0x200355e4`
  (meets the bar; consistent with the bounded ~18 KiB
  `init_rich_runtime_bgm` frame reached from page-exit teardown), while an
  app launch deepens to `at=app used=56912 free_min=3316 lw=0x200341b0`.
  Two independent recovery levers therefore exist: the app-session frame
  (~5 KiB needed) and in-place construction of the rich-audio players
  (widens the shell/Wi-Fi margin as well).
- Candidate attribution tools: sparse `stack_canary::emit_peak` brackets
  around app-launch phases (mount/load/run/present), or a one-off painted-gap
  scan after each stage; the KOTO-0251 session showed statement-level UART
  bisection works when the dip is deterministic.
- The wifi-config `.bss` delta decomposition (KOTO-0251 record) is
  `__embassy_main::POOL` +2,768 B, `network::MAILBOX` 800 B, small atomics —
  little slack on the static side; the stack side owns the recovery.

## Implementation Record

### 2026-07-21: attribution and reduction landed (static verification; hardware pending)

**Measurement method** (recorded per AC-1): static ELF attribution against the
release wifi-config image. `rust-objdump -d --demangle` over
`koto_firmware`, then a script (`stack_attrib.py`, session scratch) that (a)
computes each function's own-frame size from its prologue — `push` words,
`sub sp, #imm`, and the thumbv6m large-frame form `ldr rX, [pc, #..]; add sp,
rX` with the literal resolved from the ELF's `PT_LOAD` segments — and (b)
walks the `bl` call graph for the deepest weighted chain. A companion
`frame_layout.py` dumps a single function's sp-relative buffer bases (literal
`add rN, sp, rN` sites) so multi-KiB frame slots can be identified
individually. The static model reproduced both hardware captures before any
fix: shell dip 33,544 (main-task poll) + 18,048 (`init_rich_runtime_bgm`)
= 51.6 KiB vs measured `used=51740`; app dip 33,544 + 16,120
(`run_device_app` poll) + ~7.1 KiB (`run_app_session` + `load_skk` + SD read
chain) = 56.8 KiB vs measured `used=56912`.

**Attribution of `lw=0x200341b0`** (AC-1):

- `____embassy_main_task` poll frame: **33,544 B** — the floor under both
  shell and app paths (KOTO-0172's successor; not touched by this issue).
- `run_device_app` poll frame: **16,120 B**, dominated by *two ~7,044 B
  staging slots for the `run_app_session` future*: rustc materializes the
  inner future on the poll frame and moves it (7,044 B `memcpy8`, twice)
  before pinning. The future's size was itself dominated by `DeviceHost`'s
  by-value bulk — the ~2.3 KiB SKK leading index, 512 B scan window, memo
  editor/IME state.
- `DeviceHost::load_skk`: **5,072 B** (the `SkkLeadingIndex` by-value build),
  the deepest launch-phase call under the session frames — the measured peak
  is taken during SKK index load at app start, not steady gameplay.
- **Latent deeper path** (worse than the measured dip): the unconditional
  manifest fetch-permission parse during staging ran under the full
  `run_device_app` frame as `parse_package_fetch_permission` (1,336) →
  `parse_manifest_fetch_permission` (5,520) → `JsonCursor::origins` (3,592) →
  `FetchOrigin::parse` (352): worst case **60,664 B** — free_min would have
  gone *negative* (~ -3.5 KiB) on the wifi-config image the first time a
  fetch-declaring app (e.g. KOTO-0250) launched. The measured game sessions
  stopped in the 5,520 B frame before `origins` (no `origins` array), which
  is why 56.9 KiB was the observed ceiling.

**Reduction** (all in-place-construction / bounded-frame moves, the
KOTO-0172/0251 playbook):

1. `DeviceHostSkk` (new): SKK index + scan window + resolved file name moved
   out of `DeviceHost` into a `ConstStaticCell` in the binary (the KOTO-0134
   `app_draw` pattern), threaded through `run_device_app` /
   `run_app_session` / `DeviceHost::new`. Shrinks the app-session future
   ~2.8 KiB, which pays back **double** on the poll frame (both staging
   slots): `run_device_app` 16,120 → **9,552 B**.
2. `SkkIndex::build_from_reader_into` (koto-core): out-param variant writing
   the index directly into its resident cell; `build_from_reader` kept as a
   by-value wrapper. `load_skk` drops off the deep-frame list entirely
   (5,072 → < 1.2 KiB).
3. `parse_manifest_fetch_permission_into` + `permissions_into` /
   `network_v2_into` / `origins_into` (koto-core): the whole permission parse
   chain writes through out-references instead of returning ~1.3 KiB values;
   by-value wrappers preserved for sim/tests.
   `parse_package_fetch_permission_into` (firmware) follows suit, and
   `stage_app_code` is `#[inline(never)]` so the entire staging transient
   (3,072 B + ~1.5 KiB parse) pops before the session runs deep.
   `ManifestFetchResident::activate_fetch` / `deactivate_fetch` are
   `#[inline(never)]`, take the permission halves by reference, and
   `deactivate_fetch` zeroes the scratch union in place (`write_bytes`)
   instead of assigning a 2.3 KiB array temporary.
4. Rich-audio idle-player templates (`IDLE_BGM_PLAYER` 9,024 B /
   `IDLE_SFX_PLAYER` 1,344 B / `IDLE_CLIP_PLAYER` 8,248 B): const-built into
   `.rodata` (XIP flash, zero RAM) and copied straight into the arena slot
   via `RichSlot::init_from` — `init(T::new(..))` staged each player on the
   constructing frame first, and the ~18 KiB BGM temporary was the measured
   shell-path low-water (`at=shell used=51740`), reached on every post-Wi-Fi
   rich-audio rebuild. `init_rich_runtime_bgm` drops from 18,048 B to a
   leaf; the largest remaining init frame is `init_rich_service` (8,032 B,
   fallible constructor, deliberately left).

**Static worst-case after the change** (same tooling, same image):
**48,448 B** total (main 33,544 + `run_device_app` 9,552 + session frames +
UI event chain — no longer the launch path), vs 60,664 B latent / 56,912 B
measured before. The staging and `load_skk` chains are now ~47.5 KiB and
~45 KiB. Expected on-device `free_min` ≈ 3,316 + (56,912 − 48,448) − 32
(guard band) ≈ **11.5 KiB** on the worst app session; the shell/Wi-Fi path
drops to ~41.6 KiB used ≈ **18+ KiB** free_min. `.data+.bss` is net −16 B
(the new SKK cell is repaid by the smaller main-task future `POOL`); flash
grows ~18.6 KiB of `.rodata` templates.

**Floor guard** (AC-5, landed): the bottom `GUARD_BYTES = 32` of the painted
gap now carry a distinct `GUARD_WORD` ("toko" LE, the canary reversed). The
KOTO-0251 corruption class necessarily writes through this band, so the
`phase=176` line now ends with `guard=ok|HIT` — an explicit report for a
crossing that previously landed silently in the `.bss` tail (plus an early
warning when the stack merely comes within 32 B of the floor). Pure software
convention over the existing gap: no linker changes, no `.bss` growth.
`free_min` is measured from the top of the band, so readings shift down
32 B against pre-KOTO-0252 captures. A crash before the next scan is covered
by the KOTO-0251 `phase=91` HardFault/panic reporters.

**Gates**: koto-core 376 tests green (fetch/skk out-param equivalence via the
by-value wrappers), koto-sim green except the pre-existing
`app_gallery_skk_candidate...` failure (fails identically on the committed
tree without these changes), `check_embedded.py` all boards OK, release
builds pass for pico default / picow offline / picow wifi-config / pico2w
wifi-config, `check_audio_residency_memory.py` /
`check_wifi_residency_layout.py` / `check_network_service_budget.py` /
`check_audio_residency.py` / `check_audio_scratch.py` / `check_project.py`
all OK. Clippy on the wifi-config profile now has **zero errors** (the
pre-existing `mut_from_ref` deny-by-default hits on `RichSlot` are allowed
with rationale; toolchain-drift warnings unchanged).

**Remaining (hardware)**: flash
`koto_firmware-picocalc-picow-rp2040-wifi-config.uf2` (rebuilt), run the
KOTO-0170 worst session mix (game launches incl. KotoRun/KotoRogue) plus the
KOTO-0251 Wi-Fi-plus-stream workload, and capture:
`phase=176 stack-peak` (expect `free_min >= 8 KiB`, `guard=ok`, and a lower
`used=` at a *different* low-water than `0x200341b0`), `phase=173`
`core1_stack_free_min >= 4 KiB`, BGM/rich-audio regression after radio-off
(the rebuilt init path), SKK conversion in File Note (index now in the
static cell), and one fetch-capable app launch if available. Then re-inject
the measured margins into the machine-readable SRAM report per KOTO-0251.


### 2026-07-21: hardware-confirmed on the wifi-config image

Flashed `koto_firmware-picocalc-picow-rp2040-wifi-config.uf2` (rebuilt) and
captured a session with app launches, radio enable/associate, stream audio,
and radio-off:

```
phase=176 stack-canary bottom=0x200334bc painted_top=0x20039c48 stack_top=0x20042000 painted=26508
phase=176 stack-peak at=boot     used=41752 free_min=18444 lw=0x20037ce8 guard=ok
phase=176 stack-peak at=shell    used=41752 free_min=18444 lw=0x20037ce8 guard=ok
phase=176 stack-peak at=app      used=50332 free_min=9864  lw=0x20035b64 guard=ok
phase=176 stack-peak at=app-exit used=50332 free_min=9864  lw=0x20035b64 guard=ok
phase=176 stack-peak at=shell    used=50332 free_min=9864  lw=0x20035b64 guard=ok
```

- **`free_min = 9,864 B` at the app peak — clears the frozen `>= 8 KiB` bar
  with +1,672 B margin.** The KOTO-0251 blocker (`free_min = 3,316`) is
  resolved; that issue's threshold AC can close on the wifi-config image.
- **`lw` moved from `0x200341b0` to `0x20035b64`** (6,580 B shallower;
  `used` 56,912 → 50,332), confirming the reduction landed on the real
  runtime peak, not just the static deepest chain. The shell/Wi-Fi path is
  `used=41,752 free_min=18,444` (was 51,740 / 8,488 — the rich-audio
  flash-template move recovered ~9.9 KiB there as predicted).
- **`guard=ok` at every call site** — no floor crossing; the band is intact.
- The static worst-case estimate (48,448 B) under-predicted the measured app
  peak (50,332 B) by ~1,884 B: the runtime peak rides a slightly deeper
  indirect-call path than the static `bl`-chain walk traces. Margin holds
  regardless.
- Behavior preserved on hardware: **BGM plays after radio-off** (the rebuilt
  `init_rich_runtime_*` flash-template path) and **SKK conversion works** (the
  index now lives in the `APP_HOST_SKK` static cell, not `DeviceHost`).

**`phase=173` core1 (captured 2026-07-21, same image)**: `audio-summary
... core1_stack_free_min=6556 drops=0 underruns=0 command_drops=0
worker_late=0` both while associated (`stream_acquisitions=105`) and after
radio-off (`stream_acquisitions=32`) — 6,556 B clears the `>= 4 KiB` bar by
2,460 B and is unchanged (−32 B) from the frozen KOTO-0227 6,588 B, as
expected since this issue touches only CPU0/init-path frames. `phase=160`
confirms the stream app runs clean (`vm_us=105 underruns implicit-0 ovf=0`).

**SRAM report re-injection (done)**: regenerated
`target/koto-dev/audio_residency_memory_picow_wifi_config.json` from the
flashed ELF with `--cpu0-free-min 9864 --cpu1-stack-free-min 6556`;
`hardware_margins` now records both (`phase=176` / `phase=173 audio-summary`
sources) and every static check still passes. `baseline_delta.data_bss_delta`
is 4,140 B — 16 B below the pre-KOTO-0252 4,156 B, confirming the net
`.data+.bss` change is −16 B. All acceptance criteria met; the KOTO-0251
threshold AC (CPU0 + CPU1) is unblocked on the wifi-config image.
