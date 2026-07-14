#![no_std]

//! Board-selected RP2040 / RP2350 backend foundations.
//!
//! This crate owns the fixed board wiring. Portable KotoOS crates must consume
//! HAL traits and must not depend on these pin numbers or on `embassy-rp`.

#[cfg(all(feature = "mcu-rp2040", feature = "mcu-rp235xa"))]
compile_error!("select exactly one MCU feature: `mcu-rp2040` or `mcu-rp235xa`");

#[cfg(not(any(feature = "mcu-rp2040", feature = "mcu-rp235xa")))]
compile_error!("no MCU selected; select a `board-*` feature instead of enabling an MCU directly");

#[cfg(all(feature = "board-picocalc-pico", feature = "board-picocalc-pico2w"))]
compile_error!("select exactly one board feature");

#[cfg(not(any(feature = "board-picocalc-pico", feature = "board-picocalc-pico2w")))]
compile_error!("no board selected; enable `board-picocalc-pico` or `board-picocalc-pico2w`");

pub mod board;
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
