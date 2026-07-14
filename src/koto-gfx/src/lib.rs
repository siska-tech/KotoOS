//! KotoGFX v0 — internal graphics foundation for KotoOS.
//!
//! This crate is the first extraction of KotoOS's retained-rendering concepts
//! into a dedicated, hardware-independent place. It currently owns only the
//! *pure data structures and policy* that were previously scattered between
//! `koto_core` and the Pico firmware's `app_render`/`diag` modules:
//!
//! - [`Rect`] and its pure rectangle helpers (area, bounding box).
//! - Dirty-tile / dirty-rect coalescing ([`coalesce_dirty_tiles`],
//!   [`coalesce_rects`], [`TileBand`]).
//! - The full-repaint threshold policy ([`FullRepaintPolicy`],
//!   [`FullRepaintReason`], [`DeltaDecision`]).
//! - The dirty-rect fragmentation diagnostics model ([`DirtyRectGeometry`]).
//! - The budgeted immediate-overlay admission model ([`DrawBudget`],
//!   [`DrawClass`], [`OverlayPriority`], [`BudgetDecision`], [`BudgetStats`]) —
//!   the next step toward separating immediate effects from the retained
//!   pipeline. It is pure policy data and *does not gate any draw path*; only the
//!   cap value ([`APP_DRAW_BUDGET`]) is shared with the firmware.
//! - The observe-only metering over that model ([`classify_command`],
//!   [`BudgetObservation`]) — a generic, app-agnostic dry run that records what
//!   the budget *would* admit/degrade/reject for a frame's immediate command list
//!   (GFX-0006B observe mode). It still drops nothing; enforcement is not enabled.
//! - The retained layer data model ([`AppDrawCommand`], [`Game2dSprite`],
//!   [`Game2dStampDef`], [`Game2dText`], [`Game2dTilemap`], [`AppStaticLayer`]) —
//!   the POD layout of the Game2D layers (GFX-0002). The firmware still owns the
//!   *instances* and the VM hostcall dispatch and re-exports these types, so no
//!   field byte, hostcall ID, or behaviour changes.
//!
//! - The rasterizer surface ([`Canvas`]), its colour ([`Rgb565`]), and the
//!   bitmap font reader ([`BitmapFont`], [`Glyph`]) — the R14 group, moved down
//!   here below the compositor that uses them (GFX-0004), re-exported from
//!   `koto_core` so `shell_render` and every other consumer is unchanged.
//! - The CPU layer compositor ([`paint_app_commands`] and the per-layer
//!   [`paint_board_layer`]/[`paint_sprite_layer`]/[`paint_text_layer`]/
//!   [`paint_command_list`] passes) — the fixed-z-order chunk compositor over the
//!   retained layer model + the app heap slice (GFX-0004). The firmware still
//!   owns the strip, the clear-to-base/banding, and the present orchestration; it
//!   calls these through identically-signed adapters.
//!
//! It is deliberately *not* the final KotoGFX. There is no PSRAM-backed surface
//! and no display service here yet; the live firmware present path
//! (`present_app_delta` and friends) still owns the damage-merge and flush. This
//! is a behaviour-preserving move: every symbol below was lifted verbatim from
//! its previous home and is re-exported from there, so no rendering behaviour,
//! ABI, opcode, or bytecode changes.
//!
//! The crate is `no_std` (std is enabled only under `cfg(test)`) and never
//! allocates, so it builds for the `thumbv6m-none-eabi` firmware target.

#![cfg_attr(not(test), no_std)]

mod budget;
mod convert;
mod derive;
mod diag_profile;
mod dirty;
mod font;
mod layer;
mod observe;
mod paint;
mod raster;
mod rect;
mod repaint;
mod shadow;
mod stats;

pub use budget::{
    BudgetDecision, BudgetStats, DrawBudget, DrawClass, OverlayPriority, APP_DRAW_BUDGET,
    DRAW_CLASS_COUNT,
};
pub use convert::convert_rgb565_to_rgb666;
pub use derive::{
    board_band_rect, collect_immediate_dirty, collect_initial_scene_dirty, command_dirty_rect,
    has_retained_scene_content, is_full_screen_base, probe_command_shift_coalesce, push_dirty,
    sprite_dirty_rect, sprite_footprint_rect, stamp_cell, text_dirty_rect, text_footprint_rect,
    tilemap_bounds_rect, CommandShiftProbe, EditRegionShape, MAX_EDIT_REGION,
};
pub use diag_profile::{DiagClass, DiagProfile};
pub use dirty::{coalesce_dirty_tiles, coalesce_rects, TileBand};
pub use font::{BitmapFont, FontError, Glyph};
pub use layer::{
    is_persistent_pixels, pixel_heap_offset, AppDrawCommand, AppStaticLayer, Game2dSprite,
    Game2dStampDef, Game2dText, Game2dTilemap, LayerFull, APP_DRAW_PERSISTENT_BIT,
    GAME2D_LEGACY_COLS, GAME2D_LEGACY_ORIGIN_X, GAME2D_LEGACY_ORIGIN_Y, GAME2D_LEGACY_ROWS,
    GAME2D_STATIC_CMD_CAP, GAME2D_TEXT_BYTES, GAME2D_TILEMAP_MAX_CELLS, GAME2D_TILEMAP_MAX_COLS,
    GAME2D_TILEMAP_MAX_ROWS, GAME2D_TILE_BYTES, GAME2D_TILE_PX, MAX_APP_TEXT_BYTES,
};
pub use observe::{classify_command, BudgetObservation};
pub use paint::{
    paint_app_commands, paint_board_layer, paint_command_list, paint_sprite_layer, paint_text_layer,
};
pub use raster::{Canvas, Rgb565};
pub use rect::Rect;
pub use repaint::{
    coalesce_then_decide, decision_snapshot, force_full_repaint_after_static_rebuild,
    CoalesceDecision, CoalescePressure, DeltaDecision, DeltaInputs, FullRepaintPolicy,
    FullRepaintReason, FULL_REPAINT_AREA, FULL_REPAINT_RECTS,
};
pub use shadow::{
    collect_static_rebuild_dirty, StaticLayerShadow, StaticRebuildAlignment, STATIC_DAMAGE_CAP,
};
pub use stats::{DirtyRectGeometry, DIRTY_SAMPLE_QUADS};
