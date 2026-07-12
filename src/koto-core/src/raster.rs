//! Minimal software rasterizer for RGB565 surfaces.
//!
//! The rasterizer surface ([`Canvas`]) and colour ([`Rgb565`]) moved into the
//! KotoGFX foundation crate (KotoGFX migration Stage 4, GFX-0004 — the R14
//! rasterizer + font + colour group), where they sit below the layer compositor
//! that uses them. They are re-exported here so `koto_core::raster::{Canvas,
//! Rgb565}` and the flat `koto_core::{Canvas, Rgb565}` re-exports are unchanged
//! for every consumer (`shell_render`, KotoSim, the firmware present path).
pub use koto_gfx::{Canvas, Rgb565};
