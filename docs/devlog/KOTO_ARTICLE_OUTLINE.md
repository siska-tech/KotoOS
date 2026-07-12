# KotoOS Article Series: Outline

## Series Overview

A nine-article series covering the design, implementation, and hardware bring-up of KotoOS — a `no_std` Rust PDA-like operating system for the ClockworkPi PicoCalc (RP2040). The series progresses from motivation and architecture through the complete development arc: simulator, Japanese IME, custom bytecode VM, the PSRAM breakthrough for large apps, and the final hardware bring-up. Each article stands alone but builds on the previous one for readers following the full series.

Target readers: embedded Rust developers, hobby OS builders, Japanese text-input enthusiasts, and PicoCalc/RP2040 hardware hackers.

---

## Article 1: "Why I Built a PDA-Like OS for PicoCalc"

### Audience
Hardware hobbyists and embedded developers curious about motivation; readers who need context before implementation details.

### Hook / Opening
The PicoCalc sits on your desk: physical keyboard, color LCD, speaker. It looks like a toy but it's capable of running a real OS. What would it take to give it one — built from scratch in Rust, in Japanese, without using an RTOS or libc?

### Key Sections

- **The PicoCalc hardware** — What makes it interesting: RP2040 (133 MHz, 264 KiB SRAM), 8 MiB PSRAM, ILI9488 320×320 LCD, STM32 keyboard bridge over I2C, SD card, PWM speaker. Draw from `docs/RP2040_BRINGUP.md` and `docs/ARCHITECTURE.md`.
- **The vision** — A Japanese PDA: memo editor with romaji-kana IME, home screen launcher, small games, everything under 264 KiB. Why Japanese specifically (thumb keyboard, the author's daily writing language).
- **Why not use an existing embedded OS** — RTOS overhead, C FFI complexity, the desire for a completely auditable `no_std` Rust stack. Draw from KOTO-0018 (VM selection) and `docs/RUNTIME_VM_SELECTION.md`.
- **The design constraints** — No full-screen framebuffer (307,200 bytes); PSRAM is block-transfer only; no heap allocator in core; bounded everything (stack 16, calls 4, fuel 60,000). These constraints shape every subsequent decision.
- **Project structure** — `koto-core` (no_std) / `koto-sim` (PC simulator) / `koto-pico` (RP2040) / `koto-tools` (compiler, assembler). Draw from `README.md` and `docs/ARCHITECTURE.md`.

### Closing
What this series will cover: simulator → IME → VM → hardware. Preview of the PSRAM breakthrough and the 24 ms shell interaction measurement.

### Suitable for
- note.com: **yes** — personal motivation and project overview suit Japanese tech blogs well
- GitHub README: **yes** — this is essentially the expanded README; could replace or supplement it

### Screenshots/Logs to Include
- PicoCalc hardware photo or schematic overview
- KotoSim window showing the home screen (shell icon grid)
- Architecture diagram from `docs/ARCHITECTURE.md`

---

## Article 2: "Shell and Package Loading"

### Audience
Rust embedded developers interested in UI architecture on constrained hardware; readers curious how a launcher works without a heap allocator.

### Hook / Opening
How do you build a home screen for a device with 264 KiB of RAM and no OS primitives? The answer is dirty rectangles, a fixed-capacity package list, and a font that covers 7000 Japanese glyphs in 246 KiB.

### Key Sections

- **The KPA package format** — `.kpa` archive layout: header, table, asset ordering, sequential-read constraints. How manifests describe apps (app ID, name, description, category, icon theme, memory request). Draw from KOTO-0007, KOTO-0011, KOTO-0091 and `docs/KPA_FORMAT.md`.
- **KotoShell visual design** — From a text list (KOTO-0010) to an icon grid (KOTO-0082) to pane-shown/pane-hidden layout (KOTO-0083). The light theme palette. The command bar system. Draw from KOTO-0081 through KOTO-0087.
- **The M+ bitmap font and glyph model** — Why M+ BITMAP, how `mplus_to_kfont.py` converts BDF to `.kfont`, the binary-search index, half-width vs. full-width cell metrics. Draw from KOTO-0013, `docs/ARCHITECTURE.md`.
- **Dirty rectangles on a device with no framebuffer** — The constraint: 307,200 bytes for a full screen, 264 KiB total SRAM. The solution: `Canvas::new_viewport`, `ShellState::paint_rect`, `render_selection_change`. Only the changed tiles are rasterized and transferred. Draw from KOTO-0005, KOTO-0031, KOTO-0120.
- **SD package loading in a bounded stack** — The `load_packages` stack-fault story (KOTO-0121): 27 KB return value on a Cortex-M stack = silent fault. Fix: caller-owned `PackageList`, `StaticCell` scratch, two-stage SPI clock fallback.

### Closing
With the shell running and packages loading from SD, we need something to launch. The next article covers the Japanese text stack.

### Suitable for
- note.com: **yes** — the dirty-rect and fixed-stack stories are concrete and accessible
- GitHub README: **partial** — the KPA format and shell architecture sections could become dedicated docs

### Screenshots/Logs to Include
- KotoSim window showing the icon grid with details pane
- UART capture: `phase=137 apps-list-ok manifests=15 → phase=14 catalog-ready packages=15`
- Screenshot of the light-theme shell with Japanese app names

---

## Article 3: "Japanese Text and KotoMemo"

### Audience
Developers interested in Japanese text input on embedded hardware; anyone who has wondered how IME works at the firmware level.

### Hook / Opening
Typing Japanese on a physical thumb keyboard with 12 keys requires converting romaji keystrokes into kana syllables in real time, then optionally converting kana readings to kanji candidates from a dictionary. All of this must fit in under 1 KiB of SRAM for the state machine.

### Key Sections

- **The IME stack** — Three layers: `RomajiKanaInput` (syllable composer), `StickyShift` (one-shot shift for thumb keyboards), `SkkSession` (dictionary lookup). How they compose. Draw from KOTO-0015, KOTO-0016, KOTO-0017, KOTO-0038.
- **The SKK dictionary strategy** — SD-card-friendly: sorted UTF-8 file + SRAM-resident leading-character index. Binary-search the index, seek to the byte offset, scan forward. Avoids loading the entire dictionary into SRAM. Draw from KOTO-0017, `docs/SKK_DICTIONARY.md`.
- **Youon and the missing syllables** — A bug report story: users couldn't type `きょう` (today) or `ひゃく` (hundred) because `kya/hya/mya` rows were missing from the table (KOTO-0100). The fix was adding nine table entries. The `kw` prefix story (KOTO-0105): the test expected `kw` to fail but the table already supports `kwa`/`kwi`/`kwe`/`kwo`.
- **KotoMemo: from black screen to PDA editor** — The evolution across KOTO-0072, KOTO-0074, KOTO-0088: caret with font metrics, scrollbar, title bar, light theme (navy bars, white document, pale-blue IME panel), Japanese command labels, `保存済`/`未保存` save badges.
- **Inline IME composition** — The original large bottom panel (KOTO-0077) moved to inline display at the caret (KOTO-0106, KOTO-0107): composition text shifts following text right, candidate shows at cursor position, `候補 n/m` in the status bar. The overlap-avoidance solution (KOTO-0099): `edit_reserve_rows` keeps the cursor scrolled above the overlay band.

### Closing
With Japanese text input working end-to-end, the next question was how to make it programmable — introducing the bytecode VM.

### Suitable for
- note.com: **yes** — the IME design and the youon bug story are compelling for Japanese-language audiences
- GitHub README: **partial** — the IME architecture could become a dedicated `docs/IME.md`

### Screenshots/Logs to Include
- KotoMemo with the light theme showing Japanese text and the IME conversion panel
- Screenshot of inline candidate display at the caret
- Romaji-kana table excerpt showing the youon rows

---

## Article 4: "Building the KotoRuntime VM"

### Audience
Embedded Rust developers interested in custom language runtimes; readers curious why not to use Wasm or Lua on RP2040.

### Hook / Opening
Why write a custom bytecode VM when Wasm and Lua exist? On a device with 264 KiB of SRAM, the interpreter itself has to fit in a few kilobytes. Every existing option was too heavy. Here is what it took to build one that is small, deterministic, and safe.

### Key Sections

- **Why not Wasm/Lua/mruby** — The comparison table from `docs/RUNTIME_VM_SELECTION.md`: Wasm (30–60 KiB interpreter alone), Lua (GC + string interning), mruby (GC + object model). None pass the no-allocator test. Draw from KOTO-0018.
- **KBC1 format and the verifier** — The KBC1 bytecode format: 64-byte header, code segment, heap request, debug section. The allocation-free one-pass verifier: header magic, bounds, resource limits, opcode coverage, branch targets, stack-underflow detection, host-call ID membership. Draw from KOTO-0033, `docs/RUNTIME_BYTECODE_ABI.md`.
- **The cooperative fuel budget** — YIELD returns control; FuelExhausted suspends the VM resumably. The frame budget: 60,000 instructions per 16 ms frame. Why 60,000: KotoBlocks hard-drop frame measured at 44,277 fuel. Draw from KOTO-0034, KOTO-0060, KOTO-0101.
- **The Koto language compiler** — From Koto source to KBC1: lexer → parser → codegen (inlines all functions, branchless comparisons, `push_i16` for immediates, scratch slot 47 for return values). Why function inlining was the right choice given the VM's shared local-file model. Draw from KOTO-0045, KOTO-0046.
- **VM budget diagnostics** — `check_budgets.py`, the `--budget` CLI, high-water marks. The 100% local warning on KotoBlocks (48/48) before KOTO-0092 fixed it. What the numbers mean. Draw from KOTO-0101.
- **BytecodeSession extraction** — Why moving `BytecodeSession` to `koto-core` mattered: KotoSim and PicoCalc now share one VM lifecycle, eliminating divergence bugs. Draw from KOTO-0125.

### Closing
The VM runs apps — but apps bigger than 8 KiB of SRAM can't fit in the bytecode buffer. The next article covers the memory wall.

### Suitable for
- note.com: **yes** — the "why not Wasm" framing is engaging for a broad developer audience
- GitHub README: **no** — too detailed for a README; better as a standalone article or blog post

### Screenshots/Logs to Include
- KBC1 header format diagram
- Budget report output: `local_peak=48/48 heap_peak=3981/24576 fuel_peak=44277/60000`
- Compiler local-slot map output for KotoBlocks

---

## Article 5: "The 8 KiB Bytecode Wall"

### Audience
Embedded developers who have hit SRAM limits; readers interested in how to diagnose and architect around memory constraints.

### Hook / Opening
Every game app — KotoBlocks (91 KiB), KotoRogue (96 KiB), KotoShogi (73 KiB) — failed launch with the same error: `phase=253 launch-bytecode-oversize`. The 8 KiB SRAM bytecode buffer was the wall. But the solution wasn't to make the buffer bigger.

### Key Sections

- **The bytecode size problem** — Table of committed bytecode sizes: SDK samples (0.7–4.9 KiB), memo/sokoban (20–22 KiB), kotomines/kotosnake/kotorun (30–40 KiB), kotoshogi (73 KiB), koto_blocks (91 KiB), kotorogue (96 KiB). Why simply enlarging the buffer doesn't work: games also need larger heaps, and SRAM is already at 264 KiB total. Draw from KOTO-0127 context.
- **Investigating the KBC headers** — Key insight: `rodata = 0` in every app (string constants live in the heap via codegen, not in a rodata section). Debug section unused on device. The VM needs only the 64-byte header plus the code segment. The real constraint is code size alone.
- **The PSRAM architecture** — Why PSRAM is block-transfer only: the 8 MiB is not pointer-addressable from SRAM. The `PsramBlocks` API (256-byte aligned block transfers, out-of-range rejection). The `PsramCodeWindow` idea: tile the code segment through an existing 8 KiB SRAM buffer. Net-zero new SRAM. Draw from KOTO-0022, KOTO-0069, `docs/ARCHITECTURE.md`.
- **The `CodeSource` trait** — Making the verifier and interpreter generic over their byte source: `SliceCode` for KotoSim/tools/small apps; `PsramCodeWindow` for large device apps. One `BytecodeSession` covers both. Draw from KOTO-0127, `docs/RUNTIME_VM_SELECTION.md`.
- **Deliberate budget sizing** — Code window: 8 KiB (repurposed, net-zero). Code ceiling: 128 KiB (above largest app, tiny fraction of 8 MiB PSRAM). App heap ceiling: 16 KiB (largest app requests 10.8 KiB). Document the reasoning, not just the numbers.

### Closing
The mechanism exists — but does it work on real hardware? The next article covers the hardware validation.

### Suitable for
- note.com: **yes** — the "wall and solution" narrative is compelling and the budget sizing is concrete
- GitHub README: **no** — too implementation-specific; belongs in a dedicated doc or blog post

### Screenshots/Logs to Include
- Table: package names vs. bytecode sizes
- `stage_app_code` flowchart (code size gate → PSRAM load or SRAM fallback)
- UART trace showing `phase=253 launch-bytecode-oversize` before the fix

---

## Article 6: "PSRAM-Backed Bytecode Streaming"

### Audience
RP2040/PIO enthusiasts; embedded Rust developers interested in using PSRAM as program storage.

### Hook / Opening
The PSRAM probe took three firmware revisions to get right. The first run showed data that had been bit-rotated: every byte shifted left by one. Here is how a one-bit error in a PIO program was diagnosed from 16 bytes of UART output.

### Key Sections

- **PSRAM probe bring-up and the one-bit rotation** — First run: fail at byte 0. Diagnostic firmware logged first 16 expected and actual bytes: `expected 5aa3ec35...`, `actual b547d86a...` — exact one-bit left rotation per byte. Root cause: the "fudge" high-speed read clock sampled MISO one extra clock early. Fix: remove the unsampled clock before first `in pins`, pass `read_bits - 1` to the PIO counter. Draw from KOTO-0069, `docs/PICO_HARDWARE_LOG.md`.
- **PIO protocol details** — PIO1 at ~16.6 MHz serial clock, 16-byte FIFO transactions, 256-byte block API. The KOTO-0069 timing measurements: write 2.44 ms, read 1.62 ms per 256-byte block.
- **`stage_app_code`: SD → PSRAM at launch** — How the firmware streams code: read KBC header from SD, budget-gate against `DEVICE_CODE_CEILING` (128 KiB) and heap ceiling, stream code segment SD→PSRAM block by block, construct `PsramCodeWindow` at base 0. Log `phase=156 app-staged backing=psram code_size=N`.
- **Hardware validation of PSRAM streaming** — The UART heartbeat log from KotoBlocks launch:
```
phase=156 app-staged backing=psram code_size=65528
phase=152 app-started
phase=154 app-heartbeat frame=60  pc=6064  fuel=5219
phase=154 app-heartbeat frame=180 pc=6064  fuel=5218
phase=154 app-heartbeat frame=240 pc=16378 fuel=28174
phase=154 app-heartbeat frame=300 pc=16378 fuel=26665
```
Interpreting the trace: stable PC in the title loop, then advancing to `pc=16378` in the gameplay loop. The `fuel` values prove the VM is running correctly.
- **What PSRAM streaming enables** — All game apps (20–96 KiB) can now launch. The app heap ceiling (16 KiB, per-app from KBC header) stays in SRAM. PSRAM holds code; SRAM holds execution state.

### Closing
The VM runs — but the games don't render their boards yet. The final hardware rendering gap is the subject of the next article.

### Suitable for
- note.com: **yes** — the PIO debugging story and UART trace are concrete and visual
- GitHub README: **no** — too technical; belongs in `docs/PICO_HARDWARE_LOG.md` or a blog post

### Screenshots/Logs to Include
- The bit-rotation diagnostic: `expected 5aa3ec35... actual b547d86a...`
- Full UART trace from KOTO-0127 hardware validation
- PIO program diagram or pseudocode showing the fix

---

## Article 7: "Game2D and draw_pixels Bring-Up"

### Audience
Embedded game developers; RP2040 graphics enthusiasts; readers interested in tile-based game design under memory constraints.

### Hook / Opening
KotoBlocks launched and the VM ran — but the game board was invisible. The title screen appeared (it uses `draw_rect` and `draw_text_color`) while the board never blitted. The VM was running correctly; the host simply didn't know how to draw tiles on the device.

### Key Sections

- **KotoBlocks design** — 7 tetrominoes, 16×16 RGB565 tile blits, tile cache baked once into app heap (7 tiles × 256 bytes = 1.8 KiB), per-cell `draw_pixels_rgb565` call. Why the tile cache: baking avoids recalculating colors per frame. Draw from KOTO-0094.
- **The tile/sprite model and Game2D ABI** — `draw_pixels_rgb565(x, y, w, h, ptr, len)`: the app passes a heap pointer to little-endian RGB565 bytes, the host blits them. The Game2D ABI design (KOTO-0097): reserved call IDs for future `tile_define`, `draw_tile`, `tilemap_blit`, `sprite_flush`. Why KotoBlocks keeps its in-app tile cache for now.
- **KotoRogue and the `.kim` sprite pipeline** — `.kspr` ASCII sprite sheet (reviewable in the repo) compiled to `KIM1` format (magic + u16 width/height + RGB565 rows). `asset_load` host call copies from package into app heap. Animated tiles: 2 frames per entity, selected by `ST_ANIM` counter. Draw from KOTO-0113, KOTO-0116.
- **The device rendering gap** — `DeviceHost` returned `UNSUPPORTED` for `draw_pixels_rgb565`. The app swallowed the error silently, so the game appeared to run but blitted nothing. `MAX_APP_DRAW_COMMANDS = 16` was also far below a full board. Draw from KOTO-0129 context.
- **Implementing device tile blits** — `DeviceHost::draw_pixels_rgb565`: decodes heap offset from the resolved slice pointer, stores a `PixelsCommand` referencing the heap bytes. At compose time: re-read from app heap into `RASTER_STRIP`, convert RGB565→RGB666, DMA transfer. `MAX_APP_DRAW_COMMANDS` raised from 16 to 384 (justified: full KotoBlocks board ~100 blits). Draw from KOTO-0129.
- **The flicker fix and compositing model** — KOTO-0128: `present_app_delta` composites each changed region off-screen into the existing `RASTER_STRIP` before writing to GRAM. This eliminated the visible black erase-and-redraw artifact that appeared with partial-background apps (Dirty Rects sample).

### Closing
With tile blits working on device, KotoBlocks becomes visually playable on physical PicoCalc hardware — the last major gap closed.

### Suitable for
- note.com: **yes** — the "invisible game board" mystery and the compositing model are engaging
- GitHub README: **no** — too detailed; belongs in `docs/GAME2D_ABI.md` or a standalone article

### Screenshots/Logs to Include
- KotoSim screenshot: KotoBlocks board rendering with tiles
- KotoRogue screenshot: animated tile sprites in the dungeon
- UART trace showing `phase=157` pixel diagnostic before full board render

---

## Article 8: "Refactoring the Firmware Monolith"

### Audience
Embedded software architects; Rust developers who have experienced the "one big file" anti-pattern; readers interested in how to split an embedded codebase without breaking hardware validation.

### Hook / Opening
By the time the shell parity validation (KOTO-0126) was complete, `koto_firmware.rs` had grown to encompass LCD init, keyboard polling, SD catalog loading, shell state, runtime session, host-call dispatch, and PSRAM management. KOTO-0125's `BytecodeSession` extraction was the first split. Here is the story of the whole bring-up firmware architecture.

### Key Sections

- **The probe binary architecture** — Eight standalone probe binaries (`blink_cdc`, `lcd_fill`, `keyboard_i2c`, `sd_read`, `psram_roundtrip`, `pwm_audio`, `battery_power`) validate each peripheral independently. Each has its own `--bin` entry. The bringup archive holds historical binaries that were superseded. Draw from `docs/RP2040_BRINGUP.md`, KOTO-0065 through KOTO-0069, KOTO-0114, KOTO-0115.
- **The KOTO-0117 main loop** — First `koto_firmware`: LCD + keyboard + fixed package fixtures. The simplest possible firmware that proved the HAL boundary. Draw from KOTO-0117.
- **Incremental integration** — KOTO-0118 (SD catalog), KOTO-0119 (raster backend), KOTO-0120 (dirty-rect performance), KOTO-0121 (stack-fault fix), KOTO-0122 (icons), KOTO-0123 (preferences), KOTO-0124 (system status), KOTO-0125 (runtime launch). Each issue added one system to the firmware without touching others.
- **`BytecodeSession` extraction to `koto-core`** — The most impactful refactor: VM lifecycle code shared between KotoSim and PicoCalc. Before: divergent code paths that could silently disagree. After: one `BytecodeSession`, one `VmHost` trait, platform adapters only at the edges. Draw from KOTO-0125.
- **KotoSim module split** — The parallel story on the simulator side (KOTO-0061): `koto-sim/lib.rs` split into 14 focused modules (`host.rs`, `session.rs`, `render.rs`, `scenario.rs`, etc.). Same principle: mechanical moves first, no behavior changes, verified by CI.
- **Phase codes as architecture documentation** — Every firmware milestone emits a UART phase code (`phase=14 catalog-ready`, `phase=131 shell-init`, `phase=152 app-started`). These aren't just debugging aids — they document the expected boot sequence and the meaning of each state transition.

### Closing
The firmware is now a well-factored system with shared core logic, platform adapters, and a clear boot sequence documented through phase codes.

### Suitable for
- note.com: **yes** — the refactoring story and the probe binary architecture are broadly applicable
- GitHub README: **partial** — the probe binary table could become a section in `src/koto-pico/README.md`

### Screenshots/Logs to Include
- UART boot sequence trace: `phase=131 → phase=132 → phase=14 → phase=140 → phase=143`
- Directory listing of `src/koto-pico/src/bin/` showing probe and firmware binaries
- Before/after architecture diagram for `BytecodeSession` extraction

---

## Article 9: "What KotoOS Became"

### Audience
Everyone who has read the series; hobbyists considering a similar project; embedded Rust developers looking for a retrospective.

### Hook / Opening
129 issues later, KotoOS runs on a physical PicoCalc. The home screen loads 15 packages from SD in under a second, navigates at 24 ms per selection, persists favorites and sort preferences across reboots, and launches apps that can type Japanese text and draw 16×16 tile blits. Here is what worked, what surprised us, and what comes next.

### Key Sections

- **What works on hardware** — Draw from `docs/IMPLEMENTATION_STATUS.md`: hardware-validated list (LED/CDC, LCD, keyboard, SD, PSRAM, audio, shell, runtime, PSRAM streaming, flicker fix). Status as of current development.
- **The numbers** — Flash image: 451.8 KiB. Static SRAM: 143.0 KiB of 264 KiB. Maximum working buffer: 30,720 B (no full framebuffer). Same-page selection: 24 ms. PSRAM code ceiling: 128 KiB. App heap ceiling: 16 KiB. These aren't just specs — they reflect deliberate budget decisions documented in every milestone.
- **What the `no_std` constraint taught us** — Every decision that would have been easy with an allocator required a different approach: `SandboxPath` instead of `PathBuf`, `StaticCell` instead of `Box`, `Canvas::new_viewport` instead of a framebuffer, `PsramCodeWindow` instead of `mmap`. The constraints produced a more auditable and portable codebase.
- **Surprises and false starts** — The one-bit PSRAM rotation (KOTO-0069), the 27 KB stack fault (KOTO-0121), the async timer PCM underruns (KOTO-0114), the invisible KotoBlocks board (KOTO-0127/0129). Each failure left a concrete diagnosis that improved the architecture.
- **What the simulator approach enabled** — KotoSim ran 63 issues of development before any hardware arrived. The shared `koto-core` contract meant hardware bring-up was integration rather than rewriting. The 75 `cargo test` suite in KotoSim found bugs that would have been invisible on device.
- **What comes next** — KOTO-0129 on-device confirmation; KotoMemo on device; `draw_tile`/`tilemap_blit` Game2D API; larger SKK dictionary; RTC clock source. The project is open-ended.

### Closing
KotoOS proved that a complete Japanese PDA operating system — shell, IME, bytecode VM, games — can be built in `no_std` Rust on a 264 KiB microcontroller. The constraints were not a limitation; they were a design guide.

### Suitable for
- note.com: **yes** — retrospectives and "what I learned" posts perform well; include hardware photos
- GitHub README: **yes** — a condensed version of this article is the ideal expanded README

### Screenshots/Logs to Include
- Physical PicoCalc running KotoShell (hardware photo if available)
- UART parity validation timing table from KOTO-0126
- KotoBlocks running in KotoSim (tile blits, game board)
- KotoMemo with Japanese text in the light theme
