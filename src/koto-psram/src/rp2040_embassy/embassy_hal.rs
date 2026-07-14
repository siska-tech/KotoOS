use crate::{
    config::{Pins, TimingConfig},
    device::DeviceId,
    error::PsramError,
    pio::blocking::BlockingPio,
    pio::word_stream::{
        nibble_count, pack_stream_word, read_stream_rx_words, unpack_stream_word_full_unchecked,
        unpack_stream_word_tail, BYTES_PER_WORD,
    },
    protocol::{MemoryOp, QpiTransaction},
};

use super::boundary::{qpi_dummy_transfer_bytes, validate_chunk};
pub use super::diagnostics::{
    PayloadTransferPath, QpiChunkTiming, TransactionPioFastReadLoopVariant,
    WordStreamReadDiagnostics,
};

use embassy_rp::{
    dma,
    gpio::{Drive, Level, Pull, SlewRate},
    pio::{
        Common, Config as PioConfig, Direction, Instance, LoadedProgram, Pin as PioPinHandle,
        PioPin, ShiftDirection, StateMachine,
    },
    Peri,
};
use embassy_time::Instant;

#[path = "pac_dma.rs"]
mod pac_dma;
#[path = "programs.rs"]
mod programs;
#[path = "transaction_pio.rs"]
mod transaction_pio;

pub use transaction_pio::{
    PacDmaStatus, TransactionPioDiagnostics, TransactionPioTxDmaBufferDiagnostics,
    TransactionPioTxDmaStep,
};
use transaction_pio::{RX_DMA_WORD_CAPACITY, TX_DMA_WORD_CAPACITY};

/// Error returned by the concrete `embassy-rp` RP2040 QPI backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbassyRpQpiError {
    /// Error from the protocol-level driver contract.
    Core(PsramError),
    /// The HAL resources have already been consumed or are not configured.
    InvalidResources,
    /// The PIO instruction memory did not have room for the command skeletons.
    ProgramLoad,
    /// The blocking PIO command path did not complete before the poll budget expired.
    Timeout,
    /// The requested concrete backend operation is not implemented.
    Unsupported,
    /// The stream payload is too large for the current PIO setup contract.
    StreamLength,
}

impl From<PsramError> for EmbassyRpQpiError {
    fn from(error: PsramError) -> Self {
        Self::Core(error)
    }
}

/// Concrete blocking QPI backend shell for `embassy-rp` on RP2040.
///
/// The backend owns the PIO common handle, one state machine, the six PSRAM
/// GPIO pins, and the timing state used by the protocol-level driver.
pub struct EmbassyRpQpiBackend<'d, PIO, const SM: usize, Sio0, Sio1, Sio2, Sio3, Cs, Sck>
where
    PIO: Instance + 'd,
    Sio0: PioPin + 'd,
    Sio1: PioPin + 'd,
    Sio2: PioPin + 'd,
    Sio3: PioPin + 'd,
    Cs: PioPin + 'd,
    Sck: PioPin + 'd,
{
    common: Common<'d, PIO>,
    state_machine: StateMachine<'d, PIO, SM>,
    sio0: Option<Peri<'d, Sio0>>,
    sio1: Option<Peri<'d, Sio1>>,
    sio2: Option<Peri<'d, Sio2>>,
    sio3: Option<Peri<'d, Sio3>>,
    cs: Option<Peri<'d, Cs>>,
    sck: Option<Peri<'d, Sck>>,
    pio_sio0: Option<PioPinHandle<'d, PIO>>,
    pio_sio1: Option<PioPinHandle<'d, PIO>>,
    pio_sio2: Option<PioPinHandle<'d, PIO>>,
    pio_sio3: Option<PioPinHandle<'d, PIO>>,
    pio_cs: Option<PioPinHandle<'d, PIO>>,
    pio_sck: Option<PioPinHandle<'d, PIO>>,
    spi_command_program: Option<LoadedProgram<'d, PIO>>,
    qpi_command_program: Option<LoadedProgram<'d, PIO>>,
    qpi_write_stream_program: Option<LoadedProgram<'d, PIO>>,
    qpi_read_stream_program: Option<LoadedProgram<'d, PIO>>,
    qpi_transaction_program: Option<LoadedProgram<'d, PIO>>,
    qpi_transaction_fast_program: Option<LoadedProgram<'d, PIO>>,
    qpi_transaction_fast_program_variant: Option<TransactionPioFastReadLoopVariant>,
    pins: Option<Pins>,
    timing: TimingConfig,
    payload_transfer_path: PayloadTransferPath,
    transaction_pio_fast_read_loop_variant: TransactionPioFastReadLoopVariant,
    word_stream_read_diagnostics: WordStreamReadDiagnostics,
    transaction_pio_diagnostics: TransactionPioDiagnostics,
    tx_dma_channel: Option<dma::Channel<'d>>,
    tx_dma_channel_id: Option<u8>,
    rx_dma_channel: Option<dma::Channel<'d>>,
    rx_dma_channel_id: Option<u8>,
    tx_dma_words: [u32; TX_DMA_WORD_CAPACITY],
    rx_dma_words: [u32; RX_DMA_WORD_CAPACITY],
    last_qpi_chunk_timing: QpiChunkTiming,
    qpi_timing_accum: QpiChunkTiming,
}

impl<'d, PIO, const SM: usize, Sio0, Sio1, Sio2, Sio3, Cs, Sck>
    EmbassyRpQpiBackend<'d, PIO, SM, Sio0, Sio1, Sio2, Sio3, Cs, Sck>
where
    PIO: Instance + 'd,
    Sio0: PioPin + 'd,
    Sio1: PioPin + 'd,
    Sio2: PioPin + 'd,
    Sio3: PioPin + 'd,
    Cs: PioPin + 'd,
    Sck: PioPin + 'd,
{
    /// Creates a concrete `embassy-rp` backend from owned PIO and GPIO resources.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        common: Common<'d, PIO>,
        state_machine: StateMachine<'d, PIO, SM>,
        sio0: Peri<'d, Sio0>,
        sio1: Peri<'d, Sio1>,
        sio2: Peri<'d, Sio2>,
        sio3: Peri<'d, Sio3>,
        cs: Peri<'d, Cs>,
        sck: Peri<'d, Sck>,
    ) -> Self {
        Self::with_timing(
            common,
            state_machine,
            sio0,
            sio1,
            sio2,
            sio3,
            cs,
            sck,
            TimingConfig::DEFAULT,
        )
    }

    /// Creates a concrete backend with explicit initial timing.
    #[allow(clippy::too_many_arguments)]
    pub fn with_timing(
        common: Common<'d, PIO>,
        state_machine: StateMachine<'d, PIO, SM>,
        sio0: Peri<'d, Sio0>,
        sio1: Peri<'d, Sio1>,
        sio2: Peri<'d, Sio2>,
        sio3: Peri<'d, Sio3>,
        cs: Peri<'d, Cs>,
        sck: Peri<'d, Sck>,
        timing: TimingConfig,
    ) -> Self {
        Self {
            common,
            state_machine,
            sio0: Some(sio0),
            sio1: Some(sio1),
            sio2: Some(sio2),
            sio3: Some(sio3),
            cs: Some(cs),
            sck: Some(sck),
            pio_sio0: None,
            pio_sio1: None,
            pio_sio2: None,
            pio_sio3: None,
            pio_cs: None,
            pio_sck: None,
            spi_command_program: None,
            qpi_command_program: None,
            qpi_write_stream_program: None,
            qpi_read_stream_program: None,
            qpi_transaction_program: None,
            qpi_transaction_fast_program: None,
            qpi_transaction_fast_program_variant: None,
            pins: None,
            timing,
            payload_transfer_path: PayloadTransferPath::ByteFallback,
            transaction_pio_fast_read_loop_variant: TransactionPioFastReadLoopVariant::default(),
            word_stream_read_diagnostics: WordStreamReadDiagnostics::default(),
            transaction_pio_diagnostics: TransactionPioDiagnostics::default(),
            tx_dma_channel: None,
            tx_dma_channel_id: None,
            rx_dma_channel: None,
            rx_dma_channel_id: None,
            tx_dma_words: [0; TX_DMA_WORD_CAPACITY],
            rx_dma_words: [0; RX_DMA_WORD_CAPACITY],
            last_qpi_chunk_timing: QpiChunkTiming::default(),
            qpi_timing_accum: QpiChunkTiming::default(),
        }
    }

    /// Attaches a diagnostic-only TX DMA channel for the transaction PIO path.
    pub fn with_tx_dma_channel(mut self, channel: dma::Channel<'d>) -> Self {
        self.tx_dma_channel = Some(channel);
        self
    }

    /// Attaches a diagnostic-only TX DMA channel with an externally known ID.
    pub fn with_tx_dma_channel_id(mut self, channel: dma::Channel<'d>, channel_id: u8) -> Self {
        self.tx_dma_channel = Some(channel);
        self.tx_dma_channel_id = Some(channel_id);
        self
    }

    /// Attaches a diagnostic-only RX DMA channel for the transaction PIO path.
    pub fn with_rx_dma_channel(mut self, channel: dma::Channel<'d>) -> Self {
        self.rx_dma_channel = Some(channel);
        self
    }

    /// Attaches a diagnostic-only RX DMA channel with an externally known ID.
    pub fn with_rx_dma_channel_id(mut self, channel: dma::Channel<'d>, channel_id: u8) -> Self {
        self.rx_dma_channel = Some(channel);
        self.rx_dma_channel_id = Some(channel_id);
        self
    }

    /// Returns the configured logical pin map, if configuration has run.
    pub const fn configured_pins(&self) -> Option<Pins> {
        self.pins
    }

    /// Returns the currently configured timing.
    pub const fn timing(&self) -> TimingConfig {
        self.timing
    }

    /// Selects a diagnostic payload transfer path for benchmark examples.
    ///
    /// This does not affect command/address/dummy handling, public bus APIs, or
    /// driver chunking. The byte path remains the default known-good path.
    #[doc(hidden)]
    pub fn set_payload_transfer_path_for_diagnostics(&mut self, path: PayloadTransferPath) {
        self.payload_transfer_path = path;
    }

    /// Selects the diagnostic-only fast transaction PIO read-loop variant.
    #[doc(hidden)]
    pub fn set_transaction_pio_fast_read_loop_variant_for_diagnostics(
        &mut self,
        variant: TransactionPioFastReadLoopVariant,
    ) {
        self.transaction_pio_fast_read_loop_variant = variant;
    }

    /// Selects diagnostic-only word-stream read settings for benchmark sweeps.
    #[doc(hidden)]
    pub fn set_word_stream_read_diagnostics(&mut self, diagnostics: WordStreamReadDiagnostics) {
        self.word_stream_read_diagnostics = WordStreamReadDiagnostics {
            batch_words: diagnostics.batch_words.clamp(1, 32),
            rx_fifo_join: diagnostics.rx_fifo_join,
        };
    }

    /// Returns the current diagnostic-only word-stream read settings.
    #[doc(hidden)]
    pub const fn word_stream_read_diagnostics(&self) -> WordStreamReadDiagnostics {
        self.word_stream_read_diagnostics
    }

    /// Returns the latest CPU-fed transaction PIO diagnostic metadata.
    #[doc(hidden)]
    pub const fn transaction_pio_diagnostics(&self) -> TransactionPioDiagnostics {
        self.transaction_pio_diagnostics
    }

    /// Runs one TX-DMA transaction PIO diagnostic read while emitting inline markers.
    #[doc(hidden)]
    pub fn read_qpi_chunk_transaction_pio_tx_dma_diagnostic_for_diagnostics<F>(
        &mut self,
        addr: crate::addr::PsramAddr,
        buf: &mut [u8],
        marker: F,
    ) -> Result<(), EmbassyRpQpiError>
    where
        F: FnMut(TransactionPioTxDmaStep),
    {
        let transaction = QpiTransaction::read(addr, buf.len(), self.timing);
        self.read_qpi_chunk_transaction_pio_tx_dma_diagnostic_inner(transaction, buf, marker)
    }

    /// Runs one RX-DMA transaction PIO diagnostic read while emitting inline markers.
    #[doc(hidden)]
    pub fn read_qpi_chunk_transaction_pio_rx_dma_diagnostic_for_diagnostics<F>(
        &mut self,
        addr: crate::addr::PsramAddr,
        buf: &mut [u8],
        marker: F,
    ) -> Result<(), EmbassyRpQpiError>
    where
        F: FnMut(TransactionPioTxDmaStep),
    {
        let transaction = QpiTransaction::read(addr, buf.len(), self.timing);
        self.read_qpi_chunk_transaction_pio_rx_dma_diagnostic_inner(transaction, buf, marker)
    }

    /// Runs one direct-to-u8 RX-DMA transaction PIO diagnostic read.
    #[doc(hidden)]
    pub fn read_qpi_chunk_transaction_pio_rx_dma_u8_direct_diagnostic_for_diagnostics<F>(
        &mut self,
        addr: crate::addr::PsramAddr,
        buf: &mut [u8],
        marker: F,
    ) -> Result<(), EmbassyRpQpiError>
    where
        F: FnMut(TransactionPioTxDmaStep),
    {
        let transaction = QpiTransaction::read(addr, buf.len(), self.timing);
        self.read_qpi_chunk_transaction_pio_rx_dma_u8_direct_diagnostic_inner(
            transaction,
            buf,
            marker,
        )
    }

    /// Runs one CPU-fed RX-byte-FIFO transaction PIO diagnostic read.
    #[doc(hidden)]
    pub fn read_qpi_chunk_transaction_pio_rx_byte_fifo_diagnostic_for_diagnostics(
        &mut self,
        addr: crate::addr::PsramAddr,
        buf: &mut [u8],
    ) -> Result<(), EmbassyRpQpiError> {
        let transaction = QpiTransaction::read(addr, buf.len(), self.timing);
        self.read_qpi_chunk_transaction_pio_rx_byte_fifo_diagnostic(transaction, buf)
    }

    /// Runs one RX-byte-FIFO RX-DMA transaction PIO diagnostic read.
    #[doc(hidden)]
    pub fn read_qpi_chunk_transaction_pio_rx_byte_fifo_rx_dma_diagnostic_for_diagnostics<F>(
        &mut self,
        addr: crate::addr::PsramAddr,
        buf: &mut [u8],
        marker: F,
    ) -> Result<(), EmbassyRpQpiError>
    where
        F: FnMut(TransactionPioTxDmaStep),
    {
        let transaction = QpiTransaction::read(addr, buf.len(), self.timing);
        self.read_qpi_chunk_transaction_pio_rx_byte_fifo_rx_dma_diagnostic_inner(
            transaction,
            buf,
            marker,
            false,
        )
    }

    /// Runs one no-delay RX-byte-FIFO RX-DMA transaction PIO diagnostic read.
    #[doc(hidden)]
    pub fn read_qpi_chunk_transaction_pio_fast_rx_byte_fifo_rx_dma_diagnostic_for_diagnostics<F>(
        &mut self,
        addr: crate::addr::PsramAddr,
        buf: &mut [u8],
        marker: F,
    ) -> Result<(), EmbassyRpQpiError>
    where
        F: FnMut(TransactionPioTxDmaStep),
    {
        let transaction = QpiTransaction::read(addr, buf.len(), self.timing);
        self.read_qpi_chunk_transaction_pio_fast_rx_byte_fifo_rx_dma_diagnostic_inner(
            transaction,
            buf,
            marker,
        )
    }

    /// Reads `buf` from `addr` using the hardware-validated FastFallingClkdiv2
    /// profile: `read_clkdiv = 2.0`, falling-edge sampling, the extra-dummy-byte
    /// alignment variant, and the RX byte-FIFO + RX-DMA-direct-to-`u8` payload
    /// path (the profile exercised by `rp2040_embassy_fast_clkdiv2_validation`).
    ///
    /// This is the single non-diagnostic entry point intended for opt-in
    /// CodeWindow refill. The caller is responsible for:
    /// * attaching the RX DMA channel as PAC channel 1
    ///   (`with_rx_dma_channel_id(.., 1)`), and
    /// * configuring fast read timing (`read_clkdiv == 2.0`, the desired
    ///   `max_chunk_len`, and `timeout_polls`) before the call.
    ///
    /// The read is split into `timing.max_chunk_len` chunks. The selected
    /// payload path is restored to the safe byte-fallback path on return so a
    /// subsequent ordinary [`PsramBus`](crate::bus::PsramBus) read/write stays on
    /// the known-good path. Returns an error (without panicking) if the address
    /// is not 16-byte aligned, the RX DMA channel is missing, or a chunk read
    /// fails; the caller is expected to handle the safe fallback.
    ///
    /// It does not change driver state machine ownership, the bus contract, or
    /// the default payload path, and is gated behind `psram_fast_read_clkdiv2`.
    #[cfg(feature = "psram_fast_read_clkdiv2")]
    #[doc(hidden)]
    pub fn read_code_window_fast_falling_clkdiv2(
        &mut self,
        addr: crate::addr::PsramAddr,
        buf: &mut [u8],
    ) -> Result<(), EmbassyRpQpiError> {
        self.transaction_pio_fast_read_loop_variant =
            TransactionPioFastReadLoopVariant::FallingExtraDummyByte;
        self.payload_transfer_path =
            PayloadTransferPath::TransactionPioFastRxByteFifoRxDmaDiagnostic;
        self.reset_qpi_timing_for_diagnostics();
        let result = self.read_code_window_fast_falling_clkdiv2_chunks(addr, buf);
        self.payload_transfer_path = PayloadTransferPath::ByteFallback;
        result
    }

    #[cfg(feature = "psram_fast_read_clkdiv2")]
    fn read_code_window_fast_falling_clkdiv2_chunks(
        &mut self,
        addr: crate::addr::PsramAddr,
        buf: &mut [u8],
    ) -> Result<(), EmbassyRpQpiError> {
        let chunk_len = self.timing.max_chunk_len;
        let mut offset = 0usize;
        while offset < buf.len() {
            let len = (buf.len() - offset).min(chunk_len);
            let advance = u32::try_from(offset).map_err(|_| PsramError::OutOfRange)?;
            let chunk_addr = addr.checked_add(advance).ok_or(PsramError::OutOfRange)?;
            let transaction = QpiTransaction::read(chunk_addr, len, self.timing);
            self.read_qpi_chunk_transaction_pio_fast_rx_byte_fifo_rx_dma_diagnostic_inner(
                transaction,
                &mut buf[offset..offset + len],
                |_| {},
            )?;
            offset += len;
        }
        Ok(())
    }

    /// Runs one combined TX-DMA + RX-DMA transaction PIO diagnostic read.
    #[doc(hidden)]
    pub fn read_qpi_chunk_transaction_pio_tx_rx_dma_diagnostic_for_diagnostics<F>(
        &mut self,
        addr: crate::addr::PsramAddr,
        buf: &mut [u8],
        marker: F,
    ) -> Result<(), EmbassyRpQpiError>
    where
        F: FnMut(TransactionPioTxDmaStep),
    {
        let transaction = QpiTransaction::read(addr, buf.len(), self.timing);
        self.read_qpi_chunk_transaction_pio_tx_rx_dma_diagnostic_inner(transaction, buf, marker)
    }

    /// Returns the diagnostic timing captured for the most recent QPI chunk.
    #[doc(hidden)]
    pub const fn last_qpi_chunk_timing_for_diagnostics(&self) -> QpiChunkTiming {
        self.last_qpi_chunk_timing
    }

    /// Clears the cumulative diagnostic QPI timing counters.
    #[doc(hidden)]
    pub fn reset_qpi_timing_for_diagnostics(&mut self) {
        self.qpi_timing_accum = QpiChunkTiming::default();
    }

    /// Returns cumulative diagnostic QPI timing since the last reset.
    #[doc(hidden)]
    pub const fn qpi_timing_for_diagnostics(&self) -> QpiChunkTiming {
        self.qpi_timing_accum
    }

    fn accumulate_last_qpi_chunk_timing(&mut self) {
        self.qpi_timing_accum.command_addr_dummy_us = self
            .qpi_timing_accum
            .command_addr_dummy_us
            .saturating_add(self.last_qpi_chunk_timing.command_addr_dummy_us);
        self.qpi_timing_accum.payload_write_us = self
            .qpi_timing_accum
            .payload_write_us
            .saturating_add(self.last_qpi_chunk_timing.payload_write_us);
        self.qpi_timing_accum.payload_read_us = self
            .qpi_timing_accum
            .payload_read_us
            .saturating_add(self.last_qpi_chunk_timing.payload_read_us);
        self.qpi_timing_accum.word_stream_rx_fifo_wait_us = self
            .qpi_timing_accum
            .word_stream_rx_fifo_wait_us
            .saturating_add(self.last_qpi_chunk_timing.word_stream_rx_fifo_wait_us);
        self.qpi_timing_accum.word_stream_rx_pull_loop_us = self
            .qpi_timing_accum
            .word_stream_rx_pull_loop_us
            .saturating_add(self.last_qpi_chunk_timing.word_stream_rx_pull_loop_us);
        self.qpi_timing_accum.word_stream_unpack_loop_us = self
            .qpi_timing_accum
            .word_stream_unpack_loop_us
            .saturating_add(self.last_qpi_chunk_timing.word_stream_unpack_loop_us);
        self.qpi_timing_accum.word_stream_tail_unpack_us = self
            .qpi_timing_accum
            .word_stream_tail_unpack_us
            .saturating_add(self.last_qpi_chunk_timing.word_stream_tail_unpack_us);
        self.qpi_timing_accum.flush_us = self
            .qpi_timing_accum
            .flush_us
            .saturating_add(self.last_qpi_chunk_timing.flush_us);
    }

    /// Releases the owned Embassy resources.
    #[allow(clippy::type_complexity)]
    pub fn into_parts(
        self,
    ) -> (
        Common<'d, PIO>,
        StateMachine<'d, PIO, SM>,
        Option<Peri<'d, Sio0>>,
        Option<Peri<'d, Sio1>>,
        Option<Peri<'d, Sio2>>,
        Option<Peri<'d, Sio3>>,
        Option<Peri<'d, Cs>>,
        Option<Peri<'d, Sck>>,
        Option<PioPinHandle<'d, PIO>>,
        Option<PioPinHandle<'d, PIO>>,
        Option<PioPinHandle<'d, PIO>>,
        Option<PioPinHandle<'d, PIO>>,
        Option<PioPinHandle<'d, PIO>>,
        Option<PioPinHandle<'d, PIO>>,
        Option<LoadedProgram<'d, PIO>>,
        Option<LoadedProgram<'d, PIO>>,
        Option<LoadedProgram<'d, PIO>>,
        Option<LoadedProgram<'d, PIO>>,
        Option<LoadedProgram<'d, PIO>>,
    ) {
        (
            self.common,
            self.state_machine,
            self.sio0,
            self.sio1,
            self.sio2,
            self.sio3,
            self.cs,
            self.sck,
            self.pio_sio0,
            self.pio_sio1,
            self.pio_sio2,
            self.pio_sio3,
            self.pio_cs,
            self.pio_sck,
            self.spi_command_program,
            self.qpi_command_program,
            self.qpi_write_stream_program,
            self.qpi_read_stream_program,
            self.qpi_transaction_program,
        )
    }

    fn pins_match(&self, pins: Pins) -> bool {
        self.sio0.as_ref().map(|pin| pin.pin()) == Some(pins.sio0)
            && self.sio1.as_ref().map(|pin| pin.pin()) == Some(pins.sio1)
            && self.sio2.as_ref().map(|pin| pin.pin()) == Some(pins.sio2)
            && self.sio3.as_ref().map(|pin| pin.pin()) == Some(pins.sio3)
            && self.cs.as_ref().map(|pin| pin.pin()) == Some(pins.cs)
            && self.sck.as_ref().map(|pin| pin.pin()) == Some(pins.sck)
    }

    fn claim_pins(&mut self, pins: Pins) -> Result<(), EmbassyRpQpiError> {
        if self.pio_sio0.is_some() {
            return Ok(());
        }

        if !self.pins_match(pins) {
            return Err(EmbassyRpQpiError::Core(PsramError::InvalidState));
        }

        self.pio_sio0 = Some(
            self.common.make_pio_pin(
                self.sio0
                    .take()
                    .ok_or(EmbassyRpQpiError::InvalidResources)?,
            ),
        );
        self.pio_sio1 = Some(
            self.common.make_pio_pin(
                self.sio1
                    .take()
                    .ok_or(EmbassyRpQpiError::InvalidResources)?,
            ),
        );
        self.pio_sio2 = Some(
            self.common.make_pio_pin(
                self.sio2
                    .take()
                    .ok_or(EmbassyRpQpiError::InvalidResources)?,
            ),
        );
        self.pio_sio3 = Some(
            self.common.make_pio_pin(
                self.sio3
                    .take()
                    .ok_or(EmbassyRpQpiError::InvalidResources)?,
            ),
        );
        self.pio_cs = Some(
            self.common
                .make_pio_pin(self.cs.take().ok_or(EmbassyRpQpiError::InvalidResources)?),
        );
        self.pio_sck = Some(
            self.common
                .make_pio_pin(self.sck.take().ok_or(EmbassyRpQpiError::InvalidResources)?),
        );

        Ok(())
    }

    fn configure_claimed_pins(&mut self) -> Result<(), EmbassyRpQpiError> {
        let sio0 = self
            .pio_sio0
            .as_mut()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        let sio1 = self
            .pio_sio1
            .as_mut()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        let sio2 = self
            .pio_sio2
            .as_mut()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        let sio3 = self
            .pio_sio3
            .as_mut()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        let cs = self
            .pio_cs
            .as_mut()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        let sck = self
            .pio_sck
            .as_mut()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;

        for pin in [
            &mut *sio0, &mut *sio1, &mut *sio2, &mut *sio3, &mut *cs, &mut *sck,
        ] {
            pin.set_drive_strength(Drive::_8mA);
            pin.set_slew_rate(SlewRate::Fast);
            pin.set_pull(Pull::None);
            pin.set_input_sync_bypass(true);
        }

        self.state_machine.set_enable(false);
        self.state_machine.clear_fifos();
        self.state_machine
            .set_pin_dirs(Direction::In, &[&*sio0, &*sio1, &*sio2, &*sio3]);
        self.state_machine
            .set_pin_dirs(Direction::Out, &[&*cs, &*sck]);
        self.state_machine.set_pins(Level::High, &[&*cs]);
        self.state_machine.set_pins(Level::Low, &[&*sck]);

        Ok(())
    }

    fn configure_spi_command_sm(&mut self) -> Result<(), EmbassyRpQpiError> {
        let program = self
            .spi_command_program
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        let sio0 = self
            .pio_sio0
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        let sio1 = self
            .pio_sio1
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        let sck = self
            .pio_sck
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;

        let mut config = PioConfig::default();
        config.use_program(program, &[sck]);
        config.set_out_pins(&[sio0]);
        config.set_in_pins(&[sio1]);
        config.shift_out.auto_fill = true;
        config.shift_out.direction = ShiftDirection::Left;
        config.shift_out.threshold = 8;
        config.shift_in.auto_fill = true;
        config.shift_in.direction = ShiftDirection::Left;
        config.shift_in.threshold = 8;
        config.clock_divider = clkdiv(self.timing.write_clkdiv).into();

        self.state_machine.set_enable(false);
        self.state_machine.set_config(&config);
        self.state_machine.restart();
        self.state_machine.clkdiv_restart();
        Ok(())
    }

    fn configure_qpi_command_sm(&mut self) -> Result<(), EmbassyRpQpiError> {
        self.configure_qpi_command_sm_with_clkdiv(self.timing.write_clkdiv)
    }

    fn configure_qpi_read_sm(&mut self) -> Result<(), EmbassyRpQpiError> {
        self.configure_qpi_command_sm_with_clkdiv(self.timing.read_clkdiv)
    }

    fn configure_qpi_command_sm_with_clkdiv(
        &mut self,
        clock_divider: f32,
    ) -> Result<(), EmbassyRpQpiError> {
        let program = self
            .qpi_command_program
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        let sio0 = self
            .pio_sio0
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        let sio1 = self
            .pio_sio1
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        let sio2 = self
            .pio_sio2
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        let sio3 = self
            .pio_sio3
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        let sck = self
            .pio_sck
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;

        let mut config = PioConfig::default();
        config.use_program(program, &[sck]);
        config.set_out_pins(&[sio0, sio1, sio2, sio3]);
        config.set_in_pins(&[sio0, sio1, sio2, sio3]);
        config.shift_out.auto_fill = true;
        config.shift_out.direction = ShiftDirection::Left;
        config.shift_out.threshold = 8;
        config.shift_in.auto_fill = true;
        config.shift_in.direction = ShiftDirection::Left;
        config.shift_in.threshold = 8;
        config.clock_divider = clkdiv(clock_divider).into();

        self.state_machine.set_enable(false);
        self.state_machine.set_config(&config);
        self.state_machine.restart();
        self.state_machine.clkdiv_restart();
        Ok(())
    }

    fn configure_qpi_write_stream_sm(&mut self) -> Result<(), EmbassyRpQpiError> {
        let program = self
            .qpi_write_stream_program
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        let sio0 = self
            .pio_sio0
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        let sio1 = self
            .pio_sio1
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        let sio2 = self
            .pio_sio2
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        let sio3 = self
            .pio_sio3
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        let sck = self
            .pio_sck
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;

        let mut config = PioConfig::default();
        config.use_program(program, &[sck]);
        config.set_out_pins(&[sio0, sio1, sio2, sio3]);
        config.shift_out.auto_fill = true;
        config.shift_out.direction = ShiftDirection::Left;
        config.shift_out.threshold = 32;
        config.clock_divider = clkdiv(self.timing.write_clkdiv).into();

        self.state_machine.set_enable(false);
        self.state_machine.set_config(&config);
        self.state_machine.restart();
        self.state_machine.clkdiv_restart();
        Ok(())
    }

    fn configure_qpi_read_stream_sm(&mut self) -> Result<(), EmbassyRpQpiError> {
        if self.word_stream_read_diagnostics.rx_fifo_join {
            return Err(EmbassyRpQpiError::Unsupported);
        }

        let program = self
            .qpi_read_stream_program
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        let sio0 = self
            .pio_sio0
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        let sio1 = self
            .pio_sio1
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        let sio2 = self
            .pio_sio2
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        let sio3 = self
            .pio_sio3
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        let sck = self
            .pio_sck
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;

        let mut config = PioConfig::default();
        config.use_program(program, &[sck]);
        config.set_in_pins(&[sio0, sio1, sio2, sio3]);
        config.shift_in.auto_fill = true;
        config.shift_in.direction = ShiftDirection::Left;
        config.shift_in.threshold = 32;
        config.clock_divider = clkdiv(self.timing.read_clkdiv).into();

        self.state_machine.set_enable(false);
        self.state_machine.set_config(&config);
        self.state_machine.restart();
        self.state_machine.clkdiv_restart();
        Ok(())
    }

    fn configure_qpi_transaction_sm(&mut self) -> Result<(), EmbassyRpQpiError> {
        self.ensure_qpi_transaction_program_loaded()?;
        let program = self
            .qpi_transaction_program
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        let sio0 = self
            .pio_sio0
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        let sio1 = self
            .pio_sio1
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        let sio2 = self
            .pio_sio2
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        let sio3 = self
            .pio_sio3
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        let cs = self
            .pio_cs
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        let sck = self
            .pio_sck
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;

        let mut config = PioConfig::default();
        config.use_program(program, &[cs, sck]);
        config.set_out_pins(&[sio0, sio1, sio2, sio3]);
        config.set_in_pins(&[sio0, sio1, sio2, sio3]);
        config.set_set_pins(&[sio0, sio1, sio2, sio3]);
        config.shift_out.auto_fill = true;
        config.shift_out.direction = ShiftDirection::Left;
        config.shift_out.threshold = 32;
        config.shift_in.auto_fill = true;
        config.shift_in.direction = ShiftDirection::Left;
        config.shift_in.threshold = 32;
        config.clock_divider = clkdiv(self.timing.read_clkdiv).into();

        self.state_machine.set_enable(false);
        self.state_machine.set_config(&config);
        self.state_machine.restart();
        self.state_machine.clkdiv_restart();
        Ok(())
    }

    fn configure_qpi_transaction_rx_byte_fifo_sm(&mut self) -> Result<(), EmbassyRpQpiError> {
        self.ensure_qpi_transaction_program_loaded()?;
        let program = self
            .qpi_transaction_program
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        let sio0 = self
            .pio_sio0
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        let sio1 = self
            .pio_sio1
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        let sio2 = self
            .pio_sio2
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        let sio3 = self
            .pio_sio3
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        let cs = self
            .pio_cs
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        let sck = self
            .pio_sck
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;

        let mut config = PioConfig::default();
        config.use_program(program, &[cs, sck]);
        config.set_out_pins(&[sio0, sio1, sio2, sio3]);
        config.set_in_pins(&[sio0, sio1, sio2, sio3]);
        config.set_set_pins(&[sio0, sio1, sio2, sio3]);
        config.shift_out.auto_fill = true;
        config.shift_out.direction = ShiftDirection::Left;
        config.shift_out.threshold = 32;
        config.shift_in.auto_fill = true;
        config.shift_in.direction = ShiftDirection::Left;
        config.shift_in.threshold = 8;
        config.clock_divider = clkdiv(self.timing.read_clkdiv).into();

        self.state_machine.set_enable(false);
        self.state_machine.set_config(&config);
        self.state_machine.restart();
        self.state_machine.clkdiv_restart();
        Ok(())
    }

    fn configure_qpi_transaction_fast_rx_byte_fifo_sm(&mut self) -> Result<(), EmbassyRpQpiError> {
        self.ensure_qpi_transaction_fast_program_loaded()?;
        let program = self
            .qpi_transaction_fast_program
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        let sio0 = self
            .pio_sio0
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        let sio1 = self
            .pio_sio1
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        let sio2 = self
            .pio_sio2
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        let sio3 = self
            .pio_sio3
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        let cs = self
            .pio_cs
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        let sck = self
            .pio_sck
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;

        let mut config = PioConfig::default();
        config.use_program(program, &[cs, sck]);
        config.set_out_pins(&[sio0, sio1, sio2, sio3]);
        config.set_in_pins(&[sio0, sio1, sio2, sio3]);
        config.set_set_pins(&[sio0, sio1, sio2, sio3]);
        config.shift_out.auto_fill = true;
        config.shift_out.direction = ShiftDirection::Left;
        config.shift_out.threshold = 32;
        config.shift_in.auto_fill = true;
        config.shift_in.direction = ShiftDirection::Left;
        config.shift_in.threshold = 8;
        config.clock_divider = clkdiv(self.timing.read_clkdiv).into();

        self.state_machine.set_enable(false);
        self.state_machine.set_config(&config);
        self.state_machine.restart();
        self.state_machine.clkdiv_restart();
        Ok(())
    }

    fn clear_fifos_restart(&mut self) {
        self.state_machine.set_enable(false);
        self.state_machine.clear_fifos();
        self.state_machine.restart();
        self.state_machine.clkdiv_restart();
    }

    fn cs_assert(&mut self) -> Result<(), EmbassyRpQpiError> {
        let cs = self
            .pio_cs
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        self.state_machine.set_pins(Level::Low, &[cs]);
        Ok(())
    }

    fn cs_deassert(&mut self) -> Result<(), EmbassyRpQpiError> {
        let cs = self
            .pio_cs
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        self.state_machine.set_pins(Level::High, &[cs]);
        Ok(())
    }

    fn set_sck_idle(&mut self) -> Result<(), EmbassyRpQpiError> {
        let sck = self
            .pio_sck
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        self.state_machine.set_pins(Level::Low, &[sck]);
        Ok(())
    }

    fn set_sio_output(&mut self) -> Result<(), EmbassyRpQpiError> {
        let sio0 = self
            .pio_sio0
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        let sio1 = self
            .pio_sio1
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        let sio2 = self
            .pio_sio2
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        let sio3 = self
            .pio_sio3
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        self.state_machine
            .set_pin_dirs(Direction::Out, &[sio0, sio1, sio2, sio3]);
        Ok(())
    }

    fn set_sio_input(&mut self) -> Result<(), EmbassyRpQpiError> {
        let sio0 = self
            .pio_sio0
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        let sio1 = self
            .pio_sio1
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        let sio2 = self
            .pio_sio2
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        let sio3 = self
            .pio_sio3
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        self.state_machine
            .set_pin_dirs(Direction::In, &[sio0, sio1, sio2, sio3]);
        Ok(())
    }

    fn set_spi_command_dirs(&mut self) -> Result<(), EmbassyRpQpiError> {
        let sio0 = self
            .pio_sio0
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        let sio1 = self
            .pio_sio1
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        let sio2 = self
            .pio_sio2
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        let sio3 = self
            .pio_sio3
            .as_ref()
            .ok_or(EmbassyRpQpiError::InvalidResources)?;
        self.state_machine.set_pin_dirs(Direction::Out, &[sio0]);
        self.state_machine
            .set_pin_dirs(Direction::In, &[sio1, sio2, sio3]);
        Ok(())
    }

    fn with_cs_asserted<T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> Result<T, EmbassyRpQpiError>,
    ) -> Result<T, EmbassyRpQpiError> {
        self.cs_assert()?;
        let result = f(self);
        let deassert = self.cs_deassert();
        match (result, deassert) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error),
        }
    }

    fn push_byte(&mut self, byte: u8) -> Result<(), EmbassyRpQpiError> {
        let word = u32::from_be_bytes([byte, 0, 0, 0]);
        for _ in 0..self.timing.timeout_polls {
            if self.state_machine.tx().try_push(word) {
                return Ok(());
            }
        }
        Err(EmbassyRpQpiError::Timeout)
    }

    fn pull_byte(&mut self) -> Result<u8, EmbassyRpQpiError> {
        for _ in 0..self.timing.timeout_polls {
            if let Some(word) = self.state_machine.rx().try_pull() {
                return Ok(word as u8);
            }
        }
        Err(EmbassyRpQpiError::Timeout)
    }

    fn transfer_byte(&mut self, byte: u8) -> Result<u8, EmbassyRpQpiError> {
        self.push_byte(byte)?;
        self.pull_byte()
    }

    fn discard_rx_byte(&mut self) -> Result<(), EmbassyRpQpiError> {
        let _discard = self.pull_byte()?;
        Ok(())
    }

    fn write_payload_polling_burst(&mut self, data: &[u8]) -> Result<(), EmbassyRpQpiError> {
        let mut pushed = 0;
        let mut discarded = 0;
        let mut polls = self.timing.timeout_polls;

        while discarded < data.len() {
            let mut made_progress = false;

            while pushed < data.len() {
                let word = u32::from_be_bytes([data[pushed], 0, 0, 0]);
                if !self.state_machine.tx().try_push(word) {
                    break;
                }
                pushed += 1;
                made_progress = true;
            }

            while discarded < pushed {
                if self.state_machine.rx().try_pull().is_none() {
                    break;
                }
                discarded += 1;
                made_progress = true;
            }

            if made_progress {
                polls = self.timing.timeout_polls;
            } else if polls == 0 {
                return Err(EmbassyRpQpiError::Timeout);
            } else {
                polls -= 1;
            }
        }

        Ok(())
    }

    fn read_payload_polling_burst(&mut self, buf: &mut [u8]) -> Result<(), EmbassyRpQpiError> {
        let mut pushed = 0;
        let mut pulled = 0;
        let mut polls = self.timing.timeout_polls;

        while pulled < buf.len() {
            let mut made_progress = false;

            while pushed < buf.len() {
                if !self.state_machine.tx().try_push(0) {
                    break;
                }
                pushed += 1;
                made_progress = true;
            }

            while pulled < pushed {
                if let Some(word) = self.state_machine.rx().try_pull() {
                    buf[pulled] = word as u8;
                    pulled += 1;
                    made_progress = true;
                } else {
                    break;
                }
            }

            if made_progress {
                polls = self.timing.timeout_polls;
            } else if polls == 0 {
                return Err(EmbassyRpQpiError::Timeout);
            } else {
                polls -= 1;
            }
        }

        Ok(())
    }

    fn push_stream_count(&mut self, byte_len: usize) -> Result<(), EmbassyRpQpiError> {
        let count = nibble_count(byte_len)
            .checked_sub(1)
            .ok_or(EmbassyRpQpiError::StreamLength)?;
        self.push_word(count as u32)
    }

    fn push_word(&mut self, word: u32) -> Result<(), EmbassyRpQpiError> {
        for _ in 0..self.timing.timeout_polls {
            if self.state_machine.tx().try_push(word) {
                return Ok(());
            }
        }
        Err(EmbassyRpQpiError::Timeout)
    }

    fn pull_word(&mut self) -> Result<u32, EmbassyRpQpiError> {
        for _ in 0..self.timing.timeout_polls {
            if let Some(word) = self.state_machine.rx().try_pull() {
                return Ok(word);
            }
        }
        Err(EmbassyRpQpiError::Timeout)
    }

    fn write_payload_word_stream_polling(&mut self, data: &[u8]) -> Result<(), EmbassyRpQpiError> {
        self.state_machine.set_enable(false);
        self.configure_qpi_write_stream_sm()?;
        self.set_sio_output()?;
        self.clear_fifos_restart();
        self.push_stream_count(data.len())?;
        self.state_machine.set_enable(true);

        let mut offset = 0;
        while offset < data.len() {
            let end = (offset + BYTES_PER_WORD).min(data.len());
            self.push_word(pack_stream_word(&data[offset..end]))?;
            offset = end;
        }

        self.flush_tx()
    }

    fn read_payload_word_stream_polling(
        &mut self,
        buf: &mut [u8],
    ) -> Result<(), EmbassyRpQpiError> {
        self.state_machine.set_enable(false);
        self.configure_qpi_read_stream_sm()?;
        self.set_sio_input()?;
        self.clear_fifos_restart();
        self.push_stream_count(buf.len())?;
        self.state_machine.set_enable(true);

        let full_words = buf.len() / BYTES_PER_WORD;
        let tail_len = buf.len() % BYTES_PER_WORD;
        let mut words_done = 0;
        let mut word_buf = [0u32; 32];
        let batch_words = self
            .word_stream_read_diagnostics
            .batch_words
            .min(word_buf.len());
        let mut saw_first_rx_word = false;

        while words_done < full_words {
            let batch_len = (full_words - words_done).min(batch_words);
            let pull_start = Instant::now();
            for slot in &mut word_buf[..batch_len] {
                if !saw_first_rx_word {
                    let wait_start = Instant::now();
                    *slot = self.pull_word()?;
                    self.last_qpi_chunk_timing.word_stream_rx_fifo_wait_us = self
                        .last_qpi_chunk_timing
                        .word_stream_rx_fifo_wait_us
                        .saturating_add(Instant::now().duration_since(wait_start).as_micros());
                    saw_first_rx_word = true;
                } else {
                    *slot = self.pull_word()?;
                }
            }
            self.last_qpi_chunk_timing.word_stream_rx_pull_loop_us = self
                .last_qpi_chunk_timing
                .word_stream_rx_pull_loop_us
                .saturating_add(Instant::now().duration_since(pull_start).as_micros());

            let unpack_start = Instant::now();
            let out = unsafe { buf.as_mut_ptr().add(words_done * BYTES_PER_WORD) };
            for (batch_offset, &word) in word_buf[..batch_len].iter().enumerate() {
                unsafe {
                    unpack_stream_word_full_unchecked(word, out.add(batch_offset * BYTES_PER_WORD));
                }
            }
            self.last_qpi_chunk_timing.word_stream_unpack_loop_us = self
                .last_qpi_chunk_timing
                .word_stream_unpack_loop_us
                .saturating_add(Instant::now().duration_since(unpack_start).as_micros());
            words_done += batch_len;
        }

        if tail_len > 0 {
            let pull_start = Instant::now();
            let word = if !saw_first_rx_word {
                let wait_start = Instant::now();
                let word = self.pull_word()?;
                self.last_qpi_chunk_timing.word_stream_rx_fifo_wait_us = self
                    .last_qpi_chunk_timing
                    .word_stream_rx_fifo_wait_us
                    .saturating_add(Instant::now().duration_since(wait_start).as_micros());
                word
            } else {
                self.pull_word()?
            };
            self.last_qpi_chunk_timing.word_stream_rx_pull_loop_us = self
                .last_qpi_chunk_timing
                .word_stream_rx_pull_loop_us
                .saturating_add(Instant::now().duration_since(pull_start).as_micros());

            let unpack_start = Instant::now();
            let offset = full_words * BYTES_PER_WORD;
            unpack_stream_word_tail(word, &mut buf[offset..offset + tail_len]);
            self.last_qpi_chunk_timing.word_stream_tail_unpack_us = self
                .last_qpi_chunk_timing
                .word_stream_tail_unpack_us
                .saturating_add(Instant::now().duration_since(unpack_start).as_micros());
        } else {
            let pull_start = Instant::now();
            let _completion = if !saw_first_rx_word {
                let wait_start = Instant::now();
                let word = self.pull_word()?;
                self.last_qpi_chunk_timing.word_stream_rx_fifo_wait_us = self
                    .last_qpi_chunk_timing
                    .word_stream_rx_fifo_wait_us
                    .saturating_add(Instant::now().duration_since(wait_start).as_micros());
                word
            } else {
                self.pull_word()?
            };
            self.last_qpi_chunk_timing.word_stream_rx_pull_loop_us = self
                .last_qpi_chunk_timing
                .word_stream_rx_pull_loop_us
                .saturating_add(Instant::now().duration_since(pull_start).as_micros());
        }

        debug_assert_eq!(read_stream_rx_words(buf.len()), full_words + 1);
        self.flush_tx()
    }

    fn finish_qpi_transaction(
        &mut self,
        mut result: Result<(), EmbassyRpQpiError>,
    ) -> Result<(), EmbassyRpQpiError> {
        self.state_machine.set_enable(false);
        if result.is_ok() {
            result = self.cs_deassert();
        } else {
            let _ = self.cs_deassert();
        }
        if result.is_ok() {
            result = self.set_sck_idle();
        } else {
            let _ = self.set_sck_idle();
        }
        if result.is_ok() {
            result = self.set_sio_input();
        } else {
            let _ = self.set_sio_input();
        }

        result
    }

    fn flush_tx(&mut self) -> Result<(), EmbassyRpQpiError> {
        for _ in 0..self.timing.timeout_polls {
            if self.state_machine.tx().empty() {
                break;
            }
        }

        for _ in 0..self.timing.timeout_polls {
            if self.state_machine.tx().stalled() {
                return Ok(());
            }
        }

        Err(EmbassyRpQpiError::Timeout)
    }

    fn send_spi_command(&mut self, command: u8) -> Result<(), EmbassyRpQpiError> {
        self.configure_spi_command_sm()?;
        self.set_spi_command_dirs()?;
        self.set_sck_idle()?;
        self.clear_fifos_restart();

        self.with_cs_asserted(|backend| {
            backend.state_machine.set_enable(true);
            let _discard = backend.transfer_byte(command)?;
            backend.flush_tx()?;
            backend.state_machine.set_enable(false);
            Ok(())
        })?;

        self.set_sck_idle()
    }

    fn send_qpi_command(&mut self, command: u8) -> Result<(), EmbassyRpQpiError> {
        self.configure_qpi_command_sm()?;
        self.set_sio_output()?;
        self.set_sck_idle()?;
        self.clear_fifos_restart();

        self.with_cs_asserted(|backend| {
            backend.state_machine.set_enable(true);
            let _discard = backend.transfer_byte(command)?;
            backend.flush_tx()?;
            backend.state_machine.set_enable(false);
            Ok(())
        })?;

        self.set_sio_input()?;
        self.set_sck_idle()
    }

    fn read_id_spi_inner(&mut self) -> Result<DeviceId, EmbassyRpQpiError> {
        self.configure_spi_command_sm()?;
        self.set_spi_command_dirs()?;
        self.set_sck_idle()?;
        self.clear_fifos_restart();

        let raw = self.with_cs_asserted(|backend| {
            backend.state_machine.set_enable(true);
            let _discard = backend.transfer_byte(crate::protocol::command::READ_ID)?;
            let raw = [
                backend.transfer_byte(0)?,
                backend.transfer_byte(0)?,
                backend.transfer_byte(0)?,
            ];
            backend.flush_tx()?;
            backend.state_machine.set_enable(false);
            Ok(raw)
        })?;

        self.set_sck_idle()?;
        Ok(DeviceId::new(raw))
    }

    fn read_qpi_chunk_inner(
        &mut self,
        transaction: QpiTransaction,
        buf: &mut [u8],
    ) -> Result<(), EmbassyRpQpiError> {
        if self.payload_transfer_path == PayloadTransferPath::TransactionPioDiagnostic {
            return self.read_qpi_chunk_transaction_pio_diagnostic(transaction, buf);
        }
        if self.payload_transfer_path == PayloadTransferPath::TransactionPioTxDmaDiagnostic {
            return self.read_qpi_chunk_transaction_pio_tx_dma_diagnostic(transaction, buf);
        }
        if self.payload_transfer_path == PayloadTransferPath::TransactionPioRxDmaDiagnostic {
            return self.read_qpi_chunk_transaction_pio_rx_dma_diagnostic(transaction, buf);
        }
        if self.payload_transfer_path == PayloadTransferPath::TransactionPioTxRxDmaDiagnostic {
            return self.read_qpi_chunk_transaction_pio_tx_rx_dma_diagnostic(transaction, buf);
        }
        if self.payload_transfer_path == PayloadTransferPath::TransactionPioRxByteFifoDiagnostic {
            return self.read_qpi_chunk_transaction_pio_rx_byte_fifo_diagnostic(transaction, buf);
        }
        if self.payload_transfer_path
            == PayloadTransferPath::TransactionPioRxByteFifoRxDmaDiagnostic
        {
            return self
                .read_qpi_chunk_transaction_pio_rx_byte_fifo_rx_dma_diagnostic(transaction, buf);
        }
        if self.payload_transfer_path
            == PayloadTransferPath::TransactionPioFastRxByteFifoRxDmaDiagnostic
        {
            return self.read_qpi_chunk_transaction_pio_fast_rx_byte_fifo_rx_dma_diagnostic(
                transaction,
                buf,
            );
        }

        self.configure_qpi_read_sm()?;
        self.set_sio_output()?;
        self.set_sck_idle()?;
        self.clear_fifos_restart();

        self.cs_assert()?;
        self.state_machine.set_enable(true);
        self.last_qpi_chunk_timing = QpiChunkTiming::default();

        let result = (|| {
            let start = Instant::now();
            self.push_byte(transaction.command)?;
            self.discard_rx_byte()?;
            for &byte in &transaction.addr {
                self.push_byte(byte)?;
                self.discard_rx_byte()?;
            }
            for _ in 0..qpi_dummy_transfer_bytes(transaction.dummy_cycles) {
                self.push_byte(0)?;
                self.discard_rx_byte()?;
            }
            self.last_qpi_chunk_timing.command_addr_dummy_us =
                Instant::now().duration_since(start).as_micros();

            self.set_sio_input()?;

            let start = Instant::now();
            match self.payload_transfer_path {
                PayloadTransferPath::ByteFallback => {
                    for byte in buf {
                        *byte = self.transfer_byte(0)?;
                    }
                }
                PayloadTransferPath::PollingBurstDiagnostic => {
                    self.read_payload_polling_burst(buf)?
                }
                PayloadTransferPath::WordStreamPolling => {
                    self.read_payload_word_stream_polling(buf)?
                }
                PayloadTransferPath::TransactionPioDiagnostic => unreachable!(),
                PayloadTransferPath::TransactionPioTxDmaDiagnostic => unreachable!(),
                PayloadTransferPath::TransactionPioRxDmaDiagnostic => unreachable!(),
                PayloadTransferPath::TransactionPioTxRxDmaDiagnostic => unreachable!(),
                PayloadTransferPath::TransactionPioRxByteFifoDiagnostic => unreachable!(),
                PayloadTransferPath::TransactionPioRxByteFifoRxDmaDiagnostic => unreachable!(),
                PayloadTransferPath::TransactionPioFastRxByteFifoRxDmaDiagnostic => unreachable!(),
                PayloadTransferPath::WordStreamDma => return Err(EmbassyRpQpiError::Unsupported),
            }
            self.last_qpi_chunk_timing.payload_read_us =
                Instant::now().duration_since(start).as_micros();

            let start = Instant::now();
            let result = self.flush_tx();
            self.last_qpi_chunk_timing.flush_us = Instant::now().duration_since(start).as_micros();
            self.accumulate_last_qpi_chunk_timing();
            result
        })();

        self.finish_qpi_transaction(result)
    }

    fn write_qpi_chunk_inner(
        &mut self,
        transaction: QpiTransaction,
        data: &[u8],
    ) -> Result<(), EmbassyRpQpiError> {
        self.configure_qpi_command_sm()?;
        self.set_sio_output()?;
        self.set_sck_idle()?;
        self.clear_fifos_restart();

        self.cs_assert()?;
        self.state_machine.set_enable(true);
        self.last_qpi_chunk_timing = QpiChunkTiming::default();

        let result = (|| {
            let start = Instant::now();
            self.push_byte(transaction.command)?;
            self.discard_rx_byte()?;
            for &byte in &transaction.addr {
                self.push_byte(byte)?;
                self.discard_rx_byte()?;
            }
            for _ in 0..qpi_dummy_transfer_bytes(transaction.dummy_cycles) {
                self.push_byte(0)?;
                self.discard_rx_byte()?;
            }
            self.last_qpi_chunk_timing.command_addr_dummy_us =
                Instant::now().duration_since(start).as_micros();

            let start = Instant::now();
            match self.payload_transfer_path {
                PayloadTransferPath::ByteFallback => {
                    for &byte in data {
                        let _discard = self.transfer_byte(byte)?;
                    }
                }
                PayloadTransferPath::PollingBurstDiagnostic => {
                    self.write_payload_polling_burst(data)?
                }
                PayloadTransferPath::WordStreamPolling => {
                    self.write_payload_word_stream_polling(data)?
                }
                PayloadTransferPath::TransactionPioDiagnostic => {
                    return Err(EmbassyRpQpiError::Unsupported)
                }
                PayloadTransferPath::TransactionPioTxDmaDiagnostic => {
                    return Err(EmbassyRpQpiError::Unsupported)
                }
                PayloadTransferPath::TransactionPioRxDmaDiagnostic => {
                    return Err(EmbassyRpQpiError::Unsupported)
                }
                PayloadTransferPath::TransactionPioTxRxDmaDiagnostic => {
                    return Err(EmbassyRpQpiError::Unsupported)
                }
                PayloadTransferPath::TransactionPioRxByteFifoDiagnostic => {
                    return Err(EmbassyRpQpiError::Unsupported)
                }
                PayloadTransferPath::TransactionPioRxByteFifoRxDmaDiagnostic => {
                    return Err(EmbassyRpQpiError::Unsupported)
                }
                PayloadTransferPath::TransactionPioFastRxByteFifoRxDmaDiagnostic => {
                    return Err(EmbassyRpQpiError::Unsupported)
                }
                PayloadTransferPath::WordStreamDma => return Err(EmbassyRpQpiError::Unsupported),
            }
            self.last_qpi_chunk_timing.payload_write_us =
                Instant::now().duration_since(start).as_micros();

            let start = Instant::now();
            let result = self.flush_tx();
            self.last_qpi_chunk_timing.flush_us = Instant::now().duration_since(start).as_micros();
            self.accumulate_last_qpi_chunk_timing();
            result
        })();

        self.finish_qpi_transaction(result)
    }
}

fn clkdiv(value: f32) -> u16 {
    if value <= 1.0 {
        1
    } else if value >= u16::MAX as f32 {
        u16::MAX
    } else {
        value as u16
    }
}

impl<'d, PIO, const SM: usize, Sio0, Sio1, Sio2, Sio3, Cs, Sck> BlockingPio
    for EmbassyRpQpiBackend<'d, PIO, SM, Sio0, Sio1, Sio2, Sio3, Cs, Sck>
where
    PIO: Instance + 'd,
    Sio0: PioPin + 'd,
    Sio1: PioPin + 'd,
    Sio2: PioPin + 'd,
    Sio3: PioPin + 'd,
    Cs: PioPin + 'd,
    Sck: PioPin + 'd,
{
    type Error = EmbassyRpQpiError;

    fn configure(&mut self, pins: Pins, timing: TimingConfig) -> Result<(), Self::Error> {
        if !pins.validate() || !timing.validate() {
            return Err(PsramError::InvalidState.into());
        }

        self.claim_pins(pins)?;
        self.load_programs()?;
        self.configure_claimed_pins()?;
        self.pins = Some(pins);
        self.timing = timing;
        Ok(())
    }

    fn exit_qpi_quad(&mut self) -> Result<(), Self::Error> {
        self.send_qpi_command(crate::protocol::command::EXIT_QPI)
    }

    fn exit_qpi_spi(&mut self) -> Result<(), Self::Error> {
        self.send_spi_command(crate::protocol::command::EXIT_QPI)
    }

    fn read_id_spi(&mut self) -> Result<DeviceId, Self::Error> {
        self.read_id_spi_inner()
    }

    fn enter_qpi_spi(&mut self) -> Result<(), Self::Error> {
        self.send_spi_command(crate::protocol::command::ENTER_QPI)
    }

    fn read_qpi_chunk(
        &mut self,
        transaction: QpiTransaction,
        buf: &mut [u8],
    ) -> Result<(), Self::Error> {
        validate_chunk::<EmbassyRpQpiError>(transaction, MemoryOp::Read, buf.len(), self.timing)?;
        self.read_qpi_chunk_inner(transaction, buf)
    }

    fn write_qpi_chunk(
        &mut self,
        transaction: QpiTransaction,
        data: &[u8],
    ) -> Result<(), Self::Error> {
        validate_chunk::<EmbassyRpQpiError>(transaction, MemoryOp::Write, data.len(), self.timing)?;
        self.write_qpi_chunk_inner(transaction, data)
    }
}
