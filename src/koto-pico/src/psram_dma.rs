//! Diagnostic-only PSRAM DMA helpers (feature-gated)
//!
//! This module provides diagnostic-only DMA helpers for the 1-bit PIO PSRAM
//! path. It is not
//! enabled by default and does not change production `PsramHal::read`.
//!
//! Notes on DMA channel usage in the PicoCalc firmware (observed):
//! - LCD scanline / SPI1 transfers use `DMA_CH0` in several probe/firmware
//!   entry points (`probe_lcd.rs`, `koto_firmware.rs`).
//! - PWM/audio probe uses `DMA_CH0` as well in `probe_audio.rs` (diagnostic
//!   ring buffer implementation).
//! - Therefore `DMA_CH0` is a shared resource; any PSRAM diagnostic that
//!   attempts to allocate DMA channels should avoid colliding with live LCD
//!   or audio transfers on test hardware, or should temporarily stop those
//!   consumers while the diagnostic runs.
//!
//! The active diagnostic DMA path is `dma16_v2`:
//! - source: PIO1 RXF0 MMIO
//! - destination: `[u8; 16]`
//! - data size: 8-bit
//! - transfer count: 16 bytes
//! - read_increment=false, write_increment=true
//! - DREQ/TREQ: PIO1_RX0
//!
//! All helpers in this file are compiled only with `psram_pio_word_diag`.

use koto_core::hal::HalError;
#[cfg(feature = "psram_dma_read_api")]
use koto_core::psram::PsramError;

#[cfg(feature = "psram_dma_read_code_window")]
pub const PSRAM_SM1_FUDGE_CLKDIV3_CHUNK_BYTES: usize = 32;
/// Larger per-transaction chunk for the fudge DMA path; `out y, 16` in the PIO
/// program allows up to 8 KiB per transaction.  1 KiB gives a ~32× reduction in
/// per-chunk SM-restart overhead while staying well within the u16 y-counter range.
#[cfg(feature = "psram_dma_read_code_window")]
pub const PSRAM_SM1_FUDGE_CLKDIV3_LARGE_CHUNK_BYTES: usize = 1024;
#[cfg(feature = "psram_dma_read_code_window")]
pub const PSRAM_SM1_FUDGE_CLKDIV3_HZ: u32 = crate::psram::PSRAM_PIO_SYS_HZ / 3;

/// Diagnostic-only error type for the `psram_pio_word_diag` module.
///
/// We define a local error type so the diagnostic harness can return
/// meaningful errors without introducing `HalError::Unavailable` into
/// production-facing APIs.
#[derive(Debug)]
pub enum DiagError {
    Unavailable,
    InvalidArgument,
    Unsupported,
    DmaTimeout(Option<Dma16Diagnostic>),
    DmaDualFailure(Option<DmaDualDiagnostic>),
    FaithfulPriorArtFailure(Option<FaithfulPriorArtDiagnostic>),
    FaithfulPriorArtCpuTxFailure(Option<FaithfulPriorArtDiagnostic>),
    PriorArtConfigError(PriorArtConfigDiagnostic),
    DmaAhbError,
    Hal(HalError),
}

/// Detailed snapshot of DMA state for diagnostics.
#[derive(Debug, Clone, Copy)]
pub struct Dma16Diagnostic {
    pub pio_instance: u8,           // Always 1
    pub sm_index: u8,               // Always 0
    pub dma_channel: u8,            // Always 1
    pub dreq_numeric_pio1_rx0: u8,  // Numeric DREQ/TREQ selector for PIO1_RX0
    pub src_addr_intended: u32,     // Intended PIO RX FIFO MMIO address
    pub src_addr_written: u32,      // Value latched in DMA READ_ADDR register
    pub dst_addr: u32,              // Destination buffer address
    pub trans_count_before: u32,    // Before ARM
    pub trans_count_after_arm: u32, // After ARM
    pub trans_count_after_cmd: u32, // After command issued
    pub trans_count_timeout: u32,   // On timeout
    pub ctrl_before_arm: u32,       // Raw ctrl_trig before ARM
    pub ctrl_after_arm: u32,        // Raw ctrl_trig after ARM
    pub ctrl_after_cmd: u32,        // Raw ctrl_trig after command
    pub ctrl_on_timeout: u32,       // Raw ctrl_trig on timeout
    pub ctrl_en_timeout: bool,
    pub ctrl_busy_timeout: bool,
    pub ctrl_irq_quiet_timeout: bool,
    pub ctrl_read_error_timeout: bool,
    pub ctrl_write_error_timeout: bool,
    pub ctrl_ahb_error_timeout: bool,
    pub ctrl_data_size_timeout: u8,
    pub ctrl_incr_read_timeout: bool,
    pub ctrl_incr_write_timeout: bool,
    pub ctrl_ring_size_timeout: u8,
    pub ctrl_ring_sel_timeout: bool,
    pub ctrl_chain_to_timeout: u8,
    pub ctrl_treq_sel_timeout: u8,
    pub ctrl_sniff_en_timeout: bool,
    pub read_addr_timeout: u32,
    pub write_addr_timeout: u32,
    pub dma_intr_timeout: u32,
    pub dma_inte0_timeout: u32,
    pub dma_ints0_timeout: u32,
    pub dma_ch1_irq_pending_timeout: bool,
    pub timeout_snapshot_before_abort: bool,
    pub paced_dma_aborted_before_return: bool,
    pub ctrl_after_abort: u32,
    pub pio_rx_level_before: u32, // RX FIFO level (estimated)
    pub pio_rx_level_after_cmd: u32,
    pub pio_rx_level_timeout: u32,
    pub pio_tx_level_before: u32,
    pub pio_tx_level_after_cmd: u32,
    pub pio_tx_level_timeout: u32,
    pub pio_flevel_before: u32,
    pub pio_flevel_after_cmd: u32,
    pub pio_flevel_timeout: u32,
    pub pio_fstat_timeout: u32,
    pub pio_fdebug_timeout: u32,
    pub sm0_enabled_timeout: bool,
    pub sm0_tx_empty_timeout: bool,
    pub sm0_rx_empty_timeout: bool,
    pub sm0_shiftctrl_timeout: u32,
    pub sm0_execctrl_timeout: u32,
    pub ahb_error: bool,
    pub timed_out: bool,
}

/// Snapshot for dual-DMA diagnostic (`dma16_dual`) failures.
#[derive(Debug, Clone, Copy)]
pub struct DmaDualDiagnostic {
    pub pio_instance: u8,
    pub sm_index: u8,
    pub tx_dma_channel: u8,
    pub rx_dma_channel: u8,
    pub tx_dreq_numeric_pio1_tx0: u8,
    pub rx_dreq_numeric_pio1_rx0: u8,
    pub tx_src_addr: u32,
    pub tx_dst_addr: u32,
    pub rx_src_addr: u32,
    pub rx_dst_addr: u32,
    pub tx_ctrl: u32,
    pub rx_ctrl: u32,
    pub tx_trans_count: u32,
    pub rx_trans_count: u32,
    pub tx_read_addr: u32,
    pub tx_write_addr: u32,
    pub rx_read_addr: u32,
    pub rx_write_addr: u32,
    pub pio_flevel: u32,
    pub pio_fstat: u32,
    pub dma_intr0: u32,
    pub dma_ints0: u32,
    pub dma_inte0: u32,
    pub rx_fifo_drained_words: u32,
    pub timeout_stage: u8, // 1=tx wait, 2=rx wait, 0=other failure
    pub tx_ahb_error: bool,
    pub rx_ahb_error: bool,
}

/// Success report shared by faithful prior-art diagnostic variants.
#[derive(Debug, Clone, Copy)]
pub struct FaithfulPriorArtReport {
    pub variant_name: &'static str,
    pub pio_instance: u8,
    pub sm_index: u8,
    pub program_offset: u8,
    pub tx_dma_channel: Option<u8>,
    pub rx_dma_channel: Option<u8>,
    pub data_size_bits: u8,
    pub tx_transfer_count: Option<u32>,
    pub rx_transfer_count: Option<u32>,
    pub tx_dreq_numeric: Option<u8>,
    pub rx_dreq_numeric: Option<u8>,
    pub command_bytes: [u8; 7],
    pub first_16_output_bytes: [u8; 16],
}

/// Per-variant snapshot for SM1 FIFO/shift self-test.
#[derive(Debug, Clone, Copy)]
pub struct FifoEcho8VariantReport {
    pub variant_name: &'static str,
    pub input_byte: u8,
    pub received_byte: Option<u8>,
    pub input_word: Option<u32>,
    pub received_word: Option<u32>,
    pub pass: bool,
    pub tx_dma_channel: Option<u8>,
    pub rx_dma_channel: Option<u8>,
    pub tx_remaining: Option<u32>,
    pub rx_remaining: Option<u32>,
    pub tx_busy: Option<bool>,
    pub rx_busy: Option<bool>,
    pub fifo_levels: u32,
    pub fifo_stat: u32,
    pub fdebug_raw: u32,
    pub fdebug_sm_txstall: bool,
    pub fdebug_sm_rxstall: bool,
    pub fdebug_sm_txover: bool,
    pub fdebug_sm_rxunder: bool,
}

/// Aggregate report for diagnostic-only PIO1 SM1 FIFO/shift self-test.
#[derive(Debug, Clone, Copy)]
pub struct FifoEcho8Report {
    pub pio_variant: &'static str,
    pub pio_instance: u8,
    pub sm_index: u8,
    pub program_offset: u8,
    pub shiftctrl: u32,
    pub execctrl: u32,
    pub pinctrl: u32,
    pub sm_enable_bits: u8,
    pub sm_pc: u32,
    pub case_a_cpu_u8_tx_cpu_rx: FifoEcho8VariantReport,
    pub case_b_packed_tx_cpu_rx: FifoEcho8VariantReport,
    pub case_c_cpu_u8_tx_dma_rx: FifoEcho8VariantReport,
    pub case_d_tx_dma_rx_dma: Option<FifoEcho8VariantReport>,
    pub case_e_cpu_u32_tx_cpu_u32_rx: FifoEcho8VariantReport,
    pub pass: bool,
}

/// Unified failure snapshot shared by faithful prior-art diagnostic variants.
#[derive(Debug, Clone, Copy)]
pub struct FaithfulPriorArtDiagnostic {
    pub variant_name: &'static str,
    pub command_bytes: [u8; 7],
    pub first_16_output_bytes: [u8; 16],
    pub pio_instance: u8,
    pub sm_index: u8,
    pub program_offset: u8,
    pub tx_dma_channel: Option<u8>,
    pub rx_dma_channel: Option<u8>,
    pub data_size_bits: u8,
    pub tx_transfer_count: Option<u32>,
    pub rx_transfer_count: Option<u32>,
    pub tx_remaining: Option<u32>,
    pub rx_remaining: Option<u32>,
    pub tx_dreq_numeric: Option<u8>,
    pub rx_dreq_numeric: Option<u8>,
    pub tx_src_addr: Option<u32>,
    pub tx_dst_addr: Option<u32>,
    pub rx_src_addr: Option<u32>,
    pub rx_dst_addr: Option<u32>,
    pub tx_read_addr: Option<u32>,
    pub tx_write_addr: Option<u32>,
    pub rx_read_addr: Option<u32>,
    pub rx_write_addr: Option<u32>,
    pub tx_ctrl: Option<u32>,
    pub rx_ctrl: Option<u32>,
    pub tx_busy: Option<bool>,
    pub tx_en: Option<bool>,
    pub tx_ahb_error: Option<bool>,
    pub rx_busy: Option<bool>,
    pub rx_en: Option<bool>,
    pub rx_ahb_error: Option<bool>,
    pub sm0_tx_level: u32,
    pub sm0_rx_level: u32,
    pub sm1_tx_level: u32,
    pub sm1_rx_level: u32,
    pub pio_flevel_timeout: u32,
    pub pio_fstat_timeout: u32,
    pub pio_fdebug_timeout: u32,
    pub sm0_enabled_timeout: bool,
    pub sm1_enabled_timeout: bool,
    pub sm0_pc_timeout: u32,
    pub sm1_pc_timeout: u32,
    pub sm0_clkdiv_timeout: u32,
    pub sm1_clkdiv_timeout: u32,
    pub sm0_execctrl_timeout: u32,
    pub sm1_execctrl_timeout: u32,
    pub sm0_shiftctrl_timeout: u32,
    pub sm1_shiftctrl_timeout: u32,
    pub sm0_pinctrl_timeout: u32,
    pub sm1_pinctrl_timeout: u32,
    pub sm_enabled_before_tx: bool,
    pub sm_enabled_at_timeout: bool,
    pub sm_enabled_after_cleanup: bool,
    pub sm0_enabled_at_timeout: bool,
    pub sm1_enabled_at_timeout: bool,
    pub timeout_stage: u8, // 1=tx wait, 2=rx wait, 0=other
    pub verify_pass: Option<bool>,
    pub first_mismatch_offset: Option<u8>,
    pub first_mismatch_expected: Option<u8>,
    pub first_mismatch_actual: Option<u8>,
}

/// Configuration-state snapshot when dedicated SM1 prior-art setup is invalid.
#[derive(Debug, Clone, Copy)]
pub struct PriorArtConfigDiagnostic {
    pub pio_program_variant: &'static str,
    pub pio_instance: u8,
    pub sm_index: u8,
    pub program_offset: u8,
    pub sm0_enabled_before_tx: bool,
    pub sm1_enabled_before_tx: bool,
    pub sm0_enabled_after_cleanup: bool,
    pub sm1_enabled_after_cleanup: bool,
}

/// Diagnostic snapshot for unpaced 4-word DMA experiment.
#[derive(Debug, Clone, Copy)]
pub struct UnpacedDma4Diagnostic {
    pub dma_channel: u8,
    pub dreq_numeric_pio1_rx0: u8,
    pub dreq_ref_rp2040_pio1_rx0: u8,
    pub rx_level_before_unpaced: u32,
    pub unpaced_tc_before: u32,
    pub unpaced_tc_after: u32,
    pub unpaced_read_addr: u32,
    pub unpaced_write_addr: u32,
}

impl From<HalError> for DiagError {
    fn from(e: HalError) -> Self {
        match e {
            HalError::InvalidArgument => DiagError::InvalidArgument,
            HalError::Unsupported => DiagError::Unsupported,
            other => DiagError::Hal(other),
        }
    }
}

/// Diagnostic-only handle for a DMA-assisted PSRAM reader.
pub struct DmaPicoCalcPsram;

impl DmaPicoCalcPsram {
    /// Create the diagnostic handle.
    ///
    /// This is a no-op stub unless the `psram_pio_word_diag` feature is enabled.
    pub fn new() -> Result<Self, DiagError> {
        // Diagnostic implementation available when compiled with
        // `psram_pio_word_diag`; by default indicate Unavailable so production
        // paths are unaffected.
        Err(DiagError::Unavailable)
    }

    /// Diagnostic DMA-assisted read. Signature mirrors `PsramHal::read`.
    ///
    /// Behavior: when stubbed, returns `HalError::Unavailable`. Real
    /// diagnostic implementation must be gated behind `psram_pio_word_diag`.
    pub fn read_dma(&mut self, _address: u32, _dst: &mut [u8]) -> Result<(), DiagError> {
        Err(DiagError::Unavailable)
    }
}

/// Diagnostic entry point: perform a PIO-word-based 16B read using the PIO
/// RX FIFO word collection path and reassemble into `dst`.

/// Diagnostic entry point: read a full 256-byte PSRAM block using repeated
/// `pio_word16` operations. This is a diagnostic-only convenience that does
/// not use the DMA controller; it repeatedly issues 16B PIO reads and
/// concatenates the results into `dst`.
#[cfg(feature = "psram_pio_word_diag")]
pub fn diag_read_pio_block256(
    sm: &mut crate::psram::PicoCalcPsram<'_>,
    address: u32,
    dst: &mut [u8],
) -> Result<(), DiagError> {
    // Expect at least 256 bytes
    if dst.len() < 256 {
        return Err(DiagError::InvalidArgument);
    }
    // Read 16 chunks of 16 bytes each
    for chunk in 0..16usize {
        let chunk_addr = address + (chunk * 16) as u32;
        let dst_slice = &mut dst[chunk * 16..chunk * 16 + 16];
        // Use the existing pio-word 16B helper
        diag_impl::RealDmaPicoCalcPsram::read_pio_word16(sm, chunk_addr, dst_slice)?;
    }
    Ok(())
}

#[cfg(feature = "psram_pio_word_diag")]
pub fn diag_read_pio_word16(
    sm: &mut crate::psram::PicoCalcPsram<'_>,
    address: u32,
    dst: &mut [u8],
) -> Result<(), DiagError> {
    diag_impl::RealDmaPicoCalcPsram::read_pio_word16(sm, address, dst).map_err(Into::into)
}

/// Diagnostic entry point: perform a 16-byte read using byte-oriented DMA
/// from PIO1 RXF0 MMIO into `dst`.
#[cfg(feature = "psram_pio_word_diag")]
pub fn diag_read_dma16_v2(
    sm: &mut crate::psram::PicoCalcPsram<'_>,
    address: u32,
    dst: &mut [u8],
) -> Result<(), DiagError> {
    diag_impl::RealDmaPicoCalcPsram::read_dma16_v2(sm, address, dst)
}

/// Diagnostic entry point: dual-DMA 16-byte read.
///
/// - TX DMA: command bytes -> PIO1 TXF0 (8-bit, PIO1_TX0 paced)
/// - RX DMA: PIO1 RXF0 -> `[u8;16]` (8-bit, PIO1_RX0 paced)
#[cfg(feature = "psram_pio_word_diag")]
pub fn diag_read_dma16_dual(
    sm: &mut crate::psram::PicoCalcPsram<'_>,
    address: u32,
    dst: &mut [u8],
) -> Result<(), DiagError> {
    diag_impl::RealDmaPicoCalcPsram::read_dma16_dual(sm, address, dst)
}

/// Diagnostic entry point: faithful prior-art FIFO-byte dual DMA read for 16 bytes.
#[cfg(feature = "psram_pio_word_diag")]
pub fn diag_read_faithful_prior_art_dma16(
    sm: &mut crate::psram::PicoCalcPsram<'_>,
    address: u32,
    dst: &mut [u8],
) -> Result<FaithfulPriorArtReport, DiagError> {
    diag_impl::RealDmaPicoCalcPsram::read_faithful_prior_art_dma16(sm, address, dst)
}

/// Diagnostic entry point: prior-art command bytes pushed by CPU (8-bit writes)
/// while RX still uses byte DMA from PIO1 RXF1.
#[cfg(feature = "psram_pio_word_diag")]
pub fn diag_read_faithful_prior_art_cpu_tx_dma_rx16(
    sm: &mut crate::psram::PicoCalcPsram<'_>,
    address: u32,
    dst: &mut [u8],
) -> Result<FaithfulPriorArtReport, DiagError> {
    diag_impl::RealDmaPicoCalcPsram::read_faithful_prior_art_cpu_tx_dma_rx16(sm, address, dst)
}

/// Diagnostic entry point: run minimal SM1 FIFO/shift echo self-test.
#[cfg(feature = "psram_pio_word_diag")]
pub fn diag_run_fifo_echo8_sm1() -> Result<FifoEcho8Report, DiagError> {
    diag_impl::RealDmaPicoCalcPsram::run_fifo_echo8_sm1()
}

// Backward-compatible aliases used by existing diagnostics.
#[cfg(feature = "psram_pio_word_diag")]
pub fn diag_read_prior_art_dma16(
    sm: &mut crate::psram::PicoCalcPsram<'_>,
    address: u32,
    dst: &mut [u8],
) -> Result<FaithfulPriorArtReport, DiagError> {
    diag_read_faithful_prior_art_dma16(sm, address, dst)
}

#[cfg(feature = "psram_pio_word_diag")]
pub fn diag_read_prior_art_cpu_tx_dma_rx16(
    sm: &mut crate::psram::PicoCalcPsram<'_>,
    address: u32,
    dst: &mut [u8],
) -> Result<FaithfulPriorArtReport, DiagError> {
    diag_read_faithful_prior_art_cpu_tx_dma_rx16(sm, address, dst)
}

/// Backward-compatible alias used by older diagnostics.
#[cfg(feature = "psram_pio_word_diag")]
pub fn diag_read_dma16(
    sm: &mut crate::psram::PicoCalcPsram<'_>,
    address: u32,
    dst: &mut [u8],
) -> Result<(), DiagError> {
    diag_read_dma16_v2(sm, address, dst)
}

#[cfg(feature = "psram_pio_word_diag")]
pub fn diag_read_raw_rx_words(
    sm: &mut crate::psram::PicoCalcPsram<'_>,
    address: u32,
    raw: &mut [u32],
) -> Result<(), DiagError> {
    diag_impl::RealDmaPicoCalcPsram::read_raw_rx_words(sm, address, raw)
}

#[cfg(feature = "psram_pio_word_diag")]
pub fn diag_run_unpaced_dma4_from_rxf0(
    sm: &mut crate::psram::PicoCalcPsram<'_>,
    out4: &mut [u8; 4],
) -> Result<UnpacedDma4Diagnostic, DiagError> {
    diag_impl::RealDmaPicoCalcPsram::run_unpaced_dma4_from_rxf0(sm, out4)
}

#[cfg(feature = "psram_pio_word_diag")]
pub fn diag_dma_ch1_is_busy() -> bool {
    let dma = &embassy_rp::pac::DMA;
    dma.ch(1).ctrl_trig().read().busy()
}

/// Feature-gated reusable blocking PSRAM read API using verified SM1 RX-DMA path.
///
/// Constraints:
/// - Blocking read only
/// - CPU command TX only (no TX DMA)
/// - PIO1 SM1 + RX DMA only
/// - Supported/tested lengths: 16..=4096 bytes, 16-byte aligned
/// - Caller buffer must reside in DMA-safe SRAM
#[cfg(feature = "psram_dma_read_api")]
pub fn read_dma(
    sm: &mut crate::psram::PicoCalcPsram<'_>,
    address: u32,
    dst: &mut [u8],
) -> Result<(), PsramError> {
    let len = dst.len();
    if len < 16 || len > 4096 || (len & 0x0f) != 0 {
        return Err(PsramError::Hal(HalError::InvalidArgument));
    }
    let len_u32 = u32::try_from(len).map_err(|_| PsramError::OutOfRange)?;
    let end = address.checked_add(len_u32).ok_or(PsramError::OutOfRange)?;
    if end > crate::psram::PSRAM_CAPACITY {
        return Err(PsramError::OutOfRange);
    }
    diag_impl::RealDmaPicoCalcPsram::read_sm1_prodphase_cpu_tx_rx_dma(sm, address, dst).map_err(
        |e| match e {
            DiagError::InvalidArgument => PsramError::Hal(HalError::InvalidArgument),
            DiagError::Unavailable => PsramError::Unavailable,
            DiagError::Hal(inner) => PsramError::Hal(inner),
            _ => PsramError::Hal(HalError::Unsupported),
        },
    )
}

/// Feature-gated experimental CodeWindow read candidate using the validated
/// SM1 falling-edge+fudge program at clkdiv=3.000.
#[cfg(feature = "psram_dma_read_code_window")]
pub fn read_phase_edge_fudge_clkdiv3(
    sm: &mut crate::psram::PicoCalcPsram<'_>,
    address: u32,
    dst: &mut [u8],
) -> Result<(), PsramError> {
    let len = dst.len();
    // Alignment requirement stays 16B, but allow larger reads and let the
    // inner implementation split into bounded transactions.
    if len < 16 || (len & 0x0f) != 0 {
        return Err(PsramError::Hal(HalError::InvalidArgument));
    }
    let len_u32 = u32::try_from(len).map_err(|_| PsramError::OutOfRange)?;
    let end = address.checked_add(len_u32).ok_or(PsramError::OutOfRange)?;
    if end > crate::psram::PSRAM_CAPACITY {
        return Err(PsramError::OutOfRange);
    }
    diag_impl::RealDmaPicoCalcPsram::read_sm1_falling_edge_fudge_clkdiv3(sm, address, dst).map_err(
        |e| match e {
            DiagError::InvalidArgument => PsramError::Hal(HalError::InvalidArgument),
            DiagError::Unavailable => PsramError::Unavailable,
            DiagError::Hal(inner) => PsramError::Hal(inner),
            _ => PsramError::Hal(HalError::Unsupported),
        },
    )
}

#[cfg(any(feature = "psram_pio_word_diag", feature = "psram_dma_read_api"))]
mod diag_impl {
    use super::*;
    use crate::psram::PicoCalcPsram;
    use embassy_rp::pac;

    // Diagnostic implementation for feature-gated PSRAM read experiments.
    pub struct RealDmaPicoCalcPsram;

    impl RealDmaPicoCalcPsram {
        const DREQ_REF_RP2040_PIO1_RX0: u8 = 12;
        const PRIOR_ART_SM: u8 = 1;
        const PIO1_BASE: u32 = 0x4005_0000;
        const PRIOR_ART_PROGRAM_OFFSET: u8 = 16;
        #[cfg(feature = "psram_dma_read_code_window")]
        const PHASE_EDGE_FUDGE_PROGRAM_OFFSET: u8 = 23;
        const FIFO_ECHO_PROGRAM_OFFSET: u8 = 24;
        const PRIOR_ART_CLOCK_DIVIDER: u16 = 8;
        // `in_bits` is encoded in one byte (Y counter), so one transaction can
        // read at most 256 bits (= 32 bytes) with the current PIO program.
        const SM1_MAX_TXN_BYTES: usize = super::PSRAM_SM1_FUDGE_CLKDIV3_CHUNK_BYTES;
        const PIN_CS: u8 = crate::board::PICOCALC_WIRING.psram_cs;
        const PIN_MOSI: u8 = crate::board::PICOCALC_WIRING.psram_sio[0];
        const PIN_MISO: u8 = crate::board::PICOCALC_WIRING.psram_sio[1];
        const PIO_FDEBUG_OFFSET: u32 = 0x08;

        #[inline]
        fn dreq_numeric_pio1_rx0() -> u8 {
            use embassy_rp::pac::dma::vals::TreqSel;
            TreqSel::PIO1_RX0 as u8
        }

        #[inline]
        fn dreq_numeric_pio1_tx0() -> u8 {
            use embassy_rp::pac::dma::vals::TreqSel;
            TreqSel::PIO1_TX0 as u8
        }

        #[inline]
        fn dreq_numeric_pio1_rx1() -> u8 {
            use embassy_rp::pac::dma::vals::TreqSel;
            TreqSel::PIO1_RX1 as u8
        }

        #[inline]
        fn dreq_numeric_pio1_tx1() -> u8 {
            use embassy_rp::pac::dma::vals::TreqSel;
            TreqSel::PIO1_TX1 as u8
        }

        #[inline]
        fn mmio_write(addr: u32, value: u32) {
            // SAFETY: caller passes a valid MMIO register address.
            unsafe { core::ptr::write_volatile(addr as *mut u32, value) }
        }

        #[inline]
        fn relocate_instr(instr: u16, offset: u8) -> u16 {
            if instr & 0b1110_0000_0000_0000 == 0 {
                let address = (instr & 0b1_1111) as u8;
                let address = address.wrapping_add(offset) % 32;
                instr & !0b1_1111 | address as u16
            } else {
                instr
            }
        }

        fn faithful_prior_art_program() -> pio::Program<32> {
            let assembled = pio::pio_asm!(
                ".side_set 2",
                ".wrap_target",
                "begin:",
                "    out x, 8            side 0b01",
                "    out y, 8            side 0b01",
                "    jmp x--, writeloop  side 0b01",
                "writeloop:",
                "    out pins, 1         side 0b00",
                "    jmp x--, writeloop  side 0b10",
                "    jmp !y, begin       side 0b00",
                "readloop:",
                // On this hardware, production-phase read sampling matches the proven
                // Embassy SM1 comparison path; the prior-art phase shifts data by 1 bit.
                "    in pins, 1          side 0b10",
                "    jmp y--, readloop   side 0b00",
                ".wrap",
                options(max_program_size = 32)
            );
            assembled.program
        }

        #[cfg(feature = "psram_dma_read_code_window")]
        fn falling_edge_fudge_program() -> pio::Program<32> {
            let assembled = pio::pio_asm!(
                ".side_set 2",
                ".wrap_target",
                "begin:",
                "    out x, 8            side 0b01",
                "    out y, 16           side 0b01",
                "    jmp x--, writeloop  side 0b01",
                "writeloop:",
                "    out pins, 1         side 0b00",
                "    jmp x--, writeloop  side 0b10",
                "    jmp !y, begin       side 0b10",
                "    nop                 side 0b00",
                "readloop:",
                "    in pins, 1          side 0b10",
                "    jmp y--, readloop   side 0b00",
                ".wrap",
                options(max_program_size = 32)
            );
            assembled.program
        }

        fn configure_prior_art_sm1(wrap_source: u8, wrap_target: u8) {
            let pio1 = &pac::PIO1;
            let sm1 = pio1.sm(Self::PRIOR_ART_SM as usize);

            sm1.clkdiv().write(|w| {
                w.0 = (Self::PRIOR_ART_CLOCK_DIVIDER as u32) << 16;
            });
            sm1.execctrl().write(|w| {
                w.set_side_en(false);
                w.set_side_pindir(false);
                w.set_jmp_pin(0);
                w.set_out_en_sel(0);
                w.set_inline_out_en(false);
                w.set_out_sticky(false);
                w.set_wrap_top(wrap_source);
                w.set_wrap_bottom(wrap_target);
                w.set_status_sel(embassy_rp::pac::pio::vals::SmExecctrlStatusSel::TXLEVEL);
                w.set_status_n(0);
            });
            sm1.shiftctrl().write(|w| {
                w.set_fjoin_rx(false);
                w.set_fjoin_tx(false);
                w.set_pull_thresh(8);
                w.set_push_thresh(8);
                // Keep shift directions aligned with production SM0 while isolating
                // faithful diagnostics to TX/RX feeding behavior only.
                w.set_out_shiftdir(false);
                w.set_in_shiftdir(false);
                w.set_autopull(true);
                w.set_autopush(true);
            });
            sm1.pinctrl().write(|w| {
                w.set_sideset_count(2);
                w.set_set_count(0);
                w.set_out_count(1);
                w.set_in_base(Self::PIN_MISO);
                w.set_sideset_base(Self::PIN_CS);
                w.set_set_base(0);
                w.set_out_base(Self::PIN_MOSI);
            });
        }

        fn fifo_echo8_program() -> pio::Program<32> {
            let assembled = pio::pio_asm!(
                ".wrap_target",
                "    pull block",
                "    mov isr, osr",
                "    push block",
                ".wrap",
                options(max_program_size = 32)
            );
            assembled.program
        }

        fn configure_fifo_echo_sm1(wrap_source: u8, wrap_target: u8) {
            let pio1 = &pac::PIO1;
            let sm1 = pio1.sm(Self::PRIOR_ART_SM as usize);

            sm1.clkdiv().write(|w| {
                w.0 = (Self::PRIOR_ART_CLOCK_DIVIDER as u32) << 16;
            });
            sm1.execctrl().write(|w| {
                w.set_side_en(false);
                w.set_side_pindir(false);
                w.set_jmp_pin(0);
                w.set_out_en_sel(0);
                w.set_inline_out_en(false);
                w.set_out_sticky(false);
                w.set_wrap_top(wrap_source);
                w.set_wrap_bottom(wrap_target);
                w.set_status_sel(embassy_rp::pac::pio::vals::SmExecctrlStatusSel::TXLEVEL);
                w.set_status_n(0);
            });
            sm1.shiftctrl().write(|w| {
                w.set_fjoin_rx(false);
                w.set_fjoin_tx(false);
                w.set_pull_thresh(32);
                w.set_push_thresh(32);
                // Keep identical to production baseline for shift semantics.
                w.set_out_shiftdir(false);
                w.set_in_shiftdir(false);
                // Minimal echo program uses explicit PULL/PUSH instructions.
                w.set_autopull(false);
                w.set_autopush(false);
            });
            // No external pins are used by fifo_echo8.
            sm1.pinctrl().write(|w| {
                w.set_sideset_count(0);
                w.set_set_count(0);
                w.set_out_count(0);
                w.set_in_base(0);
                w.set_sideset_base(0);
                w.set_set_base(0);
                w.set_out_base(0);
            });
        }

        fn prepare_fifo_echo8_sm1() -> Result<(u8, bool), DiagError> {
            let pio1 = &pac::PIO1;
            let program = Self::fifo_echo8_program();
            let program_offset = Self::FIFO_ECHO_PROGRAM_OFFSET;

            let prog_len = program.code.len() as u8;
            if prog_len == 0 || program_offset.saturating_add(prog_len) > 32 {
                return Err(DiagError::Unsupported);
            }

            let sm_enable_before = pio1.ctrl().read().sm_enable();
            let sm0_was_enabled = (sm_enable_before & 0x01) != 0;

            pio1.ctrl().modify(|w| {
                w.set_sm_enable(w.sm_enable() & !((1u8 << 0) | (1u8 << Self::PRIOR_ART_SM)));
            });

            for (i, instr) in program.code.iter().enumerate() {
                let addr = (program_offset as usize + i) % 32;
                pio1.instr_mem(addr).write(|w| {
                    w.set_instr_mem(Self::relocate_instr(*instr, program_offset));
                });
            }

            let wrap_source = program.wrap.source.wrapping_add(program_offset) % 32;
            let wrap_target = program.wrap.target.wrapping_add(program_offset) % 32;
            Self::configure_fifo_echo_sm1(wrap_source, wrap_target);

            Ok((program_offset, sm0_was_enabled))
        }

        #[inline]
        fn decode_sm1_fdebug(fdebug: u32) -> (bool, bool, bool, bool) {
            let sm = Self::PRIOR_ART_SM as u32;
            let rxstall = (fdebug & (1u32 << sm)) != 0;
            let rxunder = (fdebug & (1u32 << (8 + sm))) != 0;
            let txover = (fdebug & (1u32 << (16 + sm))) != 0;
            let txstall = (fdebug & (1u32 << (24 + sm))) != 0;
            (txstall, rxstall, txover, rxunder)
        }

        #[inline]
        fn clear_fdebug_all() {
            // FDEBUG is write-1-to-clear for all sticky flags across all SMs.
            use embassy_rp::pac::pio::regs::Fdebug;
            pac::PIO1.fdebug().write_value(Fdebug(0xffff_ffff));
        }

        #[inline]
        fn clear_sm1_fifos() {
            let pio1 = &pac::PIO1;
            let sm1 = pio1.sm(Self::PRIOR_ART_SM as usize);
            let shift = sm1.shiftctrl().read();
            let orig_fjoin_rx = shift.fjoin_rx();
            let orig_fjoin_tx = shift.fjoin_tx();
            let pull_thresh = shift.pull_thresh();
            let push_thresh = shift.push_thresh();
            let out_shiftdir = shift.out_shiftdir();
            let in_shiftdir = shift.in_shiftdir();
            let autopull = shift.autopull();
            let autopush = shift.autopush();

            // RP2040 clears both FIFOs when FIFO-join mode is toggled.
            // Preserve all non-join fields explicitly when rewriting SHIFTCTRL.
            sm1.shiftctrl().write(|w| {
                w.set_fjoin_rx(!orig_fjoin_rx);
                w.set_fjoin_tx(orig_fjoin_tx);
                w.set_pull_thresh(pull_thresh);
                w.set_push_thresh(push_thresh);
                w.set_out_shiftdir(out_shiftdir);
                w.set_in_shiftdir(in_shiftdir);
                w.set_autopull(autopull);
                w.set_autopush(autopush);
            });
            sm1.shiftctrl().write(|w| {
                w.set_fjoin_rx(orig_fjoin_rx);
                w.set_fjoin_tx(orig_fjoin_tx);
                w.set_pull_thresh(pull_thresh);
                w.set_push_thresh(push_thresh);
                w.set_out_shiftdir(out_shiftdir);
                w.set_in_shiftdir(in_shiftdir);
                w.set_autopull(autopull);
                w.set_autopush(autopush);
            });
        }

        #[inline]
        fn snapshot_fifo_echo_variant(
            variant_name: &'static str,
            input_byte: u8,
            received_byte: Option<u8>,
            input_word: Option<u32>,
            received_word: Option<u32>,
            tx_ch: Option<&pac::dma::Channel>,
            rx_ch: Option<&pac::dma::Channel>,
        ) -> FifoEcho8VariantReport {
            let pio1 = &pac::PIO1;
            let flevel = pio1.flevel().read().0;
            let fstat = pio1.fstat().read().0;
            let fdebug = pio1.fdebug().read().0;
            let (txstall, rxstall, txover, rxunder) = Self::decode_sm1_fdebug(fdebug);
            let tx_ctrl = tx_ch.map(|ch| ch.ctrl_trig().read());
            let rx_ctrl = rx_ch.map(|ch| ch.ctrl_trig().read());
            let pass = if let Some(expected_word) = input_word {
                received_word == Some(expected_word)
            } else {
                received_byte == Some(input_byte)
            };
            FifoEcho8VariantReport {
                variant_name,
                input_byte,
                received_byte,
                input_word,
                received_word,
                pass,
                tx_dma_channel: tx_ch.map(|_| 0),
                rx_dma_channel: rx_ch.map(|_| 1),
                tx_remaining: tx_ch.map(|ch| ch.trans_count().read()),
                rx_remaining: rx_ch.map(|ch| ch.trans_count().read()),
                tx_busy: tx_ctrl.map(|ctrl| ctrl.busy()),
                rx_busy: rx_ctrl.map(|ctrl| ctrl.busy()),
                fifo_levels: flevel,
                fifo_stat: fstat,
                fdebug_raw: fdebug,
                fdebug_sm_txstall: txstall,
                fdebug_sm_rxstall: rxstall,
                fdebug_sm_txover: txover,
                fdebug_sm_rxunder: rxunder,
            }
        }

        fn poll_rxf1_byte(timeout_us: u32) -> Option<u8> {
            let pio1 = &pac::PIO1;
            let rxf1 = (Self::PIO1_BASE + 0x24) as *const u32;
            let mut elapsed = 0u32;
            while (pio1.fstat().read().0 & (1 << (8 + Self::PRIOR_ART_SM as u32))) != 0 {
                for _ in 0..10 {
                    core::hint::spin_loop();
                }
                elapsed = elapsed.saturating_add(1);
                if elapsed > timeout_us {
                    return None;
                }
            }
            // SAFETY: RXF1 MMIO read is valid and drains one queued word.
            let raw = unsafe { core::ptr::read_volatile(rxf1) };
            Some(raw as u8)
        }

        fn run_fifo_echo_case_cpu_u8(input_byte: u8, program_offset: u8) -> FifoEcho8VariantReport {
            Self::clear_fdebug_all();
            Self::restart_prior_art_sm1(program_offset);
            let txf1_u8 = (Self::PIO1_BASE + 0x14) as *mut u8;
            // SAFETY: TXF1 accepts MMIO byte writes in diagnostic mode.
            unsafe {
                core::ptr::write_volatile(txf1_u8, input_byte);
            }
            let received = Self::poll_rxf1_byte(2_000);
            Self::snapshot_fifo_echo_variant(
                "A_cpu_u8_tx_cpu_rx",
                input_byte,
                received,
                None,
                None,
                None,
                None,
            )
        }

        fn run_fifo_echo_case_packed(input_byte: u8, program_offset: u8) -> FifoEcho8VariantReport {
            Self::clear_fdebug_all();
            Self::restart_prior_art_sm1(program_offset);
            let txf1_u32 = (Self::PIO1_BASE + 0x14) as *mut u32;
            let packed = u32::from_le_bytes([input_byte, input_byte, input_byte, input_byte]);
            // SAFETY: TXF1 MMIO write with production-compatible packed lane.
            unsafe {
                core::ptr::write_volatile(txf1_u32, packed);
            }
            let received = Self::poll_rxf1_byte(2_000);
            Self::snapshot_fifo_echo_variant(
                "B_packed_tx_cpu_rx",
                input_byte,
                received,
                Some(packed),
                None,
                None,
                None,
            )
        }

        fn run_fifo_echo_case_cpu_u32(
            input_word: u32,
            program_offset: u8,
        ) -> FifoEcho8VariantReport {
            Self::clear_fdebug_all();
            Self::restart_prior_art_sm1(program_offset);

            let txf1_u32 = (Self::PIO1_BASE + 0x14) as *mut u32;
            let rxf1_u32 = (Self::PIO1_BASE + 0x24) as *const u32;

            // SAFETY: TXF1/RXF1 are valid MMIO registers for diagnostic access.
            unsafe {
                core::ptr::write_volatile(txf1_u32, input_word);
            }

            let pio1 = &pac::PIO1;
            let mut elapsed = 0u32;
            let mut received_word = None;
            while (pio1.fstat().read().0 & (1 << (8 + Self::PRIOR_ART_SM as u32))) != 0 {
                for _ in 0..10 {
                    core::hint::spin_loop();
                }
                elapsed = elapsed.saturating_add(1);
                if elapsed > 2_000 {
                    break;
                }
            }
            if (pio1.fstat().read().0 & (1 << (8 + Self::PRIOR_ART_SM as u32))) == 0 {
                // SAFETY: RXF1 MMIO read drains one queued word.
                received_word = Some(unsafe { core::ptr::read_volatile(rxf1_u32) });
            }

            let received_byte = received_word.map(|word| word as u8);
            Self::snapshot_fifo_echo_variant(
                "E_cpu_u32_tx_cpu_u32_rx",
                (input_word & 0xff) as u8,
                received_byte,
                Some(input_word),
                received_word,
                None,
                None,
            )
        }

        fn run_fifo_echo_case_cpu_u8_dma_rx(
            input_byte: u8,
            program_offset: u8,
        ) -> FifoEcho8VariantReport {
            use embassy_rp::pac::dma::regs::CtrlTrig;

            Self::clear_fdebug_all();
            Self::restart_prior_art_sm1(program_offset);

            let dma = &pac::DMA;
            let rx_ch = dma.ch(1);
            let rxf1 = Self::PIO1_BASE + 0x24;
            let mut out = [0u8; 1];

            rx_ch.ctrl_trig().write_value(CtrlTrig(0));
            rx_ch.read_addr().write_value(rxf1);
            rx_ch.write_addr().write_value(out.as_mut_ptr() as u32);
            rx_ch.trans_count().write_value(1);
            rx_ch.ctrl_trig().write(|w| {
                use embassy_rp::pac::dma::vals::{DataSize, TreqSel};
                w.set_data_size(DataSize::SIZE_BYTE);
                w.set_incr_read(false);
                w.set_incr_write(true);
                w.set_treq_sel(TreqSel::PIO1_RX1);
                w.set_irq_quiet(true);
                w.set_en(true);
            });

            let txf1_u8 = (Self::PIO1_BASE + 0x14) as *mut u8;
            // SAFETY: TXF1 accepts MMIO byte writes in diagnostic mode.
            unsafe {
                core::ptr::write_volatile(txf1_u8, input_byte);
            }

            let mut elapsed = 0u32;
            while rx_ch.ctrl_trig().read().busy() {
                for _ in 0..10 {
                    core::hint::spin_loop();
                }
                elapsed = elapsed.saturating_add(1);
                if elapsed > 2_000 {
                    break;
                }
            }
            let received = if rx_ch.trans_count().read() == 0 {
                Some(out[0])
            } else {
                None
            };
            let report = Self::snapshot_fifo_echo_variant(
                "C_cpu_u8_tx_dma_rx",
                input_byte,
                received,
                None,
                None,
                None,
                Some(&rx_ch),
            );
            rx_ch.ctrl_trig().write_value(CtrlTrig(0));
            report
        }

        fn run_fifo_echo_case_tx_dma_rx_dma(
            input_byte: u8,
            program_offset: u8,
        ) -> FifoEcho8VariantReport {
            use embassy_rp::pac::dma::regs::CtrlTrig;

            Self::clear_fdebug_all();
            Self::restart_prior_art_sm1(program_offset);

            let dma = &pac::DMA;
            let tx_ch = dma.ch(0);
            let rx_ch = dma.ch(1);
            let txf1 = Self::PIO1_BASE + 0x14;
            let rxf1 = Self::PIO1_BASE + 0x24;
            let tx_buf = [input_byte];
            let mut rx_buf = [0u8; 1];

            tx_ch.ctrl_trig().write_value(CtrlTrig(0));
            rx_ch.ctrl_trig().write_value(CtrlTrig(0));

            rx_ch.read_addr().write_value(rxf1);
            rx_ch.write_addr().write_value(rx_buf.as_mut_ptr() as u32);
            rx_ch.trans_count().write_value(1);
            rx_ch.ctrl_trig().write(|w| {
                use embassy_rp::pac::dma::vals::{DataSize, TreqSel};
                w.set_data_size(DataSize::SIZE_BYTE);
                w.set_incr_read(false);
                w.set_incr_write(true);
                w.set_treq_sel(TreqSel::PIO1_RX1);
                w.set_irq_quiet(true);
                w.set_en(true);
            });

            tx_ch.read_addr().write_value(tx_buf.as_ptr() as u32);
            tx_ch.write_addr().write_value(txf1);
            tx_ch.trans_count().write_value(1);
            tx_ch.ctrl_trig().write(|w| {
                use embassy_rp::pac::dma::vals::{DataSize, TreqSel};
                w.set_data_size(DataSize::SIZE_BYTE);
                w.set_incr_read(true);
                w.set_incr_write(false);
                w.set_treq_sel(TreqSel::PIO1_TX1);
                w.set_irq_quiet(true);
                w.set_en(true);
            });

            let mut elapsed = 0u32;
            while tx_ch.ctrl_trig().read().busy() || rx_ch.ctrl_trig().read().busy() {
                for _ in 0..10 {
                    core::hint::spin_loop();
                }
                elapsed = elapsed.saturating_add(1);
                if elapsed > 2_000 {
                    break;
                }
            }

            let received = if rx_ch.trans_count().read() == 0 {
                Some(rx_buf[0])
            } else {
                None
            };
            let report = Self::snapshot_fifo_echo_variant(
                "D_tx_dma_rx_dma",
                input_byte,
                received,
                None,
                None,
                Some(&tx_ch),
                Some(&rx_ch),
            );
            tx_ch.ctrl_trig().write_value(CtrlTrig(0));
            rx_ch.ctrl_trig().write_value(CtrlTrig(0));
            report
        }

        pub fn run_fifo_echo8_sm1() -> Result<FifoEcho8Report, DiagError> {
            let (program_offset, sm0_was_enabled) = Self::prepare_fifo_echo8_sm1()?;
            let pio1 = &pac::PIO1;

            Self::clear_fdebug_all();

            let a = Self::run_fifo_echo_case_cpu_u8(0xA5, program_offset);
            let b = Self::run_fifo_echo_case_packed(0x3C, program_offset);
            let c = Self::run_fifo_echo_case_cpu_u8_dma_rx(0x5A, program_offset);
            let d = Self::run_fifo_echo_case_tx_dma_rx_dma(0xC3, program_offset);
            let e = Self::run_fifo_echo_case_cpu_u32(0xDEAD_BEEF, program_offset);

            let sm1 = pio1.sm(Self::PRIOR_ART_SM as usize);
            let report = FifoEcho8Report {
                pio_variant: "fifo_echo8_sm1",
                pio_instance: 1,
                sm_index: Self::PRIOR_ART_SM,
                program_offset,
                shiftctrl: sm1.shiftctrl().read().0,
                execctrl: sm1.execctrl().read().0,
                pinctrl: sm1.pinctrl().read().0,
                sm_enable_bits: pio1.ctrl().read().sm_enable(),
                sm_pc: sm1.addr().read().0,
                case_a_cpu_u8_tx_cpu_rx: a,
                case_b_packed_tx_cpu_rx: b,
                case_c_cpu_u8_tx_dma_rx: c,
                case_d_tx_dma_rx_dma: Some(d),
                case_e_cpu_u32_tx_cpu_u32_rx: e,
                pass: a.pass && b.pass && c.pass && d.pass && e.pass,
            };

            Self::teardown_prior_art_dedicated_sm(sm0_was_enabled);
            Ok(report)
        }

        fn prepare_prior_art_dedicated_sm() -> Result<(u8, bool), DiagError> {
            let pio1 = &pac::PIO1;
            let program = Self::faithful_prior_art_program();
            let program_offset = Self::PRIOR_ART_PROGRAM_OFFSET;

            let prog_len = program.code.len() as u8;
            if prog_len == 0 || program_offset.saturating_add(prog_len) > 32 {
                return Err(DiagError::Unsupported);
            }

            let sm_enable_before = pio1.ctrl().read().sm_enable();
            let sm0_was_enabled = (sm_enable_before & 0x01) != 0;

            // Isolate pins by disabling SM0/SM1 while preparing diagnostic SM1.
            pio1.ctrl().modify(|w| {
                w.set_sm_enable(w.sm_enable() & !((1u8 << 0) | (1u8 << Self::PRIOR_ART_SM)));
            });

            for (i, instr) in program.code.iter().enumerate() {
                let addr = (program_offset as usize + i) % 32;
                pio1.instr_mem(addr).write(|w| {
                    w.set_instr_mem(Self::relocate_instr(*instr, program_offset));
                });
            }

            let wrap_source = program.wrap.source.wrapping_add(program_offset) % 32;
            let wrap_target = program.wrap.target.wrapping_add(program_offset) % 32;
            Self::configure_prior_art_sm1(wrap_source, wrap_target);

            // Bypass input synchronizer for MISO for faithful prior-art behavior.
            let bypass = pio1.input_sync_bypass().read();
            pio1.input_sync_bypass()
                .write(|w| *w = bypass | (1u32 << Self::PIN_MISO));

            Ok((program_offset, sm0_was_enabled))
        }

        #[cfg(feature = "psram_dma_read_code_window")]
        fn prepare_phase_edge_fudge_dedicated_sm() -> Result<(u8, bool), DiagError> {
            let pio1 = &pac::PIO1;
            let program = Self::falling_edge_fudge_program();
            let program_offset = Self::PHASE_EDGE_FUDGE_PROGRAM_OFFSET;

            let prog_len = program.code.len() as u8;
            if prog_len == 0 || program_offset.saturating_add(prog_len) > 32 {
                return Err(DiagError::Unsupported);
            }

            let sm_enable_before = pio1.ctrl().read().sm_enable();
            let sm0_was_enabled = (sm_enable_before & 0x01) != 0;

            pio1.ctrl().modify(|w| {
                w.set_sm_enable(w.sm_enable() & !((1u8 << 0) | (1u8 << Self::PRIOR_ART_SM)));
            });

            for (i, instr) in program.code.iter().enumerate() {
                let addr = (program_offset as usize + i) % 32;
                pio1.instr_mem(addr).write(|w| {
                    w.set_instr_mem(Self::relocate_instr(*instr, program_offset));
                });
            }

            let wrap_source = program.wrap.source.wrapping_add(program_offset) % 32;
            let wrap_target = program.wrap.target.wrapping_add(program_offset) % 32;
            Self::configure_prior_art_sm1(wrap_source, wrap_target);

            let bypass = pio1.input_sync_bypass().read();
            pio1.input_sync_bypass()
                .write(|w| *w = bypass | (1u32 << Self::PIN_MISO));

            Ok((program_offset, sm0_was_enabled))
        }

        #[inline]
        fn restart_prior_art_sm1(program_offset: u8) {
            let pio1 = &pac::PIO1;
            let sm1_mask = 1u8 << Self::PRIOR_ART_SM;

            // Keep SM1 disabled while resetting runtime state.
            pio1.ctrl().modify(|w| {
                w.set_sm_enable(w.sm_enable() & !sm1_mask);
            });

            // Clear sticky debug flags for SM1: RXUNDER/TXOVER/RXSTALL/TXSTALL.
            let sm = Self::PRIOR_ART_SM as u32;
            let fdebug_clear_mask =
                (1u32 << sm) | (1u32 << (8 + sm)) | (1u32 << (16 + sm)) | (1u32 << (24 + sm));
            Self::mmio_write(Self::PIO1_BASE + Self::PIO_FDEBUG_OFFSET, fdebug_clear_mask);

            // Drain residual RX words from SM1 FIFO.
            let rxf1 = Self::PIO1_BASE + 0x24;
            let mut drain_reads = 0u32;
            while (pio1.fstat().read().0 & (1 << (8 + Self::PRIOR_ART_SM as u32))) == 0 {
                // SAFETY: RXF1 MMIO read drains one queued FIFO word.
                unsafe {
                    core::ptr::read_volatile(rxf1 as *const u32);
                }
                drain_reads = drain_reads.saturating_add(1);
                if drain_reads > 1024 {
                    break;
                }
            }

            // Ensure both TX/RX FIFOs are reset for deterministic next transaction.
            Self::clear_sm1_fifos();

            // Restart SM1 internals and its clock divider phase.
            pio1.ctrl().modify(|w| {
                w.set_sm_restart(sm1_mask);
            });
            pio1.ctrl().modify(|w| {
                w.set_clkdiv_restart(sm1_mask);
            });

            // SMx_ADDR is read-only on RP2040; set PC by forcing a JMP instruction.
            let sm1 = pio1.sm(Self::PRIOR_ART_SM as usize);
            sm1.instr().write(|w| {
                // JMP always, <addr> encodes as 0b000_000_000_<addr[4:0]>
                w.set_instr((program_offset & 0x1f) as u16);
            });

            pio1.ctrl().modify(|w| {
                w.set_sm_enable(w.sm_enable() | sm1_mask);
            });

            // Some PAC/SM timing combinations may ignore the forced instruction while
            // disabled. Re-issue the forced JMP after enable to guarantee PC landing.
            sm1.instr().write(|w| {
                w.set_instr((program_offset & 0x1f) as u16);
            });
        }

        #[inline]
        fn teardown_prior_art_dedicated_sm(sm0_was_enabled: bool) {
            let pio1 = &pac::PIO1;
            pio1.ctrl().modify(|w| {
                w.set_sm_enable(w.sm_enable() & !(1u8 << Self::PRIOR_ART_SM));
            });
            if sm0_was_enabled {
                pio1.ctrl().modify(|w| {
                    w.set_sm_enable(w.sm_enable() | (1u8 << 0));
                });
            } else {
                pio1.ctrl().modify(|w| {
                    w.set_sm_enable(w.sm_enable() & !(1u8 << 0));
                });
            }
            // Drain any residual RX entries from dedicated SM1.
            while (pio1.fstat().read().0 & (1 << (8 + Self::PRIOR_ART_SM as u32))) == 0 {
                // SAFETY: reading RXF1 drains one queued FIFO word.
                unsafe {
                    core::ptr::read_volatile((Self::PIO1_BASE + 0x24) as *const u32);
                }
            }
        }

        #[inline]
        fn prior_art_enable_state(sm_enable: u8) -> (bool, bool) {
            (
                (sm_enable & (1u8 << 0)) != 0,
                (sm_enable & (1u8 << Self::PRIOR_ART_SM)) != 0,
            )
        }

        #[inline]
        fn snapshot_prior_art_config_error(
            pio_program_variant: &'static str,
            program_offset: u8,
            sm0_enabled_before_tx: bool,
            sm1_enabled_before_tx: bool,
            sm0_was_enabled: bool,
        ) -> PriorArtConfigDiagnostic {
            Self::teardown_prior_art_dedicated_sm(sm0_was_enabled);
            let ctrl_after = pac::PIO1.ctrl().read().sm_enable();
            let (sm0_enabled_after_cleanup, sm1_enabled_after_cleanup) =
                Self::prior_art_enable_state(ctrl_after);
            PriorArtConfigDiagnostic {
                pio_program_variant,
                pio_instance: 1,
                sm_index: Self::PRIOR_ART_SM,
                program_offset,
                sm0_enabled_before_tx,
                sm1_enabled_before_tx,
                sm0_enabled_after_cleanup,
                sm1_enabled_after_cleanup,
            }
        }

        #[inline]
        fn snapshot_faithful_prior_art_failure(
            variant_name: &'static str,
            tx_ch: Option<&pac::dma::Channel>,
            rx_ch: &pac::dma::Channel,
            pio1: &pac::pio::Pio,
            sm_index: u8,
            program_offset: u8,
            tx_dreq_numeric: Option<u8>,
            rx_dreq_numeric: Option<u8>,
            tx_src_addr: Option<u32>,
            tx_dst_addr: Option<u32>,
            rx_src_addr: u32,
            rx_dst_addr: u32,
            cmd: [u8; 7],
            first_16_output_bytes: [u8; 16],
            sm_enabled_before_tx: bool,
            timeout_stage: u8,
        ) -> FaithfulPriorArtDiagnostic {
            let tx_ctrl = tx_ch.map(|ch| ch.ctrl_trig().read().0);
            let rx_ctrl = rx_ch.ctrl_trig().read().0;
            let flevel = pio1.flevel().read().0;
            let sm_enable = pio1.ctrl().read().sm_enable();
            let (sm0_enabled_timeout, sm1_enabled_timeout) =
                Self::prior_art_enable_state(sm_enable);
            let sm0 = pio1.sm(0);
            let sm1 = pio1.sm(Self::PRIOR_ART_SM as usize);
            FaithfulPriorArtDiagnostic {
                variant_name,
                command_bytes: cmd,
                first_16_output_bytes,
                pio_instance: 1,
                sm_index,
                program_offset,
                tx_dma_channel: tx_ch.map(|_| 0),
                rx_dma_channel: Some(1),
                data_size_bits: 8,
                tx_transfer_count: tx_ch.map(|_| 7),
                rx_transfer_count: Some(16),
                tx_remaining: tx_ch.map(|ch| ch.trans_count().read()),
                rx_remaining: Some(rx_ch.trans_count().read()),
                tx_dreq_numeric,
                rx_dreq_numeric,
                tx_src_addr,
                tx_dst_addr,
                rx_src_addr: Some(rx_src_addr),
                rx_dst_addr: Some(rx_dst_addr),
                tx_read_addr: tx_ch.map(|ch| ch.read_addr().read()),
                tx_write_addr: tx_ch.map(|ch| ch.write_addr().read()),
                rx_read_addr: Some(rx_ch.read_addr().read()),
                rx_write_addr: Some(rx_ch.write_addr().read()),
                tx_ctrl,
                rx_ctrl: Some(rx_ctrl),
                tx_busy: tx_ctrl.map(|ctrl| (ctrl & (1 << 24)) != 0),
                tx_en: tx_ctrl.map(|ctrl| (ctrl & (1 << 0)) != 0),
                tx_ahb_error: tx_ctrl.map(|ctrl| (ctrl & (1 << 31)) != 0),
                rx_busy: Some((rx_ctrl & (1 << 24)) != 0),
                rx_en: Some((rx_ctrl & (1 << 0)) != 0),
                rx_ahb_error: Some((rx_ctrl & (1 << 31)) != 0),
                sm0_tx_level: flevel & 0x0f,
                sm0_rx_level: (flevel >> 4) & 0x0f,
                sm1_tx_level: (flevel >> 8) & 0x0f,
                sm1_rx_level: (flevel >> 12) & 0x0f,
                pio_flevel_timeout: flevel,
                pio_fstat_timeout: pio1.fstat().read().0,
                pio_fdebug_timeout: pio1.fdebug().read().0,
                sm0_enabled_timeout,
                sm1_enabled_timeout,
                sm0_pc_timeout: sm0.addr().read().0,
                sm1_pc_timeout: sm1.addr().read().0,
                sm0_clkdiv_timeout: sm0.clkdiv().read().0,
                sm1_clkdiv_timeout: sm1.clkdiv().read().0,
                sm0_execctrl_timeout: sm0.execctrl().read().0,
                sm1_execctrl_timeout: sm1.execctrl().read().0,
                sm0_shiftctrl_timeout: sm0.shiftctrl().read().0,
                sm1_shiftctrl_timeout: sm1.shiftctrl().read().0,
                sm0_pinctrl_timeout: sm0.pinctrl().read().0,
                sm1_pinctrl_timeout: sm1.pinctrl().read().0,
                sm_enabled_before_tx,
                sm_enabled_at_timeout: sm1_enabled_timeout,
                sm_enabled_after_cleanup: false,
                sm0_enabled_at_timeout: sm0_enabled_timeout,
                sm1_enabled_at_timeout: sm1_enabled_timeout,
                timeout_stage,
                verify_pass: None,
                first_mismatch_offset: None,
                first_mismatch_expected: None,
                first_mismatch_actual: None,
            }
        }

        #[inline]
        fn pio_sm0_levels(flevel: u32) -> (u32, u32) {
            // RP2040 PIO FLEVEL layout:
            // TX0[3:0], RX0[7:4], TX1[11:8], RX1[15:12], ...
            let tx0 = flevel & 0x0f;
            let rx0 = (flevel >> 4) & 0x0f;
            (tx0, rx0)
        }

        #[inline]
        fn pio_sm0_empty_flags(fstat: u32) -> (bool, bool) {
            // RP2040 PIO FSTAT layout includes one bit per SM for TXEMPTY/RXEMPTY.
            // We only need SM0; keep it robust by checking bit0 for TXEMPTY and bit8 for RXEMPTY.
            let tx_empty = (fstat & (1 << 0)) != 0;
            let rx_empty = (fstat & (1 << 8)) != 0;
            (tx_empty, rx_empty)
        }

        #[inline]
        fn decode_ctrl(ctrl: u32, diag: &mut Dma16Diagnostic) {
            diag.ctrl_en_timeout = (ctrl & (1 << 0)) != 0;
            diag.ctrl_data_size_timeout = ((ctrl >> 2) & 0x3) as u8;
            diag.ctrl_incr_read_timeout = (ctrl & (1 << 4)) != 0;
            diag.ctrl_incr_write_timeout = (ctrl & (1 << 5)) != 0;
            diag.ctrl_ring_size_timeout = ((ctrl >> 6) & 0xf) as u8;
            diag.ctrl_ring_sel_timeout = (ctrl & (1 << 10)) != 0;
            diag.ctrl_chain_to_timeout = ((ctrl >> 11) & 0xf) as u8;
            diag.ctrl_treq_sel_timeout = ((ctrl >> 15) & 0x3f) as u8;
            diag.ctrl_irq_quiet_timeout = (ctrl & (1 << 21)) != 0;
            diag.ctrl_sniff_en_timeout = (ctrl & (1 << 23)) != 0;
            diag.ctrl_busy_timeout = (ctrl & (1 << 24)) != 0;
            diag.ctrl_write_error_timeout = (ctrl & (1 << 29)) != 0;
            diag.ctrl_read_error_timeout = (ctrl & (1 << 30)) != 0;
            diag.ctrl_ahb_error_timeout = (ctrl & (1 << 31)) != 0;
        }

        #[allow(dead_code)]
        pub fn new() -> Result<DmaPicoCalcPsram, DiagError> {
            // For the shim we don't claim DMA resources. Signal that a
            // diagnostic implementation is available only when the feature
            // is compiled; otherwise callers should rely on the outer
            // `DmaPicoCalcPsram::new()` behavior.
            Ok(DmaPicoCalcPsram)
        }
        pub fn read_pio_word16(
            sm: &mut PicoCalcPsram<'_>,
            address: u32,
            dst: &mut [u8],
        ) -> Result<(), DiagError> {
            if dst.len() < 16 {
                return Err(DiagError::InvalidArgument);
            }
            // words buffer: collect 16 RX FIFO entries (one per byte)
            let mut words = [0u32; 16];
            sm.diag_read_words_32(address, &mut words)
                .map_err(DiagError::from)?;

            // Reconstruct bytes using the exact same rule as `pull_byte()`:
            // `pull_byte()` does `self.sm.rx().pull() as u8`, which takes the
            // least-significant byte of the pulled u32.
            for i in 0..16usize {
                dst[i] = words[i] as u8;
            }
            Ok(())
        }

        /// Diagnostic helper: return the raw RX FIFO words collected for a
        /// 16-byte read. This is useful for logging when verification fails.
        pub fn read_raw_rx_words(
            sm: &mut PicoCalcPsram<'_>,
            address: u32,
            raw: &mut [u32],
        ) -> Result<(), DiagError> {
            if raw.len() == 0 || raw.len() > 16 {
                return Err(DiagError::InvalidArgument);
            }
            sm.diag_read_words_32(address, raw).map_err(DiagError::from)
        }

        /// Real DMA CH1 read implementation for 16 bytes (byte-oriented).
        pub fn read_dma16_v2(
            sm: &mut PicoCalcPsram<'_>,
            address: u32,
            dst: &mut [u8],
        ) -> Result<(), DiagError> {
            if dst.len() < 16 {
                return Err(DiagError::InvalidArgument);
            }

            // DMA destination is a byte buffer as required by prior-art aligned v2.
            let dst_buf_ptr = dst.as_mut_ptr() as u32;

            // Get the PIO1 RX FIFO register address.
            // The PIO1 peripheral base is 0x40050000.
            // The RXF0 register is at offset 0x20 from the PIO base.
            let pio1 = &pac::PIO1;
            let pio1_base = 0x4005_0000u32;
            let rxf_offset = 0x20u32;
            let pio_rx_fifo_addr = pio1_base + rxf_offset;

            // Validate that source address is in the peripheral address space (0x40000000-0x50000000)
            // and NOT in SRAM (0x20000000-0x20042000). This catches the bug where we take
            // the address of the PAC proxy object instead of the actual register.
            debug_assert!(
                pio_rx_fifo_addr >= 0x40000000 && pio_rx_fifo_addr < 0x50000000,
                "DMA source address must be in peripheral space, got 0x{:08x}",
                pio_rx_fifo_addr
            );

            // Initialize diagnostic snapshot
            let mut diag = Dma16Diagnostic {
                pio_instance: 1,
                sm_index: 0,
                dma_channel: 1,
                dreq_numeric_pio1_rx0: Self::dreq_numeric_pio1_rx0(),
                src_addr_intended: pio_rx_fifo_addr,
                src_addr_written: 0,
                dst_addr: dst_buf_ptr,
                trans_count_before: 0,
                trans_count_after_arm: 0,
                trans_count_after_cmd: 0,
                trans_count_timeout: 0,
                ctrl_before_arm: 0,
                ctrl_after_arm: 0,
                ctrl_after_cmd: 0,
                ctrl_on_timeout: 0,
                ctrl_en_timeout: false,
                ctrl_busy_timeout: false,
                ctrl_irq_quiet_timeout: false,
                ctrl_read_error_timeout: false,
                ctrl_write_error_timeout: false,
                ctrl_ahb_error_timeout: false,
                ctrl_data_size_timeout: 0,
                ctrl_incr_read_timeout: false,
                ctrl_incr_write_timeout: false,
                ctrl_ring_size_timeout: 0,
                ctrl_ring_sel_timeout: false,
                ctrl_chain_to_timeout: 0,
                ctrl_treq_sel_timeout: 0,
                ctrl_sniff_en_timeout: false,
                read_addr_timeout: 0,
                write_addr_timeout: 0,
                dma_intr_timeout: 0,
                dma_inte0_timeout: 0,
                dma_ints0_timeout: 0,
                dma_ch1_irq_pending_timeout: false,
                timeout_snapshot_before_abort: false,
                paced_dma_aborted_before_return: false,
                ctrl_after_abort: 0,
                pio_rx_level_before: 0,
                pio_rx_level_after_cmd: 0,
                pio_rx_level_timeout: 0,
                pio_tx_level_before: 0,
                pio_tx_level_after_cmd: 0,
                pio_tx_level_timeout: 0,
                pio_flevel_before: 0,
                pio_flevel_after_cmd: 0,
                pio_flevel_timeout: 0,
                pio_fstat_timeout: 0,
                pio_fdebug_timeout: 0,
                sm0_enabled_timeout: false,
                sm0_tx_empty_timeout: false,
                sm0_rx_empty_timeout: false,
                sm0_shiftctrl_timeout: 0,
                sm0_execctrl_timeout: 0,
                ahb_error: false,
                timed_out: false,
            };

            // Set up DMA CH1.
            let dma = &pac::DMA;
            let ch = dma.ch(1);

            // Capture pre-ARM state
            diag.ctrl_before_arm = ch.ctrl_trig().read().0;
            diag.trans_count_before = ch.trans_count().read();
            diag.pio_flevel_before = pio1.flevel().read().0;
            let (tx_before, rx_before) = Self::pio_sm0_levels(diag.pio_flevel_before);
            diag.pio_tx_level_before = tx_before;
            diag.pio_rx_level_before = rx_before;

            // Clear any pending transfer-complete flag and AHB errors.
            use embassy_rp::pac::dma::regs::CtrlTrig;
            ch.ctrl_trig().write_value(CtrlTrig(0));

            // Configure channel: read from PIO1 RX (fixed), write to SRAM (increment).
            ch.read_addr().write_value(pio_rx_fifo_addr);
            diag.src_addr_written = ch.read_addr().read();
            ch.write_addr().write_value(dst_buf_ptr);
            ch.trans_count().write_value(16); // 16 bytes

            // Catch accidental SRAM source addresses in debug builds.
            // RP2040 SRAM window is 0x2000_0000..0x2004_2000.
            debug_assert!(
                !(diag.src_addr_written >= 0x2000_0000 && diag.src_addr_written < 0x2004_2000),
                "DMA source address must not be SRAM: 0x{:08x}",
                diag.src_addr_written
            );

            // Configure and ARM the DMA channel
            ch.ctrl_trig().write(|w| {
                use embassy_rp::pac::dma::vals::{DataSize, TreqSel};
                w.set_data_size(DataSize::SIZE_BYTE);
                w.set_incr_read(false); // PIO RX FIFO address is fixed
                w.set_incr_write(true); // Increment SRAM write address
                w.set_treq_sel(TreqSel::PIO1_RX0); // PIO1 SM0 RX DREQ
                w.set_irq_quiet(true); // Don't raise IRQ on completion
                w.set_en(true); // Enable and start
            });

            // Capture post-ARM state
            diag.ctrl_after_arm = ch.ctrl_trig().read().0;
            diag.trans_count_after_arm = ch.trans_count().read();

            // Push the 16-byte read command to PIO TX FIFO (same layout as production).
            for byte in &[
                40u8,
                (16 * 8 - 1) as u8,
                0x0b,
                (address >> 16) as u8,
                (address >> 8) as u8,
                address as u8,
                0,
            ] {
                sm.push_byte_internal(*byte);
            }

            // Capture post-command state
            diag.ctrl_after_cmd = ch.ctrl_trig().read().0;
            diag.trans_count_after_cmd = ch.trans_count().read();
            diag.pio_flevel_after_cmd = pio1.flevel().read().0;
            let (tx_after_cmd, rx_after_cmd) = Self::pio_sm0_levels(diag.pio_flevel_after_cmd);
            diag.pio_tx_level_after_cmd = tx_after_cmd;
            diag.pio_rx_level_after_cmd = rx_after_cmd;

            // Poll for DMA completion with a 10ms timeout.
            let timeout_us = 10_000u32;
            let mut elapsed_us = 0u32;
            let mut first_busy_seen = false;

            loop {
                let ctrl = ch.ctrl_trig().read();
                let is_busy = ctrl.busy();

                if is_busy {
                    first_busy_seen = true;
                }

                if !is_busy {
                    // Transfer complete or never started. Check for AHB errors.
                    if ctrl.ahb_error() {
                        return Err(DiagError::DmaAhbError);
                    }

                    // If we never saw busy, DMA may not have started.
                    if !first_busy_seen {
                        // DMA never started—likely TREQ/SM issue
                        return Err(DiagError::Unsupported);
                    }

                    break;
                }

                // Small delay: ~1 microsecond busy-wait per iteration.
                for _ in 0..10 {
                    core::hint::spin_loop();
                }
                elapsed_us = elapsed_us.saturating_add(1);

                if elapsed_us > timeout_us {
                    // Timeout: capture final state first, then abort DMA.
                    diag.timeout_snapshot_before_abort = true;
                    diag.ctrl_on_timeout = ch.ctrl_trig().read().0;
                    Self::decode_ctrl(diag.ctrl_on_timeout, &mut diag);
                    diag.trans_count_timeout = ch.trans_count().read();
                    diag.read_addr_timeout = ch.read_addr().read();
                    diag.write_addr_timeout = ch.write_addr().read();
                    // Global DMA interrupt/enable/status snapshots (proc0 view).
                    diag.dma_intr_timeout = dma.intr(0).read();
                    diag.dma_inte0_timeout = dma.inte(0).read();
                    diag.dma_ints0_timeout = dma.ints(0).read();
                    diag.dma_ch1_irq_pending_timeout = (diag.dma_intr_timeout & (1 << 1)) != 0;
                    diag.pio_flevel_timeout = pio1.flevel().read().0;
                    let (tx_timeout, rx_timeout) = Self::pio_sm0_levels(diag.pio_flevel_timeout);
                    diag.pio_tx_level_timeout = tx_timeout;
                    diag.pio_rx_level_timeout = rx_timeout;
                    diag.pio_fstat_timeout = pio1.fstat().read().0;
                    diag.pio_fdebug_timeout = pio1.fdebug().read().0;
                    let (tx_empty, rx_empty) = Self::pio_sm0_empty_flags(diag.pio_fstat_timeout);
                    diag.sm0_tx_empty_timeout = tx_empty;
                    diag.sm0_rx_empty_timeout = rx_empty;

                    // CTRL bit N enables SMN on RP2040.
                    let pio_ctrl = pio1.ctrl().read().0;
                    diag.sm0_enabled_timeout = (pio_ctrl & 0x1) != 0;
                    diag.sm0_shiftctrl_timeout = pio1.sm(0).shiftctrl().read().0;
                    diag.sm0_execctrl_timeout = pio1.sm(0).execctrl().read().0;
                    diag.timed_out = true;

                    ch.ctrl_trig().write_value(CtrlTrig(0));
                    diag.ctrl_after_abort = ch.ctrl_trig().read().0;
                    diag.paced_dma_aborted_before_return = true;
                    return Err(DiagError::DmaTimeout(Some(diag)));
                }
            }

            Ok(())
        }

        #[cfg(feature = "psram_dma_read_api")]
        #[inline]
        fn sm1_tx_level() -> u32 {
            (pac::PIO1.flevel().read().0 >> 8) & 0x0f
        }

        #[cfg(feature = "psram_dma_read_api")]
        #[inline]
        fn sm1_push_packed_byte(byte: u8) {
            let txf1_u32 = (Self::PIO1_BASE + 0x14) as *mut u32;
            let word = u32::from_be_bytes([byte, 0, 0, 0]);
            while Self::sm1_tx_level() >= 4 {
                for _ in 0..10 {
                    core::hint::spin_loop();
                }
            }
            // SAFETY: TXF1 is a valid MMIO target for 32-bit packed writes.
            unsafe {
                core::ptr::write_volatile(txf1_u32, word);
            }
        }

        #[cfg(feature = "psram_dma_read_code_window")]
        #[inline]
        fn sm1_push_packed_u16(val: u16) {
            let txf1_u32 = (Self::PIO1_BASE + 0x14) as *mut u32;
            // Top 16 bits carry the value; `out y, 16` in the fudge PIO program reads
            // the MSB half of the OSR word, so the u16 must be left-justified.
            let word = (val as u32) << 16;
            while Self::sm1_tx_level() >= 4 {
                for _ in 0..10 {
                    core::hint::spin_loop();
                }
            }
            // SAFETY: TXF1 is a valid MMIO target for 32-bit packed writes.
            unsafe {
                core::ptr::write_volatile(txf1_u32, word);
            }
        }

        #[cfg(feature = "psram_dma_read_api")]
        fn read_sm1_prodphase_chunk(
            address: u32,
            dst: &mut [u8],
            program_offset: u8,
        ) -> Result<(), DiagError> {
            if dst.is_empty() || dst.len() > Self::SM1_MAX_TXN_BYTES {
                return Err(DiagError::InvalidArgument);
            }

            Self::restart_prior_art_sm1(program_offset);

            let dma = &pac::DMA;
            let rx_ch = dma.ch(1);
            let rxf1 = Self::PIO1_BASE + 0x24;

            let dst_ptr = dst.as_mut_ptr() as u32;
            // RP2040 SRAM window guard: DMA write destination must be SRAM.
            if !(0x2000_0000..0x2004_2000).contains(&dst_ptr) {
                return Err(DiagError::InvalidArgument);
            }

            use embassy_rp::pac::dma::regs::CtrlTrig;
            rx_ch.ctrl_trig().write_value(CtrlTrig(0));

            rx_ch.read_addr().write_value(rxf1);
            rx_ch.write_addr().write_value(dst_ptr);
            rx_ch.trans_count().write_value(dst.len() as u32);
            rx_ch.ctrl_trig().write(|w| {
                use embassy_rp::pac::dma::vals::{DataSize, TreqSel};
                w.set_data_size(DataSize::SIZE_BYTE);
                w.set_incr_read(false);
                w.set_incr_write(true);
                w.set_treq_sel(TreqSel::PIO1_RX1);
                w.set_irq_quiet(true);
                w.set_en(true);
            });

            let cmd = [
                40u8,
                (dst.len() * 8 - 1) as u8,
                0x0b,
                (address >> 16) as u8,
                (address >> 8) as u8,
                address as u8,
                0,
            ];
            for byte in cmd {
                Self::sm1_push_packed_byte(byte);
            }

            let timeout_us = 10_000u32;
            let mut elapsed = 0u32;
            while rx_ch.ctrl_trig().read().busy() {
                for _ in 0..10 {
                    core::hint::spin_loop();
                }
                elapsed = elapsed.saturating_add(1);
                if elapsed > timeout_us {
                    rx_ch.ctrl_trig().write_value(CtrlTrig(0));
                    return Err(DiagError::DmaTimeout(None));
                }
            }

            if rx_ch.ctrl_trig().read().ahb_error() {
                rx_ch.ctrl_trig().write_value(CtrlTrig(0));
                return Err(DiagError::DmaAhbError);
            }
            if rx_ch.trans_count().read() != 0 {
                rx_ch.ctrl_trig().write_value(CtrlTrig(0));
                return Err(DiagError::DmaTimeout(None));
            }

            rx_ch.ctrl_trig().write_value(CtrlTrig(0));
            Ok(())
        }

        #[cfg(feature = "psram_dma_read_api")]
        pub fn read_sm1_prodphase_cpu_tx_rx_dma(
            _sm: &mut PicoCalcPsram<'_>,
            address: u32,
            dst: &mut [u8],
        ) -> Result<(), DiagError> {
            if dst.len() < 16 || dst.len() > 4096 || (dst.len() & 0x0f) != 0 {
                return Err(DiagError::InvalidArgument);
            }

            let pio1 = &pac::PIO1;
            let input_sync_bypass_before = pio1.input_sync_bypass().read();
            let (program_offset, sm0_was_enabled) = Self::prepare_prior_art_dedicated_sm()?;
            // Verified Embassy path uses divider=4 for SM1 prodphase reads.
            pio1.sm(Self::PRIOR_ART_SM as usize)
                .clkdiv()
                .write(|w| w.0 = 4u32 << 16);

            let run = (|| {
                let mut chunk_addr = address;
                for chunk in dst.chunks_mut(Self::SM1_MAX_TXN_BYTES) {
                    Self::read_sm1_prodphase_chunk(chunk_addr, chunk, program_offset)?;
                    chunk_addr = chunk_addr.wrapping_add(chunk.len() as u32);
                }
                Ok(())
            })();

            Self::teardown_prior_art_dedicated_sm(sm0_was_enabled);
            pio1.input_sync_bypass()
                .write(|w| *w = input_sync_bypass_before);

            run
        }

        /// Like `read_sm1_prodphase_chunk` but designed for the falling-edge fudge PIO
        /// program which uses `out y, 16`.  This allows transactions up to 4096 bytes,
        /// eliminating most per-chunk SM-restart overhead.
        #[cfg(feature = "psram_dma_read_code_window")]
        fn read_sm1_fudge_chunk_large(address: u32, dst: &mut [u8]) -> Result<(), DiagError> {
            // Must be 16-byte aligned, non-empty, at most 4096 bytes (y fits in u16).
            if dst.is_empty() || dst.len() > 4096 || (dst.len() & 0x0f) != 0 {
                return Err(DiagError::InvalidArgument);
            }

            let dma = &pac::DMA;
            let rx_ch = dma.ch(1);
            let rxf1 = Self::PIO1_BASE + 0x24;

            let dst_ptr = dst.as_mut_ptr() as u32;
            // RP2040 SRAM window guard: DMA write destination must be SRAM.
            if !(0x2000_0000..0x2004_2000).contains(&dst_ptr) {
                return Err(DiagError::InvalidArgument);
            }

            use embassy_rp::pac::dma::regs::CtrlTrig;
            rx_ch.ctrl_trig().write_value(CtrlTrig(0));

            rx_ch.read_addr().write_value(rxf1);
            rx_ch.write_addr().write_value(dst_ptr);
            rx_ch.trans_count().write_value(dst.len() as u32);
            rx_ch.ctrl_trig().write(|w| {
                use embassy_rp::pac::dma::vals::{DataSize, TreqSel};
                w.set_data_size(DataSize::SIZE_BYTE);
                w.set_incr_read(false);
                w.set_incr_write(true);
                w.set_treq_sel(TreqSel::PIO1_RX1);
                w.set_irq_quiet(true);
                w.set_en(true);
            });

            // Push command bit count (x = 40: cmd 0x0b + 3-byte addr + 1 dummy).
            Self::sm1_push_packed_byte(40u8);
            // Push data bit count (y) as 16-bit left-justified word.
            // The fudge PIO program uses `out y, 16`, so the value must be in
            // the upper 16 bits of the TX FIFO word.
            let data_bits = (dst.len() as u32 * 8 - 1) as u16;
            Self::sm1_push_packed_u16(data_bits);
            // FAST READ command, 3-byte address, 1 dummy byte.
            Self::sm1_push_packed_byte(0x0b);
            Self::sm1_push_packed_byte((address >> 16) as u8);
            Self::sm1_push_packed_byte((address >> 8) as u8);
            Self::sm1_push_packed_byte(address as u8);
            Self::sm1_push_packed_byte(0);

            let timeout_us = 10_000u32;
            let mut elapsed = 0u32;
            while rx_ch.ctrl_trig().read().busy() {
                for _ in 0..10 {
                    core::hint::spin_loop();
                }
                elapsed = elapsed.saturating_add(1);
                if elapsed > timeout_us {
                    rx_ch.ctrl_trig().write_value(CtrlTrig(0));
                    return Err(DiagError::DmaTimeout(None));
                }
            }

            if rx_ch.ctrl_trig().read().ahb_error() {
                rx_ch.ctrl_trig().write_value(CtrlTrig(0));
                return Err(DiagError::DmaAhbError);
            }
            if rx_ch.trans_count().read() != 0 {
                rx_ch.ctrl_trig().write_value(CtrlTrig(0));
                return Err(DiagError::DmaTimeout(None));
            }

            rx_ch.ctrl_trig().write_value(CtrlTrig(0));
            Ok(())
        }

        #[cfg(feature = "psram_dma_read_code_window")]
        pub fn read_sm1_falling_edge_fudge_clkdiv3(
            _sm: &mut PicoCalcPsram<'_>,
            address: u32,
            dst: &mut [u8],
        ) -> Result<(), DiagError> {
            if dst.len() < 16 || (dst.len() & 0x0f) != 0 {
                return Err(DiagError::InvalidArgument);
            }

            let pio1 = &pac::PIO1;
            let input_sync_bypass_before = pio1.input_sync_bypass().read();
            let (program_offset, sm0_was_enabled) = Self::prepare_phase_edge_fudge_dedicated_sm()?;
            pio1.sm(Self::PRIOR_ART_SM as usize)
                .clkdiv()
                .write(|w| w.0 = 3u32 << 16);

            // Restart once per read call. Repeating this per chunk dominates
            // code-window refill latency and masks chunk-size improvements.
            Self::restart_prior_art_sm1(program_offset);

            let run = (|| {
                let mut chunk_addr = address;
                // Use 1024-byte chunks with the large-fudge function.  The fudge PIO
                // program uses `out y, 16`, so a single transaction can be up to 4096B.
                // 1024B chunks reduce per-chunk SM-restart overhead ~32x vs 32B chunks.
                for chunk in dst.chunks_mut(super::PSRAM_SM1_FUDGE_CLKDIV3_LARGE_CHUNK_BYTES) {
                    Self::read_sm1_fudge_chunk_large(chunk_addr, chunk)?;
                    chunk_addr = chunk_addr.wrapping_add(chunk.len() as u32);
                }
                Ok(())
            })();

            Self::teardown_prior_art_dedicated_sm(sm0_was_enabled);
            pio1.input_sync_bypass()
                .write(|w| *w = input_sync_bypass_before);

            run
        }

        #[inline]
        fn snapshot_dual_state(
            pio1: &pac::pio::Pio,
            dma: &pac::dma::Dma,
            tx_ch: &pac::dma::Channel,
            rx_ch: &pac::dma::Channel,
            tx_src_addr: u32,
            tx_dst_addr: u32,
            rx_src_addr: u32,
            rx_dst_addr: u32,
            drained_words: u32,
            timeout_stage: u8,
        ) -> DmaDualDiagnostic {
            let tx_ctrl = tx_ch.ctrl_trig().read().0;
            let rx_ctrl = rx_ch.ctrl_trig().read().0;
            DmaDualDiagnostic {
                pio_instance: 1,
                sm_index: 0,
                tx_dma_channel: 0,
                rx_dma_channel: 1,
                tx_dreq_numeric_pio1_tx0: Self::dreq_numeric_pio1_tx0(),
                rx_dreq_numeric_pio1_rx0: Self::dreq_numeric_pio1_rx0(),
                tx_src_addr,
                tx_dst_addr,
                rx_src_addr,
                rx_dst_addr,
                tx_ctrl,
                rx_ctrl,
                tx_trans_count: tx_ch.trans_count().read(),
                rx_trans_count: rx_ch.trans_count().read(),
                tx_read_addr: tx_ch.read_addr().read(),
                tx_write_addr: tx_ch.write_addr().read(),
                rx_read_addr: rx_ch.read_addr().read(),
                rx_write_addr: rx_ch.write_addr().read(),
                pio_flevel: pio1.flevel().read().0,
                pio_fstat: pio1.fstat().read().0,
                dma_intr0: dma.intr(0).read(),
                dma_ints0: dma.ints(0).read(),
                dma_inte0: dma.inte(0).read(),
                rx_fifo_drained_words: drained_words,
                timeout_stage,
                tx_ahb_error: (tx_ctrl & (1 << 31)) != 0,
                rx_ahb_error: (rx_ctrl & (1 << 31)) != 0,
            }
        }

        /// Dual-DMA diagnostic read for 16-byte PSRAM transaction.
        pub fn read_dma16_dual(
            _sm: &mut PicoCalcPsram<'_>,
            address: u32,
            dst: &mut [u8],
        ) -> Result<(), DiagError> {
            if dst.len() < 16 {
                return Err(DiagError::InvalidArgument);
            }

            let pio1 = &pac::PIO1;
            let dma = &pac::DMA;

            // PIO1 FIFO MMIO addresses for SM0.
            let pio1_base = 0x4005_0000u32;
            let txf0_addr = pio1_base + 0x10;
            let rxf0_addr = pio1_base + 0x20;

            // Clear RX FIFO before arming DMA, as required for deterministic reads.
            let mut drained_words = 0u32;
            while (pio1.fstat().read().0 & (1 << 8)) == 0 {
                // SAFETY: RXF0 MMIO read drains one queued FIFO word.
                unsafe {
                    core::ptr::read_volatile(rxf0_addr as *const u32);
                }
                drained_words = drained_words.saturating_add(1);
                if drained_words > 32 {
                    break;
                }
            }

            let cmd = [
                40u8,
                (16 * 8) as u8,
                0x0b,
                (address >> 16) as u8,
                (address >> 8) as u8,
                address as u8,
                0,
            ];

            // RX DMA (CH1): RXF0 -> dst[16], byte paced by PIO1_RX0.
            let rx_ch = dma.ch(1);
            // TX DMA (CH0): cmd[] -> TXF0, byte paced by PIO1_TX0.
            let tx_ch = dma.ch(0);

            use embassy_rp::pac::dma::regs::CtrlTrig;
            tx_ch.ctrl_trig().write_value(CtrlTrig(0));
            rx_ch.ctrl_trig().write_value(CtrlTrig(0));

            // Program RX first.
            rx_ch.read_addr().write_value(rxf0_addr);
            rx_ch.write_addr().write_value(dst.as_mut_ptr() as u32);
            rx_ch.trans_count().write_value(16);
            rx_ch.ctrl_trig().write(|w| {
                use embassy_rp::pac::dma::vals::{DataSize, TreqSel};
                w.set_data_size(DataSize::SIZE_BYTE);
                w.set_incr_read(false);
                w.set_incr_write(true);
                w.set_treq_sel(TreqSel::PIO1_RX0);
                w.set_irq_quiet(true);
                w.set_en(true);
            });

            // Program and start TX after RX is armed.
            tx_ch.read_addr().write_value(cmd.as_ptr() as u32);
            tx_ch.write_addr().write_value(txf0_addr);
            tx_ch.trans_count().write_value(cmd.len() as u32);
            tx_ch.ctrl_trig().write(|w| {
                use embassy_rp::pac::dma::vals::{DataSize, TreqSel};
                w.set_data_size(DataSize::SIZE_BYTE);
                w.set_incr_read(true);
                w.set_incr_write(false);
                w.set_treq_sel(TreqSel::PIO1_TX0);
                w.set_irq_quiet(true);
                w.set_en(true);
            });

            // Wait TX completion first.
            let timeout_us = 10_000u32;
            let mut tx_elapsed = 0u32;
            loop {
                let tx_ctrl = tx_ch.ctrl_trig().read();
                if !tx_ctrl.busy() {
                    if tx_ctrl.ahb_error() {
                        let diag = Self::snapshot_dual_state(
                            pio1,
                            dma,
                            &tx_ch,
                            &rx_ch,
                            cmd.as_ptr() as u32,
                            txf0_addr,
                            rxf0_addr,
                            dst.as_mut_ptr() as u32,
                            drained_words,
                            1,
                        );
                        tx_ch.ctrl_trig().write_value(CtrlTrig(0));
                        rx_ch.ctrl_trig().write_value(CtrlTrig(0));
                        return Err(DiagError::DmaDualFailure(Some(diag)));
                    }
                    break;
                }
                for _ in 0..10 {
                    core::hint::spin_loop();
                }
                tx_elapsed = tx_elapsed.saturating_add(1);
                if tx_elapsed > timeout_us {
                    let diag = Self::snapshot_dual_state(
                        pio1,
                        dma,
                        &tx_ch,
                        &rx_ch,
                        cmd.as_ptr() as u32,
                        txf0_addr,
                        rxf0_addr,
                        dst.as_mut_ptr() as u32,
                        drained_words,
                        1,
                    );
                    tx_ch.ctrl_trig().write_value(CtrlTrig(0));
                    rx_ch.ctrl_trig().write_value(CtrlTrig(0));
                    return Err(DiagError::DmaDualFailure(Some(diag)));
                }
            }

            // Then wait RX completion.
            let mut rx_elapsed = 0u32;
            loop {
                let rx_ctrl = rx_ch.ctrl_trig().read();
                if !rx_ctrl.busy() {
                    if rx_ctrl.ahb_error() {
                        let diag = Self::snapshot_dual_state(
                            pio1,
                            dma,
                            &tx_ch,
                            &rx_ch,
                            cmd.as_ptr() as u32,
                            txf0_addr,
                            rxf0_addr,
                            dst.as_mut_ptr() as u32,
                            drained_words,
                            2,
                        );
                        tx_ch.ctrl_trig().write_value(CtrlTrig(0));
                        rx_ch.ctrl_trig().write_value(CtrlTrig(0));
                        return Err(DiagError::DmaDualFailure(Some(diag)));
                    }
                    break;
                }
                for _ in 0..10 {
                    core::hint::spin_loop();
                }
                rx_elapsed = rx_elapsed.saturating_add(1);
                if rx_elapsed > timeout_us {
                    let diag = Self::snapshot_dual_state(
                        pio1,
                        dma,
                        &tx_ch,
                        &rx_ch,
                        cmd.as_ptr() as u32,
                        txf0_addr,
                        rxf0_addr,
                        dst.as_mut_ptr() as u32,
                        drained_words,
                        2,
                    );
                    tx_ch.ctrl_trig().write_value(CtrlTrig(0));
                    rx_ch.ctrl_trig().write_value(CtrlTrig(0));
                    return Err(DiagError::DmaDualFailure(Some(diag)));
                }
            }

            Ok(())
        }

        pub fn read_faithful_prior_art_dma16(
            _sm: &mut PicoCalcPsram<'_>,
            address: u32,
            dst: &mut [u8],
        ) -> Result<FaithfulPriorArtReport, DiagError> {
            if dst.len() < 16 {
                return Err(DiagError::InvalidArgument);
            }

            let (program_offset, sm0_was_enabled) = Self::prepare_prior_art_dedicated_sm()?;
            Self::restart_prior_art_sm1(program_offset);
            let ctrl_before_tx = pac::PIO1.ctrl().read().sm_enable();
            let (sm0_enabled_before_tx, sm1_enabled_before_tx) =
                Self::prior_art_enable_state(ctrl_before_tx);
            if sm0_enabled_before_tx || !sm1_enabled_before_tx {
                let cfg_diag = Self::snapshot_prior_art_config_error(
                    "faithful_prior_art_dma16",
                    program_offset,
                    sm0_enabled_before_tx,
                    sm1_enabled_before_tx,
                    sm0_was_enabled,
                );
                return Err(DiagError::PriorArtConfigError(cfg_diag));
            }

            let pio1 = &pac::PIO1;
            let dma = &pac::DMA;
            let tx_ch = dma.ch(0);
            let rx_ch = dma.ch(1);
            let tx_dreq_numeric = Self::dreq_numeric_pio1_tx1();
            let rx_dreq_numeric = Self::dreq_numeric_pio1_rx1();

            let txf1 = 0x4005_0014u32;
            let rxf1 = 0x4005_0024u32;
            let cmd = [
                40u8,
                (16 * 8) as u8,
                0x0b,
                (address >> 16) as u8,
                (address >> 8) as u8,
                address as u8,
                0,
            ];

            use embassy_rp::pac::dma::regs::CtrlTrig;
            let mut first_16_output = [0u8; 16];

            // Dedicated-SM1 diagnostic path. Drain any pending RXF1 words.
            let mut drain_reads = 0u32;
            const MAX_DRAIN_READS: u32 = 1024;
            while (pio1.fstat().read().0 & (1 << 9)) == 0 {
                // SAFETY: reading RXF1 drains a pending FIFO word.
                unsafe {
                    core::ptr::read_volatile(0x4005_0024u32 as *const u32);
                }
                drain_reads = drain_reads.saturating_add(1);
                if drain_reads >= MAX_DRAIN_READS {
                    let diag = Self::snapshot_faithful_prior_art_failure(
                        "faithful_prior_art_dma16",
                        Some(&tx_ch),
                        &rx_ch,
                        pio1,
                        Self::PRIOR_ART_SM,
                        program_offset,
                        Some(tx_dreq_numeric),
                        Some(rx_dreq_numeric),
                        Some(cmd.as_ptr() as u32),
                        Some(txf1),
                        rxf1,
                        dst.as_mut_ptr() as u32,
                        cmd,
                        first_16_output,
                        sm1_enabled_before_tx,
                        0,
                    );
                    tx_ch.ctrl_trig().write_value(CtrlTrig(0));
                    rx_ch.ctrl_trig().write_value(CtrlTrig(0));
                    Self::teardown_prior_art_dedicated_sm(sm0_was_enabled);
                    let mut diag = diag;
                    let (_, sm1_after_cleanup) =
                        Self::prior_art_enable_state(pac::PIO1.ctrl().read().sm_enable());
                    diag.sm_enabled_after_cleanup = sm1_after_cleanup;
                    return Err(DiagError::FaithfulPriorArtFailure(Some(diag)));
                }
            }

            tx_ch.ctrl_trig().write_value(CtrlTrig(0));
            rx_ch.ctrl_trig().write_value(CtrlTrig(0));

            // RX first.
            rx_ch.read_addr().write_value(rxf1);
            rx_ch.write_addr().write_value(dst.as_mut_ptr() as u32);
            rx_ch.trans_count().write_value(16);
            rx_ch.ctrl_trig().write(|w| {
                use embassy_rp::pac::dma::vals::{DataSize, TreqSel};
                w.set_data_size(DataSize::SIZE_BYTE);
                w.set_incr_read(false);
                w.set_incr_write(true);
                w.set_treq_sel(TreqSel::PIO1_RX1);
                w.set_irq_quiet(true);
                w.set_en(true);
            });

            // TX command bytes next.
            tx_ch.read_addr().write_value(cmd.as_ptr() as u32);
            tx_ch.write_addr().write_value(txf1);
            tx_ch.trans_count().write_value(cmd.len() as u32);
            tx_ch.ctrl_trig().write(|w| {
                use embassy_rp::pac::dma::vals::{DataSize, TreqSel};
                w.set_data_size(DataSize::SIZE_BYTE);
                w.set_incr_read(true);
                w.set_incr_write(false);
                w.set_treq_sel(TreqSel::PIO1_TX1);
                w.set_irq_quiet(true);
                w.set_en(true);
            });

            let timeout_us = 10_000u32;
            let mut elapsed = 0u32;
            while tx_ch.ctrl_trig().read().busy() {
                for _ in 0..10 {
                    core::hint::spin_loop();
                }
                elapsed = elapsed.saturating_add(1);
                if elapsed > timeout_us {
                    let diag = Self::snapshot_faithful_prior_art_failure(
                        "faithful_prior_art_dma16",
                        Some(&tx_ch),
                        &rx_ch,
                        pio1,
                        Self::PRIOR_ART_SM,
                        program_offset,
                        Some(tx_dreq_numeric),
                        Some(rx_dreq_numeric),
                        Some(cmd.as_ptr() as u32),
                        Some(txf1),
                        rxf1,
                        dst.as_mut_ptr() as u32,
                        cmd,
                        first_16_output,
                        sm1_enabled_before_tx,
                        1,
                    );
                    tx_ch.ctrl_trig().write_value(CtrlTrig(0));
                    rx_ch.ctrl_trig().write_value(CtrlTrig(0));
                    Self::teardown_prior_art_dedicated_sm(sm0_was_enabled);
                    let mut diag = diag;
                    let (_, sm1_after_cleanup) =
                        Self::prior_art_enable_state(pac::PIO1.ctrl().read().sm_enable());
                    diag.sm_enabled_after_cleanup = sm1_after_cleanup;
                    return Err(DiagError::FaithfulPriorArtFailure(Some(diag)));
                }
            }

            if tx_ch.ctrl_trig().read().ahb_error() {
                let diag = Self::snapshot_faithful_prior_art_failure(
                    "faithful_prior_art_dma16",
                    Some(&tx_ch),
                    &rx_ch,
                    pio1,
                    Self::PRIOR_ART_SM,
                    program_offset,
                    Some(tx_dreq_numeric),
                    Some(rx_dreq_numeric),
                    Some(cmd.as_ptr() as u32),
                    Some(txf1),
                    rxf1,
                    dst.as_mut_ptr() as u32,
                    cmd,
                    first_16_output,
                    sm1_enabled_before_tx,
                    1,
                );
                tx_ch.ctrl_trig().write_value(CtrlTrig(0));
                rx_ch.ctrl_trig().write_value(CtrlTrig(0));
                Self::teardown_prior_art_dedicated_sm(sm0_was_enabled);
                let mut diag = diag;
                let (_, sm1_after_cleanup) =
                    Self::prior_art_enable_state(pac::PIO1.ctrl().read().sm_enable());
                diag.sm_enabled_after_cleanup = sm1_after_cleanup;
                return Err(DiagError::FaithfulPriorArtFailure(Some(diag)));
            }

            elapsed = 0;
            while rx_ch.ctrl_trig().read().busy() {
                for _ in 0..10 {
                    core::hint::spin_loop();
                }
                elapsed = elapsed.saturating_add(1);
                if elapsed > timeout_us {
                    let diag = Self::snapshot_faithful_prior_art_failure(
                        "faithful_prior_art_dma16",
                        Some(&tx_ch),
                        &rx_ch,
                        pio1,
                        Self::PRIOR_ART_SM,
                        program_offset,
                        Some(tx_dreq_numeric),
                        Some(rx_dreq_numeric),
                        Some(cmd.as_ptr() as u32),
                        Some(txf1),
                        rxf1,
                        dst.as_mut_ptr() as u32,
                        cmd,
                        first_16_output,
                        sm1_enabled_before_tx,
                        2,
                    );
                    tx_ch.ctrl_trig().write_value(CtrlTrig(0));
                    rx_ch.ctrl_trig().write_value(CtrlTrig(0));
                    Self::teardown_prior_art_dedicated_sm(sm0_was_enabled);
                    let mut diag = diag;
                    let (_, sm1_after_cleanup) =
                        Self::prior_art_enable_state(pac::PIO1.ctrl().read().sm_enable());
                    diag.sm_enabled_after_cleanup = sm1_after_cleanup;
                    return Err(DiagError::FaithfulPriorArtFailure(Some(diag)));
                }
            }

            if rx_ch.ctrl_trig().read().ahb_error() {
                let diag = Self::snapshot_faithful_prior_art_failure(
                    "faithful_prior_art_dma16",
                    Some(&tx_ch),
                    &rx_ch,
                    pio1,
                    Self::PRIOR_ART_SM,
                    program_offset,
                    Some(tx_dreq_numeric),
                    Some(rx_dreq_numeric),
                    Some(cmd.as_ptr() as u32),
                    Some(txf1),
                    rxf1,
                    dst.as_mut_ptr() as u32,
                    cmd,
                    first_16_output,
                    sm1_enabled_before_tx,
                    2,
                );
                tx_ch.ctrl_trig().write_value(CtrlTrig(0));
                rx_ch.ctrl_trig().write_value(CtrlTrig(0));
                Self::teardown_prior_art_dedicated_sm(sm0_was_enabled);
                let mut diag = diag;
                let (_, sm1_after_cleanup) =
                    Self::prior_art_enable_state(pac::PIO1.ctrl().read().sm_enable());
                diag.sm_enabled_after_cleanup = sm1_after_cleanup;
                return Err(DiagError::FaithfulPriorArtFailure(Some(diag)));
            }

            first_16_output.copy_from_slice(&dst[..16]);

            Self::teardown_prior_art_dedicated_sm(sm0_was_enabled);

            Ok(FaithfulPriorArtReport {
                variant_name: "faithful_prior_art_dma16",
                pio_instance: 1,
                sm_index: Self::PRIOR_ART_SM,
                program_offset,
                tx_dma_channel: Some(0),
                rx_dma_channel: Some(1),
                data_size_bits: 8,
                tx_transfer_count: Some(cmd.len() as u32),
                rx_transfer_count: Some(16),
                tx_dreq_numeric: Some(tx_dreq_numeric),
                rx_dreq_numeric: Some(rx_dreq_numeric),
                command_bytes: cmd,
                first_16_output_bytes: first_16_output,
            })
        }

        pub fn read_faithful_prior_art_cpu_tx_dma_rx16(
            _sm: &mut PicoCalcPsram<'_>,
            address: u32,
            dst: &mut [u8],
        ) -> Result<FaithfulPriorArtReport, DiagError> {
            if dst.len() < 16 {
                return Err(DiagError::InvalidArgument);
            }

            let (program_offset, sm0_was_enabled) = Self::prepare_prior_art_dedicated_sm()?;
            Self::restart_prior_art_sm1(program_offset);
            let ctrl_before_tx = pac::PIO1.ctrl().read().sm_enable();
            let (sm0_enabled_before_tx, sm1_enabled_before_tx) =
                Self::prior_art_enable_state(ctrl_before_tx);
            if sm0_enabled_before_tx || !sm1_enabled_before_tx {
                let cfg_diag = Self::snapshot_prior_art_config_error(
                    "faithful_prior_art_cpu_tx_dma_rx16",
                    program_offset,
                    sm0_enabled_before_tx,
                    sm1_enabled_before_tx,
                    sm0_was_enabled,
                );
                return Err(DiagError::PriorArtConfigError(cfg_diag));
            }

            let pio1 = &pac::PIO1;
            let dma = &pac::DMA;
            let rx_ch = dma.ch(1);
            let rx_dreq_numeric = Self::dreq_numeric_pio1_rx1();

            let txf1 = 0x4005_0014u32;
            let rxf1 = 0x4005_0024u32;
            let cmd = [
                40u8,
                (16 * 8) as u8,
                0x0b,
                (address >> 16) as u8,
                (address >> 8) as u8,
                address as u8,
                0,
            ];
            let mut first_16_output = [0u8; 16];

            use embassy_rp::pac::dma::regs::CtrlTrig;

            // Dedicated-SM1 diagnostic path. Drain any pending RXF1 words.
            let mut drain_reads = 0u32;
            const MAX_DRAIN_READS: u32 = 1024;
            while (pio1.fstat().read().0 & (1 << 9)) == 0 {
                // SAFETY: reading RXF1 drains a pending FIFO word.
                unsafe {
                    core::ptr::read_volatile(rxf1 as *const u32);
                }
                drain_reads = drain_reads.saturating_add(1);
                if drain_reads >= MAX_DRAIN_READS {
                    let diag = Self::snapshot_faithful_prior_art_failure(
                        "faithful_prior_art_cpu_tx_dma_rx16",
                        None,
                        &rx_ch,
                        pio1,
                        Self::PRIOR_ART_SM,
                        program_offset,
                        None,
                        Some(rx_dreq_numeric),
                        None,
                        None,
                        rxf1,
                        dst.as_mut_ptr() as u32,
                        cmd,
                        first_16_output,
                        sm1_enabled_before_tx,
                        0,
                    );
                    rx_ch.ctrl_trig().write_value(CtrlTrig(0));
                    Self::teardown_prior_art_dedicated_sm(sm0_was_enabled);
                    let mut diag = diag;
                    let (_, sm1_after_cleanup) =
                        Self::prior_art_enable_state(pac::PIO1.ctrl().read().sm_enable());
                    diag.sm_enabled_after_cleanup = sm1_after_cleanup;
                    return Err(DiagError::FaithfulPriorArtCpuTxFailure(Some(diag)));
                }
            }

            rx_ch.ctrl_trig().write_value(CtrlTrig(0));

            // Arm RX DMA first as required by the diagnostic.
            rx_ch.read_addr().write_value(rxf1);
            rx_ch.write_addr().write_value(dst.as_mut_ptr() as u32);
            rx_ch.trans_count().write_value(16);
            rx_ch.ctrl_trig().write(|w| {
                use embassy_rp::pac::dma::vals::{DataSize, TreqSel};
                w.set_data_size(DataSize::SIZE_BYTE);
                w.set_incr_read(false);
                w.set_incr_write(true);
                w.set_treq_sel(TreqSel::PIO1_RX1);
                w.set_irq_quiet(true);
                w.set_en(true);
            });

            // Prior-art style TX uses 8-bit volatile writes to TXF1 (no TX DMA).
            let txfifo8 = txf1 as *mut u8;
            for byte in cmd {
                // SAFETY: TXF1 MMIO supports byte writes in prior-art-compatible mode.
                unsafe {
                    core::ptr::write_volatile(txfifo8, byte);
                }
            }

            let timeout_us = 10_000u32;
            let mut elapsed = 0u32;
            while rx_ch.ctrl_trig().read().busy() {
                for _ in 0..10 {
                    core::hint::spin_loop();
                }
                elapsed = elapsed.saturating_add(1);
                if elapsed > timeout_us {
                    let diag = Self::snapshot_faithful_prior_art_failure(
                        "faithful_prior_art_cpu_tx_dma_rx16",
                        None,
                        &rx_ch,
                        pio1,
                        Self::PRIOR_ART_SM,
                        program_offset,
                        None,
                        Some(rx_dreq_numeric),
                        None,
                        None,
                        rxf1,
                        dst.as_mut_ptr() as u32,
                        cmd,
                        first_16_output,
                        sm1_enabled_before_tx,
                        2,
                    );
                    rx_ch.ctrl_trig().write_value(CtrlTrig(0));
                    Self::teardown_prior_art_dedicated_sm(sm0_was_enabled);
                    let mut diag = diag;
                    let (_, sm1_after_cleanup) =
                        Self::prior_art_enable_state(pac::PIO1.ctrl().read().sm_enable());
                    diag.sm_enabled_after_cleanup = sm1_after_cleanup;
                    return Err(DiagError::FaithfulPriorArtCpuTxFailure(Some(diag)));
                }
            }

            if rx_ch.ctrl_trig().read().ahb_error() {
                let diag = Self::snapshot_faithful_prior_art_failure(
                    "faithful_prior_art_cpu_tx_dma_rx16",
                    None,
                    &rx_ch,
                    pio1,
                    Self::PRIOR_ART_SM,
                    program_offset,
                    None,
                    Some(rx_dreq_numeric),
                    None,
                    None,
                    rxf1,
                    dst.as_mut_ptr() as u32,
                    cmd,
                    first_16_output,
                    sm1_enabled_before_tx,
                    2,
                );
                rx_ch.ctrl_trig().write_value(CtrlTrig(0));
                Self::teardown_prior_art_dedicated_sm(sm0_was_enabled);
                let mut diag = diag;
                let (_, sm1_after_cleanup) =
                    Self::prior_art_enable_state(pac::PIO1.ctrl().read().sm_enable());
                diag.sm_enabled_after_cleanup = sm1_after_cleanup;
                return Err(DiagError::FaithfulPriorArtCpuTxFailure(Some(diag)));
            }

            first_16_output.copy_from_slice(&dst[..16]);

            Self::teardown_prior_art_dedicated_sm(sm0_was_enabled);

            Ok(FaithfulPriorArtReport {
                variant_name: "faithful_prior_art_cpu_tx_dma_rx16",
                pio_instance: 1,
                sm_index: Self::PRIOR_ART_SM,
                program_offset,
                tx_dma_channel: None,
                rx_dma_channel: Some(1),
                data_size_bits: 8,
                tx_transfer_count: None,
                rx_transfer_count: Some(16),
                tx_dreq_numeric: None,
                rx_dreq_numeric: Some(rx_dreq_numeric),
                command_bytes: cmd,
                first_16_output_bytes: first_16_output,
            })
        }

        pub fn run_unpaced_dma4_from_rxf0(
            _sm: &mut PicoCalcPsram<'_>,
            out4: &mut [u8; 4],
        ) -> Result<UnpacedDma4Diagnostic, DiagError> {
            let pio1 = &pac::PIO1;
            let dma = &pac::DMA;
            let ch = dma.ch(1);
            let pio_rx_fifo_addr = 0x4005_0020u32; // PIO1 RXF0 MMIO

            let flevel = pio1.flevel().read().0;
            let (_tx_level, rx_level) = Self::pio_sm0_levels(flevel);

            // This experiment is valid only when exactly 4 words are queued.
            if rx_level != 4 {
                return Err(DiagError::Unsupported);
            }

            let mut tmp = [0u32; 4];

            let mut diag = UnpacedDma4Diagnostic {
                dma_channel: 1,
                dreq_numeric_pio1_rx0: Self::dreq_numeric_pio1_rx0(),
                dreq_ref_rp2040_pio1_rx0: Self::DREQ_REF_RP2040_PIO1_RX0,
                rx_level_before_unpaced: rx_level,
                unpaced_tc_before: 0,
                unpaced_tc_after: 0,
                unpaced_read_addr: 0,
                unpaced_write_addr: 0,
            };

            use embassy_rp::pac::dma::regs::CtrlTrig;
            ch.ctrl_trig().write_value(CtrlTrig(0));

            diag.unpaced_tc_before = ch.trans_count().read();

            ch.read_addr().write_value(pio_rx_fifo_addr);
            ch.write_addr().write_value(tmp.as_mut_ptr() as u32);
            ch.trans_count().write_value(4);
            ch.ctrl_trig().write(|w| {
                use embassy_rp::pac::dma::vals::{DataSize, TreqSel};
                w.set_data_size(DataSize::SIZE_WORD);
                w.set_incr_read(false);
                w.set_incr_write(true);
                w.set_treq_sel(TreqSel::PERMANENT);
                w.set_irq_quiet(true);
                w.set_en(true);
            });

            let timeout_us = 2_000u32;
            let mut elapsed_us = 0u32;
            loop {
                let ctrl = ch.ctrl_trig().read();
                if !ctrl.busy() {
                    if ctrl.ahb_error() {
                        ch.ctrl_trig().write_value(CtrlTrig(0));
                        return Err(DiagError::DmaAhbError);
                    }
                    break;
                }
                for _ in 0..10 {
                    core::hint::spin_loop();
                }
                elapsed_us = elapsed_us.saturating_add(1);
                if elapsed_us > timeout_us {
                    ch.ctrl_trig().write_value(CtrlTrig(0));
                    return Err(DiagError::DmaTimeout(None));
                }
            }

            diag.unpaced_tc_after = ch.trans_count().read();
            diag.unpaced_read_addr = ch.read_addr().read();
            diag.unpaced_write_addr = ch.write_addr().read();

            for i in 0..4 {
                out4[i] = tmp[i] as u8;
            }

            Ok(diag)
        }
    }
}

// Phase 3d design notes / TODOs for implementing `dma16` diagnostic:
//
// 1) DMA channel selection
//    - Use `DMA_CH1` for the diagnostic path by default. Rationale: current
//      firmware and probes (LCD scanline SPI and PWM audio) use `DMA_CH0`.
//      Choosing CH1 avoids collisions with common probe usage while leaving
//      CH0 untouched. If CH1 is unavailable on a target variant, document how
//      to pick an alternate channel.
//
// 2) DMA destination buffer format
//    - Configure DMA to transfer 32-bit words from the PIO RX FIFO into a
//      32-bit-aligned word buffer in SRAM. Rationale: the PIO RX FIFO and
//      DMA naturally operate on 32-bit AHB words; transferring words avoids
//      per-byte DMA setups and is more robust. After DMA completes, repack
//      the words into the exact byte stream using the same byte-extraction
//      rule as `pull_byte()` (i.e., interpret each pulled `u32` the same way
//      the current `pull_byte()` implementation does) to preserve semantics.
//    - Avoid direct byte-sized DMA transfers from the FIFO unless the DMA
//      controller and the PIO RX FIFO packing are proven safe on hardware.
//
// 3) Exact expected transfer size for a 16B read
//    - `in_bits_count` used by the HAL is `(chunk.len() * 8 - 1)`:
//        16 * 8 - 1 = 127
//    - The PIO state machine will therefore clock 127 data bits (which the
//      SM will present to the shift register). For packing convenience the
//      effective bit window is 128 bits (one extra pad bit may be present
//      depending on SM behaviour). Compute words = ceil((in_bits_count + 1)/32).
//      For 16B: (127 + 1) / 32 = 128/32 = 4 words.
//    - Expect DMA to move 4 x 32-bit words from PIO RX FIFO to SRAM per 16B
//      read. After reassembly, only the first 16 bytes are consumed; trailing
//      padding bits/bytes must be ignored.
//
// 4) PIO SM start/stop coordination for diagnostic read
//    - Workflow:
//      a) Ensure PIO SM is enabled (constructor does this). Clear RX FIFO.
//      b) Configure and arm the DMA channel to read `words` 32-bit words
//         from the PIO RX FIFO register address into an aligned SRAM buffer.
//      c) Push the read command bytes into the PIO TX FIFO (same command
//         layout as production: out_bits_count, in_bits_count, opcode, addr..., 0).
//      d) Start the DMA (or arm it before pushing the command to avoid races).
//      e) Wait for DMA completion (poll or IRQ — see next section).
//      f) Stop / drain as necessary and reassemble bytes from the word buffer.
//
// 5) Polling vs IRQ for completion
//    - For `psram_diag` a simple polling approach is sufficient and simpler:
//      poll DMA transfer-complete flag with a conservative timeout. This keeps
//      the diagnostic synchronous and easy to reason about.
//    - IRQ-based completion is optional for future non-blocking tests but
//      not required here.
//
// 6) Error and timeout detection
//    - Implement a wall-clock timeout for DMA completion (e.g., 2 ms per
//      256 bytes scaled down for 16B; a safe default could be 5 ms per op).
//    - On timeout: abort DMA, reset/clear PIO RX FIFO, log `dma_timeout` and
//      return a diagnostic error. Also snapshot DMA status registers for
//      debugging (remaining transfer count, AHB error flags).
//    - Detect DMA AHB errors and report `dma_ahb_err` similarly.
//
// 7) Comparing `dma16` output vs `prod16` in `psram_diag`
//    - For each test block/pattern:
//      a) Read block using production path (`PsramBlocks::read_block`) into
//         `prod_buf` and record `prod_read_us`.
//      b) Invoke diagnostic DMA-assisted read into `dma_word_buf`, reassemble
//         into `dma_buf`, record `dma_read_us`.
//      c) Compare `prod_buf` and `dma_buf` byte-for-byte. On mismatch, log
//         first mismatch offset and both expected/actual byte values.
//    - Keep `prod16` as the authoritative source of truth for correctness.
//
// 8) Why this does not affect production app launch
//    - The DMA-assisted code lives only in this diagnostic module and is
//      never selected by production. `DmaPicoCalcPsram::new()` returns
//      `HalError::Unavailable` unless the diagnostic feature is explicitly
//      compiled/enabled. No changes were made to `PsramHal::read`,
//      `PsramBlocks`, `PsramCodeWindow`, or `CODE_WINDOW_BYTES`.
//
// 9) Hardware log format (pass / fail)
//    - On success:
//        "psram diag mode=dma16 pattern=... read_us_prod=NN read_us_dma=MM verify=pass"
//    - On mismatch:
//        "psram diag mode=dma16 pattern=... read_us_prod=NN read_us_dma=MM verify=fail mismatch=OFF expected=XX actual=YY"
//    - On DMA timeout / AHB error:
//        "psram diag mode=dma16 pattern=... dma_error=timeout|ahb remaining=N status=..."
//
// Implementation notes:
// - Keep all DMA resource allocation and register access isolated inside
//   `psram_dma.rs` behind feature flags and runtime guards.
// - Provide toggles to pause other DMA consumers (LCD/audio) during the
//   diagnostic run to avoid resource contention; prefer documenting manual
//   steps (stop probe) over automatic preemption for safety.
// - After initial verification with `dma16`, only then consider `dma24`
//   experiments under the same gated diagnostic harness.
