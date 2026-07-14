#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]

//! Pure Rust PSRAM driver foundation for RP2040/RP2350 PicoCalc targets.
//!
//! The crate keeps production byte-slice access separate from diagnostics and
//! tuning helpers. Hardware-specific PIO code is isolated behind backend
//! traits so the safe API can be tested on a host before MCU integration.

#[cfg(all(feature = "rp2040-embassy", feature = "rp235xa-embassy"))]
compile_error!("select exactly one Embassy RP target feature");

pub mod addr;
pub mod bus;
pub mod config;
pub mod device;
#[cfg(feature = "diag")]
pub mod diag;
pub mod error;
pub mod pio;
pub mod protocol;
pub mod region;
#[cfg(any(feature = "rp2040-embassy", feature = "rp235xa-embassy"))]
pub mod rp2040_embassy;
pub mod state;

pub use addr::PsramAddr;
pub use bus::PsramBus;
pub use config::{Pins, TimingConfig};
pub use device::DeviceId;
pub use error::{Mismatch, PsramError};
pub use region::PsramRegion;
