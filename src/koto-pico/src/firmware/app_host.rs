//! The `VmHost` adapter the device presents to a running app: bounded draw
//! commands, sandboxed file handles, and the memo IME bridge (KOTO-0129).

use core::fmt::Write;

use embassy_time::Instant;
use embedded_sdmmc::{BlockDevice, LfnBuffer, Mode, ShortFileName, VolumeIdx, VolumeManager};
use koto_core::runtime::{host_call, ime_key};
use koto_core::{
    CellMetrics, HostCallOutcome, HostErrorCode, KotoMemoIme, MemoEditor, MemoImeKey, MemoImeLine,
    MemoImeMode, MemoMove, PixelFormat, RenderSurface, SkkError, SkkLeadingIndex, SkkRead,
    TextLayout, VmHost, VmInputSnapshot, WindowedDict, SKK_LOOKUP_WINDOW_BYTES,
};

use crate::dashboard::LineBuffer;
use crate::firmware::audio::{PcmSubmitError, PicoAudioBackend};
use crate::firmware::audio_cues::{
    builtin_bgm_cue, primary_audio_route, sfx_id_cue, PicoPrimaryCue,
};
use crate::firmware::config::{
    DiagClass, FirmwareClock, DEVICE_CELL_HEIGHT, DEVICE_CELL_WIDTH, DIAG_PROFILE,
    GAME2D_BOARD_CELLS, GAME2D_MAX_SPRITES, GAME2D_MAX_STAMPS, GAME2D_MAX_TEXT_ITEMS,
    MANIFEST_LFN_BYTES, MAX_APP_DRAW_COMMANDS, MAX_APP_TEXT_BYTES, MAX_DEVICE_OPEN_FILES,
};

// The retained Game2D layer data model — the POD layout of the immediate command
// list and the board/sprite/text/static layers — now lives in koto-gfx (GFX-0002,
// migration Stage 2). The firmware keeps the *instances* (`DeviceRuntimeHost`, its
// diff double-buffer) and the VM hostcall methods below; these re-exports keep
// every call site, field byte, and hostcall ID unchanged. `AppStaticLayer` stays
// `pub` so the binary can hold it in a `StaticCell`.
pub use koto_gfx::AppStaticLayer;
// The static-layer fingerprint shadow (GFX-0013) is `pub` for the same reason as
// `AppStaticLayer`: the binary holds the instance in its own `StaticCell`.
pub use koto_gfx::StaticLayerShadow;
pub(crate) use koto_gfx::{AppDrawCommand, Game2dBoard, Game2dSprite, Game2dStampDef, Game2dText};

// `pub` (not `pub(crate)`) so the binary target can hold the two draw-command
// lists in a `StaticCell` and pass them in by reference, keeping these ~30 KiB
// buffers out of the embassy main-task future (KOTO-0134).
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct DeviceRuntimeHost {
    pub(crate) commands: [AppDrawCommand; MAX_APP_DRAW_COMMANDS],
    pub(crate) len: usize,
    // Retained Game2D board tilemap layer (KOTO-0135): one `tile_ref` per cell
    // (app-heap byte offset of a 16x16 RGB565 tile, `-1` empty). Unlike
    // `commands`, this persists across frames — `clear_frame` does NOT reset it —
    // so the existing two-list delta (current vs previous `DeviceRuntimeHost`)
    // detects board changes by diffing it cell-by-cell, with no separate dirty
    // bitset. The present path composites it as a background layer beneath the
    // immediate command list (see `app_render::paint_app_commands`).
    pub(crate) board: Game2dBoard,
    // Retained Game2D sprite/stamp layer (KOTO-0140). Like `board`, these persist
    // across frames (`clear_frame` does NOT reset them) and ride the two-list
    // delta: stamps are session-stable descriptors, sprites are diffed by stable
    // index so a moving piece produces one small dirty band. The present path
    // composites the sprite layer above the board tilemap, beneath the immediate
    // list (fixed z-order, replacing the KOTO-0135 `Board` stream marker).
    pub(crate) stamps: [Game2dStampDef; GAME2D_MAX_STAMPS],
    pub(crate) sprites: [Game2dSprite; GAME2D_MAX_SPRITES],
    // Retained Game2D text layer (KOTO-0141). Like `board`/`sprites`, these persist
    // across frames (`clear_frame` does NOT reset them) and ride the two-list delta:
    // a text item is diffed by stable index so a changed value produces one small
    // dirty row band. The present path composites the text layer above the sprite
    // layer, beneath the immediate list (fixed z-order).
    pub(crate) text_items: [Game2dText; GAME2D_MAX_TEXT_ITEMS],
}

/// Per-frame draw-command tally by variant, emitted over UART for KOTO-0129
/// hardware bring-up so the blit budget can be read against a real app's usage.
#[derive(Clone, Copy, Default)]
pub(crate) struct DrawCommandCounts {
    pub(crate) rect: u16,
    pub(crate) text: u16,
    pub(crate) pixels: u16,
}

impl DeviceRuntimeHost {
    pub const fn new() -> Self {
        Self {
            commands: [AppDrawCommand::Empty; MAX_APP_DRAW_COMMANDS],
            len: 0,
            board: [-1; GAME2D_BOARD_CELLS],
            stamps: [Game2dStampDef::undefined(); GAME2D_MAX_STAMPS],
            sprites: [Game2dSprite::hidden(); GAME2D_MAX_SPRITES],
            text_items: [Game2dText::hidden(); GAME2D_MAX_TEXT_ITEMS],
        }
    }

    pub(crate) fn clear_frame(&mut self) {
        self.len = 0;
        // Note: `board` and the sprite/stamp/text layers are intentionally retained
        // across frames (KOTO-0135 / KOTO-0140 / KOTO-0141).
    }

    /// `true` once the command list is full: further pushes this frame return
    /// `NO_MEMORY` and the app silently drops its tail commands (KOTO-0129).
    pub(crate) fn is_full(&self) -> bool {
        self.len >= self.commands.len()
    }

    /// Count this frame's commands by variant for the UART draw-usage line.
    pub(crate) fn command_counts(&self) -> DrawCommandCounts {
        let mut counts = DrawCommandCounts::default();
        for command in &self.commands[..self.len] {
            match command {
                AppDrawCommand::Empty => {}
                AppDrawCommand::Rect { .. } => counts.rect = counts.rect.saturating_add(1),
                AppDrawCommand::Text { .. } => counts.text = counts.text.saturating_add(1),
                AppDrawCommand::Pixels { .. } => counts.pixels = counts.pixels.saturating_add(1),
            }
        }
        counts
    }

    pub(crate) fn push(&mut self, command: AppDrawCommand) -> HostCallOutcome {
        if self.len >= self.commands.len() {
            return HostCallOutcome::Err(HostErrorCode::NO_MEMORY);
        }
        self.commands[self.len] = command;
        self.len += 1;
        HostCallOutcome::Ok0
    }
}

#[derive(Clone)]
pub(crate) struct DeviceOpenFile {
    name: ShortFileName,
    offset: u32,
    mode: i32,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct AudioHostStats {
    pub(crate) audio_events: u32,
    pub(crate) samples_submitted: u32,
    pub(crate) samples_played: u32,
    pub(crate) drops: u32,
    pub(crate) underruns: u32,
    pub(crate) unsupported_count: u32,
    pub(crate) buffer_level: u32,
    pub(crate) buffer_capacity: u32,
    pub(crate) command_drops: u32,
    pub(crate) bgm_starts: u32,
    pub(crate) bgm_stops: u32,
    pub(crate) active_bgm_voices: u32,
    pub(crate) active_sfx_voices: u32,
    pub(crate) mixer_saturations: u32,
    pub(crate) worker_late: u32,
    pub(crate) worker_max_jitter_us: u32,
    pub(crate) worker_heartbeat: u32,
    pub(crate) core1_stack_free_min: u32,
}

/// [`SkkRead`] over an open SD-card dictionary file: each window fetch is one
/// `seek` plus one bounded `read`, which is exactly the bucket-seek /
/// forward-scan access pattern `SkkIndex::lookup_in_reader` was designed
/// around (HC-6). The file closes when the wrapper drops.
struct SdDictFile<'h, D, T, const MD: usize, const MF: usize, const MV: usize>
where
    D: BlockDevice,
    D::Error: core::fmt::Debug,
    T: embedded_sdmmc::TimeSource,
{
    file: embedded_sdmmc::File<'h, D, T, MD, MF, MV>,
}

impl<'h, D, T, const MD: usize, const MF: usize, const MV: usize> SkkRead
    for SdDictFile<'h, D, T, MD, MF, MV>
where
    D: BlockDevice,
    D::Error: core::fmt::Debug,
    T: embedded_sdmmc::TimeSource,
{
    fn read_at(&mut self, offset: u32, buf: &mut [u8]) -> Result<usize, SkkError> {
        // `seek_from_start` rejects offsets past EOF; report end-of-dictionary
        // instead, matching the `SkkRead` contract.
        if offset >= self.file.length() {
            return Ok(0);
        }
        if self.file.seek_from_start(offset).is_err() {
            return Err(SkkError::Io);
        }
        self.file.read(buf).map_err(|_| SkkError::Io)
    }
}

pub(crate) struct DeviceHost<'a, D>
where
    D: BlockDevice,
    D::Error: core::fmt::Debug,
{
    // Borrowed from a `StaticCell` owned by the binary so this ~30 KiB list does
    // not live inside the embassy main-task future (KOTO-0134).
    pub(crate) draw: &'a mut DeviceRuntimeHost,
    // Retained static/background layer (KOTO-0136), borrowed from its own single
    // `APP_STATIC` StaticCell (not the double-buffered `APP_DRAW` pair). While
    // `capturing_static` is set, `draw_*` calls push here instead of into `draw`.
    pub(crate) static_layer: &'a mut AppStaticLayer,
    audio: &'a mut PicoAudioBackend,
    capturing_static: bool,
    volume_mgr: &'a VolumeManager<D, FirmwareClock>,
    app_id: &'a str,
    files: [Option<DeviceOpenFile>; MAX_DEVICE_OPEN_FILES],
    editor: MemoEditor<1024>,
    ime: KotoMemoIme,
    // Base address and length of the resident app heap, used to turn a
    // `draw_pixels` slice back into a heap-relative `(off, len)` (KOTO-0129).
    heap_base: usize,
    heap_len: usize,
    // One deferred UART diagnostic line from `asset_load`; drained by the
    // frame loop in `app_runtime` after each `step_frame_with` (KOTO-0130).
    pub(crate) diag: LineBuffer,
    // SKK leading-character index, built at app startup (`load_skk`) by
    // streaming the dictionary once off the SD card. The dictionary body never
    // becomes SRAM-resident: conversions re-open the file and scan through
    // `skk_window` (KOTO-0089 windowed SD reader). `None` when the dict file
    // was not found.
    skk_index: Option<SkkLeadingIndex>,
    // Resolved SD-card name of the dictionary file, cached by `load_skk` so a
    // conversion skips the directory scan and opens the file directly.
    skk_file: Option<ShortFileName>,
    // Scan window shared by index build and per-conversion lookups; sized by
    // the koto-core packaging contract (every dictionary line must fit).
    skk_window: [u8; SKK_LOOKUP_WINDOW_BYTES],
    audio_events: u32,
    // Per-frame hostcall wall time (KOTO-0169 Stage 0b, observe-only): the VM
    // brackets every HOST_CALL dispatch with `hostcall_dispatch_begin`/`_end`;
    // the deltas accumulate here and reset in `clear_frame`. Two timer-counter
    // reads per call (~70 calls/frame steady) is trivial next to a ~20 ms
    // `vm_us`, so this is not DIAG-profile-gated.
    hostcall_started: Instant,
    host_us: u64,
}

impl<'a, D> DeviceHost<'a, D>
where
    D: BlockDevice,
    D::Error: core::fmt::Debug,
{
    pub(crate) fn new(
        volume_mgr: &'a VolumeManager<D, FirmwareClock>,
        app_id: &'a str,
        heap: &[u8],
        draw: &'a mut DeviceRuntimeHost,
        static_layer: &'a mut AppStaticLayer,
        audio: &'a mut PicoAudioBackend,
    ) -> Self {
        // The shared StaticCells may carry stale state (the boot pixel diagnostic
        // borrows the same draw buffer; a prior app left its board/static layer);
        // start each app clean. The tilemap and static layer survive `clear_frame`,
        // so reset them here too (KOTO-0135 / KOTO-0136).
        draw.clear_frame();
        draw.board.fill(-1);
        draw.stamps = [Game2dStampDef::undefined(); GAME2D_MAX_STAMPS];
        draw.sprites = [Game2dSprite::hidden(); GAME2D_MAX_SPRITES];
        draw.text_items = [Game2dText::hidden(); GAME2D_MAX_TEXT_ITEMS];
        static_layer.len = 0;
        static_layer.rebuilt = false;
        let surface = RenderSurface::new(320, 320, PixelFormat::Rgb565);
        let layout = TextLayout::new(
            surface,
            CellMetrics {
                cell_width: DEVICE_CELL_WIDTH,
                cell_height: DEVICE_CELL_HEIGHT,
            },
            2,
        )
        .expect("fixed PicoCalc text layout must be valid");
        Self {
            draw,
            static_layer,
            audio,
            capturing_static: false,
            volume_mgr,
            app_id,
            files: core::array::from_fn(|_| None),
            editor: MemoEditor::new(layout),
            ime: KotoMemoIme::new(),
            heap_base: heap.as_ptr() as usize,
            heap_len: heap.len(),
            diag: LineBuffer::new(),
            skk_index: None,
            skk_file: None,
            skk_window: [0u8; SKK_LOOKUP_WINDOW_BYTES],
            audio_events: 0,
            hostcall_started: Instant::MIN,
            host_us: 0,
        }
    }

    /// Route a draw command to the static layer while a `game2d_static_begin`
    /// capture is open, else to the per-frame immediate list (KOTO-0136).
    fn push_draw(&mut self, command: AppDrawCommand) -> HostCallOutcome {
        if self.capturing_static {
            // The static layer's POD `try_push` lives in koto-gfx (GFX-0002); map
            // its capacity result back to the hostcall outcome here, matching the
            // immediate list's `NO_MEMORY`-on-full behaviour byte-for-byte.
            match self.static_layer.try_push(command) {
                Ok(()) => HostCallOutcome::Ok0,
                Err(_) => HostCallOutcome::Err(HostErrorCode::NO_MEMORY),
            }
        } else {
            self.draw.push(command)
        }
    }

    /// Borrow the retained Game2D layers as a [`koto_game2d::Game2dScene`] for the
    /// duration of one hostcall (GFX-0006A). The app-facing Game2D semantics
    /// (tile/sprite/text set/hide/clear) live in `koto-game2d`; the firmware keeps
    /// owning the diff double-buffer and only lends the layers here, so no storage
    /// moves and no field byte changes.
    fn game2d_scene(&mut self) -> koto_game2d::Game2dScene<'_> {
        koto_game2d::Game2dScene {
            board: &mut self.draw.board,
            stamps: &mut self.draw.stamps,
            sprites: &mut self.draw.sprites,
            text_items: &mut self.draw.text_items,
        }
    }

    pub(crate) fn service_audio(&mut self) {
        self.audio.service();
    }

    pub(crate) fn audio_stats(&self) -> AudioHostStats {
        let backend = self.audio.stats();
        AudioHostStats {
            audio_events: self.audio_events,
            samples_submitted: backend.samples_submitted,
            samples_played: backend.samples_played,
            drops: backend.drops,
            underruns: backend.underruns,
            unsupported_count: backend.unsupported_count,
            buffer_level: backend.buffer_level,
            buffer_capacity: backend.buffer_capacity,
            command_drops: backend.command_drops,
            bgm_starts: backend.bgm_starts,
            bgm_stops: backend.bgm_stops,
            active_bgm_voices: backend.active_bgm_voices,
            active_sfx_voices: backend.active_sfx_voices,
            mixer_saturations: backend.mixer_saturations,
            worker_late: backend.worker_late,
            worker_max_jitter_us: backend.worker_max_jitter_us,
            worker_heartbeat: backend.worker_heartbeat,
            core1_stack_free_min: backend.core1_stack_free_min,
        }
    }

    fn log_audio_result(
        &mut self,
        call_id: u8,
        sample_rate: i32,
        frames: i32,
        channels: i32,
        bytes: usize,
        result: &str,
    ) {
        // audio per-call trace (DIAG-0001 §3.2): `phase=172` is the highest-rate
        // audio line and duplicates what `phase=173 audio-summary` aggregates, so it
        // sits one verbose step above the audio-debug summary — it emits only when the
        // profile enables both the Audio class (the subsystem) and the Verbose class
        // (the per-call firehose). That is the Verbose profile today; Audio profile
        // gets the `phase=173` rollup without this per-call spam. When gated off we
        // leave `self.diag` untouched (empty between drains) so nothing transmits.
        if !(DIAG_PROFILE.enables(DiagClass::Audio) && DIAG_PROFILE.enables(DiagClass::Verbose)) {
            return;
        }
        self.diag.clear();
        let _ = write!(
            self.diag,
            "phase=172 audio hostcall=0x{:02x} name={} sample_rate={} frames={} channels={} bytes={} backend={} result={}\r\n",
            call_id,
            host_call::name(call_id),
            sample_rate,
            frames,
            channels,
            bytes,
            self.audio.backend_name(),
            result,
        );
    }

    /// Locate `dict/skk_koto.skk` on the SD card and build the in-SRAM
    /// leading-character index by streaming the file once through the scan
    /// window — the dictionary body itself stays on the card. Writes a
    /// diagnostic to `self.diag`; drain it with `uart_write_line` before the
    /// frame loop.
    pub(crate) fn load_skk(&mut self) {
        let loaded = self.try_load_skk_dict();
        self.diag.clear();
        let _ = if loaded {
            let (dict_len, buckets) = self
                .skk_index
                .as_ref()
                .map(|i| (i.dict_len(), i.entry_count()))
                .unwrap_or((0, 0));
            write!(
                self.diag,
                "phase=159 skk-loaded dict_len={} buckets={}\r\n",
                dict_len, buckets
            )
        } else {
            write!(self.diag, "phase=259 skk-load-fail\r\n")
        };
    }

    fn try_load_skk_dict(&mut self) -> bool {
        let Ok(volume) = self.volume_mgr.open_volume(VolumeIdx(0)) else {
            return false;
        };
        let Ok(root) = volume.open_root_dir() else {
            return false;
        };
        // Use lowercase "dict" to match the embedded_sdmmc case-insensitive FAT
        // short-name matching used elsewhere (see asset_load: "sprites", "maps").
        let Ok(dict_dir) = root.open_dir("dict") else {
            return false;
        };
        let mut lfn_storage = [0u8; MANIFEST_LFN_BYTES];
        let mut short: Option<ShortFileName> = None;
        let mut lfn = LfnBuffer::new(&mut lfn_storage);
        let _ = dict_dir.iterate_dir_lfn(&mut lfn, |entry, long_name| {
            if short.is_none() && !entry.attributes.is_directory() {
                // `skk_koto.skk` is a valid 8.3 name (SKK_KOTO.SKK) so the SD
                // card may store it without an LFN entry, making long_name None.
                // Match both: LFN case-insensitive and SFN exact comparison.
                let lfn_match = long_name.is_some_and(|n| n.eq_ignore_ascii_case("skk_koto.skk"));
                let sfn_match = long_name.is_none()
                    && ShortFileName::create_from_str("SKK_KOTO.SKK")
                        .is_ok_and(|sfn| sfn == entry.name);
                if lfn_match || sfn_match {
                    short = Some(entry.name.clone());
                }
            }
        });
        // If dir scan found nothing, try the known 8.3 name directly.
        let short_name = match short {
            Some(s) => s,
            None => match ShortFileName::create_from_str("SKK_KOTO.SKK") {
                Ok(s) => s,
                Err(_) => return false,
            },
        };
        let Ok(file) = dict_dir.open_file_in_dir(&short_name, Mode::ReadOnly) else {
            return false;
        };
        let mut reader = SdDictFile { file };
        match SkkLeadingIndex::build_from_reader(&mut reader, &mut self.skk_window) {
            Ok(index) => {
                self.skk_index = Some(index);
                self.skk_file = Some(short_name);
                true
            }
            Err(_) => false,
        }
    }

    pub(crate) fn clear_frame(&mut self) {
        self.draw.clear_frame();
        // Reset per-frame static-layer state (KOTO-0136): `capturing_static` so a
        // missing `static_end` never leaks a whole frame's draws into the layer,
        // and `rebuilt` so it flags only a `game2d_static_begin` issued *this*
        // frame (consumed by the present + diagnostics before the next clear).
        self.capturing_static = false;
        self.static_layer.rebuilt = false;
        // Per-frame hostcall wall time (KOTO-0169 Stage 0b): read by the frame
        // loop after `step_frame_with`, so it resets with the other frame state.
        self.host_us = 0;
    }

    /// Wall time (µs) this frame's `HOST_CALL` dispatches spent in the host,
    /// accumulated by the `hostcall_dispatch_begin`/`_end` seam (KOTO-0169
    /// Stage 0b). `vm_us − host_us − cw_refill_us` is pure interpret+fetch time.
    pub(crate) fn last_frame_host_us(&self) -> u64 {
        self.host_us
    }

    pub(crate) fn toggle_wrap(&mut self) {
        self.editor.toggle_wrap();
    }

    fn file_slot(&mut self, handle: i32) -> Result<&mut DeviceOpenFile, HostErrorCode> {
        let index = usize::try_from(handle).map_err(|_| HostErrorCode::BAD_ARGUMENT)?;
        self.files
            .get_mut(index)
            .and_then(Option::as_mut)
            .ok_or(HostErrorCode::BAD_ARGUMENT)
    }

    fn physical_name(&self, path: &str) -> Result<ShortFileName, HostErrorCode> {
        let path =
            koto_core::SandboxPath::resolve(path).map_err(|_| HostErrorCode::BAD_ARGUMENT)?;
        let mut hash = 0x811c_9dc5u32;
        for byte in self
            .app_id
            .bytes()
            .chain(core::iter::once(b':'))
            .chain(path.as_str().bytes())
        {
            hash ^= u32::from(byte);
            hash = hash.wrapping_mul(0x0100_0193);
        }
        const HEX: &[u8; 16] = b"0123456789ABCDEF";
        let mut bytes = *b"00000000.DAT";
        for index in 0..8 {
            bytes[index] = HEX[((hash >> ((7 - index) * 4)) & 0x0f) as usize];
        }
        let text = core::str::from_utf8(&bytes).map_err(|_| HostErrorCode::BAD_ARGUMENT)?;
        ShortFileName::create_from_str(text).map_err(|_| HostErrorCode::BAD_ARGUMENT)
    }
}

/// Map a koto-game2d semantic result back onto the VM hostcall outcome (GFX-0006A).
/// The error variants reproduce the exact `HostErrorCode`s the previously-inlined
/// `game2d_*` methods returned (`BadArgument` -> `BAD_ARGUMENT`, `NoMemory` ->
/// `NO_MEMORY`), and `Ok(())` is the `Ok0` no-return outcome, so an app observes
/// byte-identical hostcall results.
fn map_game2d_result(result: koto_game2d::Game2dResult) -> HostCallOutcome {
    match result {
        Ok(()) => HostCallOutcome::Ok0,
        Err(koto_game2d::Game2dError::BadArgument) => {
            HostCallOutcome::Err(HostErrorCode::BAD_ARGUMENT)
        }
        Err(koto_game2d::Game2dError::NoMemory) => HostCallOutcome::Err(HostErrorCode::NO_MEMORY),
    }
}

impl<D> VmHost for DeviceHost<'_, D>
where
    D: BlockDevice,
    D::Error: core::fmt::Debug,
{
    fn hostcall_dispatch_begin(&mut self) {
        self.hostcall_started = Instant::now();
    }

    fn hostcall_dispatch_end(&mut self) {
        self.host_us = self
            .host_us
            .saturating_add(self.hostcall_started.elapsed().as_micros());
    }

    fn draw_rect(&mut self, x: i32, y: i32, w: i32, h: i32, rgb565: i32) -> HostCallOutcome {
        if w <= 0 || h <= 0 {
            return HostCallOutcome::Err(HostErrorCode::BAD_ARGUMENT);
        }
        self.push_draw(AppDrawCommand::Rect {
            x,
            y,
            w,
            h,
            rgb565: rgb565 as u16,
        })
    }

    fn draw_pixels_rgb565(
        &mut self,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        pixels: &[u8],
    ) -> HostCallOutcome {
        if w <= 0 || h <= 0 {
            return HostCallOutcome::Err(HostErrorCode::BAD_ARGUMENT);
        }
        // The block must carry exactly one little-endian RGB565 pixel (2 bytes)
        // per cell (`len == w * h * 2`); a mismatch means a malformed call, not a
        // partial blit, so reject it rather than re-reading past the source at
        // compose time (KOTO-0129).
        if (w as usize)
            .checked_mul(h as usize)
            .and_then(|cells| cells.checked_mul(2))
            != Some(pixels.len())
        {
            return HostCallOutcome::Err(HostErrorCode::BAD_ARGUMENT);
        }
        // The VM already resolved `pixels` from a slice of the resident app heap.
        // Recover its offset so the present path can re-read the block at compose
        // time without copying every tile into the bounded command list
        // (KOTO-0129). The pointer arithmetic is sound because a `draw_pixels`
        // slice always lies within `[heap_base, heap_base + heap_len)`.
        let start = pixels.as_ptr() as usize;
        let Some(offset) = start.checked_sub(self.heap_base) else {
            return HostCallOutcome::Err(HostErrorCode::BAD_ARGUMENT);
        };
        if offset
            .checked_add(pixels.len())
            .is_none_or(|end| end > self.heap_len)
        {
            return HostCallOutcome::Err(HostErrorCode::BAD_ARGUMENT);
        }
        self.push_draw(AppDrawCommand::Pixels {
            x,
            y,
            w,
            h,
            off: offset as u32,
            len: pixels.len() as u32,
        })
    }

    fn game2d_set_tile(&mut self, layer: i32, x: i32, y: i32, tile_ref: i32) -> HostCallOutcome {
        let heap_len = self.heap_len;
        map_game2d_result(
            self.game2d_scene()
                .set_tile(layer, x, y, tile_ref, heap_len),
        )
    }

    fn game2d_clear_layer(&mut self, layer: i32) -> HostCallOutcome {
        map_game2d_result(self.game2d_scene().clear_layer(layer))
    }

    fn game2d_present(&mut self, _heap: &[u8]) -> HostCallOutcome {
        // Fixed z-order (KOTO-0140): the retained board tilemap and sprite layer
        // composite at a fixed position (static -> tile -> sprite -> immediate) in
        // the present path, so `present` no longer injects a stream marker. Board
        // and sprite changes ride the current-vs-previous `DeviceRuntimeHost` delta
        // and are transferred by `app_render`. The acknowledgement now routes
        // through the koto-game2d present seam (GFX-0006A); still a no-op.
        map_game2d_result(koto_game2d::present())
    }

    fn game2d_stamp_define(
        &mut self,
        stamp_id: i32,
        cells_off: i32,
        count: i32,
        format: i32,
    ) -> HostCallOutcome {
        map_game2d_result(
            self.game2d_scene()
                .stamp_define(stamp_id, cells_off, count, format),
        )
    }

    fn game2d_sprite_set(
        &mut self,
        inst_id: i32,
        stamp_id: i32,
        x: i32,
        y: i32,
        tile_ref: i32,
    ) -> HostCallOutcome {
        let heap_len = self.heap_len;
        map_game2d_result(
            self.game2d_scene()
                .sprite_set(inst_id, stamp_id, x, y, tile_ref, heap_len),
        )
    }

    fn game2d_sprite_hide(&mut self, inst_id: i32) -> HostCallOutcome {
        map_game2d_result(self.game2d_scene().sprite_hide(inst_id))
    }

    fn game2d_sprite_clear_all(&mut self) -> HostCallOutcome {
        map_game2d_result(self.game2d_scene().sprite_clear_all())
    }

    fn game2d_text_set(
        &mut self,
        id: i32,
        x: i32,
        y: i32,
        text: &str,
        rgb565: i32,
    ) -> HostCallOutcome {
        map_game2d_result(self.game2d_scene().text_set(id, x, y, text, rgb565))
    }

    fn game2d_text_hide(&mut self, id: i32) -> HostCallOutcome {
        map_game2d_result(self.game2d_scene().text_hide(id))
    }

    fn game2d_text_clear_all(&mut self) -> HostCallOutcome {
        map_game2d_result(self.game2d_scene().text_clear_all())
    }

    fn game2d_static_begin(&mut self) -> HostCallOutcome {
        // Clear the retained static layer, mark it rebuilt for this frame, and
        // route subsequent draws into it (KOTO-0136). The layer persists across
        // `clear_frame`, so a rebuild starts from empty rather than appending.
        self.static_layer.begin();
        self.capturing_static = true;
        HostCallOutcome::Ok0
    }

    fn game2d_static_end(&mut self) -> HostCallOutcome {
        self.capturing_static = false;
        HostCallOutcome::Ok0
    }

    fn draw_text(&mut self, x: i32, y: i32, text: &str) -> HostCallOutcome {
        self.draw_text_color(x, y, text, 0xffff)
    }

    fn draw_text_color(&mut self, x: i32, y: i32, text: &str, rgb565: i32) -> HostCallOutcome {
        if text.len() > MAX_APP_TEXT_BYTES {
            return HostCallOutcome::Err(HostErrorCode::NO_MEMORY);
        }
        let mut bytes = [0u8; MAX_APP_TEXT_BYTES];
        bytes[..text.len()].copy_from_slice(text.as_bytes());
        self.push_draw(AppDrawCommand::Text {
            x,
            y,
            rgb565: rgb565 as u16,
            bytes,
            len: text.len() as u8,
        })
    }

    fn input_snapshot(&mut self, input: VmInputSnapshot) -> HostCallOutcome {
        HostCallOutcome::Ok2(input.held_bits as i32, input.pressed_bits as i32)
    }

    fn audio_submit_i16(&mut self, frames: i32, channels: i32, samples: &[u8]) -> HostCallOutcome {
        self.audio_events = self.audio_events.saturating_add(1);
        let sample_rate = self.audio.sample_rate_hz() as i32;
        match self
            .audio
            .submit_pcm_i16(sample_rate as u32, frames, channels, samples)
        {
            Ok(accepted) => {
                self.log_audio_result(
                    host_call::AUDIO_SUBMIT_I16,
                    sample_rate,
                    frames,
                    channels,
                    samples.len(),
                    "ok",
                );
                HostCallOutcome::Ok1(accepted)
            }
            Err(PcmSubmitError::BadArgument) => {
                self.audio.record_drop();
                self.log_audio_result(
                    host_call::AUDIO_SUBMIT_I16,
                    sample_rate,
                    frames,
                    channels,
                    samples.len(),
                    "dropped",
                );
                HostCallOutcome::Err(HostErrorCode::BAD_ARGUMENT)
            }
            Err(PcmSubmitError::Unsupported) => {
                self.log_audio_result(
                    host_call::AUDIO_SUBMIT_I16,
                    sample_rate,
                    frames,
                    channels,
                    samples.len(),
                    "unsupported",
                );
                HostCallOutcome::Err(HostErrorCode::UNSUPPORTED)
            }
        }
    }

    fn play_sfx(&mut self, id: i32) -> HostCallOutcome {
        self.audio_events = self.audio_events.saturating_add(1);
        // KOTO-0165: the id maps to a fixed authored blip sequence on the
        // KotoAudio SFX bus (the tone path is gone).
        self.audio.play_sfx_cue(sfx_id_cue(id));
        self.log_audio_result(host_call::PLAY_SFX, -1, 0, 0, 0, "seq-sfx");
        HostCallOutcome::Ok0
    }

    fn play_bgm(&mut self, id: i32) -> HostCallOutcome {
        self.audio_events = self.audio_events.saturating_add(1);
        // KOTO-0165: the id maps to a built-in authored loop on the KotoAudio
        // BGM bus (the built-in MML strings and tone stand-in are gone).
        self.audio.play_bgm_cue(builtin_bgm_cue(id));
        self.log_audio_result(host_call::PLAY_BGM, -1, 0, 0, 0, "seq-bgm");
        HostCallOutcome::Ok0
    }

    fn play_bgm_asset(&mut self, path: &str) -> HostCallOutcome {
        self.audio_events = self.audio_events.saturating_add(1);
        if !path.starts_with("audio/") || !path.ends_with(".kmml") {
            self.audio.record_drop();
            self.log_audio_result(host_call::PLAY_BGM_ASSET, -1, 0, 0, 0, "dropped");
            return HostCallOutcome::Err(HostErrorCode::BAD_ARGUMENT);
        }
        // Primary audio path (KOTO-0164/0165): the asset path is a routing key
        // into the compiled cue tables; the payload is never read. With the
        // legacy SD-load + MML chain removed, an unrouted path is a routing miss
        // and plays nothing (safe drop, `unrouted` in the trace).
        match primary_audio_route(path) {
            Some(PicoPrimaryCue::Bgm(sequence)) => {
                self.audio.play_bgm_cue(sequence);
                self.log_audio_result(host_call::PLAY_BGM_ASSET, -1, 0, 0, 0, "seq-bgm");
                HostCallOutcome::Ok0
            }
            Some(PicoPrimaryCue::Sfx(_)) | None => {
                self.audio.record_unsupported();
                self.log_audio_result(host_call::PLAY_BGM_ASSET, -1, 0, 0, 0, "unrouted");
                HostCallOutcome::Err(HostErrorCode::UNSUPPORTED)
            }
        }
    }

    fn play_sfx_asset(&mut self, path: &str) -> HostCallOutcome {
        self.audio_events = self.audio_events.saturating_add(1);
        if !path.starts_with("audio/") || !path.ends_with(".kmml") {
            self.audio.record_drop();
            self.log_audio_result(host_call::PLAY_SFX_ASSET, -1, 0, 0, 0, "dropped");
            return HostCallOutcome::Err(HostErrorCode::BAD_ARGUMENT);
        }
        // Primary audio path (KOTO-0164/0165): route the path to a one-shot
        // sequence on the KotoAudio SFX bus. See `play_bgm_asset` for the
        // routing-miss policy.
        match primary_audio_route(path) {
            Some(PicoPrimaryCue::Sfx(sequence)) => {
                self.audio.play_sfx_cue(sequence);
                self.log_audio_result(
                    host_call::PLAY_SFX_ASSET,
                    self.audio.sample_rate_hz() as i32,
                    0,
                    1,
                    0,
                    "seq-sfx",
                );
                HostCallOutcome::Ok0
            }
            Some(PicoPrimaryCue::Bgm(_)) | None => {
                self.audio.record_unsupported();
                self.log_audio_result(host_call::PLAY_SFX_ASSET, -1, 0, 0, 0, "unrouted");
                HostCallOutcome::Err(HostErrorCode::UNSUPPORTED)
            }
        }
    }

    fn stop_bgm(&mut self) -> HostCallOutcome {
        self.audio_events = self.audio_events.saturating_add(1);
        self.audio.stop_bgm();
        self.log_audio_result(host_call::STOP_BGM, -1, 0, 0, 0, "ok");
        HostCallOutcome::Ok0
    }

    fn file_open(&mut self, path: &str, mode: i32) -> HostCallOutcome {
        if !matches!(mode, 0..=2) {
            return HostCallOutcome::Err(HostErrorCode::BAD_ARGUMENT);
        }
        let name = match self.physical_name(path) {
            Ok(name) => name,
            Err(error) => return HostCallOutcome::Err(error),
        };
        let Some(handle) = self.files.iter().position(Option::is_none) else {
            return HostCallOutcome::Err(HostErrorCode::NO_MEMORY);
        };
        let Ok(volume) = self.volume_mgr.open_volume(VolumeIdx(0)) else {
            return HostCallOutcome::Err(HostErrorCode::IO_ERROR);
        };
        let Ok(root) = volume.open_root_dir() else {
            return HostCallOutcome::Err(HostErrorCode::IO_ERROR);
        };
        let open_mode = match mode {
            0 => Mode::ReadOnly,
            1 => Mode::ReadWriteCreateOrTruncate,
            2 => Mode::ReadWriteCreateOrAppend,
            _ => unreachable!(),
        };
        let Ok(file) = root.open_file_in_dir(&name, open_mode) else {
            return HostCallOutcome::Err(HostErrorCode::IO_ERROR);
        };
        if mode == 2 && file.seek_from_start(0).is_err() {
            return HostCallOutcome::Err(HostErrorCode::IO_ERROR);
        }
        drop(file);
        self.files[handle] = Some(DeviceOpenFile {
            name,
            offset: 0,
            mode,
        });
        HostCallOutcome::Ok1(handle as i32)
    }

    fn file_read(&mut self, handle: i32, dst: &mut [u8]) -> HostCallOutcome {
        let (name, offset, mode) = match self.file_slot(handle) {
            Ok(file) => (file.name.clone(), file.offset, file.mode),
            Err(error) => return HostCallOutcome::Err(error),
        };
        if mode == 1 {
            return HostCallOutcome::Err(HostErrorCode::PERMISSION_DENIED);
        }
        let Ok(volume) = self.volume_mgr.open_volume(VolumeIdx(0)) else {
            return HostCallOutcome::Err(HostErrorCode::IO_ERROR);
        };
        let Ok(root) = volume.open_root_dir() else {
            return HostCallOutcome::Err(HostErrorCode::IO_ERROR);
        };
        let Ok(file) = root.open_file_in_dir(&name, Mode::ReadOnly) else {
            return HostCallOutcome::Err(HostErrorCode::IO_ERROR);
        };
        if file.seek_from_start(offset).is_err() {
            return HostCallOutcome::Err(HostErrorCode::IO_ERROR);
        }
        let count = match file.read(dst) {
            Ok(count) => count,
            Err(_) => return HostCallOutcome::Err(HostErrorCode::IO_ERROR),
        };
        drop(file);
        if let Ok(slot) = self.file_slot(handle) {
            slot.offset = slot.offset.saturating_add(count as u32);
        }
        HostCallOutcome::Ok1(count as i32)
    }

    fn file_write(&mut self, handle: i32, src: &[u8]) -> HostCallOutcome {
        let (name, offset, mode) = match self.file_slot(handle) {
            Ok(file) => (file.name.clone(), file.offset, file.mode),
            Err(error) => return HostCallOutcome::Err(error),
        };
        if mode == 0 {
            return HostCallOutcome::Err(HostErrorCode::PERMISSION_DENIED);
        }
        let Ok(volume) = self.volume_mgr.open_volume(VolumeIdx(0)) else {
            return HostCallOutcome::Err(HostErrorCode::IO_ERROR);
        };
        let Ok(root) = volume.open_root_dir() else {
            return HostCallOutcome::Err(HostErrorCode::IO_ERROR);
        };
        let Ok(file) = root.open_file_in_dir(&name, Mode::ReadWriteCreateOrAppend) else {
            return HostCallOutcome::Err(HostErrorCode::IO_ERROR);
        };
        if file.seek_from_start(offset).is_err() || file.write(src).is_err() {
            return HostCallOutcome::Err(HostErrorCode::IO_ERROR);
        }
        if file.flush().is_err() {
            return HostCallOutcome::Err(HostErrorCode::IO_ERROR);
        }
        drop(file);
        if let Ok(slot) = self.file_slot(handle) {
            slot.offset = slot.offset.saturating_add(src.len() as u32);
        }
        HostCallOutcome::Ok1(src.len() as i32)
    }

    fn file_close(&mut self, handle: i32) -> HostCallOutcome {
        let Ok(index) = usize::try_from(handle) else {
            return HostCallOutcome::Err(HostErrorCode::BAD_ARGUMENT);
        };
        match self.files.get_mut(index) {
            Some(slot @ Some(_)) => {
                *slot = None;
                HostCallOutcome::Ok0
            }
            _ => HostCallOutcome::Err(HostErrorCode::BAD_ARGUMENT),
        }
    }

    fn close_all_files(&mut self) {
        self.files.fill(None);
    }

    fn asset_load(&mut self, path: &str, dst: &mut [u8]) -> HostCallOutcome {
        // Only "dir/file" (one-level) paths are supported (KOTO-0130).
        let (dir_name, file_name) = match path.rfind('/') {
            Some(idx) => (&path[..idx], &path[idx + 1..]),
            None => {
                self.diag.clear();
                let _ = write!(
                    self.diag,
                    "phase=258 asset-load-fail reason=no-dir path={}\r\n",
                    path
                );
                return HostCallOutcome::Err(HostErrorCode::IO_ERROR);
            }
        };
        let Ok(volume) = self.volume_mgr.open_volume(VolumeIdx(0)) else {
            self.diag.clear();
            let _ = write!(
                self.diag,
                "phase=258 asset-load-fail reason=volume path={}\r\n",
                path
            );
            return HostCallOutcome::Err(HostErrorCode::IO_ERROR);
        };
        let Ok(root) = volume.open_root_dir() else {
            self.diag.clear();
            let _ = write!(
                self.diag,
                "phase=258 asset-load-fail reason=root path={}\r\n",
                path
            );
            return HostCallOutcome::Err(HostErrorCode::IO_ERROR);
        };
        let Ok(sub) = root.open_dir(dir_name) else {
            self.diag.clear();
            let _ = write!(
                self.diag,
                "phase=258 asset-load-fail reason=dir path={}\r\n",
                path
            );
            return HostCallOutcome::Err(HostErrorCode::IO_ERROR);
        };
        // Resolve LFN → 8.3 short name (mirrors storage.rs KOTO-0121 pattern).
        let mut lfn_storage = [0u8; MANIFEST_LFN_BYTES];
        let mut short: Option<ShortFileName> = None;
        let mut lfn = LfnBuffer::new(&mut lfn_storage);
        let _ = sub.iterate_dir_lfn(&mut lfn, |entry, long_name| {
            if short.is_none()
                && !entry.attributes.is_directory()
                && long_name.is_some_and(|n| n.eq_ignore_ascii_case(file_name))
            {
                short = Some(entry.name.clone());
            }
        });
        let Some(short_name) = short else {
            self.diag.clear();
            let _ = write!(
                self.diag,
                "phase=258 asset-load-fail reason=not-found path={}\r\n",
                path
            );
            return HostCallOutcome::Err(HostErrorCode::IO_ERROR);
        };
        let Ok(file) = sub.open_file_in_dir(&short_name, Mode::ReadOnly) else {
            self.diag.clear();
            let _ = write!(
                self.diag,
                "phase=258 asset-load-fail reason=open path={}\r\n",
                path
            );
            return HostCallOutcome::Err(HostErrorCode::IO_ERROR);
        };
        let cap = dst.len().min(file.length() as usize);
        let mut total = 0;
        while total < cap {
            match file.read(&mut dst[total..cap]) {
                Ok(0) => break,
                Ok(n) => total += n,
                Err(_) => {
                    self.diag.clear();
                    let _ = write!(
                        self.diag,
                        "phase=258 asset-load-fail reason=read path={}\r\n",
                        path
                    );
                    return HostCallOutcome::Err(HostErrorCode::IO_ERROR);
                }
            }
        }
        self.diag.clear();
        let _ = write!(
            self.diag,
            "phase=158 asset-load-ok bytes={} path={}\r\n",
            total, path
        );
        HostCallOutcome::Ok1(total as i32)
    }

    fn ime_feed_key(&mut self, kind: i32, codepoint: i32) -> HostCallOutcome {
        let key = match kind {
            ime_key::CHARACTER => match u32::try_from(codepoint).ok().and_then(char::from_u32) {
                Some(ch) => MemoImeKey::Character(ch),
                None => return HostCallOutcome::Err(HostErrorCode::BAD_ARGUMENT),
            },
            ime_key::SHIFT => MemoImeKey::Shift,
            ime_key::CONVERT => MemoImeKey::Convert,
            ime_key::COMMIT => MemoImeKey::Commit,
            ime_key::CANCEL => MemoImeKey::Cancel,
            ime_key::BACKSPACE => MemoImeKey::Backspace,
            ime_key::OTHER => MemoImeKey::Other,
            ime_key::TOGGLE => MemoImeKey::Toggle,
            _ => return HostCallOutcome::Err(HostErrorCode::BAD_ARGUMENT),
        };
        match self.ime.process_key(key, &mut self.editor) {
            Ok(()) => HostCallOutcome::Ok0,
            Err(_) => HostCallOutcome::Err(HostErrorCode::IO_ERROR),
        }
    }

    fn ime_convert(&mut self) -> HostCallOutcome {
        let Some(index) = self.skk_index.as_ref() else {
            return HostCallOutcome::Err(HostErrorCode::UNSUPPORTED);
        };
        let Some(name) = self.skk_file.clone() else {
            return HostCallOutcome::Err(HostErrorCode::UNSUPPORTED);
        };
        // Re-open the dictionary for this conversion and scan it through the
        // window: one bucket seek plus short sequential reads (HC-6), instead
        // of holding the multi-KB dictionary in SRAM. Conversion is user-paced
        // (one Space press), so the per-open cost is irrelevant.
        let Ok(volume) = self.volume_mgr.open_volume(VolumeIdx(0)) else {
            return HostCallOutcome::Err(HostErrorCode::IO_ERROR);
        };
        let Ok(root) = volume.open_root_dir() else {
            return HostCallOutcome::Err(HostErrorCode::IO_ERROR);
        };
        let Ok(dict_dir) = root.open_dir("dict") else {
            return HostCallOutcome::Err(HostErrorCode::IO_ERROR);
        };
        let Ok(file) = dict_dir.open_file_in_dir(&name, Mode::ReadOnly) else {
            return HostCallOutcome::Err(HostErrorCode::IO_ERROR);
        };
        let mut access = WindowedDict {
            index,
            reader: SdDictFile { file },
            window: &mut self.skk_window,
        };
        match self.ime.convert_with_access(&mut access, &mut self.editor) {
            Ok(()) => HostCallOutcome::Ok0,
            Err(_) => HostCallOutcome::Err(HostErrorCode::IO_ERROR),
        }
    }

    fn ime_display(&mut self, dst: &mut [u8]) -> HostCallOutcome {
        let line = self.ime.line();
        let mut len = 0;
        match line.mode {
            MemoImeMode::Empty => {}
            MemoImeMode::Composing => {
                append_utf8(dst, &mut len, "comp:");
                append_utf8(dst, &mut len, line.pending_romaji);
            }
            MemoImeMode::Converting => {
                append_utf8(dst, &mut len, "read:");
                append_utf8(dst, &mut len, line.reading);
                append_utf8(dst, &mut len, line.pending_romaji);
            }
            MemoImeMode::Candidate => {
                append_utf8(dst, &mut len, "cand:");
                if let Some(candidate) = line.candidate {
                    append_utf8(dst, &mut len, candidate);
                }
            }
            MemoImeMode::MissingCandidate => {
                append_utf8(dst, &mut len, "miss:");
                append_utf8(dst, &mut len, line.reading);
            }
        }
        HostCallOutcome::Ok1(len as i32)
    }

    fn ime_query_line(&mut self, dst: &mut [u8]) -> HostCallOutcome {
        match serialize_ime_line(&self.ime.line(), dst) {
            Some(len) => HostCallOutcome::Ok1(len as i32),
            None => HostCallOutcome::Err(HostErrorCode::NO_MEMORY),
        }
    }

    fn edit_configure(&mut self, cols: i32, rows: i32) -> HostCallOutcome {
        if !self.editor.as_str().is_empty() {
            return HostCallOutcome::Err(HostErrorCode::BAD_ARGUMENT);
        }
        let (Ok(cols), Ok(rows)) = (u16::try_from(cols), u16::try_from(rows)) else {
            return HostCallOutcome::Err(HostErrorCode::BAD_ARGUMENT);
        };
        if !(8u16..=80).contains(&cols) || !(4u16..=30).contains(&rows) {
            return HostCallOutcome::Err(HostErrorCode::BAD_ARGUMENT);
        }
        // Mirror the simulator's text_editor(): size the virtual surface to the
        // requested grid so content_cols == cols and content_rows == rows exactly,
        // and use the device font's real half-width/cell-height (6 × 13) so that
        // edit_cell_width() / edit_cell_height() agree with draw_text advances.
        let Some(width) = cols.checked_mul(DEVICE_CELL_WIDTH) else {
            return HostCallOutcome::Err(HostErrorCode::BAD_ARGUMENT);
        };
        let Some(height) = rows
            .checked_add(3)
            .and_then(|r| r.checked_mul(DEVICE_CELL_HEIGHT))
        else {
            return HostCallOutcome::Err(HostErrorCode::BAD_ARGUMENT);
        };
        let surface = RenderSurface::new(width, height, PixelFormat::Rgb565);
        let layout = match TextLayout::new(
            surface,
            CellMetrics {
                cell_width: DEVICE_CELL_WIDTH,
                cell_height: DEVICE_CELL_HEIGHT,
            },
            2,
        ) {
            Ok(layout) => layout,
            Err(_) => return HostCallOutcome::Err(HostErrorCode::BAD_ARGUMENT),
        };
        self.editor = MemoEditor::new(layout);
        HostCallOutcome::Ok0
    }

    fn edit_load(&mut self, src: &[u8]) -> HostCallOutcome {
        let text = match core::str::from_utf8(src) {
            Ok(text) => text,
            Err(_) => return HostCallOutcome::Err(HostErrorCode::BAD_ARGUMENT),
        };
        match self.editor.load_str(text) {
            Ok(()) => HostCallOutcome::Ok0,
            Err(_) => HostCallOutcome::Err(HostErrorCode::NO_MEMORY),
        }
    }

    fn edit_visible_line(&mut self, row: i32, dst: &mut [u8]) -> HostCallOutcome {
        let Ok(row) = u16::try_from(row) else {
            return HostCallOutcome::Err(HostErrorCode::BAD_ARGUMENT);
        };
        let line = self.editor.visible_line(row).unwrap_or("");
        let mut end = line.len().min(dst.len());
        while end > 0 && !line.is_char_boundary(end) {
            end -= 1;
        }
        dst[..end].copy_from_slice(&line.as_bytes()[..end]);
        HostCallOutcome::Ok1(end as i32)
    }

    fn edit_cursor_view(&mut self) -> HostCallOutcome {
        let col = i32::try_from(self.editor.cursor_display_col()).unwrap_or(i32::MAX);
        let row = self
            .editor
            .cursor_visible_row()
            .map(i32::from)
            .unwrap_or(-1);
        HostCallOutcome::Ok2(col, row)
    }

    fn edit_scroll_row(&mut self) -> HostCallOutcome {
        HostCallOutcome::Ok1(i32::try_from(self.editor.scroll_row()).unwrap_or(i32::MAX))
    }

    fn edit_view_metrics(&mut self) -> HostCallOutcome {
        let cell = self.editor.layout().cell;
        HostCallOutcome::Ok2(i32::from(cell.cell_width), i32::from(cell.cell_height))
    }

    fn edit_cursor_status(&mut self, dst: &mut [u8]) -> HostCallOutcome {
        let row = self.editor.cursor_logical_line() + 1;
        let col = self.editor.cursor_column() + 1;
        let mut writer = SliceWriter::new(dst);
        let _ = write!(writer, "Ln {} Col {}", row, col);
        HostCallOutcome::Ok1(writer.written() as i32)
    }

    fn edit_total_lines(&mut self) -> HostCallOutcome {
        HostCallOutcome::Ok1(i32::try_from(self.editor.total_visual_rows()).unwrap_or(i32::MAX))
    }

    fn edit_wrap(&mut self) -> HostCallOutcome {
        HostCallOutcome::Ok1(i32::from(self.editor.is_wrap()))
    }

    fn edit_hscroll_view(&mut self) -> HostCallOutcome {
        let hscroll = i32::try_from(self.editor.hscroll()).unwrap_or(i32::MAX);
        let line_cols = i32::try_from(self.editor.cursor_line_cols()).unwrap_or(i32::MAX);
        HostCallOutcome::Ok2(hscroll, line_cols)
    }

    fn edit_move(&mut self, dir: i32) -> HostCallOutcome {
        use koto_core::runtime::edit_dir;
        let movement = match dir {
            edit_dir::LEFT => MemoMove::Left,
            edit_dir::RIGHT => MemoMove::Right,
            edit_dir::UP => MemoMove::Up,
            edit_dir::DOWN => MemoMove::Down,
            edit_dir::HOME => MemoMove::Home,
            edit_dir::END => MemoMove::End,
            _ => return HostCallOutcome::Err(HostErrorCode::BAD_ARGUMENT),
        };
        self.editor.move_cursor(movement);
        HostCallOutcome::Ok0
    }

    fn edit_delete(&mut self, kind: i32) -> HostCallOutcome {
        use koto_core::runtime::edit_delete;
        let removed = match kind {
            edit_delete::BACKSPACE => self.editor.backspace(),
            edit_delete::FORWARD => self.editor.delete(),
            _ => return HostCallOutcome::Err(HostErrorCode::BAD_ARGUMENT),
        };
        match removed {
            Ok(removed) => HostCallOutcome::Ok1(removed as i32),
            Err(_) => HostCallOutcome::Err(HostErrorCode::IO_ERROR),
        }
    }

    fn edit_query_text(&mut self, dst: &mut [u8]) -> HostCallOutcome {
        let text = self.editor.as_str().as_bytes();
        let len = text.len().min(dst.len());
        dst[..len].copy_from_slice(&text[..len]);
        HostCallOutcome::Ok2(len as i32, self.editor.cursor() as i32)
    }

    fn edit_reserve_rows(&mut self, rows: i32) -> HostCallOutcome {
        self.editor
            .set_reserved_bottom_rows(usize::try_from(rows).unwrap_or(0));
        HostCallOutcome::Ok0
    }

    fn dir_list(&mut self, index: i32, dst: &mut [u8]) -> HostCallOutcome {
        // Device save files use hashed 8.3 names — directory enumeration is not
        // yet supported. Report empty so the file-open dialog shows no entries.
        let _ = (index, dst);
        HostCallOutcome::Ok2(0, 0)
    }
}

fn append_utf8(dst: &mut [u8], len: &mut usize, text: &str) {
    if *len >= dst.len() {
        return;
    }
    let available = dst.len() - *len;
    let mut count = text.len().min(available);
    while count > 0 && !text.is_char_boundary(count) {
        count -= 1;
    }
    dst[*len..*len + count].copy_from_slice(&text.as_bytes()[..count]);
    *len += count;
}

/// `core::fmt::Write` adapter for `&mut [u8]`, used where heap allocation is
/// unavailable (KOTO-0131).
struct SliceWriter<'a> {
    buf: &'a mut [u8],
    len: usize,
}

impl<'a> SliceWriter<'a> {
    fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, len: 0 }
    }

    fn written(&self) -> usize {
        self.len
    }
}

impl Write for SliceWriter<'_> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let available = self.buf.len() - self.len;
        let n = s.len().min(available);
        self.buf[self.len..self.len + n].copy_from_slice(&s.as_bytes()[..n]);
        self.len += n;
        Ok(())
    }
}

/// Serialize the IME composition line into `dst` for `ime_query_line`. Layout:
/// `[mode:u8][sticky:u8]` then three length-prefixed UTF-8 fields (`pending`,
/// `reading`, `candidate`), each `[len:u8][bytes]`, then `[cand_index:u8]`
/// `[cand_count:u8]`. Returns the byte count, or `None` when `dst` is too small.
fn serialize_ime_line(line: &MemoImeLine<'_>, dst: &mut [u8]) -> Option<usize> {
    fn mode_byte(mode: MemoImeMode) -> u8 {
        match mode {
            MemoImeMode::Empty => 0,
            MemoImeMode::Composing => 1,
            MemoImeMode::Converting => 2,
            MemoImeMode::Candidate => 3,
            MemoImeMode::MissingCandidate => 4,
        }
    }
    let fields = [
        line.pending_romaji.as_bytes(),
        line.reading.as_bytes(),
        line.candidate.unwrap_or("").as_bytes(),
    ];
    let needed = 2 + fields.iter().map(|f| 1 + f.len()).sum::<usize>() + 2;
    if needed > dst.len() {
        return None;
    }
    let mut at = 0;
    dst[at] = mode_byte(line.mode);
    dst[at + 1] = u8::from(line.sticky_shift_armed);
    at += 2;
    for field in fields {
        dst[at] = u8::try_from(field.len()).ok()?;
        at += 1;
        dst[at..at + field.len()].copy_from_slice(field);
        at += field.len();
    }
    dst[at] = line.candidate_index.min(255) as u8;
    dst[at + 1] = line.candidate_count.min(255) as u8;
    at += 2;
    Some(at)
}
