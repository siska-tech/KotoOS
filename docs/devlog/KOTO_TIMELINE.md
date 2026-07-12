# KotoOS Development Timeline

## Story Arc

KotoOS began as a clean-room embedded OS for the ClockworkPi PicoCalc — a palm-sized RP2040 device with a physical keyboard, color LCD, and speaker. The project set out to build everything from scratch in `no_std` Rust: a package format, a text-rendering stack for Japanese, a cooperative bytecode VM, a full IME pipeline, and a game runtime — all fitting inside 264 KiB of SRAM with 8 MiB of PSRAM as overflow storage. The first 63 issues built a complete, working simulator (KotoSim) on the PC, accumulating a running Japanese memo editor, a polished home screen, and games including KotoBlocks and KotoRogue. Issues 64–69 then brought up every PicoCalc peripheral on real hardware through careful probe binaries. Issues 70–112 polished the IME, the memo app, the shell, and the game ecosystem in parallel. Issues 113–128 assembled those building blocks into the product firmware, achieving a hardware-validated shell with SD package loading, dirty-rect rendering at 24 ms per selection, persistent preferences, IME on device, and full runtime launch/return. The final open item — KOTO-0129 — closes the last rendering gap by bringing tile/sprite blits to the device, making game apps visually playable on physical hardware.

---

## Phase 1: Foundation (KOTO-0001–0012)

### Milestone: Rust Workspace Bootstrap
- Issue: KOTO-0001
- Status: done
- What happened: Created the initial Cargo workspace with `koto-core` (HAL traits, package primitives) and `koto-sim` (host simulator). Established that `cargo fmt`, `cargo test`, and `cargo clippy` all pass from day one.
- Why it mattered: All future work compiles against `koto-core`. The workspace boundary enforces the core rule: no platform deps in core.

### Milestone: Package Format and Sandbox Foundations
- Issue: KOTO-0004, KOTO-0007, KOTO-0009, KOTO-0011
- Status: done
- What happened: Defined the KPA archive format (KOTO-0007), implemented `SandboxPath` traversal rejection (KOTO-0004), implemented the host filesystem adapter (KOTO-0009), and unified manifest validation in core (KOTO-0011).
- Why it mattered: Every app that will ever run on KotoOS is sandboxed at the path resolver. The KPA format and manifest model are the interface between package authors and the OS.

### Milestone: Local CI Pipeline
- Issue: KOTO-0012
- Status: done
- What happened: Created `python harness/check_all.py` as the single command for the full check suite.
- Why it mattered: Kept the project's quality bar automated and consistent. Every subsequent milestone was gated through this command.

---

## Phase 2: Sim & I/O Layer (KOTO-0013–0032)

### Milestone: Japanese Bitmap Font and Text Layout
- Issue: KOTO-0013, KOTO-0014
- Status: done
- What happened: Adopted M+ BITMAP fonts; wrote `harness/mplus_to_kfont.py` to convert BDF to a compact `.kfont` blob. The `no_std` reader binary-searches a 10-byte-per-glyph index covering the full JIS X 0208 (~7000 glyphs). Text layout defines content and IME regions on the 320×320 surface.
- Why it mattered: Every rendered character on the PicoCalc passes through this font pipeline. It had to be `no_std`, allocation-free, and correct for half-width and full-width glyphs simultaneously.

### Milestone: Romaji-to-Kana IME Core
- Issue: KOTO-0015, KOTO-0016, KOTO-0017
- Status: done
- What happened: Implemented `RomajiKanaInput` (romaji→kana state machine), `StickyShift` (one-shot shift for thumb keyboards), and the `SkkIndex` dictionary strategy (leading-character index into a sorted SD-card dictionary file).
- Why it mattered: These three pieces together define how the PicoCalc's thumb keyboard inputs Japanese text. The strategy avoids allocating a full dictionary in SRAM.

### Milestone: Runtime VM Selection
- Issue: KOTO-0018
- Status: done
- What happened: Compared Wasm (too heavy), Lua (GC), mruby (GC + object dispatch), and a custom stack VM. Chose the custom KBC1 VM: integer-first opcodes, bounded stack/calls, no GC, no threads, host-call dispatch.
- Why it mattered: A wrong choice here would have invalidated the whole architecture. The custom VM fit 264 KiB SRAM with room for the shell, font, IME, and SD/LCD drivers alongside it.

### Milestone: First Visual Shell Pixels
- Issue: KOTO-0031, KOTO-0032
- Status: done
- What happened: Added `Canvas` RGB565 rasterizer and `ShellState::paint` to KotoSim (KOTO-0031). Then added a live `minifb` window with real-time framebuffer and PC keyboard input (KOTO-0032).
- Why it mattered: The project went from text-output CI tests to a live interactive 320×320 window showing the shell. The `Canvas` rasterizer later became the foundation for the device raster backend.

---

## Phase 3: Runtime VM (KOTO-0033–0050)

### Milestone: KBC1 Bytecode Verifier
- Issue: KOTO-0033
- Status: done
- What happened: Implemented the allocation-free KBC1 verifier: header magic, bounds, resource limits against `RuntimeLimits`, opcode coverage, branch target validity, stack-underflow detection, and host-call ID checks.
- Why it mattered: Security and safety boundary for all app code. Malformed or malicious bytecode is rejected at load time before the VM ever executes a single instruction.

### Milestone: Cooperative Bytecode VM Core
- Issue: KOTO-0034
- Status: done
- What happened: Implemented `BytecodeVm` with bounded operand stack, bounded call depth, per-frame fuel budget, `YIELD` cooperative scheduling, deterministic `VmError` traps, and `VmHost` dispatch. Apps run until they `YIELD` or exhaust fuel, then return control to the host.
- Why it mattered: The VM is the execution heart of KotoOS. Its bounded design (fuel = 60,000 instructions/frame) ensures the shell always gets its frame budget regardless of app behavior.

### Milestone: First Memo App in Simulator
- Issue: KOTO-0037, KOTO-0038, KOTO-0040
- Status: done
- What happened: Implemented `MemoEditor` in `koto-core` (KOTO-0037), connected `KotoMemoIme` composition to the editor (KOTO-0038), and validated end-to-end scripted memo: launch → ASCII input → romaji/kana → SKK conversion → commit → save → exit → reload (KOTO-0040).
- Why it mattered: Proved the stack from VM to IME to file I/O in one runnable app. This was the first product-level demonstration of KotoOS functioning as a Japanese PDA.

### Milestone: Koto High-Level Language and Compiler
- Issue: KOTO-0045, KOTO-0046
- Status: done
- What happened: Specified the Koto app language: `int`/`bool`/`buf`, `let`/`const`, `if`/`while`/`loop`, non-recursive `fn`, explicit host calls (KOTO-0045). Implemented the compiler from Koto source to KBC1: lexer, parser, codegen with function inlining, branchless comparisons, and final `verify_kbc` guarantee (KOTO-0046).
- Why it mattered: Apps no longer needed to be written in hand-assembled bytecode. The memo app, KotoBlocks, KotoRogue, and all SDK samples are Koto source files compiled ahead-of-time on the host PC.

### Milestone: Runtime Inspector and Development Experience
- Issue: KOTO-0049, KOTO-0050
- Status: done
- What happened: Added `--app APP_ID` direct launch, `InspectorReport` exposing VM state/fuel/host-calls/draw-counts, source-location diagnostics, and scripted scenario runner.
- Why it mattered: Turned the runtime from a black box into a transparent development tool. Bugs could be identified by app ID, bytecode PC, host-call name, and source line.

---

## Phase 4: App Ecosystem (KOTO-0051–0063)

### Milestone: Full App Build Pipeline
- Issue: KOTO-0048, KOTO-0051, KOTO-0052, KOTO-0053
- Status: done
- What happened: Created `apps/apps.json` registry, `harness/build_apps.py` rebuild command, in-band `KDBG` debug section for source-to-PC mapping, six SDK sample apps, and `koto-app-scaffold` CLI.
- Why it mattered: New apps could be created, compiled, tested, and validated in one documented workflow. The scaffold tool meant "add a new app" was a one-command operation.

### Milestone: Simulator Profile Unification
- Issue: KOTO-0060
- Status: done
- What happened: Unified verifier and VM construction behind `RuntimeLimits::simulator_default()` (stack 16, calls 4, heap 4 KB, fuel 60,000). Fixed a bug where the verifier was more permissive than the VM.
- Why it mattered: The accepted limits now match what the VM enforces. An app that passes the verifier is guaranteed to construct a valid VM session.

---

## Phase 5: Pico Bring-Up (KOTO-0064–0069)

### Milestone: Pico HAL Crate and First Firmware
- Issue: KOTO-0064, KOTO-0065
- Status: done
- What happened: Added `koto-pico` workspace member with `embassy-rp`. Flashed `blink_cdc` firmware via BOOTSEL, confirmed GP25 LED blink, and observed USB-CDC banner in Tera Term: `KotoOS KOTO-0065 blink+cdc v0.1.0`.
- Why it mattered: Proved the embedded toolchain worked end-to-end on real hardware before touching any peripheral.

### Milestone: Pico LCD First Pixel
- Issue: KOTO-0066
- Status: done
- What happened: Validated ILI9488 `ili9488-spi` profile at 20 MHz, RGB666 (`COLMOD=0x66`), `MADCTL=0x48`. Full-screen fills, bounded rectangle writes, and DMA scanline band all passed on hardware. Correct landscape orientation and RGB color order confirmed by corner-marker pattern.
- Why it mattered: The display had to work before any visual shell could run on device.

### Milestone: Pico Keyboard Over I2C
- Issue: KOTO-0067
- Status: done
- What happened: Validated STM32 keyboard bridge at 100 kHz I2C with bounded FIFO drain (≤4 events per 16 ms frame). All 44 `arrow-zxas` chord cases passed. JSONL selection record: `{"kind":"selection","status":"pass","selected_candidate":"arrow-zxas","reason":"first_passing_candidate"}`.
- Why it mattered: Every key event in the product firmware depends on this path. The bounded FIFO drain prevents any single burst from blocking the frame loop.

### Milestone: Pico SD Card Read
- Issue: KOTO-0068
- Status: done
- What happened: Mounted a TOSHIBA 8 GB SDHC card over SPI0 (12 MHz failed → 1 MHz fallback). Listed 15 LFN manifest filenames from `apps/`. Completed 1,545-byte sequential manifest read. The clock fallback strategy became a permanent part of the product firmware.
- Why it mattered: Without SD access the device can only show compiled-in fixture packages.

### Milestone: Pico PSRAM Round-Trip
- Issue: KOTO-0069
- Status: done
- What happened: After diagnosing a one-bit left rotation per byte (`5a→b5, a3→47, ec→d9`) caused by an extra PIO clock, fixed the PIO read phase. Final result: 256-byte block round-trip with exact equality at block 257. Write 2.44 ms, read 1.62 ms.
- Why it mattered: PSRAM is the key to running large apps (>8 KiB bytecode). Without a correct block API, game apps could never launch.

---

## Phase 6: Memo & IME Polish (KOTO-0070–0093)

### Milestone: Full Memo UI and Light Theme
- Issue: KOTO-0072, KOTO-0074, KOTO-0088
- Status: done
- What happened: Added visible caret with half/full-width metrics (KOTO-0075), scrollbar (KOTO-0076), title bar with save badge, framed document area, bottom command bar (KOTO-0074), and the full light theme: navy bars, white document area, pale-blue IME panel, Japanese command labels, `保存済`/`未保存` badges (KOTO-0088).
- Why it mattered: KotoMemo changed from raw text on a black screen to a recognizable PDA-style Japanese text editor.

### Milestone: Shell Visual Redesign
- Issue: KOTO-0081, KOTO-0082, KOTO-0083, KOTO-0084, KOTO-0085, KOTO-0086, KOTO-0087
- Status: done
- What happened: Redesigned KotoShell as a visual home screen: paged icon grid, toggleable details pane, system status bar (battery, SD, clock), truthful command bar with key-chip entries, favorites/sort/category with persistence, and eight drawn icon kinds.
- Why it mattered: KotoShell became the PDA home screen it was designed to be, matching the visual target that the PicoCalc hardware would eventually display.

### Milestone: Compiler Local Slot Reuse
- Issue: KOTO-0092, KOTO-0104
- Status: done
- What happened: `emit_block` restores `locals`/`next_slot` on scope exit so disjoint blocks reuse physical slots (KOTO-0092). Extended reuse across inline boundaries so helper functions share slots above the caller's live locals (KOTO-0104). KotoBlocks went from 44/45 to 42/45 user slots.
- Why it mattered: Without slot reuse, every new `let` and every new helper function consumed a permanent slot. The 48-slot VM register file would have been exhausted long before the apps matured.

---

## Phase 7: Games & Language (KOTO-0094–0112)

### Milestone: KotoBlocks — First Game
- Issue: KOTO-0094
- Status: done
- What happened: Shipped KotoBlocks: Tetris-style game with 7 tetrominoes, rotation, gravity, scoring, NEXT×3, HOLD, pause, game over. Tiles rendered as 16×16 RGB565 blits using `draw_pixels_rgb565`. Required raising simulator heap 2 KB→4 KB and frame fuel 10,000→60,000.
- Why it mattered: First proof that KotoOS could run a real-time interactive game within bounded SRAM budgets. Also validated `draw_pixels_rgb565` end-to-end through the VM and simulator.

### Milestone: Game2D ABI Design
- Issue: KOTO-0097
- Status: done
- What happened: Documented the host/app responsibility split for tile rendering: the VM names tiles and positions; the host turns them into pixels. Reserved ABI IDs `0x14`–`0x1F` for future `tile_define`, `draw_tile`, `tilemap_blit`, `sprite_flush`.
- Why it mattered: Established the architecture for future hardware-accelerated tile rendering without breaking the portable app model.

### Milestone: KotoMML Multi-Track Audio
- Issue: KOTO-0095, KOTO-0098
- Status: done
- What happened: Wired `audio_submit_i16` through the runtime and compiler (KOTO-0095); added `#TRACK` markers for up to 4 simultaneous voices in one score (KOTO-0098); KotoBlocks BGM updated to lead+bass+drum.
- Why it mattered: KotoOS could now play game music. The deterministic headless `--audio` capture path kept CI reliable.

### Milestone: KotoRogue and Sprite Asset Pipeline
- Issue: KOTO-0113, KOTO-0116
- Status: done
- What happened: Shipped KotoRogue (procedural dungeon crawl with fog of war, bump combat, depth progression) and the `asset_load` host call with `.kspr`→`KIM1` RGB565 pipeline. KotoRogue loads animated tiles from its package at startup.
- Why it mattered: First KotoOS app to use real sprite assets instead of procedural `draw_rect` shapes. Validated the full asset pipeline from `.kspr` ASCII art to hardware blit.

---

## Phase 8: Pico Firmware (KOTO-0113–0128)

### Milestone: Pico Firmware Main Loop
- Issue: KOTO-0117
- Status: done
- What happened: Added `koto_firmware` binary: DMA-backed LCD, STM32 keyboard, normalized `InputState` edges, `ShellState` driven on 16 ms frame cadence with three compiled-in fixture packages.
- Why it mattered: First product-firmware binary. Proved that the core/HAL boundary worked on RP2040 before adding SD or real packages.

### Milestone: SD Catalog on Device
- Issue: KOTO-0118, KOTO-0121
- Status: done
- What happened: Added FAT SD enumeration with two-stage clock fallback. Discovered and fixed a 27 KB stack-frame fault in `load_packages` (KOTO-0121) by moving scan scratch to `StaticCell`. UART capture confirmed: `phase=137 apps-list-ok manifests=15 → phase=14 catalog-ready packages=15`.
- Why it mattered: The physical SD card drives the package list. Without this fix, the firmware faulted silently before its first UART line whenever the catalog loader was called.

### Milestone: Shell Raster Backend and Dirty-Rect Performance
- Issue: KOTO-0119, KOTO-0120
- Status: done
- What happened: `Canvas::new_viewport` rasterizes into bounded horizontal strips; `paint_rect` clip avoids rerunning the full painter. After initial 193–339 ms measurements, raised SPI from 20 MHz to 62.5 MHz (RP2040 ceiling). Third hardware capture: same-page selection **24 ms**.

Notable hardware timing captures (KOTO-0120, 2026-06-22, SPI 62.5 MHz):

| Redraw             | dirty_px | raster_us | transfer_us | latency_ms |
|--------------------|----------|-----------|-------------|------------|
| First full         | 102,400  | 41,091    | 64,948      | 106        |
| Same-page select   | 17,920   | ~10,600   | ~11,550     | 24         |
| Pane-shown select  | 48,632   | ~33,300   | ~31,290     | 66         |

- Why it mattered: Shell interaction had to feel responsive. The transition from 193 ms to 24 ms per selection was the difference between a usable device and an unusable one.

### Milestone: Runtime Launch and Return
- Issue: KOTO-0125
- Status: done
- What happened: `BytecodeSession` extracted to `koto-core`; both KotoSim and PicoCalc delegate VM lifecycle to it. SDK sample apps (Actor Array, Counter, Dirty Rects, Hello Text, Input Echo) launched on device. File Note saved/read on SD. IME Playground reached `read:`/`miss:`. Game packages failed with `phase=253 launch-bytecode-oversize` (8 KiB limit) — tracked as KOTO-0127.
- Why it mattered: First apps actually ran on hardware. The shared `BytecodeSession` contract meant the same code paths tested in KotoSim were exercised on the RP2040.

### Milestone: The 8 KiB Bytecode Wall and PSRAM Streaming
- Issue: KOTO-0127
- Status: done
- What happened: Introduced `CodeSource` trait; `PsramCodeWindow` streams code from PSRAM through an 8 KiB SRAM cache (net-zero new SRAM — repurposed the old bytecode buffer). UART heartbeat confirmed KotoBlocks (65 KiB code) running in real time from PSRAM:
```
phase=156 app-staged backing=psram code_size=65528
phase=152 app-started
phase=154 app-heartbeat frame=180 pc=6064  fuel=5218    # title screen loop
phase=154 app-heartbeat frame=300 pc=16378 fuel=26665   # gameplay loop
```
- Why it mattered: Without PSRAM streaming, game apps (20–96 KiB bytecode) could never launch. The 128 KiB PSRAM code ceiling (tiny fraction of the 8 MiB PSRAM) covers all current apps with headroom.

### Milestone: App Frame Flicker Fix
- Issue: KOTO-0128
- Status: done
- What happened: `present_app_delta` now uses black as the implicit baseline when no full-screen base rect is present. Changed regions are composited off-screen in the existing `RASTER_STRIP` and transferred in one DMA, eliminating the erase-then-redraw intermediate. Dirty Rects sample confirmed flicker-free on physical hardware (2026-06-22).
- Why it mattered: Without this fix, partial-background animations showed visible black flicker on every frame. The fix required no new SRAM — it reused the existing strip.

---

## Phase 9: PSRAM & Hardware Final (KOTO-0127–0129)

### Milestone: Device `draw_pixels_rgb565` Support
- Issue: KOTO-0129
- Status: in-progress
- What happened: Implemented `DeviceHost::draw_pixels_rgb565`; added `PixelsCommand` to `DeviceRuntimeHost`; raised `MAX_APP_DRAW_COMMANDS` 16→384; added `phase=157` diagnostic pixel blit. Pending on-device confirmation for KotoBlocks board rendering.
- Why it mattered: Without tile/sprite blits on device, KotoBlocks' board never appeared — the VM ran, the title drew (via `draw_rect`), but the game board was invisible. This closes the last major rendering gap.
