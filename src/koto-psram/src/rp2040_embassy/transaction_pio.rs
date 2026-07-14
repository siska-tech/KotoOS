use crate::{
    pio::word_stream::{nibble_count, pack_stream_word, unpack_stream_word_full, BYTES_PER_WORD},
    protocol::QpiTransaction,
};

use embassy_rp::pio::{Instance, PioPin};
use embassy_time::Instant;

use super::{
    pac_dma, qpi_dummy_transfer_bytes, EmbassyRpQpiBackend, EmbassyRpQpiError, PayloadTransferPath,
};

#[path = "transaction_pio/diagnostics.rs"]
mod diagnostics;

pub use diagnostics::{
    PacDmaStatus, TransactionPioDiagnostics, TransactionPioTxDmaBufferDiagnostics,
    TransactionPioTxDmaStep,
};

pub(super) const TX_DMA_WORD_CAPACITY: usize = 8;
pub(super) const RX_DMA_WORD_CAPACITY: usize = 1024;

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
    pub(super) fn mark_transaction_pio_step(&mut self, step: u32) {
        self.transaction_pio_diagnostics.progress_flags |= step;
    }

    fn transaction_pio_read_diagnostics_for(
        &self,
        transaction: QpiTransaction,
        byte_len: usize,
    ) -> TransactionPioDiagnostics {
        let dummy_nibbles = qpi_dummy_transfer_bytes(transaction.dummy_cycles) * 2;
        TransactionPioDiagnostics {
            tx_dma: self.payload_transfer_path
                == PayloadTransferPath::TransactionPioTxDmaDiagnostic
                || self.payload_transfer_path
                    == PayloadTransferPath::TransactionPioTxRxDmaDiagnostic,
            rx_dma: self.payload_transfer_path
                == PayloadTransferPath::TransactionPioRxDmaDiagnostic
                || self.payload_transfer_path
                    == PayloadTransferPath::TransactionPioTxRxDmaDiagnostic
                || self.payload_transfer_path
                    == PayloadTransferPath::TransactionPioRxByteFifoRxDmaDiagnostic
                || self.payload_transfer_path
                    == PayloadTransferPath::TransactionPioFastRxByteFifoRxDmaDiagnostic,
            tx_dma_setup_us: 0,
            tx_dma_wait_us: 0,
            rx_dma_wait_us: 0,
            program_config_us: 0,
            tx_buffer_build_us: 0,
            tx_dma_arm_us: 0,
            rx_dma_arm_us: 0,
            sm_enable_to_tx_done_us: 0,
            sm_enable_to_rx_done_us: 0,
            rx_unpack_us: 0,
            cleanup_us: 0,
            total_chunk_us: 0,
            tx_buf_capacity: TX_DMA_WORD_CAPACITY,
            tx_len: 0,
            tx_dma_transfer_size_bytes: 0,
            tx_dma_count: 0,
            tx_dma_src_addr: 0,
            tx_dma_dst_addr: 0,
            tx_dma_dreq_id: 0,
            tx_dma_channel_id: self.tx_dma_channel_id,
            tx_dma_busy: false,
            tx_dma_read_error: false,
            tx_dma_write_error: false,
            tx_dma_ahb_error: false,
            rx_dma_transfer_size_bytes: 0,
            rx_dma_count: 0,
            rx_dma_src_addr: 0,
            rx_dma_dst_addr: 0,
            rx_dma_dreq_id: 0,
            rx_dma_channel_id: self.rx_dma_channel_id,
            rx_dma_busy: false,
            rx_dma_read_error: false,
            rx_dma_write_error: false,
            rx_dma_ahb_error: false,
            output_bytes: qpi_dummy_transfer_bytes(transaction.dummy_cycles)
                + 1
                + transaction.addr.len(),
            tx_buffer_overflow: false,
            output_nibbles: nibble_count(1 + transaction.addr.len()) + dummy_nibbles,
            input_nibbles: nibble_count(byte_len),
            byte_len,
            word_count: byte_len / BYTES_PER_WORD,
            progress_flags: 0,
        }
    }

    /// Builds the TX DMA transaction buffer without touching PIO or DMA hardware.
    ///
    /// This is diagnostic-only plumbing for UART breadcrumbs when the hardware
    /// path does not return far enough for post-call progress flags to be logged.
    pub fn transaction_pio_tx_dma_buffer_preflight_for_diagnostics(
        &mut self,
        addr: crate::addr::PsramAddr,
        byte_len: usize,
    ) -> TransactionPioTxDmaBufferDiagnostics {
        let transaction = QpiTransaction::read(addr, byte_len, self.timing);
        self.transaction_pio_diagnostics =
            self.transaction_pio_read_diagnostics_for(transaction, byte_len);
        self.transaction_pio_diagnostics.tx_dma = true;
        self.mark_transaction_pio_step(TransactionPioDiagnostics::STEP_START);
        let overflow = self
            .prepare_transaction_pio_read_words(transaction)
            .is_err();

        TransactionPioTxDmaBufferDiagnostics {
            diagnostics: self.transaction_pio_diagnostics,
            addr: transaction.addr,
            overflow,
        }
    }

    fn mark_tx_buffer_overflow(&mut self) -> EmbassyRpQpiError {
        self.transaction_pio_diagnostics.tx_buffer_overflow = true;
        EmbassyRpQpiError::StreamLength
    }

    fn checked_write_tx_dma_word(
        &mut self,
        index: usize,
        word: u32,
    ) -> Result<(), EmbassyRpQpiError> {
        let Some(slot) = self.tx_dma_words.get_mut(index) else {
            return Err(self.mark_tx_buffer_overflow());
        };
        *slot = word;
        if self.transaction_pio_diagnostics.tx_len <= index {
            self.transaction_pio_diagnostics.tx_len = index + 1;
        }
        Ok(())
    }

    fn prepare_transaction_pio_read_words(
        &mut self,
        transaction: QpiTransaction,
    ) -> Result<usize, EmbassyRpQpiError> {
        self.transaction_pio_diagnostics.progress_flags |=
            TransactionPioDiagnostics::STEP_BUFFER_BUILD_START;
        let output_count = self
            .transaction_pio_diagnostics
            .output_nibbles
            .checked_sub(1)
            .ok_or(EmbassyRpQpiError::StreamLength)?;
        let input_count = self
            .transaction_pio_diagnostics
            .input_nibbles
            .checked_sub(1)
            .ok_or(EmbassyRpQpiError::StreamLength)?;
        let dummy_bytes = qpi_dummy_transfer_bytes(transaction.dummy_cycles);
        let total_bytes = 1 + transaction.addr.len() + dummy_bytes;
        let stream_words = total_bytes.div_ceil(BYTES_PER_WORD);
        let word_len = 2 + stream_words;
        if word_len > self.tx_dma_words.len() {
            return Err(self.mark_tx_buffer_overflow());
        }

        self.transaction_pio_diagnostics.tx_len = 0;
        self.transaction_pio_diagnostics.progress_flags |=
            TransactionPioDiagnostics::STEP_COUNT_WRITE_START;
        self.checked_write_tx_dma_word(0, output_count as u32)?;
        self.checked_write_tx_dma_word(1, input_count as u32)?;
        self.transaction_pio_diagnostics.progress_flags |=
            TransactionPioDiagnostics::STEP_COUNT_WRITE_DONE;

        self.transaction_pio_diagnostics.progress_flags |=
            TransactionPioDiagnostics::STEP_CMD_WRITE_START;
        let mut bytes = [0u8; 8];
        bytes[0] = transaction.command;
        self.transaction_pio_diagnostics.progress_flags |=
            TransactionPioDiagnostics::STEP_CMD_WRITE_DONE;
        bytes[1..4].copy_from_slice(&transaction.addr);
        self.transaction_pio_diagnostics.progress_flags |=
            TransactionPioDiagnostics::STEP_ADDR_WRITE_DONE;
        for byte in bytes
            .iter_mut()
            .skip(1 + transaction.addr.len())
            .take(dummy_bytes)
        {
            *byte = 0;
        }
        self.transaction_pio_diagnostics.progress_flags |=
            TransactionPioDiagnostics::STEP_DUMMY_WRITE_DONE;

        let mut offset = 0;
        while offset < total_bytes {
            let end = (offset + BYTES_PER_WORD).min(total_bytes);
            self.checked_write_tx_dma_word(
                2 + offset / BYTES_PER_WORD,
                pack_stream_word(&bytes[offset..end]),
            )?;
            offset = end;
        }
        self.mark_transaction_pio_step(TransactionPioDiagnostics::STEP_BUFFER_READY);

        Ok(word_len)
    }

    fn push_transaction_pio_read_command(
        &mut self,
        transaction: QpiTransaction,
    ) -> Result<(), EmbassyRpQpiError> {
        let dummy_bytes = qpi_dummy_transfer_bytes(transaction.dummy_cycles);
        let mut bytes = [0u8; 8];
        bytes[0] = transaction.command;
        bytes[1..4].copy_from_slice(&transaction.addr);
        let total = 1 + transaction.addr.len() + dummy_bytes;

        let mut offset = 0;
        while offset < total {
            let end = (offset + BYTES_PER_WORD).min(total);
            self.push_word(pack_stream_word(&bytes[offset..end]))?;
            offset = end;
        }

        Ok(())
    }

    pub(super) fn read_qpi_chunk_transaction_pio_diagnostic(
        &mut self,
        transaction: QpiTransaction,
        buf: &mut [u8],
    ) -> Result<(), EmbassyRpQpiError> {
        let chunk_start = Instant::now();
        self.transaction_pio_diagnostics =
            self.transaction_pio_read_diagnostics_for(transaction, buf.len());
        self.mark_transaction_pio_step(TransactionPioDiagnostics::STEP_START);

        if buf.is_empty()
            || buf.len() != transaction.len
            || buf.len() > self.timing.max_chunk_len
            || buf.len() % BYTES_PER_WORD != 0
        {
            return self.finish_qpi_transaction(Err(EmbassyRpQpiError::StreamLength));
        }
        if transaction.addr[2] & 0x0f != 0 {
            return self.finish_qpi_transaction(Err(EmbassyRpQpiError::Unsupported));
        }

        self.state_machine.set_enable(false);
        let program_config_start = Instant::now();
        let setup = (|| {
            self.configure_qpi_transaction_sm()?;
            self.set_sio_output()?;
            self.set_sck_idle()?;
            self.cs_deassert()?;
            self.clear_fifos_restart();
            Ok(())
        })();
        if let Err(error) = setup {
            return self.finish_qpi_transaction(Err(error));
        }
        self.transaction_pio_diagnostics.program_config_us = Instant::now()
            .duration_since(program_config_start)
            .as_micros();
        self.mark_transaction_pio_step(TransactionPioDiagnostics::STEP_PROGRAM_CONFIG_DONE);

        let result = (|| {
            let tx_buffer_build_start = Instant::now();
            let output_nibbles = self.transaction_pio_diagnostics.output_nibbles;
            let input_nibbles = self.transaction_pio_diagnostics.input_nibbles;
            let output_count = output_nibbles
                .checked_sub(1)
                .ok_or(EmbassyRpQpiError::StreamLength)?;
            let input_count = input_nibbles
                .checked_sub(1)
                .ok_or(EmbassyRpQpiError::StreamLength)?;
            self.push_word(output_count as u32)?;
            self.push_word(input_count as u32)?;
            self.push_transaction_pio_read_command(transaction)?;
            self.mark_transaction_pio_step(TransactionPioDiagnostics::STEP_COUNT_PUSH_DONE);
            self.transaction_pio_diagnostics.tx_buffer_build_us = Instant::now()
                .duration_since(tx_buffer_build_start)
                .as_micros();

            let sm_enable_start = Instant::now();
            self.state_machine.set_enable(true);
            self.mark_transaction_pio_step(TransactionPioDiagnostics::STEP_SM_ENABLE);

            self.mark_transaction_pio_step(TransactionPioDiagnostics::STEP_RX_PULL_START);
            let word_count = self.transaction_pio_diagnostics.word_count;
            for chunk in buf.chunks_exact_mut(BYTES_PER_WORD).take(word_count) {
                let word = self.pull_word()?;
                unpack_stream_word_full(word, chunk.try_into().unwrap());
            }
            self.mark_transaction_pio_step(TransactionPioDiagnostics::STEP_RX_PULL_DONE);
            self.transaction_pio_diagnostics.sm_enable_to_rx_done_us =
                Instant::now().duration_since(sm_enable_start).as_micros();
            Ok(())
        })();

        let cleanup_start = Instant::now();
        let result = self.finish_qpi_transaction(result);
        self.transaction_pio_diagnostics.cleanup_us =
            Instant::now().duration_since(cleanup_start).as_micros();
        self.mark_transaction_pio_step(TransactionPioDiagnostics::STEP_CLEANUP_DONE);
        self.transaction_pio_diagnostics.total_chunk_us =
            Instant::now().duration_since(chunk_start).as_micros();
        result
    }

    pub(super) fn read_qpi_chunk_transaction_pio_tx_dma_diagnostic(
        &mut self,
        transaction: QpiTransaction,
        buf: &mut [u8],
    ) -> Result<(), EmbassyRpQpiError> {
        self.read_qpi_chunk_transaction_pio_tx_dma_diagnostic_inner(transaction, buf, |_| {})
    }

    pub(super) fn read_qpi_chunk_transaction_pio_tx_dma_diagnostic_inner<F>(
        &mut self,
        transaction: QpiTransaction,
        buf: &mut [u8],
        mut marker: F,
    ) -> Result<(), EmbassyRpQpiError>
    where
        F: FnMut(TransactionPioTxDmaStep),
    {
        let chunk_start = Instant::now();
        self.transaction_pio_diagnostics =
            self.transaction_pio_read_diagnostics_for(transaction, buf.len());
        self.mark_transaction_pio_step(TransactionPioDiagnostics::STEP_START);

        if buf.is_empty()
            || buf.len() != transaction.len
            || buf.len() > self.timing.max_chunk_len
            || buf.len() % BYTES_PER_WORD != 0
        {
            return self.finish_qpi_transaction(Err(EmbassyRpQpiError::StreamLength));
        }
        if transaction.addr[2] & 0x0f != 0 {
            return self.finish_qpi_transaction(Err(EmbassyRpQpiError::Unsupported));
        }
        if self.tx_dma_channel_id != Some(pac_dma::TX_DMA_PAC_CH as u8) {
            return self.finish_qpi_transaction(Err(EmbassyRpQpiError::Unsupported));
        }

        self.state_machine.set_enable(false);
        let program_config_start = Instant::now();
        let setup = (|| {
            self.configure_qpi_transaction_sm()?;
            self.set_sio_output()?;
            self.set_sck_idle()?;
            self.cs_deassert()?;
            self.clear_fifos_restart();
            Ok(())
        })();
        if let Err(error) = setup {
            return self.finish_qpi_transaction(Err(error));
        }
        self.transaction_pio_diagnostics.program_config_us = Instant::now()
            .duration_since(program_config_start)
            .as_micros();
        self.mark_transaction_pio_step(TransactionPioDiagnostics::STEP_PROGRAM_CONFIG_DONE);

        let result = (|| {
            let tx_buffer_build_start = Instant::now();
            let word_len = self.prepare_transaction_pio_read_words(transaction)?;
            self.transaction_pio_diagnostics.tx_buffer_build_us = Instant::now()
                .duration_since(tx_buffer_build_start)
                .as_micros();

            self.mark_transaction_pio_step(TransactionPioDiagnostics::STEP_DMA_CONFIG_START);
            marker(TransactionPioTxDmaStep::DmaConfigStart);
            marker(TransactionPioTxDmaStep::PacDmaStart);
            let tx_fifo = self.state_machine.tx_fifo_ptr();
            let tx_treq = self.state_machine.tx_treq();
            let src_addr = self.tx_dma_words.as_ptr() as u32;
            let dst_addr = tx_fifo as u32;
            let dreq_id = tx_treq as u8 as u32;
            self.transaction_pio_diagnostics.tx_dma_transfer_size_bytes =
                core::mem::size_of::<u32>();
            self.transaction_pio_diagnostics.tx_dma_count = word_len;
            self.transaction_pio_diagnostics.tx_dma_src_addr = src_addr;
            self.transaction_pio_diagnostics.tx_dma_dst_addr = dst_addr;
            self.transaction_pio_diagnostics.tx_dma_dreq_id = dreq_id;
            self.transaction_pio_diagnostics.tx_dma_channel_id = self.tx_dma_channel_id;
            marker(TransactionPioTxDmaStep::DmaConfig {
                tx_len: word_len,
                transfer_size: core::mem::size_of::<u32>(),
                count: word_len,
                src_addr,
                dst_addr,
                dreq_id,
                channel_id: self.tx_dma_channel_id,
            });

            let tx_dma_arm_start = Instant::now();
            let (reset_before, reset_after) =
                pac_dma::prepare_tx_dma_ch0(self.timing.timeout_polls)?;
            marker(TransactionPioTxDmaStep::PacDmaReset {
                before: reset_before,
                after: reset_after,
            });
            let ctrl_base = pac_dma::tx_dma_ch0_ctrl_base(tx_treq);

            self.mark_transaction_pio_step(
                TransactionPioDiagnostics::STEP_DMA_TRANSFER_CREATE_START,
            );
            marker(TransactionPioTxDmaStep::DmaTransferCreateStart);
            let (ctrl_before_arm, armed_status) =
                pac_dma::configure_tx_dma_ch0(src_addr, dst_addr, word_len, ctrl_base);
            self.transaction_pio_diagnostics.tx_dma_arm_us =
                Instant::now().duration_since(tx_dma_arm_start).as_micros();
            self.transaction_pio_diagnostics.tx_dma_setup_us = self
                .transaction_pio_diagnostics
                .tx_buffer_build_us
                .saturating_add(self.transaction_pio_diagnostics.tx_dma_arm_us);
            self.transaction_pio_diagnostics.progress_flags |=
                TransactionPioDiagnostics::STEP_DMA_CONFIG_DONE
                    | TransactionPioDiagnostics::STEP_DMA_TRANSFER_CREATE_DONE;
            marker(TransactionPioTxDmaStep::PacDmaConfig {
                read_addr: src_addr,
                write_addr: dst_addr,
                trans_count: word_len,
                ctrl_base,
                ctrl_before_arm,
                ctrl_trig: armed_status.ctrl_trig,
                dreq: dreq_id,
                chain_to: pac_dma::TX_DMA_PAC_CH as u8,
                en: (armed_status.ctrl_trig & 1) != 0,
                busy: armed_status.busy,
                read_error: armed_status.read_error,
                write_error: armed_status.write_error,
                ahb_error: armed_status.ahb_error,
            });
            marker(TransactionPioTxDmaStep::DmaTransferCreateDone);
            marker(TransactionPioTxDmaStep::PacDmaStarted);

            self.transaction_pio_diagnostics.progress_flags |=
                TransactionPioDiagnostics::STEP_SM_ENABLE_START;
            marker(TransactionPioTxDmaStep::SmEnableStart);
            let sm_enable_start = Instant::now();
            self.state_machine.set_enable(true);
            self.transaction_pio_diagnostics.progress_flags |=
                TransactionPioDiagnostics::STEP_SM_ENABLE;
            marker(TransactionPioTxDmaStep::SmEnableDone);

            self.transaction_pio_diagnostics.progress_flags |=
                TransactionPioDiagnostics::STEP_TX_WAIT_START;
            marker(TransactionPioTxDmaStep::TxWaitStart);
            marker(TransactionPioTxDmaStep::PacDmaWaitStart);
            let wait_start = Instant::now();
            let wait_result = pac_dma::poll_tx_dma_ch0_until_done(self.timing.timeout_polls);
            self.transaction_pio_diagnostics.tx_dma_wait_us =
                Instant::now().duration_since(wait_start).as_micros();
            let status = pac_dma::tx_dma_ch0_status();
            self.transaction_pio_diagnostics.tx_dma_busy = status.busy;
            self.transaction_pio_diagnostics.tx_dma_read_error = status.read_error;
            self.transaction_pio_diagnostics.tx_dma_write_error = status.write_error;
            self.transaction_pio_diagnostics.tx_dma_ahb_error = status.ahb_error;
            marker(TransactionPioTxDmaStep::PacDmaStatus {
                busy: status.busy,
                read_error: status.read_error,
                write_error: status.write_error,
                ahb_error: status.ahb_error,
            });
            if let Err(error) = wait_result {
                pac_dma::abort_tx_dma_ch0(self.timing.timeout_polls);
                let final_status = pac_dma::cleanup_tx_dma_ch0(self.timing.timeout_polls);
                marker(TransactionPioTxDmaStep::PacDmaCleanup { final_status });
                return Err(error);
            }
            if status.read_error || status.write_error || status.ahb_error {
                pac_dma::abort_tx_dma_ch0(self.timing.timeout_polls);
                let final_status = pac_dma::cleanup_tx_dma_ch0(self.timing.timeout_polls);
                marker(TransactionPioTxDmaStep::PacDmaCleanup { final_status });
                return Err(EmbassyRpQpiError::Timeout);
            }
            self.mark_transaction_pio_step(TransactionPioDiagnostics::STEP_TX_WAIT_DONE);
            self.transaction_pio_diagnostics.sm_enable_to_tx_done_us =
                Instant::now().duration_since(sm_enable_start).as_micros();
            marker(TransactionPioTxDmaStep::TxWaitDone);
            marker(TransactionPioTxDmaStep::PacDmaWaitDone);
            let dma_cleanup_start = Instant::now();
            let final_status = pac_dma::cleanup_tx_dma_ch0(self.timing.timeout_polls);
            self.transaction_pio_diagnostics.cleanup_us = self
                .transaction_pio_diagnostics
                .cleanup_us
                .saturating_add(Instant::now().duration_since(dma_cleanup_start).as_micros());
            marker(TransactionPioTxDmaStep::PacDmaCleanup { final_status });

            self.mark_transaction_pio_step(TransactionPioDiagnostics::STEP_RX_PULL_START);
            marker(TransactionPioTxDmaStep::RxPullStart);
            let word_count = self.transaction_pio_diagnostics.word_count;
            for chunk in buf.chunks_exact_mut(BYTES_PER_WORD).take(word_count) {
                let word = self.pull_word()?;
                unpack_stream_word_full(word, chunk.try_into().unwrap());
            }
            self.mark_transaction_pio_step(TransactionPioDiagnostics::STEP_RX_PULL_DONE);
            self.transaction_pio_diagnostics.sm_enable_to_rx_done_us =
                Instant::now().duration_since(sm_enable_start).as_micros();
            marker(TransactionPioTxDmaStep::RxPullDone);
            Ok(())
        })();

        let cleanup_start = Instant::now();
        let result = self.finish_qpi_transaction(result);
        self.transaction_pio_diagnostics.cleanup_us = self
            .transaction_pio_diagnostics
            .cleanup_us
            .saturating_add(Instant::now().duration_since(cleanup_start).as_micros());
        self.mark_transaction_pio_step(TransactionPioDiagnostics::STEP_CLEANUP_DONE);
        marker(TransactionPioTxDmaStep::CleanupDone);
        self.transaction_pio_diagnostics.total_chunk_us =
            Instant::now().duration_since(chunk_start).as_micros();
        result
    }

    pub(super) fn read_qpi_chunk_transaction_pio_rx_dma_diagnostic(
        &mut self,
        transaction: QpiTransaction,
        buf: &mut [u8],
    ) -> Result<(), EmbassyRpQpiError> {
        self.read_qpi_chunk_transaction_pio_rx_dma_diagnostic_inner(transaction, buf, |_| {})
    }

    pub(super) fn read_qpi_chunk_transaction_pio_rx_dma_diagnostic_inner<F>(
        &mut self,
        transaction: QpiTransaction,
        buf: &mut [u8],
        mut marker: F,
    ) -> Result<(), EmbassyRpQpiError>
    where
        F: FnMut(TransactionPioTxDmaStep),
    {
        let chunk_start = Instant::now();
        self.transaction_pio_diagnostics =
            self.transaction_pio_read_diagnostics_for(transaction, buf.len());
        self.transaction_pio_diagnostics.rx_dma = true;
        self.mark_transaction_pio_step(TransactionPioDiagnostics::STEP_START);

        if buf.is_empty()
            || buf.len() != transaction.len
            || buf.len() > self.timing.max_chunk_len
            || buf.len() % BYTES_PER_WORD != 0
        {
            return self.finish_qpi_transaction(Err(EmbassyRpQpiError::StreamLength));
        }
        if transaction.addr[2] & 0x0f != 0 {
            return self.finish_qpi_transaction(Err(EmbassyRpQpiError::Unsupported));
        }
        if self.rx_dma_channel_id != Some(pac_dma::RX_DMA_PAC_CH as u8) {
            return self.finish_qpi_transaction(Err(EmbassyRpQpiError::Unsupported));
        }

        let word_count = self.transaction_pio_diagnostics.word_count;
        if word_count > self.rx_dma_words.len() {
            return self.finish_qpi_transaction(Err(EmbassyRpQpiError::StreamLength));
        }
        self.rx_dma_words[..word_count].fill(0);

        self.state_machine.set_enable(false);
        let program_config_start = Instant::now();
        let setup = (|| {
            self.configure_qpi_transaction_sm()?;
            self.set_sio_output()?;
            self.set_sck_idle()?;
            self.cs_deassert()?;
            self.clear_fifos_restart();
            Ok(())
        })();
        if let Err(error) = setup {
            return self.finish_qpi_transaction(Err(error));
        }
        self.transaction_pio_diagnostics.program_config_us = Instant::now()
            .duration_since(program_config_start)
            .as_micros();
        self.mark_transaction_pio_step(TransactionPioDiagnostics::STEP_PROGRAM_CONFIG_DONE);

        let result = (|| {
            let tx_buffer_build_start = Instant::now();
            let output_nibbles = self.transaction_pio_diagnostics.output_nibbles;
            let input_nibbles = self.transaction_pio_diagnostics.input_nibbles;
            let output_count = output_nibbles
                .checked_sub(1)
                .ok_or(EmbassyRpQpiError::StreamLength)?;
            let input_count = input_nibbles
                .checked_sub(1)
                .ok_or(EmbassyRpQpiError::StreamLength)?;
            self.push_word(output_count as u32)?;
            self.push_word(input_count as u32)?;
            self.push_transaction_pio_read_command(transaction)?;
            self.mark_transaction_pio_step(TransactionPioDiagnostics::STEP_COUNT_PUSH_DONE);
            self.transaction_pio_diagnostics.tx_buffer_build_us = Instant::now()
                .duration_since(tx_buffer_build_start)
                .as_micros();

            self.mark_transaction_pio_step(TransactionPioDiagnostics::STEP_DMA_CONFIG_START);
            marker(TransactionPioTxDmaStep::RxDmaConfigStart);
            marker(TransactionPioTxDmaStep::PacRxDmaStart);
            let rx_fifo = self.state_machine.rx_fifo_ptr();
            let rx_treq = self.state_machine.rx_treq();
            let read_addr = rx_fifo as u32;
            let write_addr = self.rx_dma_words.as_mut_ptr() as u32;
            let dreq_id = rx_treq as u8 as u32;
            self.transaction_pio_diagnostics.rx_dma_transfer_size_bytes =
                core::mem::size_of::<u32>();
            self.transaction_pio_diagnostics.rx_dma_count = word_count;
            self.transaction_pio_diagnostics.rx_dma_src_addr = read_addr;
            self.transaction_pio_diagnostics.rx_dma_dst_addr = write_addr;
            self.transaction_pio_diagnostics.rx_dma_dreq_id = dreq_id;
            self.transaction_pio_diagnostics.rx_dma_channel_id = self.rx_dma_channel_id;

            let rx_dma_arm_start = Instant::now();
            let (_reset_before, _reset_after) =
                pac_dma::prepare_rx_dma_ch1(self.timing.timeout_polls)?;
            let ctrl_base = pac_dma::rx_dma_ch1_ctrl_base(rx_treq);
            let (ctrl_before_arm, armed_status) =
                pac_dma::configure_rx_dma_ch1(read_addr, write_addr, word_count, ctrl_base);
            self.transaction_pio_diagnostics.rx_dma_arm_us =
                Instant::now().duration_since(rx_dma_arm_start).as_micros();
            self.mark_transaction_pio_step(TransactionPioDiagnostics::STEP_DMA_CONFIG_DONE);
            marker(TransactionPioTxDmaStep::PacRxDmaConfig {
                read_addr,
                write_addr,
                trans_count: word_count,
                ctrl_base,
                ctrl_before_arm,
                ctrl_trig: armed_status.ctrl_trig,
                dreq: dreq_id,
                chain_to: pac_dma::RX_DMA_PAC_CH as u8,
                en: (armed_status.ctrl_trig & 1) != 0,
                busy: armed_status.busy,
                read_error: armed_status.read_error,
                write_error: armed_status.write_error,
                ahb_error: armed_status.ahb_error,
            });

            self.transaction_pio_diagnostics.progress_flags |=
                TransactionPioDiagnostics::STEP_SM_ENABLE_START;
            marker(TransactionPioTxDmaStep::SmEnableStart);
            let sm_enable_start = Instant::now();
            self.state_machine.set_enable(true);
            self.transaction_pio_diagnostics.progress_flags |=
                TransactionPioDiagnostics::STEP_SM_ENABLE;
            marker(TransactionPioTxDmaStep::SmEnableDone);

            marker(TransactionPioTxDmaStep::RxWaitStart);
            marker(TransactionPioTxDmaStep::PacRxDmaWaitStart);
            self.mark_transaction_pio_step(TransactionPioDiagnostics::STEP_RX_PULL_START);
            let wait_start = Instant::now();
            let wait_result = pac_dma::poll_rx_dma_ch1_until_done(self.timing.timeout_polls);
            self.transaction_pio_diagnostics.rx_dma_wait_us =
                Instant::now().duration_since(wait_start).as_micros();
            let status = pac_dma::rx_dma_ch1_status();
            self.transaction_pio_diagnostics.rx_dma_busy = status.busy;
            self.transaction_pio_diagnostics.rx_dma_read_error = status.read_error;
            self.transaction_pio_diagnostics.rx_dma_write_error = status.write_error;
            self.transaction_pio_diagnostics.rx_dma_ahb_error = status.ahb_error;
            marker(TransactionPioTxDmaStep::PacRxDmaStatus {
                busy: status.busy,
                read_error: status.read_error,
                write_error: status.write_error,
                ahb_error: status.ahb_error,
            });
            if let Err(error) = wait_result {
                pac_dma::abort_rx_dma_ch1(self.timing.timeout_polls);
                let final_status = pac_dma::cleanup_rx_dma_ch1(self.timing.timeout_polls);
                marker(TransactionPioTxDmaStep::PacRxDmaCleanup { final_status });
                return Err(error);
            }
            if status.read_error || status.write_error || status.ahb_error {
                pac_dma::abort_rx_dma_ch1(self.timing.timeout_polls);
                let final_status = pac_dma::cleanup_rx_dma_ch1(self.timing.timeout_polls);
                marker(TransactionPioTxDmaStep::PacRxDmaCleanup { final_status });
                return Err(EmbassyRpQpiError::Timeout);
            }
            marker(TransactionPioTxDmaStep::RxWaitDone);
            marker(TransactionPioTxDmaStep::PacRxDmaWaitDone);
            self.transaction_pio_diagnostics.sm_enable_to_rx_done_us =
                Instant::now().duration_since(sm_enable_start).as_micros();
            let dma_cleanup_start = Instant::now();
            let final_status = pac_dma::cleanup_rx_dma_ch1(self.timing.timeout_polls);
            self.transaction_pio_diagnostics.cleanup_us = self
                .transaction_pio_diagnostics
                .cleanup_us
                .saturating_add(Instant::now().duration_since(dma_cleanup_start).as_micros());
            marker(TransactionPioTxDmaStep::PacRxDmaCleanup { final_status });

            let mut first_words = [0u32; 4];
            let first_word_count = word_count.min(first_words.len());
            first_words[..first_word_count].copy_from_slice(&self.rx_dma_words[..first_word_count]);
            marker(TransactionPioTxDmaStep::RxWords {
                words: first_words,
                count: first_word_count,
            });

            let rx_unpack_start = Instant::now();
            for (word, chunk) in self.rx_dma_words[..word_count]
                .iter()
                .copied()
                .zip(buf.chunks_exact_mut(BYTES_PER_WORD))
            {
                unpack_stream_word_full(word, chunk.try_into().unwrap());
            }
            self.transaction_pio_diagnostics.rx_unpack_us =
                Instant::now().duration_since(rx_unpack_start).as_micros();
            self.mark_transaction_pio_step(TransactionPioDiagnostics::STEP_RX_PULL_DONE);
            Ok(())
        })();

        if result.is_err() {
            pac_dma::abort_rx_dma_ch1(self.timing.timeout_polls);
        }
        let cleanup_start = Instant::now();
        let result = self.finish_qpi_transaction(result);
        self.transaction_pio_diagnostics.cleanup_us = self
            .transaction_pio_diagnostics
            .cleanup_us
            .saturating_add(Instant::now().duration_since(cleanup_start).as_micros());
        self.mark_transaction_pio_step(TransactionPioDiagnostics::STEP_CLEANUP_DONE);
        marker(TransactionPioTxDmaStep::CleanupDone);
        self.transaction_pio_diagnostics.total_chunk_us =
            Instant::now().duration_since(chunk_start).as_micros();
        result
    }

    pub(super) fn read_qpi_chunk_transaction_pio_tx_rx_dma_diagnostic(
        &mut self,
        transaction: QpiTransaction,
        buf: &mut [u8],
    ) -> Result<(), EmbassyRpQpiError> {
        self.read_qpi_chunk_transaction_pio_tx_rx_dma_diagnostic_inner(transaction, buf, |_| {})
    }

    pub(super) fn read_qpi_chunk_transaction_pio_tx_rx_dma_diagnostic_inner<F>(
        &mut self,
        transaction: QpiTransaction,
        buf: &mut [u8],
        mut marker: F,
    ) -> Result<(), EmbassyRpQpiError>
    where
        F: FnMut(TransactionPioTxDmaStep),
    {
        let chunk_start = Instant::now();
        self.transaction_pio_diagnostics =
            self.transaction_pio_read_diagnostics_for(transaction, buf.len());
        self.transaction_pio_diagnostics.tx_dma = true;
        self.transaction_pio_diagnostics.rx_dma = true;
        self.mark_transaction_pio_step(TransactionPioDiagnostics::STEP_START);

        if buf.is_empty()
            || buf.len() != transaction.len
            || buf.len() > self.timing.max_chunk_len
            || buf.len() % BYTES_PER_WORD != 0
        {
            return self.finish_qpi_transaction(Err(EmbassyRpQpiError::StreamLength));
        }
        if transaction.addr[2] & 0x0f != 0 {
            return self.finish_qpi_transaction(Err(EmbassyRpQpiError::Unsupported));
        }
        if self.tx_dma_channel_id != Some(pac_dma::TX_DMA_PAC_CH as u8)
            || self.rx_dma_channel_id != Some(pac_dma::RX_DMA_PAC_CH as u8)
        {
            return self.finish_qpi_transaction(Err(EmbassyRpQpiError::Unsupported));
        }

        let word_count = self.transaction_pio_diagnostics.word_count;
        if word_count > self.rx_dma_words.len() {
            return self.finish_qpi_transaction(Err(EmbassyRpQpiError::StreamLength));
        }
        self.rx_dma_words[..word_count].fill(0);

        self.state_machine.set_enable(false);
        let program_config_start = Instant::now();
        let setup = (|| {
            self.configure_qpi_transaction_sm()?;
            self.set_sio_output()?;
            self.set_sck_idle()?;
            self.cs_deassert()?;
            self.clear_fifos_restart();
            Ok(())
        })();
        if let Err(error) = setup {
            return self.finish_qpi_transaction(Err(error));
        }
        self.transaction_pio_diagnostics.program_config_us = Instant::now()
            .duration_since(program_config_start)
            .as_micros();
        self.mark_transaction_pio_step(TransactionPioDiagnostics::STEP_PROGRAM_CONFIG_DONE);

        let result = (|| {
            let tx_buffer_build_start = Instant::now();
            let tx_word_len = self.prepare_transaction_pio_read_words(transaction)?;
            self.transaction_pio_diagnostics.tx_buffer_build_us = Instant::now()
                .duration_since(tx_buffer_build_start)
                .as_micros();

            self.mark_transaction_pio_step(TransactionPioDiagnostics::STEP_DMA_CONFIG_START);
            marker(TransactionPioTxDmaStep::RxDmaConfigStart);
            marker(TransactionPioTxDmaStep::PacRxDmaStart);
            let rx_fifo = self.state_machine.rx_fifo_ptr();
            let rx_treq = self.state_machine.rx_treq();
            let rx_read_addr = rx_fifo as u32;
            let rx_write_addr = self.rx_dma_words.as_mut_ptr() as u32;
            let rx_dreq_id = rx_treq as u8 as u32;
            self.transaction_pio_diagnostics.rx_dma_transfer_size_bytes =
                core::mem::size_of::<u32>();
            self.transaction_pio_diagnostics.rx_dma_count = word_count;
            self.transaction_pio_diagnostics.rx_dma_src_addr = rx_read_addr;
            self.transaction_pio_diagnostics.rx_dma_dst_addr = rx_write_addr;
            self.transaction_pio_diagnostics.rx_dma_dreq_id = rx_dreq_id;
            self.transaction_pio_diagnostics.rx_dma_channel_id = self.rx_dma_channel_id;

            let rx_dma_arm_start = Instant::now();
            let (_rx_reset_before, _rx_reset_after) =
                pac_dma::prepare_rx_dma_ch1(self.timing.timeout_polls)?;
            let rx_ctrl_base = pac_dma::rx_dma_ch1_ctrl_base(rx_treq);
            let (rx_ctrl_before_arm, rx_armed_status) = pac_dma::configure_rx_dma_ch1(
                rx_read_addr,
                rx_write_addr,
                word_count,
                rx_ctrl_base,
            );
            self.transaction_pio_diagnostics.rx_dma_arm_us =
                Instant::now().duration_since(rx_dma_arm_start).as_micros();
            marker(TransactionPioTxDmaStep::PacRxDmaConfig {
                read_addr: rx_read_addr,
                write_addr: rx_write_addr,
                trans_count: word_count,
                ctrl_base: rx_ctrl_base,
                ctrl_before_arm: rx_ctrl_before_arm,
                ctrl_trig: rx_armed_status.ctrl_trig,
                dreq: rx_dreq_id,
                chain_to: pac_dma::RX_DMA_PAC_CH as u8,
                en: (rx_armed_status.ctrl_trig & 1) != 0,
                busy: rx_armed_status.busy,
                read_error: rx_armed_status.read_error,
                write_error: rx_armed_status.write_error,
                ahb_error: rx_armed_status.ahb_error,
            });

            marker(TransactionPioTxDmaStep::DmaConfigStart);
            marker(TransactionPioTxDmaStep::PacDmaStart);
            let tx_fifo = self.state_machine.tx_fifo_ptr();
            let tx_treq = self.state_machine.tx_treq();
            let tx_src_addr = self.tx_dma_words.as_ptr() as u32;
            let tx_dst_addr = tx_fifo as u32;
            let tx_dreq_id = tx_treq as u8 as u32;
            self.transaction_pio_diagnostics.tx_dma_transfer_size_bytes =
                core::mem::size_of::<u32>();
            self.transaction_pio_diagnostics.tx_dma_count = tx_word_len;
            self.transaction_pio_diagnostics.tx_dma_src_addr = tx_src_addr;
            self.transaction_pio_diagnostics.tx_dma_dst_addr = tx_dst_addr;
            self.transaction_pio_diagnostics.tx_dma_dreq_id = tx_dreq_id;
            self.transaction_pio_diagnostics.tx_dma_channel_id = self.tx_dma_channel_id;
            marker(TransactionPioTxDmaStep::DmaConfig {
                tx_len: tx_word_len,
                transfer_size: core::mem::size_of::<u32>(),
                count: tx_word_len,
                src_addr: tx_src_addr,
                dst_addr: tx_dst_addr,
                dreq_id: tx_dreq_id,
                channel_id: self.tx_dma_channel_id,
            });

            let tx_dma_arm_start = Instant::now();
            let (tx_reset_before, tx_reset_after) =
                pac_dma::prepare_tx_dma_ch0(self.timing.timeout_polls)?;
            marker(TransactionPioTxDmaStep::PacDmaReset {
                before: tx_reset_before,
                after: tx_reset_after,
            });
            let tx_ctrl_base = pac_dma::tx_dma_ch0_ctrl_base(tx_treq);

            self.mark_transaction_pio_step(
                TransactionPioDiagnostics::STEP_DMA_TRANSFER_CREATE_START,
            );
            marker(TransactionPioTxDmaStep::DmaTransferCreateStart);
            let (tx_ctrl_before_arm, tx_armed_status) =
                pac_dma::configure_tx_dma_ch0(tx_src_addr, tx_dst_addr, tx_word_len, tx_ctrl_base);
            self.transaction_pio_diagnostics.tx_dma_arm_us =
                Instant::now().duration_since(tx_dma_arm_start).as_micros();
            self.transaction_pio_diagnostics.tx_dma_setup_us = self
                .transaction_pio_diagnostics
                .tx_buffer_build_us
                .saturating_add(self.transaction_pio_diagnostics.tx_dma_arm_us);
            self.transaction_pio_diagnostics.progress_flags |=
                TransactionPioDiagnostics::STEP_DMA_CONFIG_DONE
                    | TransactionPioDiagnostics::STEP_DMA_TRANSFER_CREATE_DONE;
            marker(TransactionPioTxDmaStep::PacDmaConfig {
                read_addr: tx_src_addr,
                write_addr: tx_dst_addr,
                trans_count: tx_word_len,
                ctrl_base: tx_ctrl_base,
                ctrl_before_arm: tx_ctrl_before_arm,
                ctrl_trig: tx_armed_status.ctrl_trig,
                dreq: tx_dreq_id,
                chain_to: pac_dma::TX_DMA_PAC_CH as u8,
                en: (tx_armed_status.ctrl_trig & 1) != 0,
                busy: tx_armed_status.busy,
                read_error: tx_armed_status.read_error,
                write_error: tx_armed_status.write_error,
                ahb_error: tx_armed_status.ahb_error,
            });
            marker(TransactionPioTxDmaStep::DmaTransferCreateDone);
            marker(TransactionPioTxDmaStep::PacDmaStarted);

            self.transaction_pio_diagnostics.progress_flags |=
                TransactionPioDiagnostics::STEP_SM_ENABLE_START;
            marker(TransactionPioTxDmaStep::SmEnableStart);
            let sm_enable_start = Instant::now();
            self.state_machine.set_enable(true);
            self.transaction_pio_diagnostics.progress_flags |=
                TransactionPioDiagnostics::STEP_SM_ENABLE;
            marker(TransactionPioTxDmaStep::SmEnableDone);

            self.transaction_pio_diagnostics.progress_flags |=
                TransactionPioDiagnostics::STEP_TX_WAIT_START;
            marker(TransactionPioTxDmaStep::TxWaitStart);
            marker(TransactionPioTxDmaStep::PacDmaWaitStart);
            let tx_wait_start = Instant::now();
            let tx_wait_result = pac_dma::poll_tx_dma_ch0_until_done(self.timing.timeout_polls);
            self.transaction_pio_diagnostics.tx_dma_wait_us =
                Instant::now().duration_since(tx_wait_start).as_micros();
            let tx_status = pac_dma::tx_dma_ch0_status();
            self.transaction_pio_diagnostics.tx_dma_busy = tx_status.busy;
            self.transaction_pio_diagnostics.tx_dma_read_error = tx_status.read_error;
            self.transaction_pio_diagnostics.tx_dma_write_error = tx_status.write_error;
            self.transaction_pio_diagnostics.tx_dma_ahb_error = tx_status.ahb_error;
            marker(TransactionPioTxDmaStep::PacDmaStatus {
                busy: tx_status.busy,
                read_error: tx_status.read_error,
                write_error: tx_status.write_error,
                ahb_error: tx_status.ahb_error,
            });
            if let Err(error) = tx_wait_result {
                pac_dma::abort_tx_dma_ch0(self.timing.timeout_polls);
                pac_dma::abort_rx_dma_ch1(self.timing.timeout_polls);
                let tx_final_status = pac_dma::cleanup_tx_dma_ch0(self.timing.timeout_polls);
                let rx_final_status = pac_dma::cleanup_rx_dma_ch1(self.timing.timeout_polls);
                marker(TransactionPioTxDmaStep::PacDmaCleanup {
                    final_status: tx_final_status,
                });
                marker(TransactionPioTxDmaStep::PacRxDmaCleanup {
                    final_status: rx_final_status,
                });
                return Err(error);
            }
            if tx_status.read_error || tx_status.write_error || tx_status.ahb_error {
                pac_dma::abort_tx_dma_ch0(self.timing.timeout_polls);
                pac_dma::abort_rx_dma_ch1(self.timing.timeout_polls);
                let tx_final_status = pac_dma::cleanup_tx_dma_ch0(self.timing.timeout_polls);
                let rx_final_status = pac_dma::cleanup_rx_dma_ch1(self.timing.timeout_polls);
                marker(TransactionPioTxDmaStep::PacDmaCleanup {
                    final_status: tx_final_status,
                });
                marker(TransactionPioTxDmaStep::PacRxDmaCleanup {
                    final_status: rx_final_status,
                });
                return Err(EmbassyRpQpiError::Timeout);
            }
            self.mark_transaction_pio_step(TransactionPioDiagnostics::STEP_TX_WAIT_DONE);
            self.transaction_pio_diagnostics.sm_enable_to_tx_done_us =
                Instant::now().duration_since(sm_enable_start).as_micros();
            marker(TransactionPioTxDmaStep::TxWaitDone);
            marker(TransactionPioTxDmaStep::PacDmaWaitDone);

            marker(TransactionPioTxDmaStep::RxWaitStart);
            marker(TransactionPioTxDmaStep::PacRxDmaWaitStart);
            self.mark_transaction_pio_step(TransactionPioDiagnostics::STEP_RX_PULL_START);
            let rx_wait_start = Instant::now();
            let rx_wait_result = pac_dma::poll_rx_dma_ch1_until_done(self.timing.timeout_polls);
            self.transaction_pio_diagnostics.rx_dma_wait_us =
                Instant::now().duration_since(rx_wait_start).as_micros();
            let rx_status = pac_dma::rx_dma_ch1_status();
            self.transaction_pio_diagnostics.rx_dma_busy = rx_status.busy;
            self.transaction_pio_diagnostics.rx_dma_read_error = rx_status.read_error;
            self.transaction_pio_diagnostics.rx_dma_write_error = rx_status.write_error;
            self.transaction_pio_diagnostics.rx_dma_ahb_error = rx_status.ahb_error;
            marker(TransactionPioTxDmaStep::PacRxDmaStatus {
                busy: rx_status.busy,
                read_error: rx_status.read_error,
                write_error: rx_status.write_error,
                ahb_error: rx_status.ahb_error,
            });
            if let Err(error) = rx_wait_result {
                pac_dma::abort_tx_dma_ch0(self.timing.timeout_polls);
                pac_dma::abort_rx_dma_ch1(self.timing.timeout_polls);
                let tx_final_status = pac_dma::cleanup_tx_dma_ch0(self.timing.timeout_polls);
                let rx_final_status = pac_dma::cleanup_rx_dma_ch1(self.timing.timeout_polls);
                marker(TransactionPioTxDmaStep::PacDmaCleanup {
                    final_status: tx_final_status,
                });
                marker(TransactionPioTxDmaStep::PacRxDmaCleanup {
                    final_status: rx_final_status,
                });
                return Err(error);
            }
            if rx_status.read_error || rx_status.write_error || rx_status.ahb_error {
                pac_dma::abort_tx_dma_ch0(self.timing.timeout_polls);
                pac_dma::abort_rx_dma_ch1(self.timing.timeout_polls);
                let tx_final_status = pac_dma::cleanup_tx_dma_ch0(self.timing.timeout_polls);
                let rx_final_status = pac_dma::cleanup_rx_dma_ch1(self.timing.timeout_polls);
                marker(TransactionPioTxDmaStep::PacDmaCleanup {
                    final_status: tx_final_status,
                });
                marker(TransactionPioTxDmaStep::PacRxDmaCleanup {
                    final_status: rx_final_status,
                });
                return Err(EmbassyRpQpiError::Timeout);
            }
            marker(TransactionPioTxDmaStep::RxWaitDone);
            marker(TransactionPioTxDmaStep::PacRxDmaWaitDone);
            self.transaction_pio_diagnostics.sm_enable_to_rx_done_us =
                Instant::now().duration_since(sm_enable_start).as_micros();

            let mut first_words = [0u32; 4];
            let first_word_count = word_count.min(first_words.len());
            first_words[..first_word_count].copy_from_slice(&self.rx_dma_words[..first_word_count]);
            marker(TransactionPioTxDmaStep::RxWords {
                words: first_words,
                count: first_word_count,
            });

            let rx_unpack_start = Instant::now();
            for (word, chunk) in self.rx_dma_words[..word_count]
                .iter()
                .copied()
                .zip(buf.chunks_exact_mut(BYTES_PER_WORD))
            {
                unpack_stream_word_full(word, chunk.try_into().unwrap());
            }
            self.transaction_pio_diagnostics.rx_unpack_us =
                Instant::now().duration_since(rx_unpack_start).as_micros();
            self.mark_transaction_pio_step(TransactionPioDiagnostics::STEP_RX_PULL_DONE);
            marker(TransactionPioTxDmaStep::RxPullDone);

            let dma_cleanup_start = Instant::now();
            let tx_final_status = pac_dma::cleanup_tx_dma_ch0(self.timing.timeout_polls);
            let rx_final_status = pac_dma::cleanup_rx_dma_ch1(self.timing.timeout_polls);
            self.transaction_pio_diagnostics.cleanup_us = self
                .transaction_pio_diagnostics
                .cleanup_us
                .saturating_add(Instant::now().duration_since(dma_cleanup_start).as_micros());
            marker(TransactionPioTxDmaStep::PacDmaCleanup {
                final_status: tx_final_status,
            });
            marker(TransactionPioTxDmaStep::PacRxDmaCleanup {
                final_status: rx_final_status,
            });
            Ok(())
        })();

        if result.is_err() {
            let tx_status = pac_dma::tx_dma_ch0_status();
            self.transaction_pio_diagnostics.tx_dma_busy = tx_status.busy;
            self.transaction_pio_diagnostics.tx_dma_read_error = tx_status.read_error;
            self.transaction_pio_diagnostics.tx_dma_write_error = tx_status.write_error;
            self.transaction_pio_diagnostics.tx_dma_ahb_error = tx_status.ahb_error;
            let rx_status = pac_dma::rx_dma_ch1_status();
            self.transaction_pio_diagnostics.rx_dma_busy = rx_status.busy;
            self.transaction_pio_diagnostics.rx_dma_read_error = rx_status.read_error;
            self.transaction_pio_diagnostics.rx_dma_write_error = rx_status.write_error;
            self.transaction_pio_diagnostics.rx_dma_ahb_error = rx_status.ahb_error;
            pac_dma::abort_tx_dma_ch0(self.timing.timeout_polls);
            pac_dma::abort_rx_dma_ch1(self.timing.timeout_polls);
        }
        let cleanup_start = Instant::now();
        let result = self.finish_qpi_transaction(result);
        self.transaction_pio_diagnostics.cleanup_us = self
            .transaction_pio_diagnostics
            .cleanup_us
            .saturating_add(Instant::now().duration_since(cleanup_start).as_micros());
        self.mark_transaction_pio_step(TransactionPioDiagnostics::STEP_CLEANUP_DONE);
        marker(TransactionPioTxDmaStep::CleanupDone);
        self.transaction_pio_diagnostics.total_chunk_us =
            Instant::now().duration_since(chunk_start).as_micros();
        result
    }

    pub(super) fn read_qpi_chunk_transaction_pio_rx_dma_u8_direct_diagnostic_inner<F>(
        &mut self,
        transaction: QpiTransaction,
        buf: &mut [u8],
        mut marker: F,
    ) -> Result<(), EmbassyRpQpiError>
    where
        F: FnMut(TransactionPioTxDmaStep),
    {
        let chunk_start = Instant::now();
        self.transaction_pio_diagnostics =
            self.transaction_pio_read_diagnostics_for(transaction, buf.len());
        self.transaction_pio_diagnostics.rx_dma = true;
        self.mark_transaction_pio_step(TransactionPioDiagnostics::STEP_START);

        if buf.is_empty()
            || buf.len() != transaction.len
            || buf.len() > self.timing.max_chunk_len
            || buf.len() % BYTES_PER_WORD != 0
        {
            return self.finish_qpi_transaction(Err(EmbassyRpQpiError::StreamLength));
        }
        if transaction.addr[2] & 0x0f != 0 {
            return self.finish_qpi_transaction(Err(EmbassyRpQpiError::Unsupported));
        }
        if self.rx_dma_channel_id != Some(pac_dma::RX_DMA_PAC_CH as u8) {
            return self.finish_qpi_transaction(Err(EmbassyRpQpiError::Unsupported));
        }

        buf.fill(0);

        self.state_machine.set_enable(false);
        let program_config_start = Instant::now();
        let setup = (|| {
            self.configure_qpi_transaction_sm()?;
            self.set_sio_output()?;
            self.set_sck_idle()?;
            self.cs_deassert()?;
            self.clear_fifos_restart();
            Ok(())
        })();
        if let Err(error) = setup {
            return self.finish_qpi_transaction(Err(error));
        }
        self.transaction_pio_diagnostics.program_config_us = Instant::now()
            .duration_since(program_config_start)
            .as_micros();
        self.mark_transaction_pio_step(TransactionPioDiagnostics::STEP_PROGRAM_CONFIG_DONE);

        let result = (|| {
            let tx_buffer_build_start = Instant::now();
            let output_nibbles = self.transaction_pio_diagnostics.output_nibbles;
            let input_nibbles = self.transaction_pio_diagnostics.input_nibbles;
            let output_count = output_nibbles
                .checked_sub(1)
                .ok_or(EmbassyRpQpiError::StreamLength)?;
            let input_count = input_nibbles
                .checked_sub(1)
                .ok_or(EmbassyRpQpiError::StreamLength)?;
            self.push_word(output_count as u32)?;
            self.push_word(input_count as u32)?;
            self.push_transaction_pio_read_command(transaction)?;
            self.mark_transaction_pio_step(TransactionPioDiagnostics::STEP_COUNT_PUSH_DONE);
            self.transaction_pio_diagnostics.tx_buffer_build_us = Instant::now()
                .duration_since(tx_buffer_build_start)
                .as_micros();

            self.mark_transaction_pio_step(TransactionPioDiagnostics::STEP_DMA_CONFIG_START);
            marker(TransactionPioTxDmaStep::RxDmaConfigStart);
            marker(TransactionPioTxDmaStep::PacRxDmaStart);
            let rx_fifo = self.state_machine.rx_fifo_ptr();
            let rx_treq = self.state_machine.rx_treq();
            let read_addr = rx_fifo as u32;
            let write_addr = buf.as_mut_ptr() as u32;
            let dreq_id = rx_treq as u8 as u32;
            self.transaction_pio_diagnostics.rx_dma_transfer_size_bytes =
                core::mem::size_of::<u8>();
            self.transaction_pio_diagnostics.rx_dma_count = buf.len();
            self.transaction_pio_diagnostics.rx_dma_src_addr = read_addr;
            self.transaction_pio_diagnostics.rx_dma_dst_addr = write_addr;
            self.transaction_pio_diagnostics.rx_dma_dreq_id = dreq_id;
            self.transaction_pio_diagnostics.rx_dma_channel_id = self.rx_dma_channel_id;

            let rx_dma_arm_start = Instant::now();
            let (_reset_before, _reset_after) =
                pac_dma::prepare_rx_dma_ch1(self.timing.timeout_polls)?;
            let ctrl_base = pac_dma::rx_dma_ch1_u8_ctrl_base(rx_treq);
            let (ctrl_before_arm, armed_status) =
                pac_dma::configure_rx_dma_ch1(read_addr, write_addr, buf.len(), ctrl_base);
            self.transaction_pio_diagnostics.rx_dma_arm_us =
                Instant::now().duration_since(rx_dma_arm_start).as_micros();
            self.mark_transaction_pio_step(TransactionPioDiagnostics::STEP_DMA_CONFIG_DONE);
            marker(TransactionPioTxDmaStep::PacRxDmaConfig {
                read_addr,
                write_addr,
                trans_count: buf.len(),
                ctrl_base,
                ctrl_before_arm,
                ctrl_trig: armed_status.ctrl_trig,
                dreq: dreq_id,
                chain_to: pac_dma::RX_DMA_PAC_CH as u8,
                en: (armed_status.ctrl_trig & 1) != 0,
                busy: armed_status.busy,
                read_error: armed_status.read_error,
                write_error: armed_status.write_error,
                ahb_error: armed_status.ahb_error,
            });

            self.transaction_pio_diagnostics.progress_flags |=
                TransactionPioDiagnostics::STEP_SM_ENABLE_START;
            marker(TransactionPioTxDmaStep::SmEnableStart);
            let sm_enable_start = Instant::now();
            self.state_machine.set_enable(true);
            self.transaction_pio_diagnostics.progress_flags |=
                TransactionPioDiagnostics::STEP_SM_ENABLE;
            marker(TransactionPioTxDmaStep::SmEnableDone);

            marker(TransactionPioTxDmaStep::RxWaitStart);
            marker(TransactionPioTxDmaStep::PacRxDmaWaitStart);
            self.mark_transaction_pio_step(TransactionPioDiagnostics::STEP_RX_PULL_START);
            let wait_start = Instant::now();
            let wait_result = pac_dma::poll_rx_dma_ch1_until_done(self.timing.timeout_polls);
            self.transaction_pio_diagnostics.rx_dma_wait_us =
                Instant::now().duration_since(wait_start).as_micros();
            let status = pac_dma::rx_dma_ch1_status();
            self.transaction_pio_diagnostics.rx_dma_busy = status.busy;
            self.transaction_pio_diagnostics.rx_dma_read_error = status.read_error;
            self.transaction_pio_diagnostics.rx_dma_write_error = status.write_error;
            self.transaction_pio_diagnostics.rx_dma_ahb_error = status.ahb_error;
            marker(TransactionPioTxDmaStep::PacRxDmaStatus {
                busy: status.busy,
                read_error: status.read_error,
                write_error: status.write_error,
                ahb_error: status.ahb_error,
            });
            if let Err(error) = wait_result {
                pac_dma::abort_rx_dma_ch1(self.timing.timeout_polls);
                let final_status = pac_dma::disable_rx_dma_ch1_now();
                marker(TransactionPioTxDmaStep::PacRxDmaCleanup { final_status });
                return Err(error);
            }
            if status.read_error || status.write_error || status.ahb_error {
                pac_dma::abort_rx_dma_ch1(self.timing.timeout_polls);
                let final_status = pac_dma::disable_rx_dma_ch1_now();
                marker(TransactionPioTxDmaStep::PacRxDmaCleanup { final_status });
                return Err(EmbassyRpQpiError::Timeout);
            }
            marker(TransactionPioTxDmaStep::RxWaitDone);
            marker(TransactionPioTxDmaStep::PacRxDmaWaitDone);
            self.transaction_pio_diagnostics.sm_enable_to_rx_done_us =
                Instant::now().duration_since(sm_enable_start).as_micros();
            self.mark_transaction_pio_step(TransactionPioDiagnostics::STEP_RX_PULL_DONE);

            let dma_cleanup_start = Instant::now();
            let final_status = pac_dma::cleanup_rx_dma_ch1(self.timing.timeout_polls);
            self.transaction_pio_diagnostics.cleanup_us = self
                .transaction_pio_diagnostics
                .cleanup_us
                .saturating_add(Instant::now().duration_since(dma_cleanup_start).as_micros());
            marker(TransactionPioTxDmaStep::PacRxDmaCleanup { final_status });
            Ok(())
        })();

        if result.is_err() {
            let status = pac_dma::rx_dma_ch1_status();
            self.transaction_pio_diagnostics.rx_dma_busy = status.busy;
            self.transaction_pio_diagnostics.rx_dma_read_error = status.read_error;
            self.transaction_pio_diagnostics.rx_dma_write_error = status.write_error;
            self.transaction_pio_diagnostics.rx_dma_ahb_error = status.ahb_error;
            pac_dma::abort_rx_dma_ch1(self.timing.timeout_polls);
            pac_dma::disable_rx_dma_ch1_now();
        }
        let cleanup_start = Instant::now();
        let result = self.finish_qpi_transaction(result);
        self.transaction_pio_diagnostics.cleanup_us = self
            .transaction_pio_diagnostics
            .cleanup_us
            .saturating_add(Instant::now().duration_since(cleanup_start).as_micros());
        self.mark_transaction_pio_step(TransactionPioDiagnostics::STEP_CLEANUP_DONE);
        marker(TransactionPioTxDmaStep::CleanupDone);
        self.transaction_pio_diagnostics.total_chunk_us =
            Instant::now().duration_since(chunk_start).as_micros();
        result
    }

    pub(super) fn read_qpi_chunk_transaction_pio_rx_byte_fifo_diagnostic(
        &mut self,
        transaction: QpiTransaction,
        buf: &mut [u8],
    ) -> Result<(), EmbassyRpQpiError> {
        let chunk_start = Instant::now();
        self.transaction_pio_diagnostics =
            self.transaction_pio_read_diagnostics_for(transaction, buf.len());
        self.mark_transaction_pio_step(TransactionPioDiagnostics::STEP_START);

        if buf.is_empty() || buf.len() != transaction.len || buf.len() > self.timing.max_chunk_len {
            return self.finish_qpi_transaction(Err(EmbassyRpQpiError::StreamLength));
        }
        if transaction.addr[2] & 0x0f != 0 {
            return self.finish_qpi_transaction(Err(EmbassyRpQpiError::Unsupported));
        }

        self.state_machine.set_enable(false);
        let program_config_start = Instant::now();
        let setup = (|| {
            self.configure_qpi_transaction_rx_byte_fifo_sm()?;
            self.set_sio_output()?;
            self.set_sck_idle()?;
            self.cs_deassert()?;
            self.clear_fifos_restart();
            Ok(())
        })();
        if let Err(error) = setup {
            return self.finish_qpi_transaction(Err(error));
        }
        self.transaction_pio_diagnostics.program_config_us = Instant::now()
            .duration_since(program_config_start)
            .as_micros();
        self.mark_transaction_pio_step(TransactionPioDiagnostics::STEP_PROGRAM_CONFIG_DONE);

        let result = (|| {
            let tx_buffer_build_start = Instant::now();
            let output_nibbles = self.transaction_pio_diagnostics.output_nibbles;
            let input_nibbles = self.transaction_pio_diagnostics.input_nibbles;
            let output_count = output_nibbles
                .checked_sub(1)
                .ok_or(EmbassyRpQpiError::StreamLength)?;
            let input_count = input_nibbles
                .checked_sub(1)
                .ok_or(EmbassyRpQpiError::StreamLength)?;
            self.push_word(output_count as u32)?;
            self.push_word(input_count as u32)?;
            self.push_transaction_pio_read_command(transaction)?;
            self.mark_transaction_pio_step(TransactionPioDiagnostics::STEP_COUNT_PUSH_DONE);
            self.transaction_pio_diagnostics.tx_buffer_build_us = Instant::now()
                .duration_since(tx_buffer_build_start)
                .as_micros();

            let sm_enable_start = Instant::now();
            self.state_machine.set_enable(true);
            self.mark_transaction_pio_step(TransactionPioDiagnostics::STEP_SM_ENABLE);

            self.mark_transaction_pio_step(TransactionPioDiagnostics::STEP_RX_PULL_START);
            for byte in buf.iter_mut() {
                *byte = self.pull_word()? as u8;
            }
            self.mark_transaction_pio_step(TransactionPioDiagnostics::STEP_RX_PULL_DONE);
            self.transaction_pio_diagnostics.sm_enable_to_rx_done_us =
                Instant::now().duration_since(sm_enable_start).as_micros();
            Ok(())
        })();

        let cleanup_start = Instant::now();
        let result = self.finish_qpi_transaction(result);
        self.transaction_pio_diagnostics.cleanup_us =
            Instant::now().duration_since(cleanup_start).as_micros();
        self.mark_transaction_pio_step(TransactionPioDiagnostics::STEP_CLEANUP_DONE);
        self.transaction_pio_diagnostics.total_chunk_us =
            Instant::now().duration_since(chunk_start).as_micros();
        result
    }

    pub(super) fn read_qpi_chunk_transaction_pio_rx_byte_fifo_rx_dma_diagnostic(
        &mut self,
        transaction: QpiTransaction,
        buf: &mut [u8],
    ) -> Result<(), EmbassyRpQpiError> {
        self.read_qpi_chunk_transaction_pio_rx_byte_fifo_rx_dma_diagnostic_inner(
            transaction,
            buf,
            |_| {},
            false,
        )
    }

    pub(super) fn read_qpi_chunk_transaction_pio_rx_byte_fifo_rx_dma_diagnostic_inner<F>(
        &mut self,
        transaction: QpiTransaction,
        buf: &mut [u8],
        mut marker: F,
        fast_pio: bool,
    ) -> Result<(), EmbassyRpQpiError>
    where
        F: FnMut(TransactionPioTxDmaStep),
    {
        let chunk_start = Instant::now();
        self.transaction_pio_diagnostics =
            self.transaction_pio_read_diagnostics_for(transaction, buf.len());
        self.transaction_pio_diagnostics.rx_dma = true;
        self.mark_transaction_pio_step(TransactionPioDiagnostics::STEP_START);

        if buf.is_empty() || buf.len() != transaction.len || buf.len() > self.timing.max_chunk_len {
            return self.finish_qpi_transaction(Err(EmbassyRpQpiError::StreamLength));
        }
        if transaction.addr[2] & 0x0f != 0 {
            return self.finish_qpi_transaction(Err(EmbassyRpQpiError::Unsupported));
        }
        if self.rx_dma_channel_id != Some(pac_dma::RX_DMA_PAC_CH as u8) {
            return self.finish_qpi_transaction(Err(EmbassyRpQpiError::Unsupported));
        }

        buf.fill(0);

        self.state_machine.set_enable(false);
        let program_config_start = Instant::now();
        let setup = (|| {
            if fast_pio {
                self.configure_qpi_transaction_fast_rx_byte_fifo_sm()?;
            } else {
                self.configure_qpi_transaction_rx_byte_fifo_sm()?;
            }
            self.set_sio_output()?;
            self.set_sck_idle()?;
            self.cs_deassert()?;
            self.clear_fifos_restart();
            Ok(())
        })();
        if let Err(error) = setup {
            return self.finish_qpi_transaction(Err(error));
        }
        self.transaction_pio_diagnostics.program_config_us = Instant::now()
            .duration_since(program_config_start)
            .as_micros();
        self.mark_transaction_pio_step(TransactionPioDiagnostics::STEP_PROGRAM_CONFIG_DONE);

        let result = (|| {
            let tx_buffer_build_start = Instant::now();
            let output_nibbles = self.transaction_pio_diagnostics.output_nibbles;
            let input_nibbles = self.transaction_pio_diagnostics.input_nibbles;
            let output_count = output_nibbles
                .checked_sub(1)
                .ok_or(EmbassyRpQpiError::StreamLength)?;
            let input_count = self.transaction_pio_fast_input_count(input_nibbles, fast_pio)?;
            self.push_word(output_count as u32)?;
            self.push_word(input_count as u32)?;
            self.push_transaction_pio_read_command(transaction)?;
            self.mark_transaction_pio_step(TransactionPioDiagnostics::STEP_COUNT_PUSH_DONE);
            self.transaction_pio_diagnostics.tx_buffer_build_us = Instant::now()
                .duration_since(tx_buffer_build_start)
                .as_micros();

            self.mark_transaction_pio_step(TransactionPioDiagnostics::STEP_DMA_CONFIG_START);
            marker(TransactionPioTxDmaStep::RxDmaConfigStart);
            marker(TransactionPioTxDmaStep::PacRxDmaStart);
            let rx_fifo = self.state_machine.rx_fifo_ptr();
            let rx_treq = self.state_machine.rx_treq();
            let read_addr = rx_fifo as u32;
            let write_addr = buf.as_mut_ptr() as u32;
            let dreq_id = rx_treq as u8 as u32;
            self.transaction_pio_diagnostics.rx_dma_transfer_size_bytes =
                core::mem::size_of::<u8>();
            self.transaction_pio_diagnostics.rx_dma_count = buf.len();
            self.transaction_pio_diagnostics.rx_dma_src_addr = read_addr;
            self.transaction_pio_diagnostics.rx_dma_dst_addr = write_addr;
            self.transaction_pio_diagnostics.rx_dma_dreq_id = dreq_id;
            self.transaction_pio_diagnostics.rx_dma_channel_id = self.rx_dma_channel_id;

            let rx_dma_arm_start = Instant::now();
            let (_reset_before, _reset_after) =
                pac_dma::prepare_rx_dma_ch1(self.timing.timeout_polls)?;
            let ctrl_base = pac_dma::rx_dma_ch1_u8_ctrl_base(rx_treq);
            let (ctrl_before_arm, armed_status) =
                pac_dma::configure_rx_dma_ch1(read_addr, write_addr, buf.len(), ctrl_base);
            self.transaction_pio_diagnostics.rx_dma_arm_us =
                Instant::now().duration_since(rx_dma_arm_start).as_micros();
            self.mark_transaction_pio_step(TransactionPioDiagnostics::STEP_DMA_CONFIG_DONE);
            marker(TransactionPioTxDmaStep::PacRxDmaConfig {
                read_addr,
                write_addr,
                trans_count: buf.len(),
                ctrl_base,
                ctrl_before_arm,
                ctrl_trig: armed_status.ctrl_trig,
                dreq: dreq_id,
                chain_to: pac_dma::RX_DMA_PAC_CH as u8,
                en: (armed_status.ctrl_trig & 1) != 0,
                busy: armed_status.busy,
                read_error: armed_status.read_error,
                write_error: armed_status.write_error,
                ahb_error: armed_status.ahb_error,
            });

            self.transaction_pio_diagnostics.progress_flags |=
                TransactionPioDiagnostics::STEP_SM_ENABLE_START;
            marker(TransactionPioTxDmaStep::SmEnableStart);
            let sm_enable_start = Instant::now();
            self.state_machine.set_enable(true);
            self.transaction_pio_diagnostics.progress_flags |=
                TransactionPioDiagnostics::STEP_SM_ENABLE;
            marker(TransactionPioTxDmaStep::SmEnableDone);

            marker(TransactionPioTxDmaStep::RxWaitStart);
            marker(TransactionPioTxDmaStep::PacRxDmaWaitStart);
            self.mark_transaction_pio_step(TransactionPioDiagnostics::STEP_RX_PULL_START);
            let wait_start = Instant::now();
            let wait_result = pac_dma::poll_rx_dma_ch1_until_done(self.timing.timeout_polls);
            self.transaction_pio_diagnostics.rx_dma_wait_us =
                Instant::now().duration_since(wait_start).as_micros();
            let status = pac_dma::rx_dma_ch1_status();
            self.transaction_pio_diagnostics.rx_dma_busy = status.busy;
            self.transaction_pio_diagnostics.rx_dma_read_error = status.read_error;
            self.transaction_pio_diagnostics.rx_dma_write_error = status.write_error;
            self.transaction_pio_diagnostics.rx_dma_ahb_error = status.ahb_error;
            marker(TransactionPioTxDmaStep::PacRxDmaStatus {
                busy: status.busy,
                read_error: status.read_error,
                write_error: status.write_error,
                ahb_error: status.ahb_error,
            });
            if let Err(error) = wait_result {
                pac_dma::abort_rx_dma_ch1(self.timing.timeout_polls);
                let final_status = pac_dma::disable_rx_dma_ch1_now();
                marker(TransactionPioTxDmaStep::PacRxDmaCleanup { final_status });
                return Err(error);
            }
            if status.read_error || status.write_error || status.ahb_error {
                pac_dma::abort_rx_dma_ch1(self.timing.timeout_polls);
                let final_status = pac_dma::disable_rx_dma_ch1_now();
                marker(TransactionPioTxDmaStep::PacRxDmaCleanup { final_status });
                return Err(EmbassyRpQpiError::Timeout);
            }
            marker(TransactionPioTxDmaStep::RxWaitDone);
            marker(TransactionPioTxDmaStep::PacRxDmaWaitDone);
            self.transaction_pio_diagnostics.sm_enable_to_rx_done_us =
                Instant::now().duration_since(sm_enable_start).as_micros();
            self.mark_transaction_pio_step(TransactionPioDiagnostics::STEP_RX_PULL_DONE);

            let dma_cleanup_start = Instant::now();
            let final_status = pac_dma::cleanup_rx_dma_ch1(self.timing.timeout_polls);
            self.transaction_pio_diagnostics.cleanup_us = self
                .transaction_pio_diagnostics
                .cleanup_us
                .saturating_add(Instant::now().duration_since(dma_cleanup_start).as_micros());
            marker(TransactionPioTxDmaStep::PacRxDmaCleanup { final_status });
            Ok(())
        })();

        if result.is_err() {
            let status = pac_dma::rx_dma_ch1_status();
            self.transaction_pio_diagnostics.rx_dma_busy = status.busy;
            self.transaction_pio_diagnostics.rx_dma_read_error = status.read_error;
            self.transaction_pio_diagnostics.rx_dma_write_error = status.write_error;
            self.transaction_pio_diagnostics.rx_dma_ahb_error = status.ahb_error;
            pac_dma::abort_rx_dma_ch1(self.timing.timeout_polls);
            pac_dma::disable_rx_dma_ch1_now();
        }
        let cleanup_start = Instant::now();
        let result = self.finish_qpi_transaction(result);
        self.transaction_pio_diagnostics.cleanup_us = self
            .transaction_pio_diagnostics
            .cleanup_us
            .saturating_add(Instant::now().duration_since(cleanup_start).as_micros());
        self.mark_transaction_pio_step(TransactionPioDiagnostics::STEP_CLEANUP_DONE);
        marker(TransactionPioTxDmaStep::CleanupDone);
        self.transaction_pio_diagnostics.total_chunk_us =
            Instant::now().duration_since(chunk_start).as_micros();
        result
    }

    pub(super) fn read_qpi_chunk_transaction_pio_fast_rx_byte_fifo_rx_dma_diagnostic(
        &mut self,
        transaction: QpiTransaction,
        buf: &mut [u8],
    ) -> Result<(), EmbassyRpQpiError> {
        self.read_qpi_chunk_transaction_pio_fast_rx_byte_fifo_rx_dma_diagnostic_inner(
            transaction,
            buf,
            |_| {},
        )
    }

    pub(super) fn read_qpi_chunk_transaction_pio_fast_rx_byte_fifo_rx_dma_diagnostic_inner<F>(
        &mut self,
        transaction: QpiTransaction,
        buf: &mut [u8],
        marker: F,
    ) -> Result<(), EmbassyRpQpiError>
    where
        F: FnMut(TransactionPioTxDmaStep),
    {
        self.read_qpi_chunk_transaction_pio_rx_byte_fifo_rx_dma_diagnostic_inner(
            transaction,
            buf,
            marker,
            true,
        )
    }

    fn transaction_pio_fast_input_count(
        &self,
        input_nibbles: usize,
        fast_pio: bool,
    ) -> Result<usize, EmbassyRpQpiError> {
        if fast_pio {
            match self.transaction_pio_fast_read_loop_variant {
                super::TransactionPioFastReadLoopVariant::FallingFudgeB
                | super::TransactionPioFastReadLoopVariant::FallingDiscardFirstNibble => {
                    return Ok(input_nibbles);
                }
                _ => {}
            }
        }

        input_nibbles
            .checked_sub(1)
            .ok_or(EmbassyRpQpiError::StreamLength)
    }
}
