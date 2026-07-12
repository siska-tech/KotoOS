# KotoOS Development Log Index

Total issues tracked: **129** (KOTO-0001 through KOTO-0129)

> KotoGFX rendering-migration issues (`GFX-0000` series) are tracked separately in [ISSUES_kotogfx.md](../ISSUES_kotogfx.md), governed by [KOTOGFX_RENDER_MIGRATION_PLAN.md](../architecture/KOTOGFX_RENDER_MIGRATION_PLAN.md).

---

## Phase 1: Foundation (KOTO-0001–0012)

| Issue | Title | Status | Tags | Summary |
|-------|-------|--------|------|---------|
| KOTO-0001 | Rust Workspace Bootstrap | done | workspace, tooling, ci | Created the initial Rust workspace with `koto-core` and `koto-sim`; established the baseline that `cargo fmt`, `cargo test`, and `cargo clippy` pass. |
| KOTO-0002 | KotoSim Package Manifest Scan | done | shell, tooling | Replaced hard-coded package entries with manifest scanning from `sdcard_mock/apps/*.kpa.json`, populating `PackageList` dynamically. |
| KOTO-0003 | Repository-Local Issue Management | done | doc, tooling | Set up `docs/issues/` file-per-issue workflow and harness checks for issue IDs, avoiding external issue-tracker dependency. |
| KOTO-0004 | KotoFS Sandbox Path Resolver | done | sd, workspace | Added the core `SandboxPath` resolver that maps app-visible virtual paths into sandbox-relative paths and rejects traversal outside the sandbox. |
| KOTO-0005 | Core Render Surface and Dirty Rectangle Harness | done | hal, shell | Introduced the core render-command model covering full, rect, and scanline updates; establishes the dirty-rectangle foundation. |
| KOTO-0006 | Scripted Host Input Harness | done | tooling, ci | Enabled KotoSim to replay scripted input sequences (up/down/confirm/cancel) for deterministic shell and IME tests. |
| KOTO-0007 | `.kpa` Package Format Specification | done | bytecode, doc, asset-pipeline | Defined the first concrete `.kpa` archive layout: header, table, asset ordering, alignment, and sequential-read constraints. |
| KOTO-0008 | RP2040 Bring-Up Plan and HAL Backend Decision | done | hal, pico-firmware, doc, spike | Compared `embassy-rp`, `rp-hal`, and Pico C SDK; chose `embassy-rp` and defined the peripheral bring-up probe sequence. |
| KOTO-0009 | Host Filesystem HAL Adapter | done | hal, sd | Implemented the host-side filesystem adapter mounting `sdcard_mock`, routing reads through `SandboxPath`. |
| KOTO-0010 | KotoShell Render Model Integration | done | shell | Made KotoShell produce render commands for package list display, with dirty-row marking on selection changes. |
| KOTO-0011 | KPA Manifest Validation in Core | done | bytecode, shell | Moved manifest validation rules into `koto-core` so KotoSim and future tools share one code path for required fields and limits. |
| KOTO-0012 | Local CI Command and Check Script | done | ci, tooling | Created a single `python harness/check_all.py` command that runs formatting, tests, Clippy, and all project harness checks. |

---

## Phase 2: Sim & I/O Layer (KOTO-0013–0032)

| Issue | Title | Status | Tags | Summary |
|-------|-------|--------|------|---------|
| KOTO-0013 | Bitmap Font Glyph Model | done | lcd, tooling, asset-pipeline | Adopted M+ BITMAP fonts; `harness/mplus_to_kfont.py` converts BDF to compact `.kfont`; `no_std` binary-search reader in `koto-core::font`. Full JIS X 0208 coverage (~7000 glyphs). |
| KOTO-0014 | Text Grid and IME Line Layout | done | ime, shell | Defined screen regions for content, status, and fixed IME input line on 320×320; tests verify regions do not overlap. |
| KOTO-0015 | Romaji-to-Kana Input Core | done | ime | Implemented `RomajiKanaInput` allocation-free state machine covering vowels, basic kana rows, representative youon, and small-っ. |
| KOTO-0016 | Sticky Shift State Machine | done | ime | Implemented one-shot Shift for thumb typing; `StickyShift` clears after one non-shift key, enabling SKK-style conversion triggers. |
| KOTO-0017 | SKK Dictionary Index Strategy | done | ime, sd, spike | Designed SD-card-friendly dictionary lookup with SRAM-resident leading-character index; prototyped as `koto_core::skk` with fixture tests. |
| KOTO-0018 | Runtime VM Selection Spike | done | vm, spike | Compared custom stack VM, Wasm, Lua, mruby; chose custom KBC1 stack VM for `no_std`, allocator-free, deterministic fuel. |
| KOTO-0019 | Runtime Host API Boundary | done | vm, compiler | Defined host-call ABI covering draw, input, audio, file, and exit; recorded in `RUNTIME_BYTECODE_ABI.md`. |
| KOTO-0020 | KPA Packer Prototype | done | asset-pipeline, tooling | Created the first host-side packer tool reading manifests and emitting deterministic package layout reports. |
| KOTO-0021 | Sequential Asset Read Harness | done | sd, asset-pipeline, ci | Added layout CSV fixtures and harness checks for monotonic asset offsets; enforces SD-card sequential-read constraints. |
| KOTO-0022 | PSRAM Block API Prototype | done | psram, hal | Prototyped core-facing PSRAM block API with in-memory mock backend; API rejects out-of-range transfers. |
| KOTO-0023 | Software PCM Mixer Core | done | audio | Created the platform-independent PCM mixer core summing multiple sample streams; output clamps safely to `i16`. |
| KOTO-0024 | Power Status Model and Shell Indicator | done | hal, shell | Defined the optional power/battery status model distinguishing unsupported, unknown, charging, and percent states. |
| KOTO-0025 | Keyboard Matrix Validation Plan | done | keyboard, doc, spike | Defined the hardware validation procedure for game button chord candidates; default mapping decision deferred to hardware. |
| KOTO-0026 | LCD Controller Init Profiles | done | lcd, hal, doc, spike | Captured ILI9488 and ST7365P-compatible LCD initialization differences; defined profile data needed by the device HAL. |
| KOTO-0027 | KotoDOS 320×200 Mode Model | done | shell | Modeled the 320×200 game region and static 320×120 UI region; core constants with region-bounds tests. |
| KOTO-0028 | KotoVN Script and Image Pipeline Spike | done | asset-pipeline, spike | Sketched visual-novel script and image asset pipeline; identified VM vs. host engine responsibilities. |
| KOTO-0029 | KotoMML Format and Playback Model | done | audio, doc, spike | Defined the first MML subset (notes, rests, tempo, volume) and how MML events feed the software PCM mixer. |
| KOTO-0030 | PicoMings Scanline Sprite Model | done | game2d, spike | Explored tile, sprite, and scanline composition model for PicoMings-style gameplay; estimated memory use for a small level. |
| KOTO-0031 | KotoSim Software Framebuffer and Image Output | done | lcd, shell, tooling | Added RGB565 rasterizer (`Canvas`), `ShellState::paint`, and `--image PATH` BMP output to KotoSim; first visual shell pixels. |
| KOTO-0032 | KotoSim Live Interactive Window | done | shell, tooling | Opened a live 320×320 `minifb` window for KotoSim with real-time framebuffer display and PC keyboard input. |

---

## Phase 3: Runtime VM (KOTO-0033–0050)

| Issue | Title | Status | Tags | Summary |
|-------|-------|--------|------|---------|
| KOTO-0033 | KBC1 Bytecode Verifier | done | vm, bytecode | Added the first `koto-core::runtime` verifier for KBC1: header, bounds, opcode, branch-target, stack-underflow, and host-call checks. |
| KOTO-0034 | Cooperative Bytecode VM Core | done | vm, bytecode | Implemented the cooperative KBC1 interpreter with bounded stack/calls/fuel, deterministic traps, and `VmHost` dispatch. |
| KOTO-0035 | KotoSim Runtime Launch Path | done | vm, shell, sd | Connected KotoShell launch to the runtime: load manifest, open bytecode from `sdcard_mock`, verify, run one bounded frame. |
| KOTO-0036 | Runtime Text and File Host Calls | done | vm, bytecode, sd, memo | Extended host-call ABI with `draw_text`, `file_open`, `file_read`, `file_write`, `file_close`; heap-range validation and sandbox paths. |
| KOTO-0037 | Memo Editor Core | done | memo, ime | Implemented `MemoEditor` in `koto-core` with fixed-capacity storage, cursor movement, scrolling, dirty line/IME rectangle reporting. |
| KOTO-0038 | Memo IME Integration | done | memo, ime | Connected KotoIME composition to `MemoEditor`; romaji/kana, Sticky Shift, SKK lookup flow into the editor with deterministic IME-line state. |
| KOTO-0039 | Memo KPA Fixture | done | memo, asset-pipeline | Created `dev.koto.memo` package fixture for KotoSim with manifest, bytecode, icons, and sandbox permissions. |
| KOTO-0040 | Memo Simulator Validation | done | memo, ime, sd, ci | Added end-to-end scripted simulation: launch, edit (ASCII + romaji/kana + SKK conversion), save, exit, relaunch, reload. |
| KOTO-0041 | Bytecode Memo App | done | memo, bytecode, compiler, ime | Shipped `dev.koto.memo` as a real KBC1-bytecode app from `apps/memo/src/main.koto`; replaced native KotoSim shortcut. |
| KOTO-0042 | Runtime Input and IME Host Calls | done | vm, bytecode, ime | Extended `VmInputSnapshot` with typed codepoint and intent bits; added `text_input`, `ime_*`, and `edit_*` host calls. |
| KOTO-0043 | KotoSim Interactive Bytecode Session | done | vm, shell, ime, memo | Created `BytecodeAppSession` with per-frame `step_frame`; KotoSim window mode now drives live VM sessions end-to-end. |
| KOTO-0044 | Bytecode Assembler and IR Target | done | compiler, bytecode, tooling | Implemented `kbc-asm` text assembler with labels, directives, and string data; harness checks drift between source and committed `.kbc`. |
| KOTO-0045 | High-Level App Language Spike | done | compiler, spike, doc | Chose and specified the Koto app language: `int`/`bool`/`buf`, `let`/`const`, `if`/`while`/`loop`, non-recursive `fn`, explicit host calls. |
| KOTO-0046 | Koto Language Compiler MVP | done | compiler, bytecode | Implemented `koto-compiler` from Koto source to KBC1; inlines all functions, branchless comparisons, verifier-guaranteed output. |
| KOTO-0047 | Bytecode SDK Prelude | done | compiler, bytecode, ime, sd | Defined and implemented the KotoSDK prelude: named wrappers for draw, input, IME, file, and lifecycle host calls. |
| KOTO-0048 | App Build and Package Loop | done | tooling, ci, asset-pipeline | Created `apps/apps.json` registry and `harness/build_apps.py`; single command rebuilds all app bytecode and checks drift. |
| KOTO-0049 | KotoSim App Development Experience | done | tooling, vm, ci | Added `--app APP_ID`, `--app-script PATH`, runtime diagnostics with PC and source location, and the `APP_DEV_LOOP.md` doc. |
| KOTO-0050 | Runtime Inspector | done | vm, tooling | Added `BytecodeAppSession::inspect` returning `InspectorReport`; exposes VM run state, PC, fuel, host-call name, and draw/file counts. |

---

## Phase 4: App Ecosystem (KOTO-0051–0063)

| Issue | Title | Status | Tags | Summary |
|-------|-------|--------|------|---------|
| KOTO-0051 | Bytecode Debug Data and Source Map | done | compiler, bytecode, tooling | Added in-band `KDBG` debug section; compiler emits `.loc` directives; KotoSim diagnostics show `file:line:column`. |
| KOTO-0052 | SDK Samples | done | compiler, bytecode, doc | Added six sample apps (hello text, input echo, counter, file note, IME playground, dirty-rects) as regression fixtures. |
| KOTO-0053 | App Scaffold Tool | done | tooling, asset-pipeline | Implemented `koto-app-scaffold` CLI: creates app source tree, manifest, icon placeholder, and `apps.json` entry with validation. |
| KOTO-0054 | Asset Development Pipeline | done | asset-pipeline, tooling, doc | Defined host-side asset conversion pipeline; image/icon conversion, font preview, and package sequential-placement harness checks. |
| KOTO-0055 | Save Data Management | done | sd, tooling | Added `--save-list` and `--save-clear APP_ID` to KotoSim; sandbox containment tests; documented in `APP_DEV_LOOP.md`. |
| KOTO-0056 | App Failure Recovery Screen | done | shell, vm | Added `AppFailureSummary` distinguishing verification failure, VM trap, and normal exit; shell returns to app list on failure. |
| KOTO-0057 | Shell App Details View | done | shell | Extended KotoShell to show selected-app details (runtime, permissions, memory, save-data presence) before launch. |
| KOTO-0058 | Golden Frame Validation | done | ci, tooling, shell | Added `--golden-frames` output and `harness/check_golden_frames.py` comparing against `harness/fixtures/golden_frames/sim.trace`. |
| KOTO-0059 | Roadmap State Cleanup | done | doc | Separated completed KotoSim baseline from the next active roadmap in `docs/ISSUES.md`; issue-status definitions clarified. |
| KOTO-0060 | KotoSim Runtime Profile Cleanup | done | vm, bytecode | Unified verifier and VM construction behind `RuntimeLimits::simulator_default()`; stack 16, call 4, heap 4 KB, fuel 60,000. |
| KOTO-0061 | KotoSim Module Split | done | workspace, refactor | Split monolithic `koto-sim/lib.rs` into 14 focused submodules (host, session, render, scenario, save_data, etc.). |
| KOTO-0062 | Manifest JSON Parser Cleanup | done | bytecode, tooling | Replaced ad-hoc manifest string scanning with structured `serde_json` parsing in host-side code; `koto-core` remains `no_std`. |
| KOTO-0063 | Documentation Implementation Status Map | done | doc | Created `docs/IMPLEMENTATION_STATUS.md` tracking `core-implemented`, `simulated`, `hardware-pending`, and `hardware-validated` states. |

---

## Phase 5: Pico Bring-Up (KOTO-0064–0069)

| Issue | Title | Status | Tags | Summary |
|-------|-------|--------|------|---------|
| KOTO-0064 | Pico HAL Crate Bootstrap | done | pico-firmware, hal, workspace | Added `koto-pico` workspace member with `embassy-rp` 0.10.0 and RP2040 bootstrap; host crates remain `default-members`. |
| KOTO-0065 | Pico Probe: Blink and USB CDC | done | pico-probe, pico-firmware | Proved embedded toolchain: `blink_cdc` binary flashed via BOOTSEL; GP25 LED blink and USB-CDC banner/heartbeat confirmed on hardware. **Notable:** `KotoOS KOTO-0065 blink+cdc v0.1.0` banner observed in Tera Term. |
| KOTO-0066 | Pico Probe: LCD Fill | done | pico-probe, lcd | Validated ILI9488 `ili9488-spi` profile at 20 MHz, RGB666, `MADCTL=0x48`; correct landscape orientation, clean partial updates, DMA scanline band. |
| KOTO-0067 | Pico Probe: Keyboard I2C | done | pico-probe, keyboard | Validated STM32 keyboard bridge at 100 kHz I2C; bounded FIFO drain (≤4 events/frame); all 44 `arrow-zxas` chord cases passed. **Notable:** `{"kind":"selection","status":"pass","selected_candidate":"arrow-zxas","reason":"first_passing_candidate"}` |
| KOTO-0068 | Pico Probe: SD Mount and Read | done | pico-probe, sd | Mounted TOSHIBA 8 GB SDHC over SPI0 at 1 MHz fallback; listed 15 LFN manifests; completed 1,545-byte sequential manifest read. |
| KOTO-0069 | Pico Probe: PSRAM Round-Trip | done | pico-probe, psram | After PIO sampling fixes (one-bit rotation diagnosis), 256-byte block round-tripped with exact equality at block 257; 2.44 ms write, 1.62 ms read. **Notable:** Initial failure showed `5a→b5, a3→47, ec→d9` (exact one-bit left rotation per byte). |

---

## Phase 6: Memo & IME Polish (KOTO-0070–0093)

| Issue | Title | Status | Tags | Summary |
|-------|-------|--------|------|---------|
| KOTO-0070 | Memo Basic Multiline Input | done | memo, ime, bytecode | Fixed space/newline insertion, vertical movement, and multiline text drawing in the memo bytecode app. |
| KOTO-0071 | IME Usability Hardening | done | ime | Documented key-action behavior table; scripted failure-prone typing flows; predictable feedback for all composition states. |
| KOTO-0072 | Memo Editor Usable UI | done | memo, shell | Added visible cursor/caret tracking, scrolling, separated IME line, and save/exit status to the memo UI. |
| KOTO-0073 | IME Toggle and Status Bar Baseline | done | ime, memo | Added `IME_TOGGLE` / `INTENT_IME_TOGGLE` constants; memo starts in ASCII mode and shows `IME:ON` / `IME:OFF`. |
| KOTO-0074 | Memo Visual Shell | done | memo, shell, lcd | Added title bar, filename, save state badge, framed document area, and bottom command/status bar with `Ln N Col M`. |
| KOTO-0075 | Memo Font Metrics and Caret Accuracy | done | memo, lcd, ime | Added `edit_view_metrics` host call (ABI minor 4); memo derives all row/caret placement from live host cell metrics. |
| KOTO-0076 | Memo Scrollbar | done | memo, shell | Added `edit_total_lines` host call (ABI minor 6); memo draws right-side track/thumb that updates as visible range changes. |
| KOTO-0077 | IME Candidate Popup UX | done | ime, memo | Added IME-only popup showing input mode, composition/candidate text, and commit/cancel hints; disappears on commit/cancel. |
| KOTO-0078 | IME Candidate List Navigation | in-progress | ime, memo | SKK exposes multiple candidates; `MemoImeLine` carries `candidate_index`/`candidate_count`; forward cycling works; previous-candidate key pending. |
| KOTO-0079 | Memo Command Bar Actions | done | memo, shell | Made command bar truthful: shows save/exit in edit mode, convert/commit/cancel in IME mode, with working key routes. |
| KOTO-0080 | Memo Open/Save Dialog Baseline | done | memo, sd, bytecode | Added `dir_list` host call (ABI minor 7); F4 opens file picker; real sandbox directory listing for open/save. |
| KOTO-0081 | Shell Visual Home | done | shell, lcd | Added top status bar, pane-shown/pane-hidden layout modes, secondary status strip, bottom command bar to KotoShell. |
| KOTO-0082 | Shell Icon Grid and Pagination | done | shell, lcd | Packages render as paged icon grid; directional navigation, page indicators, selection repaints only affected tiles. |
| KOTO-0083 | Shell Selected App Details Pane | done | shell | Converted details view to toggleable right-side pane; toggle relayouts grid; selection re-clamped after toggle. |
| KOTO-0084 | Shell System Status Indicators | done | shell, hal | Status bar shows battery gauge, SD indicator, deterministic clock (`ShellClock` or placeholder), and save/health badges. |
| KOTO-0085 | Shell Command Bar Actions | done | shell | Command bar shows truthful key-chip entries; details-pane toggle bound to Cancel; favorite/sort/category dimmed until KOTO-0086. |
| KOTO-0086 | Shell Favorites Categories and Sort | done | shell, sd | Favorite toggle (F2), category filter (F4), sort mode (F3) all wired; preferences persist to `data/dev.koto.shell/prefs.txt`. |
| KOTO-0087 | Shell Icon Asset Set | done | shell, asset-pipeline | Added eight drawn `IconKind` variants (Notepad, Calendar, Folder, Calculator, Gear, Music, Game, Terminal) with `icon_kind_for` mapping. |
| KOTO-0088 | Memo Light Theme and Colored Text | done | memo, lcd, bytecode | Added `draw_text_color` host call; memo app redrawn with navy title bar, white document area, pale-blue IME panel, Japanese command labels. |
| KOTO-0089 | Larger SKK Dictionary For Evaluation | todo | ime, spike | Open research item: evaluate `SKK-JISYO.S`-sized dictionary for license, SRAM fit, and host loading path. |
| KOTO-0090 | Memo Line Wrap and Horizontal Scroll | done | memo, lcd, ime | Added soft-wrap (default) and no-wrap/horizontal-scroll modes; F3 toggles; `edit_total_lines` returns visual rows. |
| KOTO-0091 | Package Description and Category Metadata | done | bytecode, shell, doc | Extended `ManifestFields`/`PackageInfo` with `description` (128 B max) and `category` (32 B max); fixture manifests updated. |
| KOTO-0092 | Compiler Per-Scope Local Slot Reuse | done | compiler, vm, refactor | `emit_block` snapshots/restores `locals`/`next_slot` on scope entry/exit; peak-lets precheck replaces static total. |
| KOTO-0093 | Memo Save/Save As Filename Prompt | done | memo, sd | F2 asks `上書き保存しますか? (y/n)`; `y` overwrites, `n` opens Save As; unnamed documents open Save As directly. |

---

## Phase 7: Games & Language (KOTO-0094–0112)

| Issue | Title | Status | Tags | Summary |
|-------|-------|--------|------|---------|
| KOTO-0094 | KotoBlocks Tetromino Game | done | game2d, bytecode, compiler, asset-pipeline | Shipped first KotoOS game: Tetris-style KotoBlocks using `draw_pixels_rgb565` for 16×16 RGB565 tile blits; 7 tetrominoes, rotation, scoring, HOLD. |
| KOTO-0095 | App Audio Host Call (BGM and SFX) | done | audio, compiler, bytecode | Wired `audio_submit_i16`; added host-owned KotoMML synth service (`play_sfx`/`play_bgm`); `cpal` backend in window mode with `--audio` capture. |
| KOTO-0096 | Manifest-Driven Per-App Heap Profile | done | vm, bytecode, tooling | `BytecodeVm` heap changed from const-generic to caller-supplied `&mut [u8]`; compiler emits exact heap need; launch validates against manifest. |
| KOTO-0097 | Game2D ABI Design | done | game2d, doc, spike | Designed host/app responsibility split for tile cache, `draw_tile`, tilemap, and sprite list; reserved ABI IDs `0x14`–`0x1F` in `docs/GAME2D_ABI.md`. |
| KOTO-0098 | KotoMML Multi-Track Playback | done | audio | Added `#TRACK` markers; `SimAudio` holds up to 4 concurrent `MmlPlayer` voices; KotoBlocks BGM updated to lead+bass+drum. |
| KOTO-0099 | Memo IME Candidate Overlap Avoidance | done | memo, ime | Added `edit_reserve_rows` host call (ABI minor 9); editor scrolls cursor above overlay-reserved bottom rows during conversion. |
| KOTO-0100 | Romaji-Kana Missing Youon Rows | done | ime | Added nine missing Hepburn youon entries (`kya/kyu/kyo`, `hya/hyu/hyo`, `mya/myu/myo`) to the `ROMAJI_KANA` table. |
| KOTO-0101 | Runtime Budget Diagnostics | done | vm, compiler, ci, tooling | Added `BytecodeVm` high-water tracking, `BytecodeAppSession::budget()`, `--budget` CLI, and `check_budgets.py` gate. **Notable:** KotoBlocks `local_peak` 48/48 — 100% capacity warning, correctly surfacing the pre-KOTO-0092 slot pressure. |
| KOTO-0102 | KotoBlocks Local Slot Reduction | done | compiler, game2d, refactor | Added `--slot-map` to compiler; reduced KotoBlocks `user_slots_used` from 44 to 41/45 through source-level slot consolidation. |
| KOTO-0103 | KotoBlocks Game-Feel Effects Pass | done | game2d, audio | Added four-line fanfare, "4 LINE!" banner, game-over board sweep, level-up banner, smooth fall, move/rotate flash, lock flash, score popup. |
| KOTO-0104 | Inline-Boundary Local Slot Reuse | done | compiler, vm, refactor | Extended per-scope slot reuse across inline boundaries; KotoBlocks `user_slots_used` 44→42/45; Memo 18→15/45. |
| KOTO-0105 | Fix Existing IME Test Failure | done | ime, ci | Reconciled `reports_incomplete_and_invalid_sequences` test with updated `kwa`-series table entries (`kw` is now a valid prefix). |
| KOTO-0106 | Inline Memo IME Candidate Display | done | ime, memo, lcd | Replaced large bottom IME panel with inline composition/candidate display at caret; restored all 19 document rows. |
| KOTO-0107 | Inline IME Composition Insertion Layout | done | ime, memo, lcd | Memo renderer splits the active visual row at cursor to show prefix, IME preedit, and suffix; caret sits at preedit end. |
| KOTO-0108 | Memo Input Blocked After Opening Long Document | done | memo, sd | Editor/document buffer sized to 1024 B; file opens read at most 960 B, preserving 64 B headroom for immediate editing. |
| KOTO-0109 | Romaji/Kana Punctuation and Long-Vowel Coverage | done | ime | Added `-`→`ー`, `/`→`・`; `ー` appends to SKK reading so `konpyu-ta` composes to `こんぴゅーた`. |
| KOTO-0110 | Memo Backspace/Delete Key Repeat | done | memo | Enabled standard key-repeat events for Backspace and Delete in the KotoSim window backend. |
| KOTO-0111 | Memo New Document | done | memo, sd | F5 switches to unnamed empty document; filename prompted at first Save; Cancel returns to current document. |
| KOTO-0112 | Memo Save Confirmation Flow | done | memo, sd | F2 on named document asks `上書き保存しますか? (y/n)`; `y` overwrites, `n` opens Save As; F5 creates `(新規)` immediately. |

---

## Phase 8: Pico Firmware (KOTO-0113–0128)

| Issue | Title | Status | Tags | Summary |
|-------|-------|--------|------|---------|
| KOTO-0113 | KotoRogue Turn-Based Roguelike | done | game2d, audio, bytecode, asset-pipeline | Shipped procedural dungeon crawl with bump combat, fog of war, depth progression, BGM/SFX; uses `asset_load` + `.kim` sprite tiles. |
| KOTO-0114 | Pico Probe: PWM Audio Output | done | pico-probe, audio | Validated DMA-paced PWM audio at 8 kHz sample rate; initial async timer approach failed (181 µs latency); DMA timer 0 fix produced zero underruns, audible 500 Hz tone. **Notable:** First run: all 24,000 samples reported as underruns. Fixed run: `dma_remaining=0, dma_error=0, underruns=0, result=pass`. |
| KOTO-0115 | Pico Probe: Battery and Power Status | done | pico-probe, hal | Validated STM32 I2C battery register `0x0B`; zero payload maps to `Unknown` not 0%; no battery installed, BIOS v1.6 confirmed. |
| KOTO-0116 | Package Image Assets — `asset_load` and `.kim` Pipeline | done | asset-pipeline, game2d, bytecode, compiler | Added `asset_load` host call (ABI `0x44`); `.kspr` ASCII sprite sheet → `KIM1` RGB565 compiler; KotoRogue ships animated tiles. |
| KOTO-0117 | Pico Firmware Main Loop | done | pico-firmware, shell, keyboard, lcd | Added `koto_firmware` binary: DMA LCD, STM32 keyboard, normalized `InputState` edges, `ShellState` on 16 ms frame cadence with compiled-in fixtures. |
| KOTO-0118 | Pico SD Package List | done | pico-firmware, sd, shell | SD catalog from FAT `APPS/*.kpa.json`: two-stage SPI clock fallback, LFN enumeration, `PackageInfo` validation, page indicators. |
| KOTO-0119 | Pico Shell Raster Backend | in-progress | pico-firmware, lcd, shell | Added `Canvas::new_viewport` for bounded strip rasterization; `write_rgb565_rect` RGB565→RGB666 during DMA; `ShellState::paint` on device. Performance failure found (193–339 ms); reopened. |
| KOTO-0120 | Pico Shell Dirty-Rectangle Performance | done | pico-firmware, lcd, shell | Added `ShellState::paint_rect` clip; `render_selection_change` emits only changed tiles; SPI raised to 62.5 MHz. **Notable:** Same-page selection: 24 ms at 62.5 MHz, down from 193 ms. Hardware timing table captured. |
| KOTO-0121 | Pico Shell SD Catalog Reintegration | done | pico-firmware, sd, shell | Fixed 27 KB stack-frame fault in `load_packages` by moving scan scratch to `StaticCell`; two-stage clock fallback restored. **Notable:** `phase=137 apps-list-ok manifests=15 → phase=14 catalog-ready packages=15`. |
| KOTO-0122 | Pico Shell Package Metadata and Icons | done | pico-firmware, shell, asset-pipeline | Added `parse_shell_icon_theme` in `no_std` firmware; icon asset reads from `ICONS/` into 2 KiB static scratch. **Notable:** `phase=140 icons-loaded count=15` — all 15 icons matched and loaded. |
| KOTO-0123 | Pico Shell Actions and Preferences | done | pico-firmware, shell, sd | Wired favorite/sort/category/pane-toggle actions on device; preferences persist to `SHELLPRF.TXT` with bounded 2304 B format. |
| KOTO-0124 | Pico Shell System Status | done | pico-firmware, hal, shell | Connected SD detect (GP22), STM32 battery register, BIOS version check; status cluster repaints only 2,400 px on change. |
| KOTO-0125 | Pico Shell Runtime Launch and Return | done | pico-firmware, vm, bytecode, ime | `BytecodeSession` extracted to `koto-core`; sample apps launched on device; File Note saves/reads on SD; IME Playground reaches `read:`/`miss:`. **Notable:** Game packages fail with `phase=253 launch-bytecode-oversize` (8 KiB limit). |
| KOTO-0126 | Pico KotoShell Parity Validation | done | pico-firmware, shell, ci, doc | Defined 13-row parity checklist; recorded all six UART timing captures; flash 451.8 KiB, static SRAM 143.0/264 KiB, same-page 24 ms. |
| KOTO-0127 | Pico Large App Bytecode and Heap Budget | done | pico-firmware, psram, vm, bytecode | Added `CodeSource` trait; `PsramCodeWindow` streams code through 8 KiB SRAM window; 128 KiB ceiling in PSRAM. **Notable:** `phase=156 app-staged backing=psram code_size=65528 → phase=154 app-heartbeat frame=300 pc=16378 fuel=26665` — KotoBlocks VM running from PSRAM. |
| KOTO-0128 | Pico App Runtime Frame Flicker | done | pico-firmware, lcd | `present_app_delta` now composites partial-background frames off-screen into existing `RASTER_STRIP`; Dirty Rects sample no longer flickers. |

---

## Phase 9: PSRAM & Hardware Final (KOTO-0129)

| Issue | Title | Status | Tags | Summary |
|-------|-------|--------|------|---------|
| KOTO-0129 | Pico Device Game2D `draw_pixels_rgb565` Support | in-progress | pico-firmware, game2d, lcd | Implements `DeviceHost::draw_pixels_rgb565`, adds pixel draw-command variant, raises `MAX_APP_DRAW_COMMANDS` 16→384; pending on-device confirmation for KotoBlocks visual rendering. |
