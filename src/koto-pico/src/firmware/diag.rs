//! UART0 diagnostics and per-redraw paint metrics (KOTO-0119 / KOTO-0120).

use core::fmt::Write;

use embassy_rp::uart::UartTx;
use embassy_time::Instant;
use koto_core::CodeTileTransition;

use crate::dashboard::LineBuffer;

// The full-repaint reason enum and the dirty-rect diagnostics model moved to the
// KotoGFX foundation crate (KotoGFX v0 extraction). Re-exported here so the
// firmware's `crate::firmware::diag::{FullRepaintReason, DirtyRectGeometry,
// DIRTY_SAMPLE_QUADS}` paths — and `PaintMetrics`'s use of them below — are
// unchanged.
pub use koto_gfx::{
    CoalescePressure, DirtyRectGeometry, EditRegionShape, FullRepaintReason, DIRTY_SAMPLE_QUADS,
    FULL_REPAINT_AREA, FULL_REPAINT_RECTS,
};

use crate::firmware::app_render::DIRTY_RECT_PROBE_CAP;
use crate::firmware::config::DIAG_PROFILE;

/// True on the per-frame render sample cadence for the active [`DIAG_PROFILE`]
/// (DIAG-0001): frame 1, then every `DIAG_PROFILE.sample_period()` frames. The
/// per-cadence diagnostic lines share this throttle, so it lives in one place and
/// stays decoupled from PSRAM backend selection (it replaces the old
/// `if cfg!(feature = "psram_qpi_code_window_prod_profile") { 120 } else { 30 }`
/// swap). Under `Perf` the period is 120; under `Verbose` it is 30, matching today's
/// dev build.
pub fn on_cadence(frame: u32) -> bool {
    frame == 1 || frame.is_multiple_of(DIAG_PROFILE.sample_period())
}

/// Per-redraw work accounting emitted over UART0 for KOTO-0120 timing
/// validation: raster (CPU) time, transfer (SPI/DMA) time, and the number of
/// pixels actually pushed to GRAM. Total interaction latency is measured
/// separately in the main loop, since it also covers input and state update.
#[derive(Clone, Copy, Default)]
pub struct PaintMetrics {
    raster_us: u64,
    /// Microseconds inside `canvas.clear(base)` per present, summed over rects
    /// (KOTO-0174 Stage 0). Splits `raster_us` into the base-clear pass vs the
    /// command-stack composite (`stack_us = raster_us - clear_us`), so a device
    /// run can confirm the host attribution that the clear is a minority and the
    /// per-command paint (glyphs first) is the bulk.
    clear_us: u64,
    transfer_us: u64,
    /// Split of `transfer_us` (KOTO-0174 H-A): `convert_us` is the CPU RGB565→666
    /// pass, `dma_us` the `set_window`+`RAMWR`+SPI-DMA transfer. `transfer_us`
    /// stays their sum so `phase=160` is unchanged. The split sizes the pipeline
    /// win — the overlap hides only `min(cpu, dma)`, and at 62.5 MHz SPI the DMA
    /// floor is ~0.38 µs/px, so knowing where 0.80 µs/px actually goes decides
    /// pipeline (H-A) vs cheaper/SRAM-placed convert (H-D).
    convert_us: u64,
    dma_us: u64,
    dirty_pixels: u32,
    /// LCD window transfers issued this paint. Each `record_transfer` is one
    /// `write_rgb565_rect`/`fill_rect` GRAM window, so this counts the dirty
    /// rectangles (or strips of a full repaint) actually shipped (KOTO-0131).
    transfers: u32,
    /// Set when the paint reconstructed the whole 320x320 surface (the app delta
    /// fell back to a full repaint) rather than a set of partial rectangles.
    full_repaint: bool,
    /// Why the full repaint happened (KOTO-0143), set together with `full_repaint`
    /// and latched first-wins (a single present full-composites at most once).
    full_repaint_reason: Option<FullRepaintReason>,
    /// Dirty-rect fragmentation geometry for the incremental delta path
    /// (KOTO-0159), recorded once per `present_app_delta`. These let a hardware
    /// run confirm whether slow event frames are dominated by *many small scattered
    /// rects* (each one full-scene raster pass) rather than transfer area. All zero
    /// on a full-repaint or no-change frame.
    dirty_geom: DirtyRectGeometry,
    /// Coalesce-before-decide contrast (GFX-0010 Stage 1B), set when the present path's
    /// new post-coalesce decision is interesting: a `RectsExceeded`/`AreaExceeded` full
    /// repaint that survived coalescing, or a frame the reorder *converted* from a
    /// pre-coalesce full repaint back to incremental. `None` on every other frame
    /// (an unremarkable incremental/skip, or a `CommandCountShift`-attributed repaint).
    coalesce_pressure: Option<CoalescePressure>,
    /// Wide-edit-region alignment shape + real coalesce-before-decide contrast (GFX-0011
    /// Stage 1), set on a `CommandCountShift` full-repaint frame. Stage 1 collects the wide
    /// edit region into the expanded cap and runs it through the same coalesce path as every
    /// other layer, so the pressure here is the *actual* decision contrast (not a dry-run):
    /// it classifies why the frame stayed a full repaint (truncated raw set / genuinely wide
    /// area). `None` on every other frame. A rescued (coalescible) count shift is no longer a
    /// full repaint and is reported on `phase=171` instead.
    command_shift: Option<(EditRegionShape, CoalescePressure)>,
}

impl PaintMetrics {
    pub(crate) fn record_raster(&mut self, started: Instant) {
        self.raster_us = self.raster_us.saturating_add(started.elapsed().as_micros());
    }

    /// Accumulate the base-clear time for this rect (KOTO-0174 Stage 0). Called
    /// inside `record_raster`'s window, so `clear_us <= raster_us` always.
    pub(crate) fn record_clear(&mut self, started: Instant) {
        self.clear_us = self.clear_us.saturating_add(started.elapsed().as_micros());
    }

    pub(crate) fn record_transfer(&mut self, started: Instant, pixels: u32) {
        self.transfer_us = self
            .transfer_us
            .saturating_add(started.elapsed().as_micros());
        self.dirty_pixels = self.dirty_pixels.saturating_add(pixels);
        self.transfers = self.transfers.saturating_add(1);
    }

    /// Record the CPU RGB565→666 convert for one rect (KOTO-0174 H-A). Feeds both
    /// `convert_us` and the combined `transfer_us`, so the latter stays the full
    /// convert+DMA cost `phase=160` already reports.
    pub(crate) fn record_convert(&mut self, started: Instant) {
        let elapsed = started.elapsed().as_micros();
        self.convert_us = self.convert_us.saturating_add(elapsed);
        self.transfer_us = self.transfer_us.saturating_add(elapsed);
    }

    /// Record one pipelined band's window-open prologue (`begin_rgb666`:
    /// CASET/PASET/RAMWR) and count the GRAM window (KOTO-0174 H-A2). The
    /// band's pixels are recorded when its data DMA drains
    /// ([`record_dma_exposed`](Self::record_dma_exposed)).
    pub(crate) fn record_window_open(&mut self, started: Instant) {
        let elapsed = started.elapsed().as_micros();
        self.dma_us = self.dma_us.saturating_add(elapsed);
        self.transfer_us = self.transfer_us.saturating_add(elapsed);
        self.transfers = self.transfers.saturating_add(1);
    }

    /// Record the *exposed* wall time of one pipelined band's data DMA — the
    /// join wall minus the raster/convert that ran underneath it (KOTO-0174
    /// H-A2) — plus the band's pixels. Under the pipeline, `raster_us` /
    /// `convert_us` keep the true CPU cost (their sum can exceed the frame
    /// wall), while `transfer_us`/`dma_us` shrink to what the overlap failed
    /// to hide: exactly the term the `phase=160` A/B watches.
    pub(crate) fn record_dma_exposed(&mut self, exposed_us: u64, pixels: u32) {
        self.dma_us = self.dma_us.saturating_add(exposed_us);
        self.transfer_us = self.transfer_us.saturating_add(exposed_us);
        self.dirty_pixels = self.dirty_pixels.saturating_add(pixels);
    }

    pub(crate) fn mark_full_repaint(&mut self, reason: FullRepaintReason) {
        self.full_repaint = true;
        // First-wins so the reason is deterministic; a single present composites
        // the whole surface at most once, so this latches that one attribution.
        if self.full_repaint_reason.is_none() {
            self.full_repaint_reason = Some(reason);
        }
    }

    pub fn raster_us(&self) -> u64 {
        self.raster_us
    }

    pub fn clear_us(&self) -> u64 {
        self.clear_us
    }

    pub fn convert_us(&self) -> u64 {
        self.convert_us
    }

    pub fn dma_us(&self) -> u64 {
        self.dma_us
    }

    pub fn transfer_us(&self) -> u64 {
        self.transfer_us
    }

    pub fn dirty_px(&self) -> u32 {
        self.dirty_pixels
    }

    pub fn transfers(&self) -> u32 {
        self.transfers
    }

    pub fn full_repaint(&self) -> bool {
        self.full_repaint
    }

    pub fn full_repaint_reason(&self) -> Option<FullRepaintReason> {
        self.full_repaint_reason
    }

    /// Record this frame's dirty-rect fragmentation snapshot (KOTO-0159). Called
    /// once by `present_app_delta` after collecting (and before/after coalescing)
    /// the dirty set; left at its zero default on full-repaint / no-change frames.
    pub(crate) fn record_dirty_geometry(&mut self, geom: DirtyRectGeometry) {
        self.dirty_geom = geom;
    }

    pub fn dirty_geometry(&self) -> DirtyRectGeometry {
        self.dirty_geom
    }

    /// Record this frame's coalesce-before-decide contrast (GFX-0010 Stage 1B). Called by
    /// `present_app_delta` on a surviving `RectsExceeded`/`AreaExceeded` full repaint or a
    /// frame converted back to incremental; left `None` otherwise.
    pub(crate) fn record_coalesce_pressure(&mut self, pressure: CoalescePressure) {
        self.coalesce_pressure = Some(pressure);
    }

    pub fn coalesce_pressure(&self) -> Option<CoalescePressure> {
        self.coalesce_pressure
    }

    /// Record this frame's wide-edit-region shape + real coalesce contrast (GFX-0011
    /// Stage 1). Called by `present_app_delta` on a `CommandCountShift` full repaint; left
    /// `None` otherwise. The pressure is the actual decision contrast (the wide region is now
    /// collected and coalesced on the live path), feeding `phase=169`/`phase=174`.
    pub(crate) fn record_command_shift(
        &mut self,
        shape: EditRegionShape,
        pressure: CoalescePressure,
    ) {
        self.command_shift = Some((shape, pressure));
    }

    pub fn command_shift(&self) -> Option<(EditRegionShape, CoalescePressure)> {
        self.command_shift
    }
}

/// Emit one redraw's timing breakdown over UART0: raster (CPU) microseconds,
/// transfer (SPI/DMA) microseconds, dirty pixel count, and the total
/// interaction latency in milliseconds (KOTO-0120 acceptance criterion 4).
pub fn log_paint_metrics(
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
    line: &mut LineBuffer,
    label: &str,
    metrics: PaintMetrics,
    interaction_started: Instant,
) {
    line.clear();
    let _ = write!(
        line,
        "{} raster_us={} transfer_us={} dirty_px={} latency_ms={}\r\n",
        label,
        metrics.raster_us,
        metrics.transfer_us,
        metrics.dirty_pixels,
        interaction_started.elapsed().as_millis(),
    );
    uart_write_line(uart, line);
}

/// Emit one frame's app draw-command usage over UART0 for KOTO-0129 hardware
/// validation: the used/cap budget and the per-variant Rect/Text/Pixels tally.
/// `overflow` is set only when the command cap was hit this frame (the app's
/// tail commands were dropped). Called on a throttled cadence — first frame and
/// every 60 — so it never spams the heartbeat log.
#[allow(clippy::too_many_arguments)]
pub fn log_app_draw_usage(
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
    line: &mut LineBuffer,
    frame: u32,
    used: usize,
    cap: usize,
    rect: u16,
    text: u16,
    pixels: u16,
    overflow: bool,
) {
    line.clear();
    let _ = write!(
        line,
        "phase=155 app-draw frame={} used={}/{} rect={} text={} pixels={}{}\r\n",
        frame,
        used,
        cap,
        rect,
        text,
        pixels,
        if overflow { " overflow" } else { "" },
    );
    uart_write_line(uart, line);
}

/// Emit one frame's app render-performance breakdown over UART0 (KOTO-0131).
/// This is the single line that lets the slowdown be triaged on hardware: the VM
/// step time, the raster (CPU compose) and transfer (SPI) split, the dirty pixel
/// and rectangle counts, the per-frame host-call count, the per-variant draw
/// tally, whether the delta fell back to a full repaint, and the resulting
/// frame latency / estimated fps. Called on a throttled cadence so it never
/// floods the heartbeat log.
#[allow(clippy::too_many_arguments)]
pub fn log_app_frame_metrics(
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
    line: &mut LineBuffer,
    app_id: &str,
    frame: u32,
    vm_us: u64,
    metrics: PaintMetrics,
    host_calls: u32,
    rect: u16,
    text: u16,
    pixels: u16,
    // Session high-water of the per-frame draw-command count and the running count
    // of frames that hit the cap (tail commands dropped), so the KOTO-0134 160-cap
    // can be judged against real app usage (peak) and any drops (ovf).
    peak: usize,
    overflow_count: u32,
    // PSRAM code-window fetch diagnostics (KOTO-0134): `refills` is the window
    // refills this frame and `code_tiles` the distinct tiles they touched. Once
    // the draw path is cheap, a high `refills` across few `code_tiles` is the
    // `main`<->helper thrash that dominates `vm_us`.
    refills: u32,
    code_tiles: u32,
    // Game2D static/background layer (KOTO-0136): `static_cmds` is the retained
    // layer's command count (built once, then free), and `static_rebuilt` flags
    // the frame the app rebuilt it (which forces one full repaint). `rect`/`text`/
    // `pixels` above now count only the per-frame immediate list, so they drop once
    // the chrome moves into the static layer.
    static_cmds: usize,
    static_rebuilt: bool,
    // Cumulative count of frames this session that rebuilt the Game2D static layer
    // (GFX-0009 Stage-0). A healthy session pins this at a small constant (one per
    // gameplay entry); a per-frame climb is an accidental rebuild — the silent
    // perpetual full repaint GFX-0009 hunts.
    static_rebuilds: u32,
    latency_us: u64,
) {
    // Estimate fps from the whole-frame latency (input + VM + paint + pacing),
    // flooring at one frame so a sub-millisecond idle frame never divides by zero.
    let fps = if latency_us == 0 {
        0
    } else {
        (1_000_000 / latency_us).min(999)
    };
    // Attribute every full repaint (KOTO-0143). `none` on incremental frames so the
    // field is always present and the log stays grep-parseable.
    let full_reason = metrics
        .full_repaint_reason()
        .map(FullRepaintReason::as_str)
        .unwrap_or("none");
    line.clear();
    let _ = write!(
        line,
        "phase=160 app-frame app={} frame={} vm_us={} raster_us={} transfer_us={} dirty_px={} rects={} hostcalls={} rect={} text={} pixels={} static_cmds={} static_rebuilt={} static_rebuilds={} full={} full_reason={} peak={} ovf={} refills={} code_tiles={} fps={} lat_ms={}\r\n",
        app_id,
        frame,
        vm_us,
        metrics.raster_us(),
        metrics.transfer_us(),
        metrics.dirty_px(),
        metrics.transfers(),
        host_calls,
        rect,
        text,
        pixels,
        static_cmds,
        u8::from(static_rebuilt),
        static_rebuilds,
        u8::from(metrics.full_repaint()),
        full_reason,
        peak,
        overflow_count,
        refills,
        code_tiles,
        fps,
        latency_us / 1000,
    );
    uart_write_line(uart, line);
}

/// Emit the `phase=175 app-vm-cost` companion line (KOTO-0169 Stage 0): the
/// steady-frame `vm_us` attribution inputs, on the same cadence as `phase=160`.
/// `ops` is the executed-instruction count (`last_frame_fuel`), `host_us` the
/// wall time the frame's `HOST_CALL` dispatches spent in the device host, and
/// `cw_refill_us`/`refills` the already-metered code-window refill cost — so
/// `vm_us − host_us − cw_refill_us` is pure interpret+fetch time and
/// `vm_us / ops` is the device ns/op headline. A separate sparse line (the
/// GFX-0011 Stage-0b split precedent) rather than new `phase=160` fields: the
/// headline line already runs long, and this keeps its bytes identical.
/// Observe-only: every value is read from state the frame already produced.
#[allow(clippy::too_many_arguments)]
pub fn log_app_vm_cost(
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
    line: &mut LineBuffer,
    app_id: &str,
    frame: u32,
    vm_us: u64,
    ops: u32,
    host_us: u64,
    cw_refill_us: u32,
    refills: u32,
) {
    line.clear();
    let _ = write!(
        line,
        "phase=175 app-vm-cost app={} frame={} vm_us={} ops={} host_us={} cw_refill_us={} refills={}\r\n",
        app_id, frame, vm_us, ops, host_us, cw_refill_us, refills,
    );
    uart_write_line(uart, line);
}

/// Emit the `phase=177 app-present-cost` breakdown (KOTO-0174 Stage 0): the
/// present path's `raster_us` split into the base-clear pass (`clear_us`) and
/// the command-stack composite (`stack_us = raster_us - clear_us`), plus the
/// dirty pixel count, the number of transferred rects, and the immediate
/// command mix (`rect`/`text`/`pixels`/`static_cmds`) painting into them. Lets a
/// device run confirm the host attribution (KOTO-0174 Stage 0c): the clear is a
/// minority, and the per-command paint — glyph rasterization first — is the
/// bulk. Sparse investigation line (DiagClass::Gfx), like the other present
/// diagnostics; observe-only.
#[allow(clippy::too_many_arguments)]
pub fn log_app_present_cost(
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
    line: &mut LineBuffer,
    app_id: &str,
    frame: u32,
    metrics: PaintMetrics,
    rect: u16,
    text: u16,
    pixels: u16,
    static_cmds: usize,
) {
    let raster_us = metrics.raster_us();
    let clear_us = metrics.clear_us();
    line.clear();
    let _ = write!(
        line,
        "phase=177 app-present-cost app={} frame={} raster_us={} clear_us={} stack_us={} transfer_us={} convert_us={} dma_us={} dirty_px={} rects={} rect={} text={} pixels={} static_cmds={}\r\n",
        app_id,
        frame,
        raster_us,
        clear_us,
        raster_us.saturating_sub(clear_us),
        metrics.transfer_us(),
        metrics.convert_us(),
        metrics.dma_us(),
        metrics.dirty_px(),
        metrics.transfers(),
        rect,
        text,
        pixels,
        static_cmds,
    );
    uart_write_line(uart, line);
}

/// Emit a `phase=170 app-static-rebuild` line on a frame the Game2D static/background
/// layer was rebuilt mid-session (GFX-0009 Stage-0). A static rebuild forces one
/// whole-surface `StaticRebuild` full repaint (`present_app_commands`); this line
/// surfaces *when* it happens and the running session total (`static_rebuilds`, the same counter on
/// the `phase=160` line) so a hardware run can confirm the rebuild is one-shot per
/// gameplay entry (the expected title->gameplay transition) rather than recurring
/// every frame (an accidental rebuild — a silent perpetual full repaint). Observe-only:
/// it reads state the present path already produced and changes no rendering
/// behaviour, policy, or reason. The first paint of the session (frame 1) is
/// suppressed by the caller — it is attributed `StaticRebuild` for a different reason
/// (no previous frame to diff) and is not the recurring-rebuild signal. Caller-gated
/// to mid-session rebuild frames, one-shot on the first plus the throttled sample, so
/// it stays low-volume even if a buggy app rebuilds every frame.
///
/// GFX-0013 extends the line with the shadow-diff outcome: `align=` (how the
/// rebuilt list aligned against the fingerprint shadow of the last applied
/// layer), `region=` (unmatched middle width), `would_rects=` / `would_px=`
/// (the diffed damage — the incremental cost a bounded rebuild pays, vs the
/// 102400 px a whole-surface repaint takes), and `acted=` (what the present
/// actually did: `skip` for an identical rebuild, `bounded` when the damage
/// rode the delta working set, `full` for the latch fallback — including a
/// bounded alignment rejected by the base-overdraw hazard guard).
#[allow(clippy::too_many_arguments)]
pub fn log_app_static_rebuild(
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
    line: &mut LineBuffer,
    app_id: &str,
    frame: u32,
    static_rebuilds: u32,
    static_cmds: usize,
    alignment: Option<koto_gfx::StaticRebuildAlignment>,
    would_rects: usize,
    would_px: u32,
    acted: &str,
) {
    use koto_gfx::StaticRebuildAlignment;
    let (align, region) = match alignment {
        // No diff ran this frame (first paint, or a rebuild without a baseline
        // observation) — distinct from NoShadow, where the diff ran and found no
        // valid capture.
        None => ("none", 0),
        Some(StaticRebuildAlignment::NoShadow) => ("noshadow", 0),
        Some(StaticRebuildAlignment::Identical) => ("identical", 0),
        Some(StaticRebuildAlignment::Bounded { region }) => ("bounded", region),
        Some(StaticRebuildAlignment::Wide { region }) => ("wide", region),
    };
    line.clear();
    let _ = write!(
        line,
        "phase=170 app-static-rebuild app={} frame={} static_rebuilds={} static_cmds={} align={} region={} would_rects={} would_px={} acted={}\r\n",
        app_id, frame, static_rebuilds, static_cmds, align, region, would_rects, would_px, acted,
    );
    uart_write_line(uart, line);
}

/// Emit one frame's dirty-rectangle fragmentation geometry over UART0 (KOTO-0159),
/// on the same throttled cadence as `phase=160`. This is the line that confirms on
/// hardware whether a slow event frame (line clear / hard drop / game over) is
/// dominated by *per-rect raster overhead* — many small scattered dirty rects, each
/// driving a full-scene recomposite pass — rather than transfer area: a high
/// `rects_pre` over a tiny `area_pre`, with `bbox` far larger than `area_pre`
/// (scattered, not one block), is that signature. `rects_post` shows how far
/// coalescing collapsed the passes. Skipped (caller-gated) on full-repaint / idle
/// frames where the geometry is all zero.
pub fn log_dirty_rect_geometry(
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
    line: &mut LineBuffer,
    app_id: &str,
    frame: u32,
    geom: DirtyRectGeometry,
) {
    line.clear();
    let _ = write!(
        line,
        "phase=164 dirty-rects app={} frame={} rects_pre={} rects_post={} area_pre={} bbox={} max_area={} min_area={} sample=",
        app_id,
        frame,
        geom.rects_pre,
        geom.rects_post,
        geom.area_pre,
        geom.bbox_area,
        geom.max_area,
        geom.min_area,
    );
    let mut first = true;
    for quad in 0..geom.sample_len.min(DIRTY_SAMPLE_QUADS as u8) as usize {
        if !first {
            let _ = write!(line, ";");
        }
        let base = quad * 4;
        let _ = write!(
            line,
            "{},{},{},{}",
            geom.sample[base],
            geom.sample[base + 1],
            geom.sample[base + 2],
            geom.sample[base + 3],
        );
        first = false;
    }
    let _ = write!(line, "\r\n");
    uart_write_line(uart, line);
}

/// Emit one frame's PSRAM code-window refill histogram and top tile→tile
/// transitions (KOTO-0136 triage), so a high `refills=` on the `phase=160` line
/// can be classified: a few buckets with high counts (and a dominant `cw_trans`
/// pair) is a hot-path tile ping-pong that bytecode/function layout can fix; an
/// even spread across many buckets is a many-tile walk. Emitted on the same
/// throttled cadence as `phase=160`. Bounded for UART safety: at most
/// `MAX_HIST` non-zero histogram buckets are printed, and the transition table is
/// already small, so the line stays well under the `LineBuffer` capacity.
#[allow(clippy::too_many_arguments)]
pub fn log_code_window_fetch(
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
    line: &mut LineBuffer,
    frame: u32,
    refills: u32,
    code_tiles: u32,
    refill_us: u32,
    refill_max_us: u32,
    refill_bytes: u32,
    read_mode: &str,
    read_chunk: usize,
    sm_hz: u32,
    dummy_cycles: u8,
    hist: &[u16],
    transitions: &[CodeTileTransition],
    log_tag: &str,
) {
    const MAX_HIST: usize = 24;
    line.clear();
    // Refill-timing fields (KOTO-0132 phase 1) sit between `code_tiles` and the
    // histogram so a high `refills=` can be read against the microseconds it cost:
    // `cw_refill_us` total, `cw_refill_max_us` worst single refill, `cw_bytes` moved.
    let _ = write!(
        line,
        "phase=163 cw frame={} refills={} code_tiles={} cw_refill_us={} cw_refill_max_us={} cw_bytes={} read_mode={} chunk={} sm_hz={} dummy={} cw_tag={} cw_hist=",
        frame,
        refills,
        code_tiles,
        refill_us,
        refill_max_us,
        refill_bytes,
        read_mode,
        read_chunk,
        sm_hz,
        dummy_cycles,
        log_tag
    );
    let mut first = true;
    let mut printed = 0;
    for (tile, &count) in hist.iter().enumerate() {
        if count == 0 {
            continue;
        }
        if !first {
            let _ = write!(line, ",");
        }
        let _ = write!(line, "{}:{}", tile, count);
        first = false;
        printed += 1;
        if printed >= MAX_HIST {
            break;
        }
    }
    let _ = write!(line, " cw_trans=");
    first = true;
    for t in transitions {
        if t.count == 0 {
            continue;
        }
        if !first {
            let _ = write!(line, ",");
        }
        let _ = write!(line, "{}>{}:{}", t.from, t.to, t.count);
        first = false;
    }
    let _ = write!(line, "\r\n");
    uart_write_line(uart, line);
}

/// Emit the one-shot `phase=162 app-draw-overflow` line the first time an app's
/// per-frame draw-command list hits the `MAX_APP_DRAW_COMMANDS` cap and drops its
/// tail commands (KOTO-0134). One-shot (latched by the caller) so a sustained
/// overflow never floods UART; the running total rides the throttled `phase=160`
/// line as `ovf=`.
pub fn log_app_draw_overflow(
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
    line: &mut LineBuffer,
    frame: u32,
    used: usize,
    cap: usize,
    peak: usize,
) {
    line.clear();
    let _ = write!(
        line,
        "phase=162 app-draw-overflow frame={} used={}/{} peak={} note=tail-dropped\r\n",
        frame, used, cap, peak,
    );
    uart_write_line(uart, line);
}

/// Emit a `phase=180 key` line for each press/release keyboard-bridge event
/// while an app is running (KOTO-0177): the raw scan code and state the STM32
/// bridge delivered, plus whether a shift key was held. Key-mapping questions
/// (what does Shift+F5 actually arrive as?) become answerable from a normal
/// firmware session instead of a `probe_keyboard` reflash. HOLD repeats are
/// skipped by the caller so a held key cannot flood UART; press/release
/// events are human-rate.
pub fn log_key_event(
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
    line: &mut LineBuffer,
    frame: u32,
    state: u8,
    key: u8,
    shift_held: bool,
) {
    line.clear();
    let _ = write!(
        line,
        "phase=180 key frame={} state={} key={:#04x} shift={}\r\n",
        frame,
        state,
        key,
        u8::from(shift_held),
    );
    uart_write_line(uart, line);
}

/// Emit one frame's observe-only immediate-overlay budget verdict over UART0
/// (GFX-0006B observe mode). The firmware dry-runs the finished immediate command
/// list through `koto_gfx::APP_DRAW_BUDGET` *without gating any draw*, so this line
/// reports what the budget *would* have admitted/degraded/rejected for the frame —
/// nothing is dropped. Generic, app-agnostic classification only (see
/// `koto_gfx::classify_command`): the per-class fields are commands grouped by
/// `DrawClass` from primitive kind + geometry, not any app's palette. Emitted on a
/// throttled cadence plus a one-shot the first frame pressure appears, so it never
/// floods the heartbeat log.
pub fn log_app_budget_observation(
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
    line: &mut LineBuffer,
    app_id: &str,
    frame: u32,
    observation: &koto_gfx::BudgetObservation,
) {
    use koto_gfx::DrawClass;
    let first = observation
        .first_pressure()
        .map(DrawClass::as_str)
        .unwrap_or("none");
    line.clear();
    let _ = write!(
        line,
        "phase=168 app-budget-obs app={} frame={} mode=observe total={} core={} actor={} ui={} part={} decor={} debug={} would_admit={} would_degrade={} would_reject={} first_pressure={}\r\n",
        app_id,
        frame,
        observation.total(),
        observation.requested(DrawClass::CoreGameplay),
        observation.requested(DrawClass::Actor),
        observation.requested(DrawClass::CriticalUi),
        observation.requested(DrawClass::Particles),
        observation.requested(DrawClass::Decoration),
        observation.requested(DrawClass::Debug),
        observation.would_admit(),
        observation.would_degrade(),
        observation.would_reject(),
        first,
    );
    uart_write_line(uart, line);
}

/// Emit one `CommandCountShift` frame's edit-region *shape* summary over UART0
/// (GFX-0011 Stage 0 / Stage 0b). When a frame's full repaint was attributed to a shift
/// in the immediate command-list length (`full_reason=CommandCountShift` on `phase=160`),
/// this reports *why* the aligned diff fell back — the prefix/suffix alignment and whether
/// it skipped the edit-region diff entirely (the `rects_pre=0` tell):
///
/// - `prefix_len`/`suffix_len`/`edit_region_prev`/`edit_region_cur`/`max_edit_region` — the
///   alignment shape; near-zero anchors with a wide region (`> max_edit_region`) is a
///   smoothly-shifting list (the whole immediate list moved), not a localizable single edit.
/// - `dirty_skipped=1` — [`koto_gfx::collect_immediate_dirty`] bailed on the wide region
///   *before* diffing it (the region exceeded even the live `DIRTY_RECT_PROBE_CAP`), which is
///   why such a frame reports `rects_pre=0` upstream. Post-GFX-0011-Stage-1 the boundary is the
///   expanded cap, so this now fires only on genuinely huge restructures (case #4).
/// - `fallback_reason` — the classification derived from the *real* post-coalesce contrast:
///   `probe_truncated` (region exceeds even the expanded cap, or a layer overflowed upstream),
///   `area_exceeded` (the collected+coalesced damage is genuinely wide), `rects_exceeded`
///   (irreducible scatter over threshold), or `bounded` (collected within thresholds — a
///   rescued frame is no longer a full repaint and does not reach this line).
///
/// Stage 0b split: the coalesce measurements live on the sparse `phase=174`
/// (`log_app_cmdshift_probe`) so this line stays short enough to transmit intact on hardware
/// (the combined line truncated mid-field). The budget/class correlation the original
/// `phase=169` carried is dropped here — `phase=168` already reports budget/class data on its
/// own cadence, and GFX-0008 established these frames are not a budget population. Caller-gated
/// to CommandCountShift frames, one-shot on the first plus the throttled sample; low-volume.
pub fn log_app_cmdshift_correlation(
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
    line: &mut LineBuffer,
    app_id: &str,
    frame: u32,
    prev_cmds: usize,
    cur_cmds: usize,
    shift: Option<(EditRegionShape, CoalescePressure)>,
) {
    let (shape, pressure) = shift.unwrap_or_default();
    // Classify why the frame stayed a full repaint from the real post-coalesce contrast. On a
    // `CommandCountShift` full repaint the policy escalated on one of these signals (the
    // reason is reordered to CommandCountShift because the count changed), so exactly one
    // applies; `bounded` is the unreachable default (a collected-within-thresholds frame is
    // incremental, not a full repaint).
    let fallback_reason = if pressure.truncated {
        "probe_truncated"
    } else if pressure.area_coalesced >= FULL_REPAINT_AREA {
        "area_exceeded"
    } else if pressure.rects_coalesced as u32 > FULL_REPAINT_RECTS {
        "rects_exceeded"
    } else {
        "bounded"
    };
    line.clear();
    let _ = write!(
        line,
        "phase=169 app-cmdshift app={} frame={} reason=CommandCountShift prev_cmds={} cur_cmds={} prefix_len={} suffix_len={} edit_region_prev={} edit_region_cur={} max_edit_region={} dirty_skipped={} fallback_reason={}\r\n",
        app_id,
        frame,
        prev_cmds,
        cur_cmds,
        shape.prefix_len,
        shape.suffix_len,
        shape.edit_region_prev,
        shape.edit_region_cur,
        DIRTY_RECT_PROBE_CAP, // the live collection cap the shape was computed against
        u8::from(shape.bailed),
        fallback_reason,
    );
    uart_write_line(uart, line);
}

/// Emit one `CommandCountShift` frame's real coalesce-before-decide measurements over UART0
/// (GFX-0011 Stage 1). Split out of `phase=169` so each line stays short enough to transmit
/// intact on hardware. Post-Stage-1 the wide edit region is *collected* on the live path and
/// run through the same coalesce path as every other layer, so these are the actual decision
/// quantities (no dry-run), contrasting the pre-coalesce order with the post-coalesce one:
///
/// - `rects_pre`/`rects_coalesced` — the wide region's union rects before and after coalescing.
/// - `area_pre`/`area_coalesced`/`bbox` — summed and coalesced area, and the pre-coalesce
///   bounding box (`bbox` far over `area_pre` reads as scattered).
/// - `probe_truncated=1` — the raw set overflowed even the expanded cap (or a layer overflowed
///   upstream): an incomplete set, so the fail-safe forced the full repaint.
/// - `old_reason`/`new_reason` — what the pre-coalesce 25-cap order and the post-coalesce order
///   decided; on this line `new_reason=CommandCountShift` (the count changed and the frame still
///   escalated). `converted_to_incremental` is 0 here by construction — a rescued count shift is
///   incremental, reported on `phase=171`, not this line.
///
/// Emitted only when the Stage-1 contrast was recorded (a CommandCountShift full repaint), on
/// the same low-volume cadence as `phase=169`.
pub fn log_app_cmdshift_probe(
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
    line: &mut LineBuffer,
    app_id: &str,
    frame: u32,
    pressure: &CoalescePressure,
) {
    let reason_str =
        |r: Option<FullRepaintReason>| r.map(FullRepaintReason::as_str).unwrap_or("none");
    line.clear();
    let _ = write!(
        line,
        "phase=174 app-cmdshift-probe app={} frame={} rects_pre={} rects_coalesced={} area_pre={} area_coalesced={} bbox={} probe_truncated={} old_reason={} new_reason={} converted_to_incremental={}\r\n",
        app_id,
        frame,
        pressure.rects_pre,
        pressure.rects_coalesced,
        pressure.area_pre,
        pressure.area_coalesced,
        pressure.bbox_area,
        u8::from(pressure.truncated),
        reason_str(pressure.old_reason),
        reason_str(pressure.new_reason),
        u8::from(pressure.converted_to_incremental),
    );
    uart_write_line(uart, line);
}

/// Emit one frame's coalesce-before-decide contrast over UART0 (GFX-0010 Stage 1B). The
/// present path now batch-coalesces the full expanded raw dirty set
/// (`DIRTY_RECT_PROBE_CAP`) *before* `koto_gfx::FullRepaintPolicy::decide`, so a
/// fragmented-but-coalescible frame stays incremental at its post-coalesce pass count
/// instead of escalating on the raw rect count (see
/// GFX-0010-rectsexceeded-pressure-investigation.md). This line reports the *actual*
/// decision path — not a dry-run — contrasting the pre-coalesce order with the new one:
///
/// - `old_reason` — what the pre-coalesce 25-cap order (`decision_rects` /
///   `decision_truncated`) would have decided (`none` if it stayed incremental).
/// - `new_reason` — what the post-coalesce order actually decided (`none` if incremental).
/// - `converted_to_incremental=1` — the headline rescue: the pre-coalesce order would have
///   full-repainted but the coalesced set (`rects_coalesced` ≤ threshold,
///   `area_coalesced` under the area bound) stays incremental. A high `bbox` over a small
///   `area_pre` corroborates the scattered-but-mergeable shape.
/// - `new_reason=RectsExceeded`/`AreaExceeded` with `probe_truncated=0` — irreducible:
///   the scatter does not merge within the waste budget (or the post-coalesce area is
///   genuinely wide), so the full repaint is a defensible raster-pass tradeoff.
/// - `probe_truncated=1` — the expanded collection itself overflowed (or the board band
///   buffer / a wide command restructure dropped rects upstream): the raw set is
///   incomplete, so the reorder fails safe to a full repaint regardless of the count.
///
/// Classification is generic and app-agnostic. Caller-gated to recorded frames, one-shot
/// on the first plus the throttled sample, so it stays low-volume.
pub fn log_app_coalesce_pressure(
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
    line: &mut LineBuffer,
    app_id: &str,
    frame: u32,
    pressure: &CoalescePressure,
) {
    let reason_str =
        |r: Option<FullRepaintReason>| r.map(FullRepaintReason::as_str).unwrap_or("none");
    line.clear();
    let _ = write!(
        line,
        "phase=171 app-coalesce-decide app={} frame={} old_reason={} new_reason={} converted_to_incremental={} decision_rects={} decision_truncated={} rects_pre={} rects_coalesced={} probe_truncated={} area_pre={} area_coalesced={} bbox={}\r\n",
        app_id,
        frame,
        reason_str(pressure.old_reason),
        reason_str(pressure.new_reason),
        u8::from(pressure.converted_to_incremental),
        pressure.decision_rects,
        u8::from(pressure.decision_truncated),
        pressure.rects_pre,
        pressure.rects_coalesced,
        u8::from(pressure.truncated),
        pressure.area_pre,
        pressure.area_coalesced,
        pressure.bbox_area,
    );
    uart_write_line(uart, line);
}

pub fn uart_log(uart: &mut UartTx<'_, embassy_rp::uart::Blocking>, text: &str) {
    let _ = uart.blocking_write(text.as_bytes());
}

pub fn uart_write_line(uart: &mut UartTx<'_, embassy_rp::uart::Blocking>, line: &LineBuffer) {
    let _ = uart.blocking_write(line.as_bytes());
}
