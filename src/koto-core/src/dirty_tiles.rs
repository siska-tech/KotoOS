//! Dirty-tile / dirty-rect coalescing moved to the `koto-gfx` foundation crate
//! (KotoGFX v0 extraction). Re-exported here so existing `koto_core` paths
//! (`koto_core::dirty_tiles::*`, `koto_core::{coalesce_dirty_tiles,
//! coalesce_rects, TileBand}`) are unchanged. See `koto_gfx::dirty` for the
//! implementation and tests.

pub use koto_gfx::{coalesce_dirty_tiles, coalesce_rects, TileBand};
