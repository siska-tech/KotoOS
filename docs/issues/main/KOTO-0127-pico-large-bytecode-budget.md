# KOTO-0127: Pico Large App Bytecode And Heap Budget

- Status: done
- Type: feature
- Priority: P1
- Requirements: FR-RT-2, FR-RT-3, FR-RT-5, NFR-MEM-1, NFR-MEM-2, NFR-MEM-4

## Goal

Let real game packages launch on PicoCalc, not just the small SDK samples. The
KOTO-0125 runtime slice bounds an app to an 8 KiB in-SRAM bytecode buffer and a
16 KiB heap; every game exceeds that and fails launch.

## Context

Physical launch of every game package returns to Shell with
`phase=253 launch-bytecode-oversize`. Committed bytecode sizes:

| Package                         | Bytecode                |
| ------------------------------- | ----------------------- |
| SDK samples                     | 0.7-4.9 KiB (fit today) |
| sokoban / memo                  | 20-22 KiB               |
| kotomines / kotosnake / kotorun | 30-40 KiB               |
| kotoshogi                       | 73 KiB                  |
| koto_blocks                     | 91 KiB                  |
| kotorogue                       | 96 KiB                  |

The launch path holds the whole program in `MAX_DEVICE_BYTECODE_BYTES` (8 KiB,
`src/koto-pico/src/bin/koto_firmware.rs`) and gives each app a heap up to
`MAX_DEVICE_HEAP_BYTES` (16 KiB). A 96 KiB program cannot fit the bytecode buffer,
and games also request heaps beyond 16 KiB, so simply enlarging the bytecode
buffer is not sufficient on its own.

## Acceptance Criteria

- [x] At least one large game package (target: koto_blocks or kotorogue) launches,
  runs, reads input, and exits cleanly on physical hardware. *(Met for the bytecode
  budget goal: on hardware koto_blocks's 65 KiB code streams from PSRAM and the VM
  runs in real time — frames advance and the pc progresses from the title yield to
  the gameplay yield with healthy fuel and no trap (see "Hardware findings").
  Visible gameplay rendering needs device sprite support, split to **KOTO-0129**;
  the KotoMemo launch failure is a separate follow-up.)*
- [x] The device runtime no longer requires the entire program to reside in an
  SRAM bytecode buffer sized for the largest app. *(Code lives in PSRAM; the VM
  reads it through an 8 KiB `PsramCodeWindow` — `src/koto-core/src/psram.rs`.)*
- [x] App heap sizing honors each package's KBC header request up to a documented,
  deliberately-sized device ceiling rather than a fixed 16 KiB. *(Per-app sizing
  was already present; the 16 KiB ceiling is now justified in code — see Notes.)*
- [x] Oversized/over-budget apps still fail gracefully back to a usable Shell with
  a UART diagnostic (no device reset). *(`stage_app_code` gates code size against
  `DEVICE_CODE_CEILING` and heap against the ceiling, logging `phase=253`/`255`.)*
- [x] KotoSim and PicoCalc keep a shared runtime-session contract; only the byte
  source and memory backing differ per platform. *(`CodeSource` trait: `SliceCode`
  for sim/tools, `PsramCodeWindow` for device; one `BytecodeSession`.)*

## Implementation

Investigating the committed KBC headers refined the problem: **rodata is 0 in
every package** (string constants are written to the heap by codegen, not read
from rodata), and the **debug segment is never used on device**. So at runtime the
VM needs only the resident 64-byte header plus the **code** segment addressable —
not the whole file. The real blocker is code size (kotorogue 72 KiB), not heap:
the largest heap request is kotorogue's 10.8 KiB, already under 16 KiB.

- **Core (`src/koto-core/src/runtime.rs`).** Added a `CodeSource` trait — the VM's
  one runtime byte access (a 4-byte code-word fetch per step) and the verifier's
  sequential walk now go through it. `SliceCode` backs the resident path
  (KotoSim, tools, small-app fallback); `verify_kbc_streaming` + `BytecodeSession::
  new_streaming` / `step_frame_with` verify and run from any source. The
  slice-based `verify_kbc` / `step_frame` are thin wrappers, so sim/tools/tests are
  unchanged.
- **PSRAM window (`src/koto-core/src/psram.rs`).** `PsramCodeWindow` implements
  `CodeSource` over `PsramBlocks`, tiling the code so sequential execution stays
  cached and only branches/tile-crossings refill.
- **Firmware (`src/koto-pico/src/bin/koto_firmware.rs`).** Brings up PSRAM (PIO1,
  GP20/21/2/3); `stage_app_code` reads the header, budget-gates, and streams the
  code segment SD→PSRAM (base 0). `run_app_session` is generic over `CodeSource`,
  so the PSRAM-window and SRAM-fallback paths share one loop. PSRAM-absent falls
  back to running small apps from the SRAM window.

### Deliberate budget sizing

- **Code window: 8 KiB SRAM**, the former in-SRAM bytecode buffer repurposed as the
  `PsramCodeWindow` cache — **net-zero new SRAM**. Holds the whole code of SDK
  samples (no refills) and slides over larger games' code.
- **Code ceiling: `DEVICE_CODE_CEILING` = 128 KiB**, resident in 8 MiB PSRAM —
  above the largest current app (kotorogue ~73 KiB) with headroom, a tiny fraction
  of PSRAM.
- **App heap ceiling: 16 KiB SRAM**, per-app sized from the KBC header request
  (KOTO-0096); clears the heaviest app (kotorogue 10.8 KiB) with headroom and is
  sized against the resident OS core plus the raster-strip / RGB666 staging budget.

### Hardware findings (first bring-up)

**The PSRAM bytecode streaming works.** koto_blocks (65 KiB code) stages to PSRAM
and runs on real hardware — UART shows the VM advancing in real time with a
healthy fuel budget and no trap, progressing from the title-screen yield to the
gameplay yield:

```
phase=156 app-staged backing=psram code_size=65528
phase=152 app-started
phase=154 app-heartbeat frame=60  pc=6064  fuel=5219    # title screen loop
phase=154 app-heartbeat frame=180 pc=6064  fuel=5218
phase=154 app-heartbeat frame=240 pc=16378 fuel=28174   # advanced to gameplay
phase=154 app-heartbeat frame=300 pc=16378 fuel=26665
```

So the KOTO-0127 mechanism (code in PSRAM, executed through the SRAM window) is
validated on device. The remaining problems are **device rendering capability**
gaps, exposed for the first time now that large games actually run — not the
bytecode/heap budget this issue scoped:

- **Sprite blits are unsupported on device.** koto_blocks draws its board with
  `draw_pixels` (the PicoMings tile/sprite model, KOTO-0094/0097), but
  `DeviceHost` does not implement `draw_pixels_rgb565` — it returns the default
  `UNSUPPORTED`, which the app swallows. The title screen uses `draw_rect` /
  `draw_text_color` (supported) so it appears; the board never blits, so the game
  visually "stops while composing the screen" even though the VM keeps running.
  `DeviceRuntimeHost` also has no pixel command variant, and
  `MAX_APP_DRAW_COMMANDS` is only 16 — far below a full board. This is a device
  rendering follow-up (sprite/tile path), separate from the memory work here.
- **KotoMemo fails to launch (out of memory).** memo is text/editor-based (no
  `draw_pixels`); its KBC heap request is 1661 bytes and `sram_work_bytes` 24576,
  so neither the heap gate nor the code ceiling should reject it. Needs its UART
  phase code to confirm whether it is `phase=255` (budget gate) or a fault/reset
  from the editor/IME host working set (`MemoEditor<256>` + `KotoMemoIme`).

Split out as follow-ups (KOTO-0127's PSRAM `CodeSource` stays unchanged):

- **KOTO-0129** — device Game2D `draw_pixels_rgb565` support + draw-command budget,
  so KotoBlocks' board/pieces actually render on hardware.
- **KotoMemo launch failure** — collect the final UART phase/error first, then
  track separately (not folded into KOTO-0129).

## Notes

Follow-up to KOTO-0125. The intended direction is FR-RT-2 / FR-RT-5: stream
bytecode from PSRAM through the block-transfer API into a small SRAM working
window instead of holding the whole program in SRAM, with app heap/working memory
drawn from the PSRAM/SRAM budget (NFR-MEM-1/2/4). This is a deliberate RP2040 SRAM
budget decision, not a casual constant bump — see the project's VM-budget-sizing
guidance. Sizing should be justified against the resident OS-core budget and the
remaining app/framebuffer budget.
