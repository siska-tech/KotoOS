//! Host-side bytecode fixture runner (regression / profiling harness).
//!
//! Runs an existing compiled `.kbc` app fixture through the public KotoVM API on
//! the host, with no PicoCalc hardware and no platform graphics/audio/input. It is
//! deliberately *not* the full [`koto_sim`] [`BytecodeAppSession`]: it pairs the VM
//! with a tiny [`RecordingHost`] mock that only records what each hostcall asked
//! for (id, argument count, return status) plus simple draw/audio/input tallies,
//! and reads code through [`CountingCode`] so a run reports [`VmStats`] and the
//! code-fetch count. The point is observation — instruction/hostcall/frame/code-read
//! metrics for a fixed fixture — not VM optimisation, so it changes no VM behaviour.
//!
//! The fixtures live in `package_inputs/bytecode/` (where the rest of `koto-sim`
//! already reads them), so the harness sits in `koto-sim` rather than in the
//! `no_std`, app-asset-free `koto-vm` crate.

use koto_core::runtime::{
    host_call, text_intent, BytecodeSession, CodeSource, CountingCode, HostCallOutcome,
    HostErrorCode, SessionError, SliceCode, VmBudget, VmError, VmHost, VmInputSnapshot,
    VmRunResult, VmStats,
};
use koto_gfx::{BudgetStats, DrawClass, APP_DRAW_BUDGET, DRAW_CLASS_COUNT, GAME2D_STATIC_CMD_CAP};
use koto_sim::{SIM_FRAME_FUEL, SIM_VM_CALL_DEPTH, SIM_VM_STACK_SLOTS};

// Mirror the canonical simulator VM profile so the harness verifies and runs each
// fixture under the exact stack/call/fuel limits a real launch would (KOTO-0060).
const STACK: usize = SIM_VM_STACK_SLOTS;
const CALLS: usize = SIM_VM_CALL_DEPTH;

/// The interactive game fixture exercised by the harness: it drives the retained
/// Game2D tile/sprite/text layers, immediate draws, and per-asset audio, so it is
/// the richest single fixture for hostcall coverage.
const KOTO_BLOCKS: &[u8] = include_bytes!("../../../package_inputs/bytecode/koto_blocks.kbc");
/// Sokoban uses a package-local 32x32 tile sheet and captures the visible board
/// viewport into the retained Game2D static layer.
const SOKOBAN: &[u8] = include_bytes!("../../../package_inputs/bytecode/sokoban.kbc");
/// KotoSnake migrates its fixed chrome (page/field/grid/header+HUD bars/labels) to
/// the retained Game2D static layer and its live HUD numbers to retained text,
/// keeping the flowing-rainbow snake / apple / particles on the immediate path.
const KOTOSNAKE: &[u8] = include_bytes!("../../../package_inputs/bytecode/kotosnake.kbc");
/// A second, far simpler fixture, to show the runner is fixture-agnostic.
const COUNTER_LOOP: &[u8] =
    include_bytes!("../../../package_inputs/bytecode/sample_counter_loop.kbc");

const KOTOSHOGI: &[u8] = include_bytes!("../../../package_inputs/bytecode/kotoshogi.kbc");

const KOTOMINES: &[u8] = include_bytes!("../../../package_inputs/bytecode/kotomines.kbc");

const KOTORUN: &[u8] = include_bytes!("../../../package_inputs/bytecode/kotorun.kbc");

const KOTOROGUE: &[u8] = include_bytes!("../../../package_inputs/bytecode/kotorogue.kbc");

const RETAINED_TILEMAP: &[u8] =
    include_bytes!("../../../package_inputs/bytecode/sample_retained_tilemap.kbc");

const RETAINED_TILEMAP_SCROLL: &[u8] =
    include_bytes!("../../../package_inputs/bytecode/sample_retained_tilemap_scroll.kbc");

const FULL_COLOR_TILE_IMAGE: &[u8] =
    include_bytes!("../../../package_inputs/bytecode/sample_full_color_tile_image.kbc");
const FULL_COLOR_TILE_SHEET: &[u8] = include_bytes!("../../../package_inputs/images/landscape.kim");
const FULL_COLOR_CITY_SHEET: &[u8] = include_bytes!("../../../package_inputs/images/city.kim");
const FULL_COLOR_AUTUMN_SHEET: &[u8] = include_bytes!("../../../package_inputs/images/autumn.kim");
const FULL_COLOR_SPACE_SHEET: &[u8] = include_bytes!("../../../package_inputs/images/space.kim");

const RETAINED_TILEMAP_MAP: &[u8] =
    include_bytes!("../../../apps/samples/retained_tilemap/maps/world.map");
const RETAINED_TILEMAP_SCROLL_MAP: &[u8] =
    include_bytes!("../../../apps/samples/retained_tilemap_scroll/maps/world.map");
const SOKOBAN_MAP_1: &[u8] = include_bytes!("../../../apps/sokoban/maps/01-switchback.map");
const SOKOBAN_MAP_2: &[u8] = include_bytes!("../../../apps/sokoban/maps/02-cross-dock.map");
const SOKOBAN_MAP_3: &[u8] = include_bytes!("../../../apps/sokoban/maps/03-last-mile.map");

/// The return status a hostcall reported, distilled from [`HostCallOutcome`] so the
/// harness can tally success vs. failure without retaining the (heap-bound) payloads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HostStatus {
    Ok0,
    Ok1,
    Ok2,
    Err(i32),
}

impl HostStatus {
    fn of(outcome: HostCallOutcome) -> Self {
        match outcome {
            HostCallOutcome::Ok0 => HostStatus::Ok0,
            HostCallOutcome::Ok1(_) => HostStatus::Ok1,
            HostCallOutcome::Ok2(_, _) => HostStatus::Ok2,
            HostCallOutcome::Err(code) => HostStatus::Err(code.0),
        }
    }

    fn is_err(self) -> bool {
        matches!(self, HostStatus::Err(_))
    }
}

/// One recorded hostcall: which host-call id, how many bytecode arguments it
/// consumed, and the status it returned. Enough to profile the hostcall mix of a
/// run without coupling to the host's data model.
#[derive(Clone, Copy, Debug)]
struct HostCallRecord {
    id: u8,
    args: u8,
    status: HostStatus,
}

/// A mock [`VmHost`] that records every hostcall and tallies simple draw/audio/input
/// activity, returning success for the calls the fixtures make so they run down their
/// normal path. It owns no graphics/audio/file backend — recording is the only effect,
/// so it never changes what the VM does.
#[derive(Default)]
struct RecordingHost {
    calls: Vec<HostCallRecord>,
    draws: u32,
    audio: u32,
    inputs: u32,
    /// Every `game2d_set_tile(layer, x, y, tile_ref)` placed this run, in order, so a
    /// test can prove *which* board cells a lock wrote (e.g. KotoBlocks' hard drop
    /// locking the active piece against the floor).
    tiles_set: Vec<(i32, i32, i32, i32)>,
    /// Every retained tilemap configuration, including geometry and pixel origin.
    tilemaps_configured: Vec<(i32, i32, i32, i32, i32)>,
    /// Every immediate `draw_rect(x, y, w, h, rgb565)` this run, in order, so a test
    /// can assert the *shape* of an effect's rect composition (e.g. KotoBlocks'
    /// line-clear blink drawing one full-width band per row instead of per-cell).
    rects: Vec<(i32, i32, i32, i32, i32)>,
    /// `true` while between `game2d_static_begin`/`_end`: draws issued in this window
    /// are captured into the host's retained static layer on device and never reach
    /// the per-frame immediate `APP_DRAW` list, so the budget observer must skip them.
    capturing: bool,
    /// Every immediate draw (rect / text / pixels) this run, in order, each tagged
    /// with whether it landed inside a static-capture window. This is what the KotoGFX
    /// immediate-overlay budget observer classifies and meters; it is recorded for
    /// *every* fixture but only consumed by the KotoSnake budget-observation test.
    imm: Vec<ImmDraw>,
    /// Non-static immediate draws (rect/text/pixels) issued since the last
    /// `game2d_present` — i.e. one frame's immediate command list, the same set
    /// the device pushes into its fixed `commands[MAX_APP_DRAW_COMMANDS]` buffer.
    imm_frame: usize,
    /// Peak `imm_frame` across all presented frames. The device silently drops
    /// every immediate command past `APP_DRAW_BUDGET.total_commands()`
    /// (`DeviceRuntimeHost::push` returns `NO_MEMORY`), so a fixture whose peak
    /// exceeds that cap renders correctly in the sim but is truncated on hardware
    /// — the KOTO-0185 device-only Sokoban bug. Guards assert this stays ≤ cap.
    imm_frame_peak: usize,
    /// Draws captured in the current `game2d_static_begin/end` window. The retained
    /// static layer is bounded the same way (`GAME2D_STATIC_CMD_CAP`) with the same
    /// silent tail-drop, so a per-stage chrome/board rebuild must fit it too.
    static_frame: usize,
    /// Peak `static_frame` across all rebuilds this run.
    static_frame_peak: usize,
    /// Optional test-only truncation applied to package map loads. `None` copies
    /// the complete authored asset; a limit exercises the app's bounded decoder.
    map_asset_limit: Option<usize>,
}

/// Which immediate draw primitive an [`ImmDraw`] came from. Geometry is meaningful
/// for `Rect`/`Pixels`; `Text`'s extent is glyph-dependent, so the classifier keys
/// text off its colour/role instead of `w`/`h`.
#[derive(Clone, Copy)]
enum ImmKind {
    Rect,
    Text,
    Pixels,
}

/// One recorded immediate draw, with just the fields the KotoSnake budget classifier
/// needs ([`classify_kotosnake`]): the primitive kind, whether it was inside a static
/// capture (and so excluded from the immediate list), its width/height, and its
/// RGB565 colour (`-1` when the primitive carries none, e.g. plain `draw_text`).
#[derive(Clone, Copy)]
struct ImmDraw {
    kind: ImmKind,
    in_static: bool,
    w: i32,
    h: i32,
    color: i32,
}

impl RecordingHost {
    /// Record a hostcall and pass its outcome straight back to the VM.
    fn record(&mut self, id: u8, args: u8, outcome: HostCallOutcome) -> HostCallOutcome {
        self.calls.push(HostCallRecord {
            id,
            args,
            status: HostStatus::of(outcome),
        });
        outcome
    }

    /// Count one immediate draw toward the current frame's command list, unless it
    /// is inside a `game2d_static_begin/end` window (those land in the retained
    /// static layer on device, not the per-frame immediate `commands[]` buffer).
    fn note_immediate(&mut self) {
        if self.capturing {
            self.static_frame += 1;
        } else {
            self.imm_frame += 1;
        }
    }

    /// Close one VM frame's immediate list and fold its size into the peak. The
    /// device's `clear_frame` resets the immediate `commands[]` buffer at the start
    /// of every frame regardless of present model (retained `game2d_present` or a
    /// plain immediate app), so the frame boundary — not a present call — is the
    /// right segmentation. The harness calls this after each stepped frame.
    fn roll_frame(&mut self) {
        self.imm_frame_peak = self.imm_frame_peak.max(self.imm_frame);
        self.imm_frame = 0;
    }

    fn host_call_count(&self) -> usize {
        self.calls.len()
    }

    fn failed_calls(&self) -> usize {
        self.calls.iter().filter(|c| c.status.is_err()).count()
    }

    /// Total bytecode arguments consumed across every recorded hostcall.
    fn total_args(&self) -> u64 {
        self.calls.iter().map(|c| u64::from(c.args)).sum()
    }
}

impl VmHost for RecordingHost {
    // --- Immediate draw surface ---------------------------------------------
    fn draw_rect(&mut self, x: i32, y: i32, w: i32, h: i32, rgb565: i32) -> HostCallOutcome {
        self.draws += 1;
        self.rects.push((x, y, w, h, rgb565));
        self.imm.push(ImmDraw {
            kind: ImmKind::Rect,
            in_static: self.capturing,
            w,
            h,
            color: rgb565,
        });
        self.note_immediate();
        self.record(host_call::DRAW_RECT, 5, HostCallOutcome::Ok0)
    }

    fn draw_text(&mut self, _x: i32, _y: i32, _text: &str) -> HostCallOutcome {
        self.draws += 1;
        self.imm.push(ImmDraw {
            kind: ImmKind::Text,
            in_static: self.capturing,
            w: 0,
            h: 0,
            color: -1,
        });
        self.note_immediate();
        self.record(host_call::DRAW_TEXT, 4, HostCallOutcome::Ok0)
    }

    fn draw_text_color(&mut self, _x: i32, _y: i32, _text: &str, rgb565: i32) -> HostCallOutcome {
        self.draws += 1;
        self.imm.push(ImmDraw {
            kind: ImmKind::Text,
            in_static: self.capturing,
            w: 0,
            h: 0,
            color: rgb565,
        });
        self.note_immediate();
        self.record(host_call::DRAW_TEXT_COLOR, 5, HostCallOutcome::Ok0)
    }

    fn draw_pixels_rgb565(
        &mut self,
        _x: i32,
        _y: i32,
        w: i32,
        h: i32,
        _pixels: &[u8],
    ) -> HostCallOutcome {
        self.draws += 1;
        self.imm.push(ImmDraw {
            kind: ImmKind::Pixels,
            in_static: self.capturing,
            w,
            h,
            color: -1,
        });
        self.note_immediate();
        self.record(host_call::DRAW_PIXELS_RGB565, 6, HostCallOutcome::Ok0)
    }

    fn draw_pixels_persistent_rgb565(
        &mut self,
        _x: i32,
        _y: i32,
        w: i32,
        h: i32,
        _pixels: &[u8],
    ) -> HostCallOutcome {
        self.draws += 1;
        self.imm.push(ImmDraw {
            kind: ImmKind::Pixels,
            in_static: false,
            w,
            h,
            color: -1,
        });
        self.note_immediate();
        self.record(
            host_call::DRAW_PIXELS_PERSISTENT_RGB565,
            6,
            HostCallOutcome::Ok0,
        )
    }

    // --- Retained Game2D layers ---------------------------------------------
    fn game2d_set_tile(&mut self, layer: i32, x: i32, y: i32, tile_ref: i32) -> HostCallOutcome {
        self.tiles_set.push((layer, x, y, tile_ref));
        self.record(host_call::GAME2D_SET_TILE, 4, HostCallOutcome::Ok0)
    }

    fn game2d_clear_layer(&mut self, _layer: i32) -> HostCallOutcome {
        self.record(host_call::GAME2D_CLEAR_LAYER, 1, HostCallOutcome::Ok0)
    }

    fn game2d_configure_tilemap(
        &mut self,
        layer: i32,
        columns: i32,
        rows: i32,
        origin_x: i32,
        origin_y: i32,
    ) -> HostCallOutcome {
        self.tilemaps_configured
            .push((layer, columns, rows, origin_x, origin_y));
        self.record(host_call::GAME2D_CONFIGURE_TILEMAP, 5, HostCallOutcome::Ok0)
    }

    fn game2d_present(&mut self, _heap: &[u8]) -> HostCallOutcome {
        self.draws += 1;
        self.record(host_call::GAME2D_PRESENT, 0, HostCallOutcome::Ok0)
    }

    fn game2d_static_begin(&mut self) -> HostCallOutcome {
        self.capturing = true;
        // A rebuild clears the layer on device, so its command count starts fresh.
        self.static_frame = 0;
        self.record(host_call::GAME2D_STATIC_BEGIN, 0, HostCallOutcome::Ok0)
    }

    fn game2d_static_end(&mut self) -> HostCallOutcome {
        self.capturing = false;
        self.static_frame_peak = self.static_frame_peak.max(self.static_frame);
        self.record(host_call::GAME2D_STATIC_END, 0, HostCallOutcome::Ok0)
    }

    fn game2d_stamp_define(
        &mut self,
        _stamp_id: i32,
        _cells_off: i32,
        _count: i32,
        _format: i32,
    ) -> HostCallOutcome {
        self.record(host_call::GAME2D_STAMP_DEFINE, 4, HostCallOutcome::Ok0)
    }

    fn game2d_sprite_set(
        &mut self,
        _inst_id: i32,
        _stamp_id: i32,
        _x: i32,
        _y: i32,
        _tile_ref: i32,
    ) -> HostCallOutcome {
        self.record(host_call::GAME2D_SPRITE_SET, 5, HostCallOutcome::Ok0)
    }

    fn game2d_sprite_hide(&mut self, _inst_id: i32) -> HostCallOutcome {
        self.record(host_call::GAME2D_SPRITE_HIDE, 1, HostCallOutcome::Ok0)
    }

    fn game2d_sprite_clear_all(&mut self) -> HostCallOutcome {
        self.record(host_call::GAME2D_SPRITE_CLEAR_ALL, 0, HostCallOutcome::Ok0)
    }

    fn game2d_text_set(
        &mut self,
        _id: i32,
        _x: i32,
        _y: i32,
        _text: &str,
        _rgb565: i32,
    ) -> HostCallOutcome {
        self.record(host_call::GAME2D_TEXT_SET, 6, HostCallOutcome::Ok0)
    }

    fn game2d_text_hide(&mut self, _id: i32) -> HostCallOutcome {
        self.record(host_call::GAME2D_TEXT_HIDE, 1, HostCallOutcome::Ok0)
    }

    fn game2d_text_clear_all(&mut self) -> HostCallOutcome {
        self.record(host_call::GAME2D_TEXT_CLEAR_ALL, 0, HostCallOutcome::Ok0)
    }

    // --- Input ---------------------------------------------------------------
    fn input_snapshot(&mut self, input: VmInputSnapshot) -> HostCallOutcome {
        self.inputs += 1;
        let outcome = HostCallOutcome::Ok2(input.held_bits as i32, input.pressed_bits as i32);
        self.record(host_call::INPUT_SNAPSHOT, 0, outcome)
    }

    fn text_input(&mut self, input: VmInputSnapshot) -> HostCallOutcome {
        self.inputs += 1;
        let outcome = HostCallOutcome::Ok2(input.text_codepoint as i32, input.intent_bits as i32);
        self.record(host_call::TEXT_INPUT, 0, outcome)
    }

    // --- Audio ---------------------------------------------------------------
    fn audio_submit_i16(
        &mut self,
        frames: i32,
        _channels: i32,
        _samples: &[u8],
    ) -> HostCallOutcome {
        self.audio += 1;
        self.record(host_call::AUDIO_SUBMIT_I16, 4, HostCallOutcome::Ok1(frames))
    }

    fn play_sfx(&mut self, _id: i32) -> HostCallOutcome {
        self.audio += 1;
        self.record(host_call::PLAY_SFX, 1, HostCallOutcome::Ok0)
    }

    fn play_bgm(&mut self, _id: i32) -> HostCallOutcome {
        self.audio += 1;
        self.record(host_call::PLAY_BGM, 1, HostCallOutcome::Ok0)
    }

    fn play_bgm_asset(&mut self, _path: &str) -> HostCallOutcome {
        self.audio += 1;
        self.record(host_call::PLAY_BGM_ASSET, 2, HostCallOutcome::Ok0)
    }

    fn play_sfx_asset(&mut self, _path: &str) -> HostCallOutcome {
        self.audio += 1;
        self.record(host_call::PLAY_SFX_ASSET, 2, HostCallOutcome::Ok0)
    }

    fn stop_bgm(&mut self) -> HostCallOutcome {
        self.audio += 1;
        self.record(host_call::STOP_BGM, 0, HostCallOutcome::Ok0)
    }

    // --- Files / assets ------------------------------------------------------
    // No backing store: report calls as recorded failures so a fixture sees a clean
    // "unavailable" status rather than fabricated data.
    fn file_open(&mut self, _path: &str, _mode: i32) -> HostCallOutcome {
        self.record(
            host_call::FILE_OPEN,
            3,
            HostCallOutcome::Err(HostErrorCode::NOT_FOUND),
        )
    }

    fn file_read(&mut self, _handle: i32, _dst: &mut [u8]) -> HostCallOutcome {
        self.record(
            host_call::FILE_READ,
            3,
            HostCallOutcome::Err(HostErrorCode::BAD_ARGUMENT),
        )
    }

    fn file_write(&mut self, _handle: i32, _src: &[u8]) -> HostCallOutcome {
        self.record(
            host_call::FILE_WRITE,
            3,
            HostCallOutcome::Err(HostErrorCode::BAD_ARGUMENT),
        )
    }

    fn file_close(&mut self, _handle: i32) -> HostCallOutcome {
        self.record(
            host_call::FILE_CLOSE,
            1,
            HostCallOutcome::Err(HostErrorCode::BAD_ARGUMENT),
        )
    }

    fn asset_load(&mut self, path: &str, dst: &mut [u8]) -> HostCallOutcome {
        let source = match path {
            "maps/world.map" if dst.len() == 440 => Some(RETAINED_TILEMAP_MAP),
            "maps/world.map" if dst.len() == 816 => Some(RETAINED_TILEMAP_SCROLL_MAP),
            "maps/01-switchback.map" => Some(SOKOBAN_MAP_1),
            "maps/02-cross-dock.map" => Some(SOKOBAN_MAP_2),
            "maps/03-last-mile.map" => Some(SOKOBAN_MAP_3),
            _ => None,
        };
        let copied = if let Some(source) = source {
            let limit = self.map_asset_limit.unwrap_or(usize::MAX);
            let len = source.len().min(dst.len()).min(limit);
            dst[..len].copy_from_slice(&source[..len]);
            len
        } else {
            // Non-map assets are irrelevant to this recording host's geometry
            // checks; report an empty successful load as before.
            0
        };
        self.record(
            host_call::ASSET_LOAD,
            4,
            HostCallOutcome::Ok1(copied as i32),
        )
    }

    fn asset_load_range(&mut self, path: &str, offset: usize, dst: &mut [u8]) -> HostCallOutcome {
        let source = match path {
            "images/landscape.kim" => Some(FULL_COLOR_TILE_SHEET),
            "images/city.kim" => Some(FULL_COLOR_CITY_SHEET),
            "images/autumn.kim" => Some(FULL_COLOR_AUTUMN_SHEET),
            "images/space.kim" => Some(FULL_COLOR_SPACE_SHEET),
            _ => None,
        };
        let copied = source
            .and_then(|source| source.get(offset..))
            .map_or(0, |source| {
                let len = source.len().min(dst.len());
                dst[..len].copy_from_slice(&source[..len]);
                len
            });
        self.record(
            host_call::ASSET_LOAD_RANGE,
            5,
            HostCallOutcome::Ok1(copied as i32),
        )
    }
}

/// One stepped frame's own metrics, captured as the run progresses so the profile
/// can show the per-frame shape (warm-up vs. steady state) rather than only the
/// session totals. Each value is that frame's delta, derived from the session's
/// per-frame accessors and a [`CountingCode`] read snapshot taken around the step.
#[derive(Clone, Copy, Debug)]
struct FrameMetrics {
    /// Instructions stepped this frame (`last_frame_fuel`): the fuel this frame used.
    instructions: u32,
    /// Per-frame fuel left unspent (`frame_fuel - instructions`).
    fuel_remaining: u32,
    /// VM-level `HOST_CALL`s this frame (`last_frame_host_calls`); includes the
    /// internally handled `yield_frame`/`exit` the mock host never sees.
    vm_host_calls: u32,
    /// Hostcalls the mock host actually recorded this frame (external dispatches).
    recorded_host_calls: usize,
    /// Code-word fetches served this frame (CountingCode delta around the step).
    code_reads: u64,
    /// The frame's run result.
    result: VmRunResult,
}

/// The metrics a fixture run produced. Combines the VM's own cumulative
/// [`VmStats`], the per-session high-water [`VmBudget`] peaks, the [`CountingCode`]
/// code-read count, the mock host's recorded hostcall activity, a per-frame
/// breakdown, and — behind the `opcode_stats` feature — the opcode histogram.
struct FixtureReport {
    frames_run: u32,
    final_result: VmRunResult,
    trap: Option<VmError>,
    stats: VmStats,
    budget: VmBudget,
    /// The per-frame instruction budget the session ran each frame against.
    frame_fuel: u32,
    code_reads: u64,
    host_calls: usize,
    failed_host_calls: usize,
    host_call_args: u64,
    /// VM-level hostcalls beyond what the mock host recorded: the internally
    /// handled `yield_frame`/`exit` calls the VM counts but never routes to a
    /// [`VmHost`] method (`stats.host_calls - host_calls`).
    internal_host_calls: u64,
    draws: u32,
    audio: u32,
    inputs: u32,
    /// Worst-case per-frame immediate command count (rect/text/pixels outside a
    /// static-capture window), as segmented by `game2d_present`. Must stay within
    /// the device's immediate cap (`APP_DRAW_BUDGET.total_commands()`); above it the
    /// firmware drops the tail and the panel loses whatever drew last (KOTO-0185).
    immediate_frame_peak: usize,
    /// Worst-case per-rebuild retained static-layer command count (draws captured in a
    /// `game2d_static_begin/end` window). Must stay within `GAME2D_STATIC_CMD_CAP`.
    static_frame_peak: usize,
    /// Retained tile writes and configurations retained for sample contract tests.
    tiles_set: Vec<(i32, i32, i32, i32)>,
    tilemaps_configured: Vec<(i32, i32, i32, i32, i32)>,
    /// Per host-call-id totals, sorted by id, for the printed summary.
    by_id: Vec<(u8, usize)>,
    /// One entry per stepped frame, in execution order.
    per_frame: Vec<FrameMetrics>,
    /// Per-opcode execution counts `(opcode, count)` for opcodes that ran at least
    /// once, sorted by descending count. Only captured under the `opcode_stats`
    /// feature; empty otherwise.
    #[cfg(feature = "opcode_stats")]
    opcode_hist: Vec<(u8, u64)>,
}

impl FixtureReport {
    /// How many times the host recorded a given hostcall id over the whole run
    /// (0 if it never fired). Reads the per-id tally the report already built.
    fn calls_to(&self, id: u8) -> usize {
        self.by_id
            .iter()
            .find(|(rid, _)| *rid == id)
            .map(|(_, count)| *count)
            .unwrap_or(0)
    }

    fn print(&self, label: &str) {
        println!("=== fixture run: {label} ===");
        println!(
            "  frames={} result={:?} trap={:?}",
            self.frames_run, self.final_result, self.trap
        );
        println!(
            "  VmStats (cumulative): instructions={} host_calls={} frames={}",
            self.stats.instructions, self.stats.host_calls, self.stats.frames
        );
        println!(
            "  hostcalls: recorded={} (failed={}) internal(yield/exit)={} args={}",
            self.host_calls, self.failed_host_calls, self.internal_host_calls, self.host_call_args
        );
        println!(
            "  CountingCode: code_reads={} (~{} bytes)",
            self.code_reads,
            self.code_reads * 4
        );
        println!(
            "  budget peaks: stack_slots={} call_depth={} local_slots={} heap_bytes={} frame_fuel={} hostcalls/frame={}",
            self.budget.stack_slots_peak,
            self.budget.call_depth_peak,
            self.budget.local_slots_peak,
            self.budget.heap_bytes_peak,
            self.budget.frame_fuel_peak,
            self.budget.host_calls_per_frame_peak,
        );
        println!(
            "  fuel: per_frame_budget={} cumulative_used={} cumulative_remaining={}",
            self.frame_fuel,
            self.stats.instructions,
            (self.frame_fuel as u64 * u64::from(self.frames_run))
                .saturating_sub(self.stats.instructions),
        );
        println!(
            "  draws={} audio={} inputs={} immediate_frame_peak={}/{} static_rebuild_peak={}/{}",
            self.draws,
            self.audio,
            self.inputs,
            self.immediate_frame_peak,
            APP_DRAW_BUDGET.total_commands(),
            self.static_frame_peak,
            GAME2D_STATIC_CMD_CAP,
        );

        println!("  per-frame:");
        println!(
            "    {:>3}  {:>8}  {:>10}  {:>8}  {:>8}  {:>9}  {:?}",
            "f", "instrs", "fuel_left", "vm_hc", "rec_hc", "code_rd", "result"
        );
        for (i, frame) in self.per_frame.iter().enumerate() {
            println!(
                "    {:>3}  {:>8}  {:>10}  {:>8}  {:>8}  {:>9}  {:?}",
                i,
                frame.instructions,
                frame.fuel_remaining,
                frame.vm_host_calls,
                frame.recorded_host_calls,
                frame.code_reads,
                frame.result,
            );
        }

        println!("  hostcalls by id:");
        for (id, count) in &self.by_id {
            println!("    [{:#04x}] {:<22} x{}", id, host_call::name(*id), count);
        }

        #[cfg(feature = "opcode_stats")]
        {
            println!("  opcode histogram (executed opcodes, by count):");
            for (op, count) in &self.opcode_hist {
                let pct = if self.stats.instructions > 0 {
                    *count as f64 * 100.0 / self.stats.instructions as f64
                } else {
                    0.0
                };
                println!(
                    "    [{:#04x}] {:<14} x{:<10} {:>5.1}%",
                    op,
                    opcode_name(*op),
                    count,
                    pct
                );
            }
        }
    }
}

/// A short, stable mnemonic for a VM opcode byte, for the `opcode_stats` histogram.
/// Kept here (test-only) so the profiler can label the histogram without adding a
/// name table to the `no_std` runtime crate. Unknown bytes show as `"unknown"`.
#[cfg(feature = "opcode_stats")]
fn opcode_name(op: u8) -> &'static str {
    use koto_core::runtime::opcode::*;
    match op {
        NOP => "nop",
        HALT => "halt",
        BR => "br",
        BR_IF_ZERO => "br_if_zero",
        CALL => "call",
        RET => "ret",
        PUSH_I16 => "push_i16",
        DUP => "dup",
        DROP => "drop",
        SWAP => "swap",
        LOAD_LOCAL => "load_local",
        STORE_LOCAL => "store_local",
        ADD_I32 => "add_i32",
        SUB_I32 => "sub_i32",
        MUL_I32 => "mul_i32",
        DIV_I32 => "div_i32",
        AND_I32 => "and_i32",
        OR_I32 => "or_i32",
        XOR_I32 => "xor_i32",
        SHL_I32 => "shl_i32",
        SHR_I32 => "shr_i32",
        LOAD8 => "load8",
        STORE8 => "store8",
        LOAD16 => "load16",
        STORE16 => "store16",
        LOAD32 => "load32",
        STORE32 => "store32",
        HOST_CALL => "host_call",
        _ => "unknown",
    }
}

/// Verify `bytes` under the simulator profile, then step it for up to `max_frames`
/// frames (or until it exits) through a [`RecordingHost`], reading code through a
/// [`CountingCode`]. `input` is supplied to every frame.
fn run_fixture(
    bytes: &[u8],
    max_frames: u32,
    input: VmInputSnapshot,
) -> Result<FixtureReport, SessionError> {
    run_fixture_core(bytes, max_frames, |_| input)
}

/// Like [`run_fixture`] but drives a *scripted* input, one [`VmInputSnapshot`] per
/// frame: frame `f` is fed `inputs[f]`, and once the script runs out the final
/// entry repeats. This lets a test walk an interactive fixture through state
/// transitions (e.g. KotoBlocks' title -> play) that a single fixed input cannot
/// reach.
fn run_fixture_scripted(
    bytes: &[u8],
    inputs: &[VmInputSnapshot],
) -> Result<FixtureReport, SessionError> {
    assert!(!inputs.is_empty(), "scripted run needs at least one frame");
    let last = inputs.len() - 1;
    run_fixture_core(bytes, inputs.len() as u32, |frame| {
        inputs[(frame as usize).min(last)]
    })
}

/// Shared body of [`run_fixture`] / [`run_fixture_scripted`]: verifies `bytes`,
/// then steps it for up to `max_frames` frames (or until it exits), asking
/// `input_for` for the [`VmInputSnapshot`] to feed each frame (by zero-based frame
/// index).
fn run_fixture_core(
    bytes: &[u8],
    max_frames: u32,
    input_for: impl FnMut(u32) -> VmInputSnapshot,
) -> Result<FixtureReport, SessionError> {
    run_fixture_core_with_map_limit(bytes, max_frames, None, input_for)
}

fn run_fixture_core_with_map_limit(
    bytes: &[u8],
    max_frames: u32,
    map_asset_limit: Option<usize>,
    mut input_for: impl FnMut(u32) -> VmInputSnapshot,
) -> Result<FixtureReport, SessionError> {
    let mut session = BytecodeSession::<STACK, CALLS>::new(
        bytes,
        koto_core::RuntimeLimits::simulator_default(),
        SIM_FRAME_FUEL,
    )?;

    // Per-app heap sized to the program's own KBC header request (KOTO-0096),
    // initialised with the const heap image (rodata -> heap[0..rodata_size],
    // KOTO-0139). The verifier has already bounded the copy in range.
    let heap_bytes = session.program().header().max_heap_bytes as usize;
    let mut heap = vec![0u8; heap_bytes];
    if let Some((start, end)) = session.program().rodata_range() {
        heap[..end - start].copy_from_slice(&bytes[start..end]);
    }

    let mut host = RecordingHost {
        map_asset_limit,
        ..RecordingHost::default()
    };
    let mut code = CountingCode::new(SliceCode::new(bytes, session.program().code_range().0));

    let mut frames_run = 0;
    let mut final_result = session.result();
    let mut trap = None;
    let mut per_frame: Vec<FrameMetrics> = Vec::new();
    // Track cumulative code reads / recorded hostcalls so each frame's delta can be
    // derived without resetting the counters the cumulative report still relies on.
    let mut prev_code_reads = code.reads();
    let mut prev_recorded = host.host_call_count();
    for frame_index in 0..max_frames {
        if session.has_exited() {
            break;
        }
        match session.step_frame_with(&mut code, &mut host, input_for(frame_index), &mut heap) {
            Ok(result) => {
                frames_run += 1;
                final_result = result;
                // Segment the immediate-command peak per frame (matches the device's
                // per-frame `clear_frame`), independent of whether the app presents
                // via the retained compositor or the plain immediate path.
                host.roll_frame();
                let code_reads = code.reads() - prev_code_reads;
                let recorded_host_calls = host.host_call_count() - prev_recorded;
                prev_code_reads = code.reads();
                prev_recorded = host.host_call_count();
                per_frame.push(FrameMetrics {
                    instructions: session.last_frame_fuel(),
                    fuel_remaining: session.last_frame_fuel_remaining(),
                    vm_host_calls: session.last_frame_host_calls(),
                    recorded_host_calls,
                    code_reads,
                    result,
                });
            }
            Err(error) => {
                trap = Some(error);
                break;
            }
        }
    }

    // Universal device-parity invariant (KOTO-0185). The device flushes each frame's
    // immediate draws from a fixed `commands[MAX_APP_DRAW_COMMANDS]` buffer and
    // `DeviceRuntimeHost::push` silently drops everything past the cap; the sim's
    // immediate list is an unbounded `Vec`, so a fixture can render perfectly here yet
    // lose most of its frame on hardware (Sokoban's ~340-command board was the
    // symptom). Every fixture must keep its worst per-frame immediate count within the
    // cap — non-moving art belongs in the retained static/board/text layers, which do
    // not count against it.
    let immediate_cap = APP_DRAW_BUDGET.total_commands();
    assert!(
        host.imm_frame_peak <= immediate_cap,
        "immediate command overflow: per-frame peak {} > device cap {} — the firmware \
         would drop {} commands every frame and the panel would lose whatever drew last \
         (KOTO-0185). Move fixed art into the retained static layer or thin the per-frame \
         immediate draws.",
        host.imm_frame_peak,
        immediate_cap,
        host.imm_frame_peak - immediate_cap,
    );
    // Same invariant for the retained static layer's per-rebuild command count.
    assert!(
        host.static_frame_peak <= GAME2D_STATIC_CMD_CAP,
        "static layer overflow: rebuild peak {} > cap {} — the firmware would drop {} \
         chrome/board commands (KOTO-0185). Thin the static rebuild.",
        host.static_frame_peak,
        GAME2D_STATIC_CMD_CAP,
        host.static_frame_peak - GAME2D_STATIC_CMD_CAP,
    );

    // Tally recorded hostcalls per id (sorted) for the summary.
    let mut by_id: Vec<(u8, usize)> = Vec::new();
    for record in &host.calls {
        match by_id.iter_mut().find(|(id, _)| *id == record.id) {
            Some((_, count)) => *count += 1,
            None => by_id.push((record.id, 1)),
        }
    }
    by_id.sort_by_key(|(id, _)| *id);

    let stats = session.stats();
    let host_calls = host.host_call_count();
    let internal_host_calls = stats.host_calls.saturating_sub(host_calls as u64);

    // Snapshot the opcode histogram while the session is still alive: pull out the
    // opcodes that actually executed, ordered by descending count for the report.
    #[cfg(feature = "opcode_stats")]
    let opcode_hist = {
        let mut hist: Vec<(u8, u64)> = session
            .opcode_counts()
            .iter()
            .enumerate()
            .filter(|(_, &count)| count > 0)
            .map(|(op, &count)| (op as u8, count))
            .collect();
        hist.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        hist
    };

    Ok(FixtureReport {
        frames_run,
        final_result,
        trap,
        stats,
        budget: session.budget(),
        frame_fuel: session.frame_fuel(),
        code_reads: code.reads(),
        host_calls,
        failed_host_calls: host.failed_calls(),
        host_call_args: host.total_args(),
        internal_host_calls,
        immediate_frame_peak: host.imm_frame_peak,
        static_frame_peak: host.static_frame_peak,
        tiles_set: host.tiles_set,
        tilemaps_configured: host.tilemaps_configured,
        draws: host.draws,
        audio: host.audio,
        inputs: host.inputs,
        by_id,
        per_frame,
        #[cfg(feature = "opcode_stats")]
        opcode_hist,
    })
}

/// KOTO-0200: the static SDK sample connects an editor-authored 20x20 ASCII map
/// to the generic retained layer. It configures once, uploads each of the 400
/// cells once, presents every frame, and becomes write-free once initialized.
#[test]
fn retained_tilemap_sample_uploads_once_then_idles() {
    const FRAMES: u32 = 24;
    let report = run_fixture(RETAINED_TILEMAP, FRAMES, VmInputSnapshot::empty())
        .expect("retained tilemap sample verifies");

    assert_eq!(report.trap, None);
    assert_eq!(report.frames_run, FRAMES);
    assert_eq!(report.tilemaps_configured, vec![(0, 20, 20, 0, 0)]);
    assert_eq!(report.calls_to(host_call::GAME2D_CONFIGURE_TILEMAP), 1);
    assert_eq!(report.calls_to(host_call::ASSET_LOAD), 2);
    assert_eq!(report.calls_to(host_call::GAME2D_STATIC_BEGIN), 0);
    assert_eq!(report.calls_to(host_call::GAME2D_STATIC_END), 0);
    assert_eq!(report.calls_to(host_call::GAME2D_SET_TILE), 400);
    assert_eq!(report.calls_to(host_call::GAME2D_PRESENT), FRAMES as usize);
    assert_eq!(report.tiles_set.len(), 400);
    assert!(report
        .tiles_set
        .iter()
        .all(|&(layer, x, y, tile_ref)| layer == 0
            && (0..20).contains(&x)
            && (0..20).contains(&y)
            && tile_ref >= 0));
}

/// Four exact 320x320 RGB565 images stream eight scanlines at a time while the
/// gallery alternates a dissolve and column wipe using persistent LCD updates.
#[test]
fn full_color_image_streams_scanline_bands_with_bounded_heap() {
    const FRAMES: u32 = 500;
    let report = run_fixture(FULL_COLOR_TILE_IMAGE, FRAMES, VmInputSnapshot::empty())
        .expect("full-color image transition gallery verifies");

    assert_eq!(report.trap, None);
    assert_eq!(report.frames_run, FRAMES);
    assert!(report.tilemaps_configured.is_empty());
    assert_eq!(report.calls_to(host_call::ASSET_LOAD_RANGE), 120);
    assert_eq!(report.calls_to(host_call::PLAY_BGM_ASSET), 1);
    assert_eq!(
        report.calls_to(host_call::DRAW_PIXELS_PERSISTENT_RGB565),
        920
    );
    assert!(report.immediate_frame_peak <= 20);
}

#[test]
fn retained_tilemap_rejects_truncated_map_asset_before_upload() {
    let report = run_fixture_core_with_map_limit(RETAINED_TILEMAP, 1, Some(16), |_| {
        VmInputSnapshot::empty()
    })
    .expect("truncated map exits safely");

    assert_eq!(report.trap, None);
    assert_eq!(report.final_result, VmRunResult::Exited(2));
    assert_eq!(report.calls_to(host_call::GAME2D_CONFIGURE_TILEMAP), 0);
    assert_eq!(report.calls_to(host_call::GAME2D_SET_TILE), 0);
}

/// KOTO-0200: the scrolling sample retains a bounded 20x20 viewport over its
/// larger authored world. One right input starts a staged content diff: at least
/// one visible cell changes, but homogeneous cells do not get rewritten.
#[test]
fn retained_tilemap_scroll_updates_only_changed_visible_cells() {
    const FRAMES: u32 = 24;
    let idle = run_fixture(RETAINED_TILEMAP_SCROLL, FRAMES, VmInputSnapshot::empty())
        .expect("scrolling tilemap sample verifies");
    let moved = run_fixture_core(RETAINED_TILEMAP_SCROLL, FRAMES, |frame| {
        if frame == 16 {
            VmInputSnapshot {
                intent_bits: text_intent::RIGHT,
                ..VmInputSnapshot::empty()
            }
        } else {
            VmInputSnapshot::empty()
        }
    })
    .expect("scrolling tilemap sample verifies with input");

    assert_eq!(idle.trap, None);
    assert_eq!(idle.tilemaps_configured, vec![(0, 20, 20, 0, 0)]);
    assert_eq!(idle.calls_to(host_call::ASSET_LOAD), 2);
    assert_eq!(idle.calls_to(host_call::GAME2D_STATIC_BEGIN), 0);
    assert_eq!(idle.calls_to(host_call::GAME2D_STATIC_END), 0);
    assert_eq!(idle.calls_to(host_call::GAME2D_SET_TILE), 400);
    assert_eq!(idle.calls_to(host_call::GAME2D_PRESENT), FRAMES as usize);

    assert_eq!(moved.trap, None);
    assert_eq!(moved.tilemaps_configured, vec![(0, 20, 20, 0, 0)]);
    let changed = moved.tiles_set.len() - idle.tiles_set.len();
    assert!(changed > 0, "camera movement changed no visible tile");
    assert!(
        changed < 400,
        "camera movement rewrote the complete viewport instead of diffing it"
    );
    assert!(moved.tiles_set[400..]
        .iter()
        .all(|&(layer, x, y, _)| { layer == 0 && (0..20).contains(&x) && (0..20).contains(&y) }));
}

/// Repeated left steps reach camera x=0. A further left input is clamped and
/// therefore schedules no extra viewport scan or retained writes.
#[test]
fn retained_tilemap_scroll_clamps_at_world_boundary() {
    let scripted = |frame| {
        if matches!(frame, 8 | 16 | 24 | 32 | 40 | 48 | 56) {
            VmInputSnapshot {
                intent_bits: text_intent::LEFT,
                ..VmInputSnapshot::empty()
            }
        } else {
            VmInputSnapshot::empty()
        }
    };
    let at_boundary = run_fixture_core(RETAINED_TILEMAP_SCROLL, 56, scripted)
        .expect("scrolling tilemap reaches left boundary");
    let past_boundary = run_fixture_core(RETAINED_TILEMAP_SCROLL, 64, scripted)
        .expect("scrolling tilemap clamps past left boundary");

    assert_eq!(at_boundary.trap, None);
    assert_eq!(past_boundary.trap, None);
    assert_eq!(
        past_boundary.tiles_set.len(),
        at_boundary.tiles_set.len(),
        "a clamped camera step emitted retained tile writes"
    );
    assert_eq!(past_boundary.tilemaps_configured, vec![(0, 20, 20, 0, 0)]);
}

/// The richest fixture: KotoBlocks runs cleanly for a handful of frames and drives a
/// broad hostcall mix, so the harness can assert real VM + code-read + hostcall work.
#[test]
fn koto_blocks_runs_and_reports_metrics() {
    const FRAMES: u32 = 8;
    let report =
        run_fixture(KOTO_BLOCKS, FRAMES, VmInputSnapshot::empty()).expect("koto_blocks verifies");
    report.print("koto_blocks");

    // The fixture runs without trapping under the mock host.
    assert_eq!(report.trap, None, "koto_blocks trapped: {:?}", report.trap);
    // It is an interactive app (no early exit), so it steps the full frame budget.
    assert_eq!(report.frames_run, FRAMES);
    assert_eq!(report.stats.frames, u64::from(FRAMES));

    // The VM actually executed work, and every executed instruction was fetched
    // through the counting code source (one code-word read per stepped instruction).
    assert!(report.stats.instructions > 0, "no instructions executed");
    assert!(
        report.code_reads >= report.stats.instructions,
        "code reads < instructions"
    );

    // It made hostcalls. The VM's own cumulative hostcall counter is at least what
    // the mock host recorded: the VM also counts `yield_frame`/`exit`, which it
    // handles internally and never routes to a `VmHost` method. The surplus here is
    // exactly the one `yield_frame` per stepped frame.
    assert!(report.host_calls > 0, "no hostcalls recorded");
    assert!(
        report.stats.host_calls >= report.host_calls as u64,
        "VM hostcall count {} below recorded {}",
        report.stats.host_calls,
        report.host_calls
    );
    assert_eq!(
        report.stats.host_calls - report.host_calls as u64,
        u64::from(report.frames_run),
        "surplus VM hostcalls should be one yield_frame per frame"
    );

    // KotoBlocks draws every frame, so the draw tally is non-zero.
    assert!(report.draws > 0, "no draw hostcalls recorded");

    // Per-frame capture lines up with the cumulative totals, and the run stayed
    // within the simulator profile it was verified under (no fuel exhaustion, peaks
    // under the configured capacities) — confirming the metrics are self-consistent.
    assert_eq!(report.per_frame.len(), report.frames_run as usize);
    let frame_instr_sum: u64 = report
        .per_frame
        .iter()
        .map(|f| u64::from(f.instructions))
        .sum();
    assert_eq!(frame_instr_sum, report.stats.instructions);
    assert_eq!(report.internal_host_calls, u64::from(report.frames_run));
    assert!(
        report.budget.frame_fuel_peak < report.frame_fuel,
        "a frame exhausted its fuel"
    );
    assert!(report.budget.stack_slots_peak <= STACK as u16);
    assert!(report.budget.call_depth_peak <= CALLS as u16);
}

/// KotoBlocks renders *gameplay* through the retained Game2D layers
/// (KOTO-0135 tilemap, KOTO-0136 static UI, KOTO-0140 sprites, KOTO-0141 text), not
/// the immediate draw path. The 8-frame empty-input profile in
/// [`koto_blocks_runs_and_reports_metrics`] (and `docs/devlog/KOTO_VM_PROFILE_KOTOBLOCKS.md`)
/// only ever exercises the *title screen*, which still draws immediately and never
/// crosses the title->play transition — so it records zero `game2d_*` calls. This
/// test scripts the input past that transition into live play and proves the
/// retained layers are the ones doing the per-frame board/piece/stats rendering.
#[test]
fn koto_blocks_play_uses_retained_game2d_layers() {
    let empty = VmInputSnapshot::empty();
    let start = VmInputSnapshot {
        intent_bits: text_intent::NEWLINE,
        ..VmInputSnapshot::empty()
    };
    // Space (codepoint 32) is KotoBlocks' hard drop: it locks the active piece this
    // frame, mutating board cells so the next render `game2d_set_tile`s them. Without
    // it the board stays empty for the short window we step (gravity is ~33 frames
    // per row at level 1), and `set_tile` — which only fires on a changed cell —
    // would not run.
    let hard_drop = VmInputSnapshot {
        text_codepoint: 32,
        ..VmInputSnapshot::empty()
    };

    // The title screen bakes one tetromino tile per frame (4 rows/frame x 7 tiles =
    // 28 frames) before the F1 "start" prompt arms, so step well past the bake, feed
    // the start intent (which builds the retained static layer and defines the sprite
    // stamps on the title->play transition), then play: spawn a piece, hard-drop it
    // to force a board mutation, and run a few more frames.
    let mut inputs = vec![empty; 34];
    inputs.push(start); // title -> play: static_begin/end + stamp_define
    inputs.push(empty); // first play frame: spawn piece, place sprites/text, present
    inputs.push(empty);
    inputs.push(hard_drop); // lock the piece -> board cells change
    inputs.extend(std::iter::repeat_n(empty, 4));

    let report = run_fixture_scripted(KOTO_BLOCKS, &inputs).expect("koto_blocks verifies");
    report.print("koto_blocks_play");

    assert_eq!(
        report.trap, None,
        "koto_blocks trapped in play: {:?}",
        report.trap
    );

    // The retained Game2D layers — and not just immediate draws — are exercised in
    // play. Each of these hostcalls is one the title-screen-only profile never makes.
    assert!(
        report.calls_to(host_call::GAME2D_STATIC_BEGIN) >= 1,
        "retained static UI layer was never built (title->play transition not reached)"
    );
    assert!(
        report.calls_to(host_call::GAME2D_STAMP_DEFINE) >= 1,
        "no sprite stamps defined for the retained sprite layer"
    );
    assert!(
        report.calls_to(host_call::GAME2D_SPRITE_SET) >= 1,
        "active piece / ghost / NEXT / HOLD never placed as retained sprites"
    );
    assert!(
        report.calls_to(host_call::GAME2D_TEXT_SET) >= 1,
        "status values never rendered through the retained text layer"
    );
    assert!(
        report.calls_to(host_call::GAME2D_CONFIGURE_TILEMAP) >= 1,
        "retained tilemap geometry was not configured"
    );
    assert!(
        report.calls_to(host_call::GAME2D_SET_TILE) >= 1,
        "locked board cells never written to the retained tilemap"
    );
    assert!(
        report.calls_to(host_call::GAME2D_PRESENT) >= 1,
        "retained layers were never composited via game2d_present"
    );
}

/// Sokoban renders its board with the retained static layer plus a thin immediate
/// pass, not the 16x16 tilemap ABI (its cells are 28px). The floor, walls, and
/// empty-goal rings are captured once per stage into the retained static layer;
/// changing HUD values go through retained text; and only the moving crates and
/// porter stay on the per-frame immediate path. This keeps the immediate command
/// count far under the device's `MAX_APP_DRAW_COMMANDS` cap — the KOTO-0185 fix,
/// since drawing the whole board immediately every frame (~340 commands) overflowed
/// the cap and the device silently dropped most of the board (invisible in the sim,
/// whose immediate list is unbounded).
#[test]
fn sokoban_play_uses_retained_game2d_chrome_and_text() {
    const FRAMES: u32 = 4;
    let report = run_fixture(SOKOBAN, FRAMES, VmInputSnapshot::empty()).expect("sokoban verifies");
    report.print("sokoban_play");

    assert_eq!(
        report.trap, None,
        "sokoban trapped in play: {:?}",
        report.trap
    );
    assert_eq!(report.frames_run, FRAMES);

    assert!(
        report.calls_to(host_call::GAME2D_STATIC_BEGIN) >= 1,
        "retained static chrome layer was never built"
    );
    assert!(
        report.calls_to(host_call::GAME2D_STATIC_END) >= 1,
        "retained static chrome capture never ended"
    );
    assert!(
        report.calls_to(host_call::GAME2D_TEXT_SET) >= 3,
        "stage/steps/push counters were not rendered through retained text"
    );
    assert!(
        report.calls_to(host_call::GAME2D_PRESENT) >= FRAMES as usize,
        "gameplay frames were not composited via game2d_present"
    );
    // KOTO-0185 device-parity guard. The pre-fix board issued ~340 immediate
    // draw_rect commands per frame — ~3.5x the device cap — so the firmware dropped
    // most of the board on hardware while the sim rendered it in full. With the fixed
    // art in the retained static layer, only crates + porter are immediate (~30/frame).
    // The shared runner already asserts the cap for every fixture; this pins it
    // explicitly against Sokoban so a future board-art change creeping back toward the
    // cap fails on this named expectation, not a generic runner panic.
    assert!(
        report.immediate_frame_peak <= APP_DRAW_BUDGET.total_commands(),
        "Sokoban per-frame immediate draws {} exceed the device cap {} — board art must \
         stay in the retained static layer",
        report.immediate_frame_peak,
        APP_DRAW_BUDGET.total_commands(),
    );
    assert!(
        report.immediate_frame_peak > 0,
        "Sokoban drew nothing on the immediate path — the crates and porter render there"
    );
    assert!(
        report.calls_to(host_call::DRAW_PIXELS_RGB565) == 0,
        "Sokoban draws with rects, not pixel blits — no draw_pixels expected"
    );
    assert_eq!(
        report.calls_to(host_call::GAME2D_SET_TILE),
        0,
        "Sokoban should not force its 28px board through the 16px retained tilemap"
    );
}

/// KotoSnake renders steady gameplay through the retained Game2D layers rather than
/// re-emitting its chrome every frame. The empty-input profile only ever sees the
/// title screen (which draws immediately and never crosses the title->play
/// transition), so it records zero `game2d_*` calls; this scripts the F1 start
/// intent to cross into play and proves the static chrome + retained HUD text are the
/// path doing the per-frame work. KotoSnake uses neither the 16x16 retained tilemap
/// (whose fixed geometry is KotoBlocks' 10x20 well, not the 18x14 field) nor sprites
/// — its flowing-rainbow snake, breathing apple, and particle bursts are per-frame-
/// animated immediate draws — so it issues no `game2d_set_tile` and needs no present.
#[test]
fn kotosnake_play_uses_retained_static_and_text_layers() {
    let empty = VmInputSnapshot::empty();
    let start = VmInputSnapshot {
        intent_bits: text_intent::NEWLINE,
        ..VmInputSnapshot::empty()
    };
    // Frame 0 crosses title->play (initialises the game and builds the retained
    // static chrome layer); the following frames are steady gameplay, each setting
    // the score/length/best retained text values.
    let mut inputs = vec![start];
    inputs.extend(std::iter::repeat_n(empty, 4));

    let report = run_fixture_scripted(KOTOSNAKE, &inputs).expect("kotosnake verifies");
    report.print("kotosnake_play");

    assert_eq!(
        report.trap, None,
        "kotosnake trapped in play: {:?}",
        report.trap
    );

    // The fixed page/field/grid/header+HUD chrome is captured into the retained
    // static layer once, on the title->play transition.
    assert!(
        report.calls_to(host_call::GAME2D_STATIC_BEGIN) >= 1,
        "retained static chrome layer was never built (title->play not reached)"
    );
    assert!(
        report.calls_to(host_call::GAME2D_STATIC_END) >= 1,
        "retained static chrome capture never ended"
    );
    // The live HUD values (score, length, best) render as retained text every
    // gameplay frame, so a steady play frame sets at least three text items.
    assert!(
        report.calls_to(host_call::GAME2D_TEXT_SET) >= 3,
        "score/length/best were not rendered through the retained text layer"
    );
    // KotoSnake's animated board stays immediate, so it never drives the 16x16
    // retained tilemap nor composites a tilemap/sprite layer via present.
    assert_eq!(
        report.calls_to(host_call::GAME2D_SET_TILE),
        0,
        "KotoSnake should not force its 18x14 field through the 16px retained tilemap"
    );
    assert_eq!(
        report.calls_to(host_call::GAME2D_PRESENT),
        0,
        "KotoSnake uses no retained tilemap/sprite layer, so it should not present"
    );
}

/// `true` if `col` is one of KotoSnake's 12 flowing-rainbow hues (the `rainbow()`
/// table in `apps/kotosnake/src/main.koto`), which tint the snake body, the tail
/// runs, the food-pickup sparks, and the eat-flash / banner accent lines.
fn is_snake_rainbow(col: i32) -> bool {
    matches!(
        col,
        63488 | 64256 | 65344 | 38880 | 2016 | 2034 | 1887 | 1119 | 8479 | 28703 | 49183 | 63512
    )
}

/// Classify one KotoSnake immediate draw into a KotoGFX [`DrawClass`], host-side, from
/// the palette + geometry signatures in `apps/kotosnake/src/main.koto` (no bytecode or
/// hostcall change — the app is unmodified; this reads what it already emits).
///
/// KotoSnake routes its fixed board/grid/bars to the retained Game2D *static* layer and
/// its score/length/best HUD to retained *text*, so the immediate path carries **no**
/// `CoreGameplay` and **no** `CriticalUi`. What remains is: the animated actors (snake
/// body + head chrome/eyes, and the apple body/halo/shine), the food-pickup particle
/// pool, and transient decorative overlays (the colour-cycling header logo, the
/// eat-flash frame, the "+10" popup, the SPEED UP / music banners, the game-over panel).
fn classify_kotosnake(d: &ImmDraw) -> DrawClass {
    // Palette constants mirrored from apps/kotosnake/src/main.koto.
    const C_TEXT: i32 = 65535; // white snake/apple chrome + panel accents
    const C_FOOD: i32 = 63694; // apple body
    const C_FOODGLOW: i32 = 41032; // apple halo
    const C_EYE: i32 = 2113; // snake eye ink
    const C_TOP: i32 = 20632; // SPEED UP banner fill
    const C_OVER: i32 = 63754; // game-over / music banner fill

    match d.kind {
        // The colour-cycling logo, the "+10" popup, and the banner / game-over text are
        // all decorative overlays. (KotoSnake's *critical* text — score/len/best — is
        // retained, so it never reaches this immediate path.)
        ImmKind::Text => DrawClass::Decoration,
        // KotoSnake emits no immediate pixel blits, but keep the path total: treat an
        // unknown blit as decorative rather than mis-crediting it to gameplay.
        ImmKind::Pixels => DrawClass::Decoration,
        ImmKind::Rect => {
            let (w, h, col) = (d.w, d.h, d.color);
            if col == C_FOOD || col == C_FOODGLOW || col == C_EYE {
                // Apple body/halo and the snake's eyes — always-visible actors.
                DrawClass::Actor
            } else if col == C_TOP || col == C_OVER {
                // Banner / game-over panel fills — decorative overlays.
                DrawClass::Decoration
            } else if col == C_TEXT {
                // White chrome: the snake glint / head outline and the apple shine /
                // sparkle are small (<= a cell) and belong to the actors; the game-over
                // panel's 240px accent line is the one wide white rect, and is decor.
                if w >= 200 && h <= 4 {
                    DrawClass::Decoration
                } else {
                    DrawClass::Actor
                }
            } else if is_snake_rainbow(col) {
                if (w == 14 && h >= 14) || (h == 14 && w > 14) {
                    // A 14x14 near-head body cell, or a coalesced tail run (one side a
                    // cell, the other a multi-cell stretch) — the snake body, an actor.
                    DrawClass::Actor
                } else if w <= 6 && h <= 6 {
                    // A food-pickup spark (size 2..6 px, shrinking with its life).
                    DrawClass::Particles
                } else {
                    // The eat-flash frame's long thin lines and the banner accent lines.
                    DrawClass::Decoration
                }
            } else {
                DrawClass::Decoration
            }
        }
    }
}

/// What the KotoGFX immediate-overlay budget *would* have decided for one frame's
/// immediate draws — recorded without gating any draw (the app still drew everything).
#[derive(Clone, Copy)]
struct SnakeBudget {
    /// The admission ledger after metering the frame (admitted / degraded / rejected
    /// per class). `stats.total_used()` can never exceed [`APP_DRAW_BUDGET`]'s cap.
    stats: BudgetStats,
    /// Commands requested per [`DrawClass`] this frame (before any budget verdict),
    /// indexed by [`DrawClass::index`]. The sum equals the frame's `app_draw` count.
    requested: [u16; DRAW_CLASS_COUNT],
}

/// Run one frame's immediate (non-static) draws through the KotoGFX immediate-overlay
/// budget in *observation mode*: classify each draw, group consecutive same-class draws
/// into one logical request (so the model can *degrade* an effect — draw fewer sparks /
/// a shorter tail — instead of only admit/reject single commands), and meter it against
/// [`APP_DRAW_BUDGET`]. Nothing here changes what was drawn; it only records the verdict.
fn observe_snake_budget(frame_imm: &[ImmDraw]) -> SnakeBudget {
    let budget = APP_DRAW_BUDGET;
    let mut stats = BudgetStats::new();
    let mut requested = [0u16; DRAW_CLASS_COUNT];
    let mut run: Option<(DrawClass, u16)> = None;
    for d in frame_imm.iter().filter(|d| !d.in_static) {
        let class = classify_kotosnake(d);
        match run {
            Some((c, ref mut cost)) if c == class => *cost += 1,
            _ => {
                if let Some((c, cost)) = run {
                    requested[c.index()] += cost;
                    budget.request(&mut stats, c, cost);
                }
                run = Some((class, 1));
            }
        }
    }
    if let Some((c, cost)) = run {
        requested[c.index()] += cost;
        budget.request(&mut stats, c, cost);
    }
    SnakeBudget { stats, requested }
}

/// One frame's measurement from [`play_kotosnake_greedy`].
struct SnakeFrame {
    /// Immediate draw commands this frame (`draw_rect` + `draw_text*` + `draw_pixels`)
    /// that landed in the *per-frame* list — i.e. excluding the one-time chrome the app
    /// captures into the retained static layer (the mock host records every draw, so
    /// the drive masks the `game2d_static_begin`/`_end` window). This is exactly what
    /// fills the device's `MAX_APP_DRAW_COMMANDS` budget; retained `game2d_*` do not.
    app_draw: usize,
    /// Body rects with one side exactly 14 px (a cell) and the other > 14 px — the
    /// unique signature of a *multi-cell* coalesced tail run (single cells are 14x14;
    /// the retained static field is far larger on both sides), so a non-zero count
    /// proves the coalesced-tail path ran this frame.
    coalesced_runs: usize,
    /// Snake length this frame, read from the `occ` grid in the heap.
    len: usize,
    /// Instructions this frame executed (`last_frame_fuel`), so the long-body
    /// profile (KOTO-0169 Stage 0c) can read fuel against body length.
    instructions: u32,
    /// What the KotoGFX immediate-overlay budget would have decided for this frame's
    /// immediate draws (observation only — no draw was gated).
    budget: SnakeBudget,
}

/// A whole greedy KotoSnake run: the per-frame measurements plus the session-level
/// execution profile (KOTO-0169 Stage 0c). The cumulative stats and the opcode
/// histogram are dominated by steady long-body play (6000 frames), which is
/// exactly the regime the device's `vm_us ≈ 165 ms` long-body frames run in.
struct SnakeRun {
    frames: Vec<SnakeFrame>,
    /// Cumulative session [`VmStats`] (instructions, host_calls, frames).
    stats: VmStats,
    /// Per-opcode execution counts, descending, as in [`FixtureReport`].
    #[cfg(feature = "opcode_stats")]
    opcode_hist: Vec<(u8, u64)>,
}

/// Play KOTOSNAKE for up to `max_frames` with a built-in greedy apple-seeker so the
/// snake actually grows long (length and apple position are app-local, so the only way
/// to a long snake is to play). Each frame the controller reads the head from the
/// `occ`-grid delta (heap byte 256: `buf body` then `buf occ`) and the apple from the
/// recorded `C_FOOD` rect, then steers head→apple (avoiding an immediate reversal).
/// Returns one [`SnakeFrame`] per stepped frame plus the session profile.
fn play_kotosnake_greedy(max_frames: usize) -> SnakeRun {
    const C_FOOD: i32 = 63694; // apple body colour (see apps/kotosnake/src/main.koto)
    const COLS: i32 = 18;
    const ROWS: i32 = 14;

    let mut session = BytecodeSession::<STACK, CALLS>::new(
        KOTOSNAKE,
        koto_core::RuntimeLimits::simulator_default(),
        SIM_FRAME_FUEL,
    )
    .expect("kotosnake verifies");
    let heap_bytes = session.program().header().max_heap_bytes as usize;
    let mut heap = vec![0u8; heap_bytes];
    if let Some((start, end)) = session.program().rodata_range() {
        heap[..end - start].copy_from_slice(&KOTOSNAKE[start..end]);
    }
    let mut host = RecordingHost::default();
    let mut code = SliceCode::new(KOTOSNAKE, session.program().code_range().0);

    let newline = VmInputSnapshot {
        intent_bits: text_intent::NEWLINE,
        ..VmInputSnapshot::empty()
    };
    let turn = |dir: i32| VmInputSnapshot {
        intent_bits: match dir {
            0 => text_intent::RIGHT,
            1 => text_intent::DOWN,
            2 => text_intent::LEFT,
            _ => text_intent::UP,
        },
        ..VmInputSnapshot::empty()
    };

    let mut prev_occ = [0u8; 252];
    let mut head: i32 = 7 * COLS + 8; // initial head cell (8, 7)
    let mut dir = 0i32; // 0 R, 1 D, 2 L, 3 U
    let mut apple: Option<(i32, i32)> = None;
    let mut frames = Vec::with_capacity(max_frames);

    for frame in 0..max_frames {
        if session.has_exited() {
            break;
        }
        // Choose this frame's input: F1 to enter play (and to retry if it ever dies),
        // otherwise steer the head toward the apple, preferring the axis with the
        // larger gap and never feeding a straight reversal (which the app ignores).
        let input = if frame == 0 {
            newline
        } else if let Some((ax, ay)) = apple {
            let (hx, hy) = (head % COLS, head / COLS);
            let rev = (dir + 2) % 4;
            let hdir = if ax > hx {
                0
            } else if ax < hx {
                2
            } else {
                -1
            };
            let vdir = if ay > hy {
                1
            } else if ay < hy {
                3
            } else {
                -1
            };
            let want = if hdir >= 0 && hdir != rev {
                hdir
            } else if vdir >= 0 && vdir != rev {
                vdir
            } else {
                dir // already aligned or only a reversal available: keep going
            };
            turn(want)
        } else {
            newline // still on the title screen
        };

        let calls_before = host.calls.len();
        let rects_before = host.rects.len();
        let imm_before = host.imm.len();
        session
            .step_frame_with(&mut code, &mut host, input, &mut heap)
            .expect("frame steps cleanly");

        // Count immediate draw commands, masking the retained static-layer capture
        // window (the mock host records those draws too, but on device they never hit
        // APP_DRAW).
        let mut app_draw = 0usize;
        let mut capturing = false;
        for c in &host.calls[calls_before..] {
            match c.id {
                host_call::GAME2D_STATIC_BEGIN => capturing = true,
                host_call::GAME2D_STATIC_END => capturing = false,
                host_call::DRAW_RECT
                | host_call::DRAW_TEXT
                | host_call::DRAW_TEXT_COLOR
                | host_call::DRAW_PIXELS_RGB565
                    if !capturing =>
                {
                    app_draw += 1
                }
                _ => {}
            }
        }
        let coalesced_runs = host.rects[rects_before..]
            .iter()
            .filter(|(_, _, w, h, _)| (*w == 14 && *h > 14) || (*h == 14 && *w > 14))
            .count();

        // Update the head from the occ-grid delta (the cell that became occupied this
        // step is the new head), and the apple from this frame's C_FOOD rect.
        let occ = &heap[256..256 + 252];
        let len = occ.iter().filter(|b| **b != 0).count();
        for cell in 0..252 {
            if occ[cell] != 0 && prev_occ[cell] == 0 {
                let nh = cell as i32;
                let (nx, ny) = (nh % COLS, nh / COLS);
                let (hx, hy) = (head % COLS, head / COLS);
                // Derive the heading from the step (handle edge wrap, where the delta
                // is COLS-1 / ROWS-1 the other way).
                if nx != hx {
                    dir = if nx == (hx + 1) % COLS { 0 } else { 2 };
                } else if ny != hy {
                    dir = if ny == (hy + 1) % ROWS { 1 } else { 3 };
                }
                head = nh;
            }
        }
        prev_occ.copy_from_slice(occ);
        apple = host.rects[rects_before..]
            .iter()
            .find_map(|(x, y, w, h, rgb)| {
                (*rgb == C_FOOD).then(|| {
                    let cx = x + w / 2;
                    let cy = y + h / 2;
                    (
                        ((cx - 16 - 8) as f32 / 16.0).round() as i32,
                        ((cy - 44 - 8) as f32 / 16.0).round() as i32,
                    )
                })
            });

        // Observe (do not gate) what the KotoGFX immediate-overlay budget would have
        // decided for this frame's immediate draws. The observer skips static-capture
        // draws itself, so it sees exactly the per-frame immediate list `app_draw` counts.
        let budget = observe_snake_budget(&host.imm[imm_before..]);

        frames.push(SnakeFrame {
            app_draw,
            coalesced_runs,
            len,
            instructions: session.last_frame_fuel(),
            budget,
        });
    }

    #[cfg(feature = "opcode_stats")]
    let opcode_hist = {
        let mut hist: Vec<(u8, u64)> = session
            .opcode_counts()
            .iter()
            .enumerate()
            .filter(|(_, &count)| count > 0)
            .map(|(op, &count)| (op as u8, count))
            .collect();
        hist.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        hist
    };

    SnakeRun {
        frames,
        stats: session.stats(),
        #[cfg(feature = "opcode_stats")]
        opcode_hist,
    }
}

/// Long-snake APP_DRAW budget (the length-aware render cap). KotoSnake's animated
/// snake/apple/particles stay on the immediate path; before the cap, a long snake's
/// per-cell body blits overran the device's 96-command APP_DRAW budget (hardware
/// `used=96/96 overflow full_reason=AreaExceeded/RectsExceeded`). The near-head zone
/// now renders richly while the older tail is coalesced into a bounded number of
/// collinear run rects, so the per-frame immediate command count is capped by
/// construction (head 8 + (RICH_N-1)*2 + TAIL_BUDGET body rects, plus apple/particles/
/// overlays) regardless of length.
///
/// Length/apple are app-local state (not heap-seedable), so the only way to a long
/// snake is to play it: [`play_kotosnake_greedy`] runs a built-in apple-seeker that
/// grows the snake well past RICH_N. Across every frame the immediate draw-command
/// count must stay under 96, and the coalesced-tail path must actually have run.
#[test]
fn kotosnake_long_snake_stays_under_app_draw_budget() {
    // The device's per-frame immediate draw-command budget
    // (`koto_pico::firmware::config::MAX_APP_DRAW_COMMANDS`). Overflowing it forces the
    // app-draw-overflow / full-repaint this change exists to prevent.
    const APP_DRAW_CAP: usize = 96;

    let frames = play_kotosnake_greedy(6000).frames;
    let peak = frames.iter().map(|f| f.app_draw).max().unwrap_or(0);
    let max_len = frames.iter().map(|f| f.len).max().unwrap_or(0);
    let coalesced_runs: usize = frames.iter().map(|f| f.coalesced_runs).sum();
    println!(
        "kotosnake greedy: max_len={max_len} APP_DRAW_peak={peak} coalesced_run_rects={coalesced_runs} over {} frames",
        frames.len()
    );

    // The greedy controller actually grew a long snake (past RICH_N = 12) ...
    assert!(
        max_len > 12,
        "greedy play never grew the snake past RICH_N (max_len={max_len}); coalescing untested"
    );
    // ... so the coalesced-tail path ran (multi-cell run rects were drawn) ...
    assert!(
        coalesced_runs > 0,
        "no multi-cell coalesced run rects were drawn, despite a long snake"
    );
    // ... and no frame ever reached the APP_DRAW cap, so a long snake never overflows.
    assert!(
        peak < APP_DRAW_CAP,
        "long-snake immediate draw-command peak {peak} reached/over the {APP_DRAW_CAP} APP_DRAW cap"
    );
}

/// KotoGFX immediate-overlay budget *observation* (the next KotoGFX step, wired into
/// diagnostics only). KotoSnake's immediate draws are classified into [`DrawClass`]es
/// host-side ([`classify_kotosnake`], from the app's existing palette/geometry — no
/// bytecode/hostcall change) and metered against [`APP_DRAW_BUDGET`] to record what the
/// budget model *would* have decided. Nothing is gated: the app still draws every
/// command, so this changes no rendered output — it only surfaces the budget pressure a
/// long snake + food-pickup bursts put on each class.
///
/// What this documents (see `docs/devlog/KOTO_KOTOSNAKE_BUDGET_OBSERVATION.md`):
/// - The cap itself is never exceeded by the model (`total_used <= 96` every frame), by
///   construction — the whole point of the reservation/floor policy.
/// - The classifier accounts for *every* immediate command (per-class requested sums to
///   the frame's `app_draw`), so the observation is exhaustive and non-destructive.
/// - KotoSnake's immediate path has **no** CoreGameplay and **no** CriticalUi (those are
///   on the retained static/text layers), so the default budget's 28 commands reserved
///   for them sit stranded — which is exactly what squeezes Particles/Decoration.
/// - Under a long snake, the Actor body claims most of the shared pool, so the model
///   would *degrade* food-pickup particles and *reject* later decorative overlays
///   *before* the hard cap — the targeted behaviour, observed here without enacting it.
#[test]
fn kotosnake_immediate_overlay_budget_observation() {
    let frames = play_kotosnake_greedy(6000).frames;
    assert!(!frames.is_empty(), "greedy play produced no frames");

    // Per-class run totals.
    let mut requested = [0u64; DRAW_CLASS_COUNT];
    let mut admitted = [0u64; DRAW_CLASS_COUNT];
    let mut rejected = [0u64; DRAW_CLASS_COUNT];
    let mut degraded = [0u64; DRAW_CLASS_COUNT];
    let mut max_total_used = 0u16;

    for f in &frames {
        let b = &f.budget;
        // Invariant 1: the model never admits past the hard cap, by construction.
        let used = b.stats.total_used();
        max_total_used = max_total_used.max(used);
        assert!(
            used <= APP_DRAW_BUDGET.total_commands() as u16,
            "budget model admitted {used} > cap {}",
            APP_DRAW_BUDGET.total_commands()
        );
        // Invariant 2: the classifier accounts for every immediate command this frame,
        // so observation is exhaustive (nothing silently uncounted / dropped).
        let req_sum: u64 = b.requested.iter().map(|&r| u64::from(r)).sum();
        assert_eq!(
            req_sum, f.app_draw as u64,
            "classified {req_sum} commands but the frame's immediate list was {}",
            f.app_draw
        );
        for class in DrawClass::ALL {
            let i = class.index();
            // Invariant 3: a class is never admitted more than it requested.
            assert!(
                b.stats.admitted(class) <= b.requested[i],
                "{} admitted {} > requested {}",
                class.as_str(),
                b.stats.admitted(class),
                b.requested[i]
            );
            requested[i] += u64::from(b.requested[i]);
            admitted[i] += u64::from(b.stats.admitted(class));
            rejected[i] += u64::from(b.stats.rejected(class));
            degraded[i] += u64::from(b.stats.degraded(class));
        }
    }

    let max_len = frames.iter().map(|f| f.len).max().unwrap_or(0);
    let peak_app_draw = frames.iter().map(|f| f.app_draw).max().unwrap_or(0);
    println!("=== KotoSnake immediate-overlay budget observation ===");
    println!(
        "  frames={} max_len={max_len} peak_app_draw={peak_app_draw} cap={} max_total_used={max_total_used}",
        frames.len(),
        APP_DRAW_BUDGET.total_commands(),
    );
    println!(
        "    {:<12} {:>4} {:>10} {:>10} {:>10} {:>10}",
        "class", "rsv", "requested", "admitted", "degraded", "rejected"
    );
    for class in DrawClass::ALL {
        let i = class.index();
        println!(
            "    {:<12} {:>4} {:>10} {:>10} {:>10} {:>10}",
            class.as_str(),
            APP_DRAW_BUDGET.reserved(class),
            requested[i],
            admitted[i],
            degraded[i],
            rejected[i],
        );
    }

    // The scenario actually grew a long snake (so the body claims the shared pool).
    assert!(
        max_len > 12,
        "greedy play never grew the snake past RICH_N (max_len={max_len})"
    );

    // Actors (snake body + apple) are observed and admitted — the budget protects them.
    let actor = DrawClass::Actor.index();
    assert!(
        requested[actor] > 0 && admitted[actor] > 0,
        "no actor draws observed/admitted"
    );

    // Food-pickup particles are observed. At the lengths the greedy seeker reaches
    // (~19), the Actor body has not yet claimed enough of the shared pool to squeeze the
    // particle pool, so every spark still fits here — particle pressure only appears at
    // the structural worst case, covered by
    // [`kotosnake_worst_case_long_snake_budget_pressure`].
    let particles = DrawClass::Particles.index();
    assert!(
        requested[particles] > 0,
        "no food-pickup particle draws observed"
    );

    // Decorative overlays emitted after the body/particles are the pressure real greedy
    // play actually reaches: once the snake body + a particle burst have spent the pool
    // down past the decoration floor, late overlays (eat-flash / popup / banners) would
    // be rejected before the hard cap — the targeted protection, observed (not enacted).
    let decoration = DrawClass::Decoration.index();
    assert!(
        rejected[decoration] > 0,
        "no decorative overlay was rejected, so no late-overlay pressure was observed"
    );

    // KotoSnake puts its board on the retained static layer and its HUD on retained
    // text, so the immediate path carries no CoreGameplay / CriticalUi: the default
    // budget's reservations for those classes go entirely unused (stranded headroom that
    // tightens the pool for Particles/Decoration — documented, not yet re-tuned).
    let core = DrawClass::CoreGameplay.index();
    let critical = DrawClass::CriticalUi.index();
    assert_eq!(
        requested[core], 0,
        "unexpected immediate CoreGameplay draws"
    );
    assert_eq!(
        requested[critical], 0,
        "unexpected immediate CriticalUi draws"
    );
    assert!(
        APP_DRAW_BUDGET.reserved(DrawClass::CoreGameplay)
            + APP_DRAW_BUDGET.reserved(DrawClass::CriticalUi)
            > 0,
        "default budget reserves nothing for the classes KotoSnake leaves unused"
    );
}

/// Build one synthetic immediate frame at KotoSnake's *structural* worst case — the
/// length-aware body bound documented in `docs/devlog/KOTO_KOTOSNAKE_LONG_SNAKE_BUDGET.md`
/// (head 8 + (RICH_N-1)*2 + TAIL_BUDGET body rects) plus the apple, a full 24-spark
/// food-pickup burst, and the eat-flash/popup overlays it co-occurs with. Each entry
/// mirrors a real `draw_*` signature from `apps/kotosnake/src/main.koto`, so it routes
/// through the same [`classify_kotosnake`] the live fixture uses. This is a maximally
/// coiled long snake mid-bite — reachable in real play but not by the simple greedy
/// seeker (which traps itself around length 19).
fn worst_case_snake_frame() -> Vec<ImmDraw> {
    // Mirror the app's body-budget constants (apps/kotosnake/src/main.koto).
    const RICH_N: usize = 12;
    const TAIL_BUDGET: usize = 16;
    const C_TEXT: i32 = 65535;
    const C_FOOD: i32 = 63694;
    const C_FOODGLOW: i32 = 41032;
    const C_EYE: i32 = 2113;

    let rect = |w: i32, h: i32, color: i32| ImmDraw {
        kind: ImmKind::Rect,
        in_static: false,
        w,
        h,
        color,
    };
    let text = |color: i32| ImmDraw {
        kind: ImmKind::Text,
        in_static: false,
        w: 0,
        h: 0,
        color,
    };

    let mut v = vec![
        // Header logo (colour-cycling) — emitted first, decorative.
        text(49183),
        // Apple: halo, breathing body, shine, orbiting sparkle — actors.
        rect(20, 20, C_FOODGLOW),
        rect(14, 14, C_FOOD),
        rect(3, 3, C_TEXT),
        rect(3, 3, C_TEXT),
        // Snake head cell: body + glint + 4 outline edges + 2 eyes = 8 commands.
        rect(14, 14, 63488),
        rect(6, 3, C_TEXT),
        rect(16, 2, C_TEXT),
        rect(16, 2, C_TEXT),
        rect(2, 16, C_TEXT),
        rect(2, 16, C_TEXT),
        rect(3, 3, C_EYE),
        rect(3, 3, C_EYE),
    ];
    // The next RICH_N-1 near-head cells: body + glint each.
    for _ in 0..(RICH_N - 1) {
        v.push(rect(14, 14, 2016));
        v.push(rect(6, 3, C_TEXT));
    }
    // The coalesced tail at its budget: TAIL_BUDGET multi-cell run rects.
    for _ in 0..TAIL_BUDGET {
        v.push(rect(14, 46, 8479));
    }
    // A full food-pickup burst: 24 live sparks.
    for _ in 0..24 {
        v.push(rect(4, 4, 1119));
    }
    // The eat-flash frame (4 lines) and the rising "+10" popup (shadow + face).
    for _ in 0..4 {
        v.push(rect(294, 3, 63512));
    }
    v.push(text(2080));
    v.push(text(63512));
    v
}

/// KotoGFX immediate-overlay budget observation at the *structural worst case*: a
/// maximally coiled long snake mid-bite (the documented body bound + a full 24-spark
/// burst + overlays). This is the "long-snake / particle case" the model is meant to
/// protect: the Actor body legitimately claims the bulk of the shared pool, so the
/// budget would *degrade* the food-pickup particle burst and *reject* the trailing
/// decorative overlays — both *before* the 96-command hard cap, i.e. before the
/// tail-drop + full-repaint the cap alone would cause. Observation only: this meters a
/// synthesised command list; it gates nothing and changes no rendering.
#[test]
fn kotosnake_worst_case_long_snake_budget_pressure() {
    let frame = worst_case_snake_frame();
    let b = observe_snake_budget(&frame);

    let actor = DrawClass::Actor.index();
    let particles = DrawClass::Particles.index();

    println!("=== KotoSnake worst-case immediate budget pressure ===");
    println!(
        "  immediate commands = {} (cap {})",
        frame.len(),
        APP_DRAW_BUDGET.total_commands()
    );
    for class in DrawClass::ALL {
        let i = class.index();
        println!(
            "    {:<12} requested={:>3} admitted={:>3} degraded={:>2} rejected={:>3}",
            class.as_str(),
            b.requested[i],
            b.stats.admitted(class),
            b.stats.degraded(class),
            b.stats.rejected(class),
        );
    }

    // The body's full 8 + (RICH_N-1)*2 + TAIL_BUDGET = 46 rects, plus the apple's 4,
    // were classified as Actor, and the 24-spark burst as Particles.
    assert_eq!(
        b.requested[actor], 50,
        "actor body/apple command count drifted"
    );
    assert_eq!(
        b.requested[particles], 24,
        "particle burst command count drifted"
    );

    // The body fits (it is protected), but the particle burst is squeezed: the shared
    // pool is spent down to the particle floor, so the model *degrades* the burst —
    // exactly "shed individual particles under pressure".
    assert_eq!(
        b.stats.admitted(DrawClass::Actor),
        b.requested[actor],
        "the actor body should always be admitted in full"
    );
    assert!(
        b.stats.admitted(DrawClass::Particles) < b.requested[particles],
        "the particle burst was not squeezed at the worst case (admitted {} of {})",
        b.stats.admitted(DrawClass::Particles),
        b.requested[particles]
    );
    assert!(
        b.stats.degraded(DrawClass::Particles) > 0,
        "the particle burst should be degraded (partially admitted), not all-or-nothing"
    );

    // The trailing decorative overlays are rejected — yielded to the gameplay actors ...
    assert!(
        b.stats.rejected(DrawClass::Decoration) > 0,
        "trailing decorative overlays should be rejected under pressure"
    );

    // ... and crucially, all of this happens strictly *below* the hard cap: no tail-drop,
    // no resulting full repaint.
    assert!(
        b.stats.total_used() < APP_DRAW_BUDGET.total_commands() as u16,
        "the model spent up to the hard cap; the policy should shed effects before that"
    );
}

/// Drive KOTO_BLOCKS through a scripted input (one [`VmInputSnapshot`] per frame,
/// the last entry repeating) and hand back the [`RecordingHost`] so a test can
/// inspect the exact retained-layer writes the run produced (e.g. which board cells
/// a lock placed). A thin sibling of [`run_fixture_scripted`]: same verify-then-step
/// path, but it returns the host instead of distilling a [`FixtureReport`].
fn play_koto_blocks(inputs: &[VmInputSnapshot]) -> RecordingHost {
    drive_koto_blocks(inputs, |_, _| {}).0
}

/// One immediate `draw_rect(x, y, w, h, rgb565)`.
type Rect = (i32, i32, i32, i32, i32);

/// KotoBlocks' tile cache is `main`'s first `buf` (`tiles[4096]`), so the board
/// (`buf board[200]`) lives at heap byte 4096; cell `(row, col)` is `board[row*10+col]`.
/// Tests use this to seed a board state (e.g. a near-full well) deterministically.
const KOTO_BLOCKS_BOARD_OFFSET: usize = 4096;

/// Drive KOTO_BLOCKS through a scripted input, returning the [`RecordingHost`] and the
/// per-frame `draw_rect` count (immediate rect-composition cost, the metric the
/// line-clear / game-over effects spend). `before` runs just before each frame steps,
/// given the zero-based frame index and the live heap, so a test can inject board
/// state (the board is in-heap; the VM reads it back) to reach a state — a full row, a
/// topped-out well — that precise gameplay input would take far longer to set up.
fn drive_koto_blocks(
    inputs: &[VmInputSnapshot],
    mut before: impl FnMut(usize, &mut [u8]),
) -> (RecordingHost, Vec<Vec<Rect>>) {
    let mut session = BytecodeSession::<STACK, CALLS>::new(
        KOTO_BLOCKS,
        koto_core::RuntimeLimits::simulator_default(),
        SIM_FRAME_FUEL,
    )
    .expect("koto_blocks verifies");

    let heap_bytes = session.program().header().max_heap_bytes as usize;
    let mut heap = vec![0u8; heap_bytes];
    if let Some((start, end)) = session.program().rodata_range() {
        heap[..end - start].copy_from_slice(&KOTO_BLOCKS[start..end]);
    }

    let mut host = RecordingHost::default();
    let mut code = SliceCode::new(KOTO_BLOCKS, session.program().code_range().0);
    let last = inputs.len() - 1;
    let mut rects_per_frame: Vec<Vec<Rect>> = Vec::with_capacity(inputs.len());
    for frame in 0..inputs.len() {
        if session.has_exited() {
            break;
        }
        before(frame, &mut heap);
        let before_rects = host.rects.len();
        session
            .step_frame_with(&mut code, &mut host, inputs[frame.min(last)], &mut heap)
            .expect("frame steps cleanly");
        rects_per_frame.push(host.rects[before_rects..].to_vec());
    }
    (host, rects_per_frame)
}

// KotoBlocks effect palette entries the effect tests assert against (see
// `apps/koto_blocks/src/main.koto`): the dim slate of a game-over board cell, the
// near-black board-sweep overlay, and the white line-clear blink fill.
const C_DEAD: i32 = 25390;
const C_SWEEP: i32 = 2113;
const C_PANEL: i32 = 65535;

/// Line-clear effect coverage + optimization guard (KOTO-0158). A completed row blinks
/// while it clears. The old effect drew one 16x16 white rect *per cell* (up to 40 for a
/// tetris); the optimized effect draws one full-width band *per row* — identical pixels
/// (ten contiguous cells == one 160px band), a tenth of the immediate-rect cost.
///
/// Reaching a tetris through pure input would take a long, brittle move sequence, so the
/// scenario seeds the bottom four rows full in-heap (the board lives in the app heap; the
/// VM reads it back) and then hard-drops one piece: locking it runs the row scan, which
/// finds rows 16..19 complete and starts the four-line clear.
#[test]
fn koto_blocks_line_clear_blinks_as_row_bands() {
    let empty = VmInputSnapshot::empty();
    let start = VmInputSnapshot {
        intent_bits: text_intent::NEWLINE,
        ..VmInputSnapshot::empty()
    };
    let space = VmInputSnapshot {
        text_codepoint: 32,
        ..VmInputSnapshot::empty()
    };

    let mut inputs = vec![empty; 34];
    inputs.push(start); // title -> play
    inputs.push(empty); // spawn the active piece at the top
    inputs.push(space); // hard drop -> lock -> the scan clears the seeded rows
    inputs.extend(std::iter::repeat_n(empty, 28)); // play out the blink animation

    let (_host, frames) = drive_koto_blocks(&inputs, |frame, heap| {
        // Just before the spawn frame, fill rows 16..19 completely (a four-high wall).
        // A hard-dropped piece lands on top, locks, and the lock's full-row scan clears
        // all four — without ever touching the spawn rows, so there is no false topout.
        if frame == 35 {
            for cell in 160..200 {
                heap[KOTO_BLOCKS_BOARD_OFFSET + cell] = 1;
            }
        }
    });

    // A full-width blink band: white, spanning the 160px well at x=8, one row tall.
    let is_band = |r: &&Rect| r.4 == C_PANEL && r.0 == 8 && r.2 == 160 && r.3 == 16;
    // The *old* per-cell blink shape: a white 16x16 cell. The optimization must have
    // removed every one of these (a static panel box is far larger, so none match).
    let is_cell = |r: &&Rect| r.4 == C_PANEL && r.2 == 16 && r.3 == 16;

    let per_cell_blinks = frames.iter().flatten().filter(is_cell).count();
    assert_eq!(
        per_cell_blinks, 0,
        "line-clear blink still composits per-cell white rects ({per_cell_blinks} found)"
    );

    // A blink-on frame draws exactly the four cleared rows as bands, top to bottom
    // (rows 16..19 -> y = 256, 272, 288, 304), proving both the four-line clear fired
    // and that each row is a single band.
    let blink_frame = frames
        .iter()
        .find(|f| f.iter().filter(is_band).count() == 4)
        .expect("four-line clear never drew its four blink bands");
    let band_rows: Vec<i32> = blink_frame.iter().filter(is_band).map(|r| r.1).collect();
    assert_eq!(
        band_rows,
        vec![256, 272, 288, 304],
        "blink bands should cover exactly the four seeded rows 16..19"
    );
}

/// Game-over effect coverage + optimization guard (KOTO-0158). On top-out the board
/// freezes and a near-black sweep descends the well one row per frame until it rests
/// covering the whole well. The board behind it was redrawn as one dim rect *per
/// occupied cell* every frame — up to ~200 immediate rects forever, even once the
/// opaque sweep hides all of them. The optimization starts the dim fill at the sweep's
/// leading row, so covered rows are skipped and the resting state draws none.
///
/// The scenario stacks pieces in place (Space every frame keeps them at the spawn
/// column) until a spawn tops out, then idles in game over.
#[test]
fn koto_blocks_game_over_skips_swept_board_fill() {
    let empty = VmInputSnapshot::empty();
    let start = VmInputSnapshot {
        intent_bits: text_intent::NEWLINE,
        ..VmInputSnapshot::empty()
    };
    let space = VmInputSnapshot {
        text_codepoint: 32,
        ..VmInputSnapshot::empty()
    };

    let mut inputs = vec![empty; 34];
    inputs.push(start);
    inputs.extend(std::iter::repeat_n(space, 60)); // stack to the top -> top out
    inputs.extend(std::iter::repeat_n(empty, 30)); // idle: the sweep descends and rests

    let (_host, frames) = drive_koto_blocks(&inputs, |_, _| {});

    let dim_cells = |f: &Vec<Rect>| f.iter().filter(|r| r.4 == C_DEAD).count();

    // The well really did fill (so the unoptimized fill *would* be heavy): some
    // game-over frame, before the sweep covers it, drew many dim board cells.
    let peak_dim = frames.iter().map(dim_cells).max().unwrap_or(0);
    assert!(
        peak_dim >= 10,
        "well never filled, so the test would not exercise the board fill (peak {peak_dim})"
    );

    // A resting game-over frame: the sweep has reached full well height (320px). There
    // it must draw *zero* dim board cells (all covered) and only a handful of rects
    // total — the sweep itself plus the game-over panel — instead of one per cell.
    let resting = frames
        .iter()
        .rfind(|f| {
            f.iter()
                .any(|r| r.0 == 8 && r.1 == 0 && r.3 == 320 && r.4 == C_SWEEP)
        })
        .expect("game over never reached its resting full-well sweep");
    assert_eq!(
        dim_cells(resting),
        0,
        "resting game-over still redraws board cells hidden by the sweep"
    );
    assert!(
        resting.len() <= 4,
        "resting game-over frame is still rect-heavy ({} rects)",
        resting.len()
    );
}

/// KotoBlocks hard-drop regression (KOTO-0157). Pressing **Space** (codepoint 32)
/// during gameplay must instantly hard-drop the active piece to the floor and lock
/// it. The lock writes the four landed cells into the retained board tilemap, so the
/// host sees `game2d_set_tile(layer 0, x, y, tile_ref >= 0)` for cells in the bottom
/// rows of the 20-row well — a state the same script *without* Space never reaches in
/// the short window stepped (level-1 gravity is ~33 frames per row, so an untouched
/// piece is still near the top). This locks in the input path the firmware once broke
/// by routing typed characters through `is_ascii_graphic()`, which excludes 0x20.
#[test]
fn koto_blocks_space_hard_drops_active_piece() {
    let empty = VmInputSnapshot::empty();
    let start = VmInputSnapshot {
        intent_bits: text_intent::NEWLINE,
        ..VmInputSnapshot::empty()
    };
    let space = VmInputSnapshot {
        text_codepoint: 32,
        ..VmInputSnapshot::empty()
    };

    // Common prefix: bake the title tile cache (4 rows/frame x 7 tiles), take the
    // title->play transition, then one frame to spawn the active piece at the top.
    let to_play = |tail: &[VmInputSnapshot]| {
        let mut inputs = vec![empty; 34];
        inputs.push(start);
        inputs.push(empty); // first play frame: spawn piece at px=3, py=0
        inputs.extend_from_slice(tail);
        inputs
    };

    // Baseline: no Space. The piece falls under gravity only, so within these frames
    // it never reaches the floor and never locks — the board tilemap stays untouched.
    let idle = play_koto_blocks(&to_play(&[empty; 6]));
    let idle_board_writes: Vec<_> = idle
        .tiles_set
        .iter()
        .filter(|(layer, _, _, tile_ref)| *layer == 0 && *tile_ref >= 0)
        .collect();
    assert!(
        idle_board_writes.is_empty(),
        "active piece locked without Space (board cells written: {idle_board_writes:?})"
    );

    // Hard drop: a single Space frame must lock the piece this frame. The next render
    // writes the four landed cells into the board tilemap at the bottom of the well.
    let mut tail = vec![space];
    tail.extend(std::iter::repeat_n(empty, 5));
    let dropped = play_koto_blocks(&to_play(&tail));
    let board_writes: Vec<_> = dropped
        .tiles_set
        .iter()
        .filter(|(layer, _, _, tile_ref)| *layer == 0 && *tile_ref >= 0)
        .collect();

    // The lock placed real tiles (a tetromino is four cells) ...
    assert_eq!(
        board_writes.len(),
        4,
        "Space hard-drop should lock exactly the four piece cells; got {board_writes:?}"
    );
    // ... and they landed against the floor, proving a *hard* drop to the bottom and
    // not merely a one-row soft step (the lowest cell of a piece dropped from py=0
    // reaches row 18 or 19 of the 20-row well).
    let bottom_row = board_writes
        .iter()
        .map(|(_, _, y, _)| *y)
        .max()
        .expect("at least one cell");
    assert!(
        bottom_row >= 18,
        "Space did not hard-drop to the floor; lowest locked row was {bottom_row} (expected >= 18)"
    );
}

/// Device-tile CodeWindow attribution wrapper (KOTO-0155 diagnostic).
///
/// Wraps [`SliceCode`] and, on every word fetch, replays the single-tile
/// [`PsramCodeWindow`] math (`base = (index / capacity) * capacity`, one cached tile
/// at a time) so a host run reports the refill/distinct-tile pattern that shape of
/// hardware window would see — without any PSRAM mock. Since KOTO-0173 the device
/// runs a **two-tile** MRU/LRU cache, so this single-tile model is a conservative
/// upper bound on device refills (device ≤ model; a 0-refill lock-in like the
/// KotoBlocks test below is exact either way). The window state (`win_base`)
/// persists across frames like the device cache; only the per-frame counters reset
/// (mirroring `reset_fetch_metrics`). This is observation only: `word` forwards
/// verbatim, so the VM sees identical bytes.
struct TileRecorder<'a> {
    inner: SliceCode<'a>,
    /// Window capacity in code words (`CODE_WINDOW_BYTES / 4` = 4096 = a 16 KiB tile).
    capacity: u32,
    /// First code word of the single cached tile, persisted across frames (`None`
    /// until the first fetch), exactly like the device window.
    win_base: Option<u32>,
    /// Per-frame refills and a tile bitmask, reset each frame.
    refills: u32,
    tiles_mask: u64,
    /// Per-frame hot-word extent and which tiles' words were *executed* this frame.
    min_word: u32,
    max_word: u32,
}

impl<'a> TileRecorder<'a> {
    fn new(inner: SliceCode<'a>, capacity: u32) -> Self {
        Self {
            inner,
            capacity,
            win_base: None,
            refills: 0,
            tiles_mask: 0,
            min_word: u32::MAX,
            max_word: 0,
        }
    }

    /// Reset only the per-frame counters (the window cache persists), mirroring the
    /// device's `reset_fetch_metrics` called once per frame.
    fn begin_frame(&mut self) {
        self.refills = 0;
        self.tiles_mask = 0;
        self.min_word = u32::MAX;
        self.max_word = 0;
    }

    fn distinct_tiles(&self) -> u32 {
        self.tiles_mask.count_ones()
    }
}

impl CodeSource for TileRecorder<'_> {
    fn word(&mut self, index: u32) -> Option<[u8; 4]> {
        let base = (index / self.capacity) * self.capacity;
        if self.win_base != Some(base) {
            self.win_base = Some(base);
            self.refills += 1;
        }
        let tile = base / self.capacity;
        if tile < 64 {
            self.tiles_mask |= 1 << tile;
        }
        self.min_word = self.min_word.min(index);
        self.max_word = self.max_word.max(index);
        self.inner.word(index)
    }
}

/// KOTO-0155/0156 diagnostic + regression guard: attribute KotoBlocks' real per-frame
/// execution to the device's 16 KiB code-window tiles. KOTO-0155 established that the
/// old steady `refills=2 / code_tiles=2` was the *structural floor* of a per-frame loop
/// body that exceeded one tile (a monotone tile0->tile1 walk plus loop-back, NOT a
/// refill ping-pong). KOTO-0156 then removed that floor: the committed `koto_blocks.kbc`
/// is now built with the preamble-relocation + cold-block-outlining layout opts
/// (app.json `codegen`), which front-load the steady loop into a single tile, so steady
/// gameplay now fetches **0 refills / 1 code tile**. This test locks that in. Pure
/// observation: the VM reads identical bytes; only the wrapper accounts.
#[test]
fn koto_blocks_code_window_tile_profile() {
    // The device window is CODE_WINDOW_BYTES = 16 KiB -> 4096 code words per tile.
    const TILE_WORDS: u32 = (16 * 1024) / 4;

    // Same scripted input as `koto_blocks_play_uses_retained_game2d_layers`: step
    // past the title tile-cache bake, take the title->play transition, spawn + hard
    // drop a piece, then run several steady gameplay frames (the regime the hardware
    // logs sampled).
    let empty = VmInputSnapshot::empty();
    let start = VmInputSnapshot {
        intent_bits: text_intent::NEWLINE,
        ..VmInputSnapshot::empty()
    };
    let hard_drop = VmInputSnapshot {
        text_codepoint: 32,
        ..VmInputSnapshot::empty()
    };
    let mut inputs = vec![empty; 34];
    inputs.push(start);
    inputs.push(empty);
    inputs.push(empty);
    inputs.push(hard_drop);
    inputs.extend(std::iter::repeat_n(empty, 6));

    let mut session = BytecodeSession::<STACK, CALLS>::new(
        KOTO_BLOCKS,
        koto_core::RuntimeLimits::simulator_default(),
        SIM_FRAME_FUEL,
    )
    .expect("koto_blocks verifies");
    let code_words = session.program().code_words();
    let tiles_total = code_words.div_ceil(TILE_WORDS);

    let heap_bytes = session.program().header().max_heap_bytes as usize;
    let mut heap = vec![0u8; heap_bytes];
    if let Some((start, end)) = session.program().rodata_range() {
        heap[..end - start].copy_from_slice(&KOTO_BLOCKS[start..end]);
    }

    let mut host = RecordingHost::default();
    let mut code = TileRecorder::new(
        SliceCode::new(KOTO_BLOCKS, session.program().code_range().0),
        TILE_WORDS,
    );

    println!("=== KotoBlocks code-window tile profile (KOTO-0155) ===");
    println!(
        "  code_words={code_words} tile_words={TILE_WORDS} (16 KiB) -> spans {tiles_total} tiles"
    );
    println!(
        "    {:>3}  {:>8}  {:>7}  {:>10}  {:>18}  {:>12}",
        "f", "instrs", "refills", "code_tiles", "hot_words[min..max]", "hot_kib"
    );

    let mut steady_refills: Vec<u32> = Vec::new();
    let mut steady_tiles: Vec<u32> = Vec::new();
    for (frame_index, input) in inputs.iter().enumerate() {
        if session.has_exited() {
            break;
        }
        code.begin_frame();
        session
            .step_frame_with(&mut code, &mut host, *input, &mut heap)
            .expect("frame steps cleanly");
        let span_words = code.max_word.saturating_sub(code.min_word) + 1;
        println!(
            "    {:>3}  {:>8}  {:>7}  {:>10}  {:>8}..{:<8}  {:>9} KiB",
            frame_index,
            session.last_frame_fuel(),
            code.refills,
            code.distinct_tiles(),
            code.min_word,
            code.max_word,
            (span_words * 4) / 1024,
        );
        // Steady gameplay = the trailing all-`empty` frames after the hard drop.
        if frame_index >= 38 {
            steady_refills.push(code.refills);
            steady_tiles.push(code.distinct_tiles());
        }
    }

    // The whole code segment still spans more than one 16 KiB tile (outlining moves
    // code, it does not shrink the total) — the one-time preamble and the relocated
    // title block now live in the tail tile.
    assert!(
        tiles_total >= 2,
        "fixture no longer spans >1 tile; revisit diagnosis"
    );
    assert!(
        !steady_tiles.is_empty(),
        "no steady gameplay frames were sampled"
    );

    // The decisive check (KOTO-0156): steady gameplay now fits a single code tile and
    // fetches zero refills. With the preamble relocated to the tail and the cold title
    // block outlined, the per-frame loop body packs into one tile, so the hardware no
    // longer pays the old 2-refill boundary-straddle cost (and never a ping-pong). If
    // the `codegen` opts regress or get disabled this trips, flagging the lost win.
    for (&r, &t) in steady_refills.iter().zip(&steady_tiles) {
        assert_eq!(
            (r, t),
            (0, 1),
            "steady gameplay should fit one tile with 0 refills (KOTO-0156); got refills={r}, tiles={t}"
        );
    }
}

/// KotoShogi code-window tile profile. Hardware (2026-07-05) showed the migrated
/// KotoShogi at `refills=166 / code_tiles=4 / vm_us~230ms / fps=4`: the steady play
/// walk crossed four 16 KiB tiles and the 81-cell glyph loop straddled a tile
/// boundary, ping-ponging the device's single-tile window twice per iteration
/// (81 * 2 ≈ 162, plus the walk). Fixed by the KOTO-0156 layout pair (app.json
/// `codegen`: relocate_preamble + outline_cold_blocks over the `continue`-ending
/// title/victory blocks) plus a packed kanji-table glyph loop in the app source.
/// This locks in the repaired steady profile for idle play *and* for the
/// ghost-map build frames after a selection (the other per-frame 81-cell walk).
#[test]
fn kotoshogi_code_window_tile_profile() {
    const TILE_WORDS: u32 = (16 * 1024) / 4;

    let empty = VmInputSnapshot::empty();
    let start = VmInputSnapshot {
        intent_bits: text_intent::NEWLINE,
        ..VmInputSnapshot::empty()
    };
    // Title -> start -> let the opening effects (fx=20) decay -> steady idle,
    // then select the pawn under the initial cursor and let the ghost map build.
    let mut inputs = vec![empty; 2];
    inputs.push(start);
    inputs.extend(std::iter::repeat_n(empty, 30));
    inputs.push(start);
    inputs.extend(std::iter::repeat_n(empty, 16));

    let mut session = BytecodeSession::<STACK, CALLS>::new(
        KOTOSHOGI,
        koto_core::RuntimeLimits::simulator_default(),
        SIM_FRAME_FUEL,
    )
    .expect("kotoshogi verifies");
    let code_words = session.program().code_words();
    let tiles_total = code_words.div_ceil(TILE_WORDS);

    let heap_bytes = session.program().header().max_heap_bytes as usize;
    let mut heap = vec![0u8; heap_bytes];
    if let Some((start, end)) = session.program().rodata_range() {
        heap[..end - start].copy_from_slice(&KOTOSHOGI[start..end]);
    }

    let mut host = RecordingHost::default();
    let mut code = TileRecorder::new(
        SliceCode::new(KOTOSHOGI, session.program().code_range().0),
        TILE_WORDS,
    );

    println!("=== KotoShogi code-window tile profile ===");
    println!(
        "  code_words={code_words} tile_words={TILE_WORDS} (16 KiB) -> spans {tiles_total} tiles"
    );
    println!(
        "    {:>3}  {:>8}  {:>7}  {:>10}  {:>18}",
        "f", "instrs", "refills", "code_tiles", "hot_words[min..max]"
    );

    let mut steady: Vec<(u32, u32)> = Vec::new();
    let mut ghost_build: Vec<(u32, u32)> = Vec::new();
    for (frame_index, input) in inputs.iter().enumerate() {
        if session.has_exited() {
            break;
        }
        code.begin_frame();
        session
            .step_frame_with(&mut code, &mut host, *input, &mut heap)
            .expect("frame steps cleanly");
        println!(
            "    {:>3}  {:>8}  {:>7}  {:>10}  {:>8}..{:<8}",
            frame_index,
            session.last_frame_fuel(),
            code.refills,
            code.distinct_tiles(),
            code.min_word,
            code.max_word,
        );
        // Steady idle play: opening fx has decayed, nothing selected.
        if (26..33).contains(&frame_index) {
            steady.push((code.refills, code.distinct_tiles()));
        }
        // Ghost-map build frames after the selection at frame 33.
        if (34..48).contains(&frame_index) {
            ghost_build.push((code.refills, code.distinct_tiles()));
        }
    }

    assert!(
        tiles_total >= 2,
        "fixture no longer spans >1 tile; revisit diagnosis"
    );
    assert!(!steady.is_empty(), "no steady play frames were sampled");
    assert!(
        !ghost_build.is_empty(),
        "no ghost-build frames were sampled"
    );

    // The device window holds one tile, so per-frame refills ~= tile transitions.
    // Steady idle play must stay a short monotone walk (no per-iteration
    // ping-pong: the 166-refill regression would trip this immediately).
    for &(r, t) in &steady {
        assert!(
            r <= 3 && t <= 3,
            "steady play should walk at most 3 tiles/refills; got refills={r}, tiles={t}"
        );
    }
    for &(r, t) in &ghost_build {
        assert!(
            r <= 6 && t <= 3,
            "ghost-build frames should not ping-pong the window; got refills={r}, tiles={t}"
        );
    }
}

/// KotoMines code-window tile profile: same KOTO-0156 guard as KotoShogi. The
/// retained-render rewrite keeps the steady play walk to a short monotone tile
/// sequence; the flood-fill turn may spill across frames on fuel, but must not
/// ping-pong the single-tile window.
#[test]
fn kotomines_code_window_tile_profile() {
    const TILE_WORDS: u32 = (16 * 1024) / 4;

    let empty = VmInputSnapshot::empty();
    let start = VmInputSnapshot {
        intent_bits: text_intent::NEWLINE,
        ..VmInputSnapshot::empty()
    };
    // Menu -> start (default 9x9) -> first open (mine gen + flood) -> steady idle.
    let mut inputs = vec![empty; 2];
    inputs.push(start);
    inputs.push(empty);
    inputs.push(start);
    inputs.extend(std::iter::repeat_n(empty, 20));

    let mut session = BytecodeSession::<STACK, CALLS>::new(
        KOTOMINES,
        koto_core::RuntimeLimits::simulator_default(),
        SIM_FRAME_FUEL,
    )
    .expect("kotomines verifies");
    let code_words = session.program().code_words();
    let tiles_total = code_words.div_ceil(TILE_WORDS);

    let heap_bytes = session.program().header().max_heap_bytes as usize;
    let mut heap = vec![0u8; heap_bytes];
    if let Some((start, end)) = session.program().rodata_range() {
        heap[..end - start].copy_from_slice(&KOTOMINES[start..end]);
    }

    let mut host = RecordingHost::default();
    let mut code = TileRecorder::new(
        SliceCode::new(KOTOMINES, session.program().code_range().0),
        TILE_WORDS,
    );

    println!("=== KotoMines code-window tile profile ===");
    println!(
        "  code_words={code_words} tile_words={TILE_WORDS} (16 KiB) -> spans {tiles_total} tiles"
    );

    let mut steady: Vec<(u32, u32)> = Vec::new();
    for (frame_index, input) in inputs.iter().enumerate() {
        if session.has_exited() {
            break;
        }
        code.begin_frame();
        session
            .step_frame_with(&mut code, &mut host, *input, &mut heap)
            .expect("frame steps cleanly");
        println!(
            "    {:>3}  {:>8}  {:>7}  {:>10}  {:>8}..{:<8}",
            frame_index,
            session.last_frame_fuel(),
            code.refills,
            code.distinct_tiles(),
            code.min_word,
            code.max_word,
        );
        // Steady play: several frames after the flood settled.
        if frame_index >= 10 {
            steady.push((code.refills, code.distinct_tiles()));
        }
    }

    assert!(!steady.is_empty(), "no steady play frames were sampled");
    // Measured after the retained-render rewrite: steady play fits tile 0 with
    // zero refills. Allow one tile of drift before tripping so innocent source
    // growth does not fail the guard, while any walk/ping-pong regression does.
    for &(r, t) in &steady {
        assert!(
            r <= 1 && t <= 2,
            "steady play should stay within a 2-tile walk; got refills={r}, tiles={t}"
        );
    }
}

/// Shared KOTO-0156 tile-profile driver: step `inputs` through `bytes` with the
/// device-tile [`TileRecorder`] and return per-frame `(refills, distinct_tiles)`.
fn run_tile_profile(name: &str, bytes: &[u8], inputs: &[VmInputSnapshot]) -> Vec<(u32, u32)> {
    const TILE_WORDS: u32 = (16 * 1024) / 4;

    let mut session = BytecodeSession::<STACK, CALLS>::new(
        bytes,
        koto_core::RuntimeLimits::simulator_default(),
        SIM_FRAME_FUEL,
    )
    .unwrap_or_else(|e| panic!("{name} verifies: {e:?}"));

    let heap_bytes = session.program().header().max_heap_bytes as usize;
    let mut heap = vec![0u8; heap_bytes];
    if let Some((start, end)) = session.program().rodata_range() {
        heap[..end - start].copy_from_slice(&bytes[start..end]);
    }

    let mut host = RecordingHost::default();
    let mut code = TileRecorder::new(
        SliceCode::new(bytes, session.program().code_range().0),
        TILE_WORDS,
    );

    println!("=== {name} code-window tile profile ===");
    let mut profile = Vec::new();
    for (frame_index, input) in inputs.iter().enumerate() {
        if session.has_exited() {
            break;
        }
        code.begin_frame();
        session
            .step_frame_with(&mut code, &mut host, *input, &mut heap)
            .expect("frame steps cleanly");
        println!(
            "    {:>3}  {:>8}  {:>7}  {:>10}  {:>8}..{:<8}",
            frame_index,
            session.last_frame_fuel(),
            code.refills,
            code.distinct_tiles(),
            code.min_word,
            code.max_word,
        );
        profile.push((code.refills, code.distinct_tiles()));
    }
    profile
}

/// KotoSnake code-window tile profile. Measured baseline (no codegen opts):
/// steady play is a 2-refill / 2-tile monotone walk — the KOTO-0155 structural
/// floor of a >1-tile loop. The KOTO-0156 codegen pair was tried and REGRESSED
/// snake to a 50-refill ping-pong (the shifted boundary lands in a hot loop),
/// so snake deliberately ships without codegen opts; this guards the floor.
#[test]
fn kotosnake_code_window_tile_profile() {
    let empty = VmInputSnapshot::empty();
    let start = VmInputSnapshot {
        intent_bits: text_intent::NEWLINE,
        ..VmInputSnapshot::empty()
    };
    let mut inputs = vec![empty; 2];
    inputs.push(start);
    inputs.extend(std::iter::repeat_n(empty, 20));

    let profile = run_tile_profile("KotoSnake", KOTOSNAKE, &inputs);
    assert!(profile.len() > 10, "no steady play frames were sampled");
    for &(r, t) in &profile[8..] {
        assert!(
            r <= 2 && t <= 2,
            "steady play should stay a 2-tile monotone walk; got refills={r}, tiles={t}"
        );
    }
}

/// KotoRun code-window tile profile. Measured: steady play is a 2-refill /
/// 2-tile monotone walk both with and without the KOTO-0156 codegen pair (the
/// span exceeds one tile either way and nothing hot straddles the boundary),
/// so kotorun ships without codegen opts; this guards against a ping-pong.
#[test]
fn kotorun_code_window_tile_profile() {
    let empty = VmInputSnapshot::empty();
    let start = VmInputSnapshot {
        intent_bits: text_intent::NEWLINE,
        ..VmInputSnapshot::empty()
    };
    let mut inputs = vec![empty; 2];
    inputs.push(start);
    inputs.extend(std::iter::repeat_n(empty, 20));

    let profile = run_tile_profile("KotoRun", KOTORUN, &inputs);
    assert!(profile.len() > 10, "no steady play frames were sampled");
    for &(r, t) in &profile[8..] {
        assert!(
            r <= 2 && t <= 2,
            "steady play should stay a 2-tile monotone walk; got refills={r}, tiles={t}"
        );
    }
}

/// KOTO-0169 Stage 0c: host-side execution profile of KotoRun *steady play*,
/// captured with the KOTO-VM-0005 method (this fixture runner with its
/// `RecordingHost` and `CountingCode`; run with `--features opcode_stats --
/// --nocapture` for the opcode histogram). The script crosses the title->run
/// transition at frame 2 and then plays a flat steady run — the exact regime
/// the device's `phase=160` lines showed floored at `vm_us` 17.5–24 ms. Wall
/// time is not meaningful on this host harness; instructions/frame and the
/// opcode mix are, and they are the numerator the device `ops=`/`vm_us` lines
/// (Stage 0a/0b) divide against.
#[test]
fn kotorun_steady_play_execution_profile() {
    let empty = VmInputSnapshot::empty();
    let start = VmInputSnapshot {
        intent_bits: text_intent::NEWLINE,
        ..VmInputSnapshot::empty()
    };
    let mut inputs = vec![empty; 2];
    inputs.push(start);
    inputs.extend(std::iter::repeat_n(empty, 60));

    let report = run_fixture_scripted(KOTORUN, &inputs).expect("kotorun verifies");
    report.print("kotorun_steady_play (KOTO-0169 Stage 0c)");

    assert_eq!(report.trap, None, "kotorun trapped: {:?}", report.trap);
    assert_eq!(report.frames_run, inputs.len() as u32);
    // The run actually reached steady play (the flat-run test above pins the
    // transition at frame 2); a title-screen-only capture would be far idler.
    assert!(
        report.budget.frame_fuel_peak > 1_000,
        "steady play never ran (fuel peak {})",
        report.budget.frame_fuel_peak
    );
}

/// KOTO-0169 Stage 0c: host-side execution profile of a *long-body* KotoSnake
/// run, for the device's `vm_us ≈ 165 ms` long-body contrast (GFX-0011). The
/// greedy apple-seeker grows the snake far past RICH_N, and the per-frame fuel
/// is bucketed against body length so the instructions-vs-length shape is
/// explicit. Run with `--features opcode_stats -- --nocapture` for the opcode
/// histogram (cumulative over the run, dominated by steady long-body play).
#[test]
fn kotosnake_long_body_execution_profile() {
    let run = play_kotosnake_greedy(6000);
    assert!(!run.frames.is_empty(), "greedy play produced no frames");

    let max_len = run.frames.iter().map(|f| f.len).max().unwrap_or(0);
    assert!(
        max_len > 12,
        "greedy play never grew a long snake (max_len={max_len})"
    );

    println!("=== kotosnake long-body execution profile (KOTO-0169 Stage 0c) ===");
    println!(
        "  frames={} max_len={} cumulative: instructions={} host_calls={}",
        run.frames.len(),
        max_len,
        run.stats.instructions,
        run.stats.host_calls
    );

    // Instructions vs body length, in 4-cell buckets.
    println!(
        "  {:>9}  {:>7}  {:>10}  {:>10}",
        "len", "frames", "avg_instr", "max_instr"
    );
    let mut bucket = 0usize;
    while bucket * 4 <= max_len {
        let (lo, hi) = (bucket * 4, bucket * 4 + 3);
        let mut count = 0u64;
        let mut sum = 0u64;
        let mut max = 0u32;
        for f in &run.frames {
            if f.len >= lo && f.len <= hi {
                count += 1;
                sum += u64::from(f.instructions);
                max = max.max(f.instructions);
            }
        }
        if count > 0 {
            println!(
                "  {:>4}-{:<4}  {:>7}  {:>10}  {:>10}",
                lo,
                hi,
                count,
                sum / count,
                max
            );
        }
        bucket += 1;
    }

    let peak = run
        .frames
        .iter()
        .enumerate()
        .max_by_key(|(_, f)| f.instructions)
        .expect("non-empty");
    println!(
        "  peak frame: index={} instructions={} len={} app_draw={}",
        peak.0, peak.1.instructions, peak.1.len, peak.1.app_draw
    );

    #[cfg(feature = "opcode_stats")]
    {
        println!("  opcode histogram (executed opcodes, by count):");
        for (op, count) in &run.opcode_hist {
            let pct = if run.stats.instructions > 0 {
                *count as f64 * 100.0 / run.stats.instructions as f64
            } else {
                0.0
            };
            println!(
                "    [{:#04x}] {:<14} x{:<10} {:>5.1}%",
                op,
                opcode_name(*op),
                count,
                pct
            );
        }
    }
}

/// KotoRun steady-play immediate-command stability (KOTO-0168). The present
/// path's dirty derivation diffs the immediate list positionally, so KotoRun's
/// render was rebuilt around byte-stable commands: the flat ground and parallax
/// base bands are static-layer chrome, background motion is quantized into
/// coarse steps, and the HUD score text refreshes on an 8-frame cadence. On a
/// steady flat run (no hazards, no particles) the per-frame rect list must
/// therefore keep a constant length with only a small number of slots changing
/// per frame — that is what keeps steady dirty small and lets a later command
/// count shift localize instead of full-repainting. This test replays a flat
/// run and asserts that contract; it fails if someone reintroduces per-frame
/// churn (e.g. an every-frame parallax step or a frame-varying ground rect).
#[test]
fn kotorun_steady_flat_run_keeps_immediate_rects_byte_stable() {
    const APP_DRAW_CAP: usize = 96;
    // Total frames stepped: play reaches cam ≈ 390, so the visible course
    // grows to the seg-5 spike, all three drones (segs 6-8), and a coin —
    // the realistic mid-run command load — while the runner (wx = cam + 83)
    // stays short of the first collision window (cam ≈ 432).
    const TOTAL: usize = 100;
    // Byte-stability window: cam ∈ [48, 92], where the visible segment span
    // (first = -1, last = 5) and its one spike are constant, no coin has
    // appeared (cs ≤ 1), no drone is on screen yet (seg 6 shows at cam ≥ 96),
    // and the run-start HUD refresh (cam < 40) is over.
    const WINDOW: std::ops::Range<usize> = 14..26;

    let mut session = BytecodeSession::<STACK, CALLS>::new(
        KOTORUN,
        koto_core::RuntimeLimits::simulator_default(),
        SIM_FRAME_FUEL,
    )
    .expect("kotorun verifies");
    let heap_bytes = session.program().header().max_heap_bytes as usize;
    let mut heap = vec![0u8; heap_bytes];
    if let Some((start, end)) = session.program().rodata_range() {
        heap[..end - start].copy_from_slice(&KOTORUN[start..end]);
    }
    let mut host = RecordingHost::default();
    let mut code = SliceCode::new(KOTORUN, session.program().code_range().0);

    let empty = VmInputSnapshot::empty();
    let start_input = VmInputSnapshot {
        intent_bits: text_intent::NEWLINE,
        ..VmInputSnapshot::empty()
    };

    type RectCall = (i32, i32, i32, i32, i32);
    let mut frames: Vec<Vec<RectCall>> = Vec::new();
    let mut app_draw_peak = 0usize;
    for frame in 0..TOTAL {
        let input = if frame == 2 { start_input } else { empty };
        let calls_before = host.calls.len();
        let rects_before = host.rects.len();
        session
            .step_frame_with(&mut code, &mut host, input, &mut heap)
            .expect("frame steps cleanly");
        // Immediate draw count, masking the one-time static-layer capture
        // window exactly as the KotoSnake budget test does.
        let mut app_draw = 0usize;
        let mut capturing = false;
        for c in &host.calls[calls_before..] {
            match c.id {
                host_call::GAME2D_STATIC_BEGIN => capturing = true,
                host_call::GAME2D_STATIC_END => capturing = false,
                host_call::DRAW_RECT
                | host_call::DRAW_TEXT
                | host_call::DRAW_TEXT_COLOR
                | host_call::DRAW_PIXELS_RGB565
                    if !capturing =>
                {
                    app_draw += 1
                }
                _ => {}
            }
        }
        app_draw_peak = app_draw_peak.max(app_draw);
        frames.push(host.rects[rects_before..].to_vec());
    }

    // Every frame stays well inside the device APP_DRAW budget.
    assert!(
        app_draw_peak < APP_DRAW_CAP,
        "flat-run immediate draw peak {app_draw_peak} reached the {APP_DRAW_CAP} cap"
    );

    let window = &frames[WINDOW.start..WINDOW.end];
    let mut total_changed = 0usize;
    let mut quiet_pairs = 0usize;
    for pair in window.windows(2) {
        assert_eq!(
            pair[0].len(),
            pair[1].len(),
            "flat-run rect count shifted between frames (nothing enters or leaves the scene)"
        );
        // Only on-screen churn matters: a command that changes while fully
        // off-screen (e.g. a hazard scrolling toward the viewport) has a
        // `None` footprint on both sides of the device diff and contributes
        // no dirty. `on_screen` mirrors the derive-layer clip.
        let on_screen = |r: &(i32, i32, i32, i32, i32)| {
            r.0 < 320 && r.0 + r.2 > 0 && r.1 < 320 && r.1 + r.3 > 0
        };
        let changed = pair[0]
            .iter()
            .zip(pair[1].iter())
            .filter(|(a, b)| a != b && (on_screen(a) || on_screen(b)))
            .count();
        if changed <= 4 {
            quiet_pairs += 1;
        }
        total_changed += changed;
    }
    let pairs = window.len() - 1;
    // Quantized motion: only the runner's animation slots change every frame;
    // a parallax layer's slots change only on its step frame (the mid layer's
    // step moves its towers and window lights together). Measured: 67 on-screen
    // changed slots over 11 pairs (≈ 6.1/frame; the replay is deterministic).
    // Pre-KOTO-0168 every layer and the ground stepped every frame at ~30+
    // on-screen slots/frame, so a bound of 10/frame catches a reintroduced
    // every-frame step — or a frame-varying ground rect — with margin on both
    // sides.
    assert!(
        total_changed <= pairs * 10,
        "flat-run immediate rects churn too much: {total_changed} changed slots over {pairs} pairs"
    );
    // And genuinely quiet frames exist (only the runner's animation slots move).
    assert!(
        quiet_pairs > 0,
        "no quiet frame in the window: some layer steps every frame"
    );
}

/// KotoRun per-state immediate command-count stability (KOTO-0175 lever 1).
/// The device present path diffs the immediate list positionally, and in a
/// scrolled scene a COUNT change misaligns the whole tail and escalates to a
/// ~100 ms CommandCountShift full repaint (fps 8 on device). KotoRun therefore
/// pads every conditional draw with a fixed off-screen slot: hazards entering/
/// leaving the course, the coin appearing/being collected, particles bursting/
/// dying, the dive streaks, the SMASH!/chain HUD — none of them may change the
/// per-frame command count. This test replays title -> an unassisted run into
/// the seg-5 spike -> game over, and asserts the counts form exactly three
/// plateaus (one per app state); any conditional draw losing its pad shows up
/// as an extra plateau. The transition edges themselves are the one remaining
/// (accepted) count shift: a state change is a scene change.
#[test]
fn kotorun_immediate_command_count_is_state_stable() {
    const APP_DRAW_CAP: usize = 96;
    // 2 title frames, start on frame 2, then run unassisted: the runner meets
    // the seg-5 spike (wx ≈ 515) at cam ≈ 432, i.e. ~frame 110 at base speed,
    // leaving ~70 game-over frames — enough for the SMASH-less death burst's
    // 12 particles to decay to zero *inside* the game-over plateau.
    const TOTAL: usize = 183;

    let mut session = BytecodeSession::<STACK, CALLS>::new(
        KOTORUN,
        koto_core::RuntimeLimits::simulator_default(),
        SIM_FRAME_FUEL,
    )
    .expect("kotorun verifies");
    let heap_bytes = session.program().header().max_heap_bytes as usize;
    let mut heap = vec![0u8; heap_bytes];
    if let Some((start, end)) = session.program().rodata_range() {
        heap[..end - start].copy_from_slice(&KOTORUN[start..end]);
    }
    let mut host = RecordingHost::default();
    let mut code = SliceCode::new(KOTORUN, session.program().code_range().0);

    let empty = VmInputSnapshot::empty();
    let start_input = VmInputSnapshot {
        intent_bits: text_intent::NEWLINE,
        ..VmInputSnapshot::empty()
    };

    let mut counts = Vec::new();
    for frame in 0..TOTAL {
        let input = if frame == 2 { start_input } else { empty };
        let imm_before = host.imm.len();
        session
            .step_frame_with(&mut code, &mut host, input, &mut heap)
            .expect("frame steps cleanly");
        counts.push(
            host.imm[imm_before..]
                .iter()
                .filter(|d| !d.in_static)
                .count(),
        );
    }

    let mut plateaus: Vec<(usize, usize)> = Vec::new();
    for &count in &counts {
        match plateaus.last_mut() {
            Some((value, frames)) if *value == count => *frames += 1,
            _ => plateaus.push((count, 1)),
        }
    }
    println!("kotorun command-count plateaus (count, frames): {plateaus:?}");
    assert_eq!(
        plateaus.len(),
        3,
        "expected exactly one plateau per state (title/play/over); a conditional \
         draw lost its pad slot: {plateaus:?}"
    );
    // The play plateau must actually span the eventful part of the course
    // (hazards scrolling in, the seg-3/4 coins, the death-frame handoff).
    assert!(
        plateaus[1].1 > 80,
        "play plateau too short to have crossed hazards and coins: {plateaus:?}"
    );
    assert!(
        plateaus[2].1 > 30,
        "game-over plateau too short to have outlived the death burst: {plateaus:?}"
    );
    for &(count, _) in &plateaus {
        assert!(
            count < APP_DRAW_CAP,
            "a fixed per-state count reached the {APP_DRAW_CAP}-command cap: {plateaus:?}"
        );
    }
}

/// KotoRogue code-window tile profile: KOTO-0156 guard after the retained-render
/// rewrite (fog + HUD in the static layer, lit tiles immediate). Turn frames run
/// the level logic plus the static rebuild and may spill on fuel, but neither
/// idle nor turn frames may ping-pong the single-tile window.
#[test]
fn kotorogue_code_window_tile_profile() {
    let empty = VmInputSnapshot::empty();
    let start = VmInputSnapshot {
        intent_bits: text_intent::NEWLINE,
        ..VmInputSnapshot::empty()
    };
    let step = VmInputSnapshot {
        intent_bits: text_intent::RIGHT,
        ..VmInputSnapshot::empty()
    };
    // Title -> start -> idle -> a few turns (move right) -> idle.
    let mut inputs = vec![empty; 2];
    inputs.push(start);
    inputs.extend(std::iter::repeat_n(empty, 6));
    for _ in 0..4 {
        inputs.push(step);
        inputs.push(empty);
    }
    inputs.extend(std::iter::repeat_n(empty, 8));

    let profile = run_tile_profile("KotoRogue", KOTOROGUE, &inputs);
    assert!(profile.len() > 12, "no steady play frames were sampled");
    for &(r, t) in &profile[4..] {
        assert!(
            r <= 4 && t <= 4,
            "play frames should stay a short monotone walk; got refills={r}, tiles={t}"
        );
    }
}

/// A second fixture confirms the runner is not specialised to KotoBlocks: a simple
/// counter-loop sample also verifies and produces non-zero VM + code-read metrics.
#[test]
fn counter_loop_runs_and_reports_metrics() {
    const FRAMES: u32 = 4;
    let report =
        run_fixture(COUNTER_LOOP, FRAMES, VmInputSnapshot::empty()).expect("counter_loop verifies");
    report.print("sample_counter_loop");

    assert_eq!(report.trap, None, "counter_loop trapped: {:?}", report.trap);
    assert!(report.frames_run >= 1, "no frames ran");
    assert!(report.stats.instructions > 0, "no instructions executed");
    assert!(
        report.code_reads >= report.stats.instructions,
        "code reads < instructions"
    );
}
