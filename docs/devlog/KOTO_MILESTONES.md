# KotoOS Milestones

The ~20 most significant technical challenges and breakthroughs across the KotoOS development arc, documented in detail.

---

## 1. Rust Workspace Bootstrap

- **Issue**: KOTO-0001
- **Status**: done
- **Problem**: No project skeleton, no CI baseline, no agreement on which crates would exist.
- **Diagnosis**: Starting from scratch meant first establishing the workspace boundary (what lives in `koto-core` vs. `koto-sim` vs. `koto-pico`) and the invariant that core must compile for both `std` (simulator) and `thumbv6m-none-eabi` (RP2040) without platform deps.
- **Solution**: Created `koto-core` (HAL traits, package primitives, no platform deps) and `koto-sim` (host binaries). Wired `cargo fmt --check`, `cargo test`, and `cargo clippy --all-targets -- -D warnings` as the day-one check suite.
- **Result**: Every subsequent issue compiles against a known baseline. The no-platform-deps invariant in core has held through 129 issues.

---

## 2. KBC1 Bytecode Verifier

- **Issue**: KOTO-0033
- **Status**: done
- **Problem**: No mechanism to reject malformed or malicious app bytecode before execution.
- **Diagnosis**: The VM needed a one-pass allocation-free verifier that could run on the RP2040 before any heap or stack was committed to a new app.
- **Solution**: Implemented `verify_kbc` in `koto-core::runtime`: checks KBC1 header magic, bounds, resource limits against caller-provided `RuntimeLimits`, opcode coverage, branch/call target validity, static stack-underflow detection, and host-call ID membership.
- **Result**: Any app bytecode that passes `verify_kbc` under the configured limits is guaranteed to construct a valid `BytecodeVm` session. Invalid apps are rejected at load time with a deterministic `VerifyError` before touching SRAM budgets.

---

## 3. Cooperative VM Core with Fuel Budget

- **Issue**: KOTO-0034
- **Status**: done
- **Problem**: No runtime to actually execute KBC1 bytecode within bounded SRAM and time.
- **Diagnosis**: The RP2040 is single-core (second core reserved for audio PCM) and has no preemptive scheduler. Apps must yield cooperatively and cannot be allowed to spin the CPU for more than one frame.
- **Solution**: `BytecodeVm<STACK, CALLS>` with bounded operand stack (16 slots), bounded call depth (4), per-frame fuel budget (60,000 instructions), deterministic `VmError` traps on overflow/underflow/divide-by-zero/bad-branch, and a `VmHost` trait for host-call dispatch. `YIELD` returns control; `FuelExhausted` suspends the VM resumably.
- **Result**: Apps can draw, read input, and manage files while the shell frame loop always gets its turn. The fuel budget prevents any app from blocking the system.

---

## 4. IME Romaji-Kana Input

- **Issue**: KOTO-0015, KOTO-0071, KOTO-0100
- **Status**: done
- **Problem**: The PicoCalc keyboard is a physical thumb keyboard. Typing Japanese requires a software romaji-to-kana converter.
- **Diagnosis**: The table needed to cover vowels, all basic kana rows, youon (compound sounds), `っ` (consonant doubling), and the `n` boundary — with allocation-free design for RP2040.
- **Solution**: `RomajiKanaInput` state machine with a `ROMAJI_KANA` lookup table in `koto-core::ime`. KOTO-0071 hardened edge cases and documented all key behaviors. KOTO-0100 added nine missing youon rows (`kya/kyu/kyo`, `hya/hyu/hyo`, `mya/myu/myo`) after real-word input exposed the gaps.
- **Result**: Users can type common Japanese text including compound sounds and punctuation (`-`→`ー`, `/`→`・`, `,`→`、`, `.`→`。`) without allocation.

---

## 5. First KotoMemo in Simulator

- **Issue**: KOTO-0037, KOTO-0040, KOTO-0041
- **Status**: done
- **Problem**: The project needed a real product app to validate the full stack: VM → IME → file I/O → shell lifecycle.
- **Diagnosis**: The memo app initially ran as native Rust in KotoSim, bypassing the VM. That pattern hid gaps in the bytecode ABI and gave false confidence about device compatibility.
- **Solution**: KOTO-0037 implemented `MemoEditor` in `koto-core`; KOTO-0040 added a scripted end-to-end scenario (ASCII input → romaji/kana → SKK conversion → commit → save → exit → reload); KOTO-0041 shipped the app as a real `KBC1` bytecode program authored in `apps/memo/src/main.koto`.
- **Result**: The native-Rust memo shortcut was removed. The memo app now runs through the same VM, verifier, host-call ABI, and file sandbox that any other KotoOS app uses.

---

## 6. Koto High-Level Language and Compiler

- **Issue**: KOTO-0045, KOTO-0046
- **Status**: done
- **Problem**: Hand-authoring bytecode assembly (`.kbc.asm`) was too tedious for real apps. KotoBlocks (91 KiB bytecode) and KotoRogue (96 KiB bytecode) would have been impossible to write by hand.
- **Diagnosis**: The VM is integer-only, non-recursive, and inlines everything. The compiler had to respect those constraints rather than generating a general-purpose IR.
- **Solution**: Specified the Koto app language (KOTO-0045): `int`/`bool`/`buf`, `let`/`const`, `if`/`while`/`loop`/`break`, non-recursive `fn`, explicit `host_call` results. Implemented `koto-compiler` (KOTO-0046): lexer → parser → codegen (inlines all functions, branchless comparisons, routes `return` through scratch slot 47) → final `verify_kbc`. Source-location errors include filename, line, and column.
- **Result**: All apps and SDK samples are Koto source files. The compiler rejects recursion, undeclared names, type mismatches, and arity errors with readable diagnostics.

---

## 7. Compiler Local Slot Reuse

- **Issue**: KOTO-0092, KOTO-0104
- **Status**: done
- **Problem**: KotoBlocks reached 44/45 user local slots (the 48-slot register file minus 3 compiler scratch slots). Every new `let` and every new helper function consumed a permanent slot. Adding effects or features would overflow.
- **Diagnosis**: The compiler allocated a fresh slot per `let` and never freed it. Because all functions are inlined, helpers stacked their slots on top of `main`'s permanently.
- **Solution**: KOTO-0092: `emit_block` snapshots and restores `locals`/`next_slot` on block exit so disjoint `if`/`while` branches reuse physical slots. KOTO-0104: extended reuse across inline boundaries — each inline expansion allocates starting at the caller's current `next_slot` and releases those slots when the expansion ends.
- **Result**: KotoBlocks went from 44/45 → 41/45 (KOTO-0102) → 42/45 (KOTO-0104, with helpers), then absorbed game-feel effects to reach 43/45 with two slots still free. Memo dropped from 18/45 to 15/45.

---

## 8. Pico HAL Crate Bootstrap

- **Issue**: KOTO-0064
- **Status**: done
- **Problem**: The project had a complete simulator but no embedded crate. Adding `embassy-rp` dependencies to the host workspace would break normal `cargo test`.
- **Diagnosis**: The workspace needed to keep `koto-core` and `koto-sim` as `default-members` while adding `koto-pico` with `embassy-rp` that only compiles for `thumbv6m-none-eabi`.
- **Solution**: Added `koto-pico` as a workspace member with `embassy-rp` 0.10.0 and an RP2040 bootstrap binary. Pin assignments (`src/koto-pico/src/pins.rs`) own the fixed PicoCalc hardware map. Cargo target configuration and `memory.x` (2 MB flash, 264 KB SRAM) live inside `koto-pico`.
- **Result**: `python harness/check_all.py` continues to run on the PC without touching the embedded target. Engineers can build firmware with `cargo build -p koto-pico --target thumbv6m-none-eabi` without affecting the simulator workflow.

---

## 9. Pico LCD First Pixel

- **Issue**: KOTO-0066
- **Status**: done
- **Problem**: Unknown if the ILI9488 initialization sequence, color order, and MADCTL orientation settings were correct for the specific PicoCalc panel.
- **Diagnosis**: `KOTO-0026` had documented init profile candidates. The probe needed to disambiguate them through physical observation (corner marker colors and positions).
- **Solution**: `lcd_fill` probe initializes with `ili9488-spi` profile, `COLMOD=0x66` (RGB666), `MADCTL=0x48` (MX + BGR), at 20 MHz over SPI1. Draws four colored corner markers, centered yellow rectangle, and 8-scanline cyan DMA band at y=200.
- **Result**: Hardware observation on 2026-06-21 confirmed correct landscape orientation and RGB color order from the corner-marker pattern. Clean partial-update boundaries and continuous DMA band. No visible artifacts.

---

## 10. Pico Keyboard Over I2C

- **Issue**: KOTO-0067
- **Status**: done
- **Problem**: Unknown I2C speed, FIFO register protocol, and whether any chord would exceed the 16.667 ms frame budget.
- **Diagnosis**: Initial 10 kHz test failed the frame budget (22.2 ms stable, 44.5 ms drain frames). Root cause: unbounded FIFO drain consumed all events in one frame. Two fixes: switch to 100 kHz; bound drain to 4 events per 16 ms frame.
- **Solution**: `keyboard_i2c` probe polls FIFO register `0x09` at 100 kHz I2C, drains ≤4 events per frame, requires 5 stable samples for a chord. `keyboard_matrix` firmware stepped through all 44 required chords for each candidate.
- **Result**: All 44 `arrow-zxas` chords passed. Selection record: `{"kind":"selection","status":"pass","selected_candidate":"arrow-zxas","reason":"first_passing_candidate"}`. Three-key chord `up+left+A` stabilized in 2.126 ms. Default mapping: `arrow-zxas`.

---

## 11. Pico SD Card Read

- **Issue**: KOTO-0068
- **Status**: done
- **Problem**: Unknown if the specific TOSHIBA 8 GB SDHC card would work at the target SPI clock.
- **Diagnosis**: 12 MHz attempt returned `CardNotFound`. Cards of this class often need a slow init clock.
- **Solution**: `sd_read` probe attempts 12 MHz; on failure reconfigures the bus and retries at 1 MHz. This two-stage strategy became a permanent fixture of the product firmware (KOTO-0121).
- **Result**: Card mounted at 1 MHz, `apps/` directory listed 15 LFN manifest filenames, `kotomines.kpa.json` completed a 1,545-byte sequential read. The slow-fallback pattern later prevented the product firmware from hanging on card init.

---

## 12. Pico PSRAM Round-Trip

- **Issue**: KOTO-0069
- **Status**: done
- **Problem**: PSRAM is critical for large-app bytecode streaming. But the PIO protocol had never been tested on real hardware.
- **Diagnosis**: First run failed at byte 0. Diagnostic firmware revealed an exact one-bit left rotation per byte: `expected 5aa3ec35... actual b547d86a...`. This pointed to the high-speed "fudge" read clock in the PIO program.
- **Solution**: Removed the fudge clock. When the non-fudge phase still shifted by one bit, removed the unsampled clock before the first `in pins` instruction and passed `read_bits - 1` to the PIO loop counter.
- **Result**: All 256 bytes matched exactly at block 257. Write 2.439 ms, read 1.618 ms. Out-of-range block (index 32768) correctly returned `OutOfRange`.
- **Quote/Log**: Initial diagnostic bytes: `expected 5aa3ec35...`, `actual b547d86a...` — exact one-bit left rotation per byte.

---

## 13. KotoBlocks — First Game

- **Issue**: KOTO-0094
- **Status**: done
- **Problem**: No game app existed. The `draw_pixels_rgb565` host call was reserved but unimplemented end-to-end.
- **Diagnosis**: A game app needed: a tile blit primitive, enough heap to cache 7 tetromino tile sets (needed `2 KB→4 KB`), and enough frame fuel for a full-screen board repaint (needed `10,000→60,000`).
- **Solution**: Wired `draw_pixels_rgb565` end-to-end through the VM, simulator host, and `Canvas::blit_rgb565`. Implemented KotoBlocks in `apps/koto_blocks/src/main.koto`: 7 tetrominoes, rotation, gravity/soft/hard drop, line clears, NEXT×3, HOLD, pause, game-over.
- **Result**: First real-time game running within bounded SRAM budgets. Validates the full tile/sprite pipeline from Koto source to `draw_pixels_rgb565` blit on screen.

---

## 14. Game2D ABI Design

- **Issue**: KOTO-0097
- **Status**: done
- **Problem**: KotoBlocks did all tile caching and blitting in the VM (slow, high heap usage). A host tile API could offload that work, but the boundary wasn't defined.
- **Diagnosis**: Analyzed what the VM should own (tile names, positions, cells) versus what the host should own (pixel data, blit execution). The existing `draw_pixels_rgb565` was a building block, not the full architecture.
- **Solution**: Documented the split in `docs/GAME2D_ABI.md`. Reserved host-call IDs `0x14`–`0x1F` for `tile_define`, `tile_palette`, `draw_tile`, `tilemap_set`, `tilemap_blit`, `sprite_set`, `sprite_flush`. KotoBlocks continues using the per-cell `draw_pixels_rgb565` path until a tile cache is implemented.
- **Result**: Clear architecture for future hardware-accelerated tile rendering. The reserved IDs prevent future ABI conflicts.

---

## 15. Pico Firmware Main Loop

- **Issue**: KOTO-0117
- **Status**: done
- **Problem**: No product firmware existed. The peripheral probes were standalone experiments, not an integrated OS.
- **Diagnosis**: The firmware needed to initialize LCD, keyboard, and shell state in the correct order, run a bounded 16 ms frame loop, and convert raw STM32 key events to `koto_core::InputState` edges.
- **Solution**: `koto_firmware` binary: DMA-backed LCD init, STM32 I2C keyboard with ≤4 FIFO events per frame, `InputState` held/pressed/released edge detection, `ShellState` update with three compiled-in package fixtures, bounded card rectangles on selection change.
- **Result**: First product-firmware binary. Arrow keys moved the portable shell selection; Z/Enter confirmed with visual feedback. Proved the core/HAL boundary worked end-to-end on RP2040.

---

## 16. Pico Shell with SD Catalog

- **Issue**: KOTO-0118, KOTO-0119, KOTO-0121
- **Status**: done
- **Problem**: KOTO-0119's shell raster backend exposed a silent stack-fault in the SD catalog loader. The firmware halted before printing its first UART line.
- **Diagnosis**: `load_packages` allocated a 27 KB `PackageList` return value, 27 KB manifest scratch, and LFN buffers on the Cortex-M main stack — on top of `main`'s existing ~55 KB of large locals. The zero-initialization prologue faulted.
- **Solution**: Changed `load_packages` to fill a caller-owned `&mut PackageList`; moved scan scratch (`MANIFEST_NAMES`, `MANIFEST_BYTES`, `MANIFEST_LFN`, ~2.9 KB) to `StaticCell`. Restored two-stage SPI clock fallback.
- **Result**: UART capture confirmed: `phase=181 sd-fast-init-failed → phase=132 sd-card-init-ok clock=1000000 → phase=137 apps-list-ok manifests=15 → phase=14 catalog-ready packages=15`. Shell rendered the real catalog with no stack fault.

---

## 17. Shell Dirty-Rect Performance

- **Issue**: KOTO-0120
- **Status**: done
- **Problem**: Initial shell raster backend showed 193–339 ms per selection change. The LCD showed in-progress bands as visual noise and input appeared stalled.
- **Diagnosis**: The 16-line viewport approach reran the full shell painter ~20 times per frame. At 20 MHz SPI with per-scanline DMA setup, fixed overhead dominated: ~96 µs/row × 20 rows = ~2 ms per row, totaling far beyond the 33 ms target.
- **Solution**: (1) `ShellState::paint_rect` clips painting to a specific rectangle, skipping unchanged regions. (2) `render_selection_change` emits only the previous tile, current tile, and details pane. (3) SPI clock raised from 20 MHz to 62.5 MHz (RP2040 ceiling).
- **Result**: Same-page selection: **24 ms** at 62.5 MHz. Transfer scales as `3.125×` clock ratio (entirely SPI-clock-bound; RGB565→RGB666 conversion hides in the DMA transfer).
- **Quote/Log**: Third hardware capture (2026-06-22, SPI 62.5 MHz): `same-page select: dirty_px=17920, raster_us=~10600, transfer_us=~11550, latency_ms=24`.

---

## 18. Runtime Launch and Return on Hardware

- **Issue**: KOTO-0125
- **Status**: done
- **Problem**: Apps verified and ran in KotoSim but had never executed on RP2040. The `BytecodeSession` lifecycle was split between simulator and firmware code.
- **Diagnosis**: The two code paths had diverged. Extracting `BytecodeSession` to `koto-core` would guarantee both platforms used identical VM lifecycle code.
- **Solution**: `BytecodeSession` moved to `koto-core`: owns verification, VM construction, per-frame stepping, fuel results, exit state, traps, frame count, and open-file cleanup. KotoSim and PicoCalc provide only platform adapters. Device host implements bounded app file handles via 8.3 FAT names derived from the sandboxed app ID.
- **Result**: Actor Array, Counter, Dirty Rects, Hello Text, Input Echo all launched on device. File Note saved and reloaded text on SD. IME Playground reached `read:`/`miss:` state with physical keyboard input. Game packages correctly returned `phase=253 launch-bytecode-oversize` at the deliberate 8 KiB limit.

---

## 19. The 8 KiB Bytecode Wall and PSRAM Streaming

- **Issue**: KOTO-0127
- **Status**: done
- **Problem**: Every game package (KotoBlocks 91 KiB, KotoRogue 96 KiB, KotoShogi 73 KiB) failed launch with `phase=253 launch-bytecode-oversize`. The 8 KiB SRAM bytecode buffer was too small.
- **Diagnosis**: Investigated KBC headers: `rodata` is 0 in every app (string constants are written to the heap by codegen, not stored in rodata); the debug section is unused on device. The VM needs only the 64-byte header plus the code segment. The largest app needs 72 KiB of code — far more than any practical SRAM buffer but a tiny fraction of the 8 MiB PSRAM.
- **Solution**: Introduced `CodeSource` trait so both the verifier and interpreter can access bytecode through any source. `PsramCodeWindow` implements `CodeSource` over `PsramBlocks`, tiling the code segment through the existing 8 KiB SRAM buffer (net-zero new SRAM). `stage_app_code` streams code SD→PSRAM at launch. PSRAM-absent falls back to the SRAM path for small apps.
- **Result**: KotoBlocks (65 KiB code) staged to PSRAM and ran in real time on hardware. UART heartbeat:
```
phase=156 app-staged backing=psram code_size=65528
phase=152 app-started
phase=154 app-heartbeat frame=60  pc=6064  fuel=5219    # title screen loop
phase=154 app-heartbeat frame=300 pc=16378 fuel=26665   # gameplay loop
```
VM advancing in real time with healthy fuel and no trap. Code ceiling: 128 KiB in PSRAM.

---

## 20. Device `draw_pixels_rgb565` Support

- **Issue**: KOTO-0129
- **Status**: in-progress
- **Problem**: After KOTO-0127 proved the PSRAM bytecode pipeline, KotoBlocks launched but its game board never appeared. The title screen (using `draw_rect`/`draw_text_color`) rendered correctly; the board (using `draw_pixels`) was invisible.
- **Diagnosis**: `DeviceHost` does not implement `draw_pixels_rgb565` — it returned `HostErrorCode::UNSUPPORTED`. The app swallowed that status silently. Additionally, `MAX_APP_DRAW_COMMANDS` was only 16, far below a full board's blit count.
- **Solution**: Implemented `DeviceHost::draw_pixels_rgb565`; added `PixelsCommand` variant to `DeviceRuntimeHost`; raised `MAX_APP_DRAW_COMMANDS` from 16 to 384 (justified: a full KotoBlocks board needs ~100 blits per frame, plus safety headroom). Added `phase=157` diagnostic to confirm the blit path on device before launching the full game.
- **Result**: Device blit path implemented. Pending on-device confirmation of `phase=157` diagnostic and KotoBlocks board rendering. This is the last item before game apps are visually playable on physical PicoCalc hardware.
