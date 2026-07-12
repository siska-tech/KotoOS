//! Device constants and the firmware FAT timestamp source.
//!
//! These were extracted verbatim from the monolithic `koto_firmware` entry point
//! so the StaticCell array sizes, SD/LCD/runtime/draw budgets, and preference
//! markers stay in one place. The `StaticCell` instances themselves remain in the
//! binary; only the sizing constants live here.

use embedded_sdmmc::{TimeSource, Timestamp};
use koto_core::Rect;

// Runtime diagnostic verbosity profiles (DIAG-0001 Stage 1). The profile *logic*
// (which classes a profile enables, and its render sample cadence) lives beside the
// diagnostics model in KotoGFX so it is host-testable; the firmware pins the active
// profile here and gates each per-cadence emit site on `DIAG_PROFILE.enables(class)`
// / the `sample_period()` cadence (see `diag::on_cadence`). Re-exported so call sites
// read `config::{DiagProfile, DiagClass}` like every other shared constant.
pub use koto_gfx::{DiagClass, DiagProfile};

/// The active diagnostic verbosity profile (DIAG-0001). Compile-time constant: a
/// disabled class's emit branch is dead-code-eliminated (zero RAM; quieting logs also
/// shrinks `.text`), and changing profile means a rebuild + reflash. Default `Perf`:
/// a clean `phase=160` (+ thinned `phase=168`) for regression / perf-smoke runs with
/// no per-cadence render/audio/CodeWindow chatter. `DiagProfile::Verbose` reproduces
/// today's dev emit set and 30-frame cadence as a faithful A/B baseline against `main`.
pub const DIAG_PROFILE: DiagProfile = DiagProfile::Perf;

pub const KEYBOARD_REGISTER_SETTLE_US: u64 = 250;
// The STM32 services battery-register reads on a much slower interval than its
// keyboard FIFO. KOTO-0115 validated the official reader's 16 ms delay.
pub const BATTERY_REGISTER_SETTLE_US: u64 = 16_000;
pub const POWER_POLL_MS: u64 = 5_000;
pub const MAX_EVENTS_PER_FRAME: usize = 4;
// Validated SD init strategy (KOTO-0068 sd-read probe): attempt the fast clock,
// then reconfigure the bus and retry once at the conservative fallback clock.
// Reproducing both stages — not a single fixed clock — is what reliably mounts
// the physical PicoCalc card.
pub const SD_FAST_SPI_HZ: u32 = 12_000_000;
pub const SD_FALLBACK_SPI_HZ: u32 = 1_000_000;
pub const MAX_MANIFEST_BYTES: usize = 2304;
// Keep the async task state modest while reducing the first build's 20 passes.
pub const RASTER_STRIP_LINES: usize = 16;
pub const RASTER_STRIP_BYTES: usize = 320 * RASTER_STRIP_LINES * 2;
// RGB666 conversion target for one strip: 3 bytes/px instead of the 2 bytes/px
// RGB565 source. Sized to the widest strip so the whole band ships in one DMA.
pub const RGB666_STRIP_BYTES: usize = 320 * RASTER_STRIP_LINES * 3;
// KOTO-0174 H-A2: software-pipeline the app present — while band N's RGB666
// bytes are on the SPI data DMA, band N+1 rasters and converts on the CPU. The
// `phase=178 spi-overlap` boot bench proved embassy-rp's `Spi::write().await`
// yields completely during the data DMA (join wall == max(dma, cpu), zero join
// overhead), so the original H-A "no overlap" was a structural bug, not a HAL
// limit. Zero extra SRAM: the existing RGB666 scratch ping-pongs as two halves
// (bands cap at 8 full-width rows instead of 16; per-band `begin` is ~74 µs
// against ~1.2 ms of hidden DMA). `false` restores the serial drain through
// the same band stream, for A/B and bisecting — pixels are identical either
// way, only the wait structure changes.
pub const PRESENT_PIPELINE: bool = true;
// Long-filename scratch for one directory entry at a time (KOTO-0121).
pub const MANIFEST_LFN_BYTES: usize = 192;
// A 40x40 KICON1 text asset is ~1.7 KiB (header + 40 lines of 40 cells); round
// up for an optional trailing newline / CRLF (KOTO-0122).
pub const KICON_BYTES: usize = 2048;
// Root-level 8.3 preference file. The device adapter intentionally avoids the
// simulator's long nested save-data path while preserving its logical lines.
pub const SHELL_PREFS_FILE: &str = "SHELLPRF.TXT";
pub const SHELL_PREFS_VERSION: &str = "version=1";
pub const SHELL_PREFS_COMPLETE: &str = "end=1";
// Reuse the manifest-sized bound: enough for 32 maximum-length favorite app
// IDs plus sort/category/pane metadata, but still a small sequential FAT I/O.
pub const MAX_SHELL_PREFS_BYTES: usize = MAX_MANIFEST_BYTES;
// KOTO-0127 code working-set budget. A program's code segment is staged into the
// 8 MiB PSRAM and executed through an SRAM window (`PsramCodeWindow`), so an app
// never has to fit wholly in SRAM. The window *tiles* the code: a call or branch
// outside the cached tile refills it from PSRAM over SPI.
//
// KOTO-0131 sizing (measured on hardware): at 8 KiB this window thrashed badly.
// KotoBlocks' 64 KiB code spans 8 tiles, and its hot loop ping-pongs between
// `main` (high tiles) and the `shape`/`pmid`/`blit_piece` helpers (tile 0), so
// every helper call refilled a tile to jump there and another to return —
// ~3 us/instruction, a ~30x slowdown that pinned gameplay at ~11 fps with the
// render path already near-idle (`phase=160` showed `vm_us` ~80 ms while
// `raster_us`/`transfer_us` were ~0). At 64 KiB the whole of KotoBlocks (and
// every app at or below this size) loads as a single tile: one fill at launch,
// then zero refills, i.e. full SRAM execution speed. Apps between this size and
// the `DEVICE_CODE_CEILING` (e.g. kotorogue, ~73 KiB) still tile.
//
// SRAM ceiling history (measured the hard way, KOTO-0131): the firmware's `.bss`
// used to leave a thin stack margin above it, and even an 8 KiB *increase* of
// this window overflowed the boot stack — a HardFault that hangs in the default
// handler loop rather than resetting (the linker cannot catch it; the stack grows
// down into `.bss` only at runtime). That invisible ceiling is what also sank the
// first 2-tile cache attempt (KOTO-0134's launch hang). KOTO-0170/0172 replaced
// the guesswork with a measured budget: the phase=176 canary puts the core-0
// stack peak at 26,616 B with ~81 KiB of margin, so KOTO-0173 re-lands the
// 2-tile cache as a deliberate +16 KiB `.bss` spend (margin stays > 60 KiB and
// the canary trips on any regression).
//
// Shape: `CODE_WINDOW_BYTES` is the **tile** size (unchanged at 16 KiB, so tile
// indices in phase=160 `refills=/code_tiles=` stay comparable across firmwares);
// the static buffer holds `CODE_WINDOW_TILES` of them and the launch path wraps
// it with `PsramCodeWindow::new_two_tile`, so two far-apart hot regions (e.g.
// `main` in a high tile, helpers in tile 0 — the ~3 us/instruction ping-pong
// signature) each keep their tile resident instead of evicting each other on
// every call/return. If PSRAM is unavailable the launch path runs directly from
// this buffer as a plain slice, so apps with code up to
// `CODE_WINDOW_TOTAL_BYTES` now launch without PSRAM.
pub const CODE_WINDOW_BYTES: usize = 16 * 1024;
// Resident tile-cache slots (KOTO-0173). 2 breaks the two-region ping-pong; more
// slots would pay lookup cost on the fetch hot path for a pattern no current app
// exhibits. Raising this is a deliberate-budget change (+16 KiB `.bss` each).
pub const CODE_WINDOW_TILES: usize = 2;
pub const CODE_WINDOW_TOTAL_BYTES: usize = CODE_WINDOW_BYTES * CODE_WINDOW_TILES;
pub const DEVICE_CODE_CEILING: usize = 128 * 1024;
// App heap ceiling. Each app is given a heap sized to its own KBC header request
// (per-app profile, KOTO-0096); this static is the deliberate device ceiling that
// request may not exceed. 16 KiB clears the heaviest current app (kotorogue, 10.8
// KiB) with headroom and is sized against the resident OS core plus the raster
// strip / RGB666 framebuffer staging budget.
pub const MAX_DEVICE_HEAP_BYTES: usize = 16 * 1024;
pub const DEVICE_FRAME_FUEL: u32 = 60_000;
pub const DEVICE_VM_STACK_SLOTS: usize = 16;
pub const DEVICE_VM_CALL_DEPTH: usize = 4;
// Per-frame app draw-command budget (KOTO-0129). The two retained command lists
// (current + previous frame, for the KOTO-0128 delta) live in the `APP_DRAW`
// StaticCell at `AppDrawCommand`'s ~80 B/command, so this constant directly sets
// ~160 B/command of SRAM across both lists.
//
// Held at 160, hardware-validated by KOTO-0135 Phase 1. KOTO-0134 first tried 160
// on the immediate renderer and KotoBlocks flickered at ~1/3 board fill — the list
// hit 160/160 and dropped its tail (panel text), forcing a full repaint
// (`dirty_px=102400 full=1 fps=3`) — because the immediate board re-blit grew one
// command per occupied cell (up to ~200 near a full 10x20 board, peaking ~310-340
// with static/panel/overlay). KOTO-0135 moved the locked board to a host-retained
// tilemap, so that fixed-board term is gone: hardware now reads `pixels=16` (it no
// longer grows with occupancy), `peak=119 ovf=0`, `dirty_px=1664 full=0`, and no
// right-side flicker on a heavily filled board. 160 holds with headroom, so the
// 384 stopgap is no longer needed.
//
// SRAM cost: ~80 B/command in each of the current+previous lists, so 160 → ~24.3
// KiB across both (vs ~58.4 KiB at 384), keeping stack headroom near the ~90 KiB
// build instead of ~59 KiB. Do not raise this back to 384 — the structural fix
// (KOTO-0135) removed the growth that motivated it; raising it only burns SRAM.
//
// The peak/overflow diagnostics (`phase=160 ... peak= ovf=`, one-shot
// `phase=162 app-draw-overflow`) are kept so any future app's command count can be
// judged against this cap. Remaining per-frame cost is now VM execution, not the
// draw list (see KOTO-0134 code-window thrashing).
//
// The cap now lives as data in KotoGFX's immediate-overlay budget
// (`koto_gfx::APP_DRAW_BUDGET`) rather than as a bare scalar — this is the value
// the budget model's reservations are sized against. The number is identical
// (96), so the command-list sizing and every diagnostic are unchanged; the budget
// model itself is not yet consulted on the draw path.
pub const MAX_APP_DRAW_COMMANDS: usize = koto_gfx::APP_DRAW_BUDGET.total_commands();
// KOTO-0135 Game2D board tilemap layer geometry: 10x20 cells of 16x16 px at the
// KotoBlocks well origin (8, 0). The host retains this layer across frames (it
// lives in `DeviceRuntimeHost`, so the two-list delta diffs it); apps write only
// the cells that change. A cell holds the app-heap byte offset of a 16x16 RGB565
// tile, or `-1` for empty. SRAM cost is GAME2D_BOARD_CELLS * 4 B per list = 800 B
// (1.6 KiB across the current + previous lists), within the stack headroom note.
// The board geometry and shape now live with the retained layer data model in
// koto-gfx (GFX-0002); re-exported here so firmware call sites are unchanged.
pub use koto_gfx::{GAME2D_BOARD_CELLS, GAME2D_BOARD_COLS, GAME2D_BOARD_ROWS};
// KOTO-0136 Game2D static/background command layer: a bounded retained list of
// `AppDrawCommand`s the app builds once (between `game2d_static_begin`/`_end`) for
// its page/well/grid/panel/label chrome, so that static UI no longer costs a host
// call and an immediate command every frame. The presenter composites it beneath
// the board tilemap and the per-frame immediate list. KotoBlocks' chrome is 56
// rects + 9 text labels = 65 commands; 80 leaves headroom for a small layout
// change without re-tuning.
//
// SRAM: this is a `single` retained layer in its own `APP_STATIC` StaticCell
// (`AppStaticLayer`), NOT inside the double-buffered `APP_DRAW` pair. The first cut
// stored it inside `DeviceRuntimeHost`, which duplicated it across the current +
// previous draw hosts (~152 B/slot = ~12 KiB at cap 80) and dropped boot-stack
// headroom enough to hang boot after `phase=146 battery`. As retained app-session
// state — not a positional-diff target — it needs no previous copy, so a single
// instance halves the cost to ~80 B/command, ~6.1 KiB at cap 80. It is NOT counted
// against MAX_APP_DRAW_COMMANDS; the immediate cap stays at 160.
// Capacity now lives with `AppStaticLayer` in koto-gfx (GFX-0002); re-exported.
pub use koto_gfx::GAME2D_STATIC_CMD_CAP;
// KOTO-0140 Game2D retained sprite/stamp layer. Stamps are reusable cell patterns
// (descriptor only — `cells_off`/`count`/`format`, the cell bytes stay in the app
// heap); sprites are retained placed instances diffed by stable `inst_id`. Both
// live in `DeviceRuntimeHost` (like `board`), so the existing current-vs-previous
// two-list delta diffs them with no separate dirty bitset.
//
// SRAM: 32 stamps * 8 B + 16 sprites * 12 B = ~448 B per list, ~0.9 KiB across the
// current + previous lists — within the §9 budget and under the 1 KiB target. 32
// stamps cover KotoBlocks' 28 piece orientations; 16 sprites cover its active,
// ghost, NEXTx3, and HOLD instances with headroom.
pub const GAME2D_MAX_STAMPS: usize = 32;
pub const GAME2D_MAX_SPRITES: usize = 16;
// KOTO-0141 Game2D retained text layer. Each item is an id-keyed string pinned at
// a pixel position with a colour, diffed across the current-vs-previous two-list
// delta (like `board`/`sprites`) so a value that does not change costs nothing and
// a change repaints only its own row band — removing the per-frame `draw_text`
// churn that shifted the immediate command count and forced positional-diff full
// repaints (KOTO-0143 `CommandCountShift`).
//
// SRAM: 12 items * (GAME2D_TEXT_BYTES + ~8 B header) = ~480 B per list, ~0.96 KiB
// across the current + previous lists — within the §9 budget. 12 items cover
// KotoBlocks' status text (run-state badge, score, level, lines, hold hint) with
// headroom. GAME2D_TEXT_BYTES caps one item's UTF-8 length (a longer string is
// rejected, like the immediate `draw_text` MAX_APP_TEXT_BYTES cap), kept smaller
// than the immediate cap because retained status values are short.
pub const GAME2D_MAX_TEXT_ITEMS: usize = 12;
// One retained text item's UTF-8 cap now lives with `Game2dText` in koto-gfx
// (GFX-0002); re-exported here.
pub use koto_gfx::GAME2D_TEXT_BYTES;
// Board tile size and origin now live with the retained layer model / dirty
// derivation in koto-gfx (GFX-0003); the tile-byte size joined them with the
// compositor move (GFX-0004). Re-exported here so firmware call sites and
// `GAME2D_TILE_BYTES` are unchanged.
pub use koto_gfx::{GAME2D_ORIGIN_X, GAME2D_ORIGIN_Y, GAME2D_TILE_BYTES, GAME2D_TILE_PX};
// The immediate-command UTF-8 cap now lives with `AppDrawCommand` in koto-gfx
// (GFX-0002); re-exported here.
pub use koto_gfx::MAX_APP_TEXT_BYTES;
pub const MAX_DEVICE_OPEN_FILES: usize = 4;
// The SKK dictionary is no longer SRAM-resident (KOTO-0089): the windowed SD
// reader streams `dict/skk_koto.skk` (~71KB) through a scan window sized by
// `koto_core::SKK_LOOKUP_WINDOW_BYTES` (512B, was a 4KiB whole-dict buffer).
// Cell metrics for the embedded M+ bitmap font (mplus12.kfont): half-width = 6
// pixels, full cell height = 13 pixels. mplus12 satisfies full_w = 2 × half_w
// (12 = 2 × 6), which is required by the 2-cell model used by cursor_display_col
// and the KotoMemo caret formula. mplus10 (full_w=10) breaks this invariant.
// These must match the font file's half_w and cell_h header fields.
pub const DEVICE_CELL_WIDTH: u16 = 6;
pub const DEVICE_CELL_HEIGHT: u16 = 13;
// Battery/storage/save indicators occupy the right side of the 20 px header.
// Live status changes repaint only this cluster instead of the full surface.
pub const SYSTEM_STATUS_RECT: Rect = Rect {
    x: 200,
    y: 0,
    w: 120,
    h: 20,
};

#[derive(Clone, Copy)]
pub struct FirmwareClock;

impl TimeSource for FirmwareClock {
    fn get_timestamp(&self) -> Timestamp {
        Timestamp {
            year_since_1970: 56,
            zero_indexed_month: 5,
            zero_indexed_day: 20,
            hours: 0,
            minutes: 0,
            seconds: 0,
        }
    }
}
