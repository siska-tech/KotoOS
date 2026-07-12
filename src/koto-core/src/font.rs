//! Read-only access to the compact `.kfont` bitmap font blob.
//!
//! The font reader moved into the KotoGFX foundation crate (KotoGFX migration
//! Stage 4, GFX-0004 — the R14 rasterizer + font + colour group). It is
//! re-exported here so `koto_core::font::*` and `koto_core::{BitmapFont,
//! FontError, Glyph}` are unchanged for every consumer.
pub use koto_gfx::{BitmapFont, FontError, Glyph};
