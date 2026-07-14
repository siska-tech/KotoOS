//! RP2040/RP2350 blocking QPI backend boundary for Embassy-based board code.
//!
//! This module is intentionally feature-gated and keeps concrete Embassy types
//! outside the protocol and production driver modules. Board support code owns
//! the actual `embassy-rp` PIO, state-machine, and GPIO types by placing them in
//! [`Rp2040QpiResources`], then supplies an [`Rp2040QpiExecutor`] that performs
//! the register/PIO operations for those concrete types.

mod boundary;
#[cfg(target_os = "none")]
mod diagnostics;
#[cfg(target_os = "none")]
mod embassy_hal;
#[cfg(test)]
mod tests;

pub use boundary::{Rp2040QpiBackend, Rp2040QpiExecutor, Rp2040QpiResources};
#[cfg(target_os = "none")]
pub use embassy_hal::{
    EmbassyRpQpiBackend, EmbassyRpQpiError, PacDmaStatus, PayloadTransferPath, QpiChunkTiming,
    TransactionPioDiagnostics, TransactionPioFastReadLoopVariant,
    TransactionPioTxDmaBufferDiagnostics, TransactionPioTxDmaStep, WordStreamReadDiagnostics,
};
