//! Display-service seam (GFX-0005): route the frame loop's present through an
//! explicit service boundary instead of inlining the compose/flush decision.
//!
//! Per `design/KOTOOS_RESOURCE_OWNERSHIP.md` §4 a present is a *request*, not proof
//! of owning the live panel. This module is that seam. The frame loop builds a
//! [`PresentRequest`] and calls [`DisplayService::present`], which runs the §4
//! flow:
//!
//! 1. receive the present request,
//! 2. collect damage (the existing dirty derivation),
//! 3. system overlay / status-bar composite hook — a **no-op placeholder** here,
//! 4. decide ([`koto_gfx::FullRepaintPolicy`]),
//! 5. coalesce,
//! 6. compose (the koto-gfx CPU compositor),
//! 7. flush (the `PicoCalcLcd` / HAL path).
//!
//! Steps 2 and 4–7 are **unchanged**: they still live in
//! [`present_app_delta`](super::app_render::present_app_delta) /
//! [`present_app_commands`](super::app_render::present_app_commands), which the
//! service calls. The service only adds the boundary itself, the no-op overlay
//! hook (step 3), and the path selection the old call site already did (delta vs
//! first-frame full build). That makes this stage byte-equivalent: no visible
//! rendering change, and the present trigger still fires on the same frames.
//!
//! What this stage deliberately does **not** add (future, separate work): a real
//! overlay / status bar / display takeover / capture, an async present queue, a
//! surface registry, or any CPU0/CPU1 re-homing — the service runs inline on the
//! caller's core exactly as the old call site did, so a future decision to pin a
//! display service to `system_service_core` (resource-ownership §3) stays a
//! localised change. `PaintMetrics` and the `phase=160/164/161` diagnostics stay
//! in firmware (`diag` / `app_runtime`); the service only threads `&mut metrics`.

use koto_core::BitmapFont;

use crate::firmware::app_host::{AppStaticLayer, DeviceRuntimeHost};
use crate::firmware::app_render::{present_app_commands, present_app_delta};
use crate::firmware::config::{RASTER_STRIP_BYTES, RGB666_STRIP_BYTES};
use crate::firmware::diag::{FullRepaintReason, PaintMetrics};
use crate::lcd::PicoCalcLcd;

/// One frame's present request (the `frame_inputs` of the §4 flow). Bundles the
/// app-produced frame the service must present, the last presented frame it diffs
/// against (retained-GRAM baseline), the shared retained static layer, and the app
/// heap that backs pixel/sprite sources. The app surface is the fixed 320x320 panel
/// throughout this path, so it is implicit rather than carried here.
pub(crate) struct PresentRequest<'a> {
    /// The frame the app just produced this step — the present target.
    pub current: &'a DeviceRuntimeHost,
    /// The previously presented frame, treated as retained GRAM for the damage
    /// diff. Only meaningful when `has_previous` is set.
    pub previous: &'a DeviceRuntimeHost,
    /// The retained static/background layer (a single shared instance, not
    /// double-buffered with `current`/`previous`).
    pub static_layer: &'a AppStaticLayer,
    /// The app heap backing `Pixels`/sprite sources for this frame.
    pub heap: &'a [u8],
    /// Whether a previously presented frame exists to diff against. The first
    /// present of a session has none and composites the whole surface from scratch.
    pub has_previous: bool,
    /// Force a whole-surface full repaint even though a previous frame exists
    /// (BUG-GFX-0012): the present immediately following a retained-static-layer
    /// rebuild, whose chrome the rebuild frame composited but may have overdrawn,
    /// so the incremental path cannot be trusted to reveal it. Set by the frame
    /// loop's one-shot latch; steady frames leave it `false` and take the delta path.
    pub force_full: bool,
    /// This frame rebuilt the static layer and the shadow diff could NOT
    /// positively bound the change (no shadow / wide relayout / base-overdraw
    /// hazard): the delta path takes the whole-surface `StaticRebuild` repaint,
    /// exactly as every rebuild did pre-GFX-0013.
    pub static_rebuild_full: bool,
    /// A *bounded* static rebuild's damage (GFX-0013): the unmatched commands'
    /// old∪new union rects from the frame loop's shadow diff, fed into the delta
    /// working set alongside every other layer's dirty rects. Empty on steady,
    /// identical-rebuild, and fallback-rebuild frames.
    pub static_damage: &'a [koto_core::Rect],
}

/// The firmware display service (GFX-0005). Today it is a stateless boundary: it
/// owns no app state and no surface registry — it only routes a [`PresentRequest`]
/// through the §4 flow and is the place a future overlay / status bar / takeover
/// will compose. Held by the frame loop across the session so that future state
/// (damage registry, overlay) has a home without another call-site change.
pub(crate) struct DisplayService;

impl DisplayService {
    pub(crate) const fn new() -> Self {
        Self
    }

    /// Satisfy one present request: run the system overlay/status-bar hook (no-op
    /// placeholder), then the existing collect → decide → coalesce → compose →
    /// flush body via the present path the old call site selected. Behaviour and
    /// metrics are identical to the pre-seam inline call; the service only makes the
    /// boundary explicit so present is no longer a straight shot to the LCD.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn present(
        &mut self,
        request: PresentRequest<'_>,
        lcd: &mut PicoCalcLcd<'_>,
        font: &BitmapFont<'_>,
        strip: &mut [u8; RASTER_STRIP_BYTES],
        scratch: &mut [u8; RGB666_STRIP_BYTES],
        metrics: &mut PaintMetrics,
    ) -> Result<(), ()> {
        // §4 step 3: system overlay / status-bar composite. No-op placeholder in
        // this stage — present is routed through the seam where that composite will
        // run instead of going straight to the LCD.
        self.composite_system_overlay(&request);

        // §4 steps 2, 4–7: collect damage → decide (FullRepaintPolicy) → coalesce →
        // compose (koto-gfx) → flush (HAL). This body is unchanged and still lives in
        // the present_app_* functions; the service only selects the path the old
        // call site did (delta vs first-frame full build) and threads metrics
        // through unchanged.
        if request.has_previous && !request.force_full {
            present_app_delta(
                lcd,
                font,
                request.previous,
                request.current,
                request.static_layer,
                request.heap,
                strip,
                scratch,
                request.static_rebuild_full,
                request.static_damage,
                metrics,
            )
            .await
        } else {
            present_app_commands(
                lcd,
                font,
                request.current,
                request.static_layer,
                request.heap,
                strip,
                scratch,
                // First paint of the session (no previous frame to diff), or the
                // forced reveal following a static-layer rebuild (BUG-GFX-0012):
                // build the whole surface from scratch, attributed to the static
                // build (KOTO-0143) — the reason the old call site passed and the
                // reason the reveal is a full compose.
                FullRepaintReason::StaticRebuild,
                metrics,
            )
            .await
        }
    }

    /// System overlay / status-bar composite hook (GFX-0005). Intentionally empty:
    /// the overlay / status bar / foreground takeover is a system-app-only
    /// capability (resource-ownership §9) and is **not** built in this stage. This
    /// is only the seam where that composite — and the overlay damage merge — will
    /// run once it exists, so present already routes through a service boundary
    /// rather than straight to the panel.
    fn composite_system_overlay(&mut self, _request: &PresentRequest<'_>) {}
}
