#![no_std]

//! PicoCalc-specific RP2040 backend foundations.
//!
//! This crate owns the fixed board wiring. Portable KotoOS crates must consume
//! HAL traits and must not depend on these pin numbers or on `embassy-rp`.

pub mod dashboard;
pub mod firmware;
pub mod keyboard;
pub mod lcd;
pub mod pins;
pub mod power;
pub mod psram;
pub mod psram_ext;

#[cfg(any(feature = "psram_pio_word_diag", feature = "psram_dma_read_api"))]
pub mod psram_dma;
