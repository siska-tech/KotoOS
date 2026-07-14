/// Diagnostic metadata from the CPU-fed full-transaction PIO read path.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TransactionPioDiagnostics {
    /// Whether TX DMA fed the command/metadata stream.
    pub tx_dma: bool,
    /// Whether RX DMA drained the read payload.
    pub rx_dma: bool,
    /// TX DMA setup duration.
    pub tx_dma_setup_us: u64,
    /// TX DMA completion wait duration.
    pub tx_dma_wait_us: u64,
    /// RX DMA completion wait duration.
    pub rx_dma_wait_us: u64,
    /// PIO program and state-machine configuration duration.
    pub program_config_us: u64,
    /// TX command buffer construction duration.
    pub tx_buffer_build_us: u64,
    /// TX DMA arm/configuration duration.
    pub tx_dma_arm_us: u64,
    /// RX DMA arm/configuration duration.
    pub rx_dma_arm_us: u64,
    /// Duration from state-machine enable to TX completion.
    pub sm_enable_to_tx_done_us: u64,
    /// Duration from state-machine enable to RX completion.
    pub sm_enable_to_rx_done_us: u64,
    /// RX staging unpack/copy duration.
    pub rx_unpack_us: u64,
    /// Transaction cleanup duration.
    pub cleanup_us: u64,
    /// End-to-end diagnostic chunk read duration.
    pub total_chunk_us: u64,
    /// TX DMA buffer capacity, in FIFO words.
    pub tx_buf_capacity: usize,
    /// TX DMA transfer length, in FIFO words.
    pub tx_len: usize,
    /// TX DMA transfer element size, in bytes.
    pub tx_dma_transfer_size_bytes: usize,
    /// TX DMA transfer count, in transfer elements.
    pub tx_dma_count: usize,
    /// TX DMA source address.
    pub tx_dma_src_addr: u32,
    /// TX DMA destination address.
    pub tx_dma_dst_addr: u32,
    /// TX DMA DREQ selector.
    pub tx_dma_dreq_id: u32,
    /// TX DMA channel number, when known by the caller.
    pub tx_dma_channel_id: Option<u8>,
    /// Last observed TX DMA busy bit.
    pub tx_dma_busy: bool,
    /// Last observed TX DMA read-error bit.
    pub tx_dma_read_error: bool,
    /// Last observed TX DMA write-error bit.
    pub tx_dma_write_error: bool,
    /// Last observed TX DMA AHB-error bit.
    pub tx_dma_ahb_error: bool,
    /// RX DMA transfer element size, in bytes.
    pub rx_dma_transfer_size_bytes: usize,
    /// RX DMA transfer count, in transfer elements.
    pub rx_dma_count: usize,
    /// RX DMA source address.
    pub rx_dma_src_addr: u32,
    /// RX DMA destination address.
    pub rx_dma_dst_addr: u32,
    /// RX DMA DREQ selector.
    pub rx_dma_dreq_id: u32,
    /// RX DMA channel number, when known by the caller.
    pub rx_dma_channel_id: Option<u8>,
    /// Last observed RX DMA busy bit.
    pub rx_dma_busy: bool,
    /// Last observed RX DMA read-error bit.
    pub rx_dma_read_error: bool,
    /// Last observed RX DMA write-error bit.
    pub rx_dma_write_error: bool,
    /// Last observed RX DMA AHB-error bit.
    pub rx_dma_ahb_error: bool,
    /// Command/address/dummy byte count before the read phase.
    pub output_bytes: usize,
    /// Whether TX DMA buffer construction exceeded its bounded storage.
    pub tx_buffer_overflow: bool,
    /// Number of QPI nibbles clocked out before the read phase.
    pub output_nibbles: usize,
    /// Number of QPI nibbles clocked in during the read phase.
    pub input_nibbles: usize,
    /// Payload byte length.
    pub byte_len: usize,
    /// Full RX FIFO words expected for this diagnostic read.
    pub word_count: usize,
    /// Completed progress markers for post-error diagnostics.
    pub progress_flags: u32,
}

impl TransactionPioDiagnostics {
    /// PIO transaction helper started.
    pub const STEP_START: u32 = 1 << 0;
    /// PIO program and state-machine config completed.
    pub const STEP_PROGRAM_CONFIG_DONE: u32 = 1 << 1;
    /// TX DMA buffer was prepared.
    pub const STEP_BUFFER_READY: u32 = 1 << 2;
    /// TX DMA channel was configured.
    pub const STEP_DMA_CONFIG_DONE: u32 = 1 << 3;
    /// Transaction count and command words were pushed.
    pub const STEP_COUNT_PUSH_DONE: u32 = 1 << 4;
    /// State machine was enabled.
    pub const STEP_SM_ENABLE: u32 = 1 << 5;
    /// TX DMA completion wait started.
    pub const STEP_TX_WAIT_START: u32 = 1 << 6;
    /// TX DMA completion wait completed.
    pub const STEP_TX_WAIT_DONE: u32 = 1 << 7;
    /// CPU RX FIFO pull loop started.
    pub const STEP_RX_PULL_START: u32 = 1 << 8;
    /// CPU RX FIFO pull loop completed.
    pub const STEP_RX_PULL_DONE: u32 = 1 << 9;
    /// Cleanup completed.
    pub const STEP_CLEANUP_DONE: u32 = 1 << 10;
    /// TX DMA transaction buffer construction started.
    pub const STEP_BUFFER_BUILD_START: u32 = 1 << 11;
    /// TX DMA count words write started.
    pub const STEP_COUNT_WRITE_START: u32 = 1 << 12;
    /// TX DMA count words write completed.
    pub const STEP_COUNT_WRITE_DONE: u32 = 1 << 13;
    /// TX DMA command word write started.
    pub const STEP_CMD_WRITE_START: u32 = 1 << 14;
    /// TX DMA command byte was written.
    pub const STEP_CMD_WRITE_DONE: u32 = 1 << 15;
    /// TX DMA address bytes were written.
    pub const STEP_ADDR_WRITE_DONE: u32 = 1 << 16;
    /// TX DMA dummy bytes were written.
    pub const STEP_DUMMY_WRITE_DONE: u32 = 1 << 17;
    /// TX DMA config construction started.
    pub const STEP_DMA_CONFIG_START: u32 = 1 << 18;
    /// TX DMA transfer object creation started.
    pub const STEP_DMA_TRANSFER_CREATE_START: u32 = 1 << 19;
    /// TX DMA transfer object creation completed.
    pub const STEP_DMA_TRANSFER_CREATE_DONE: u32 = 1 << 20;
    /// State machine enable started.
    pub const STEP_SM_ENABLE_START: u32 = 1 << 21;
}

/// Result from building the diagnostic TX DMA transaction buffer.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransactionPioTxDmaBufferDiagnostics {
    /// Detailed transaction diagnostics after buffer construction.
    pub diagnostics: TransactionPioDiagnostics,
    /// Transaction address used for this preflight.
    pub addr: [u8; 3],
    /// Whether bounded TX buffer construction overflowed.
    pub overflow: bool,
}

/// Inline marker emitted by the TX-DMA transaction diagnostic path.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionPioTxDmaStep {
    MemStart,
    MemConfig {
        tx_len: usize,
        src_addr: u32,
        dst_addr: u32,
        transfer_size: usize,
        count: usize,
    },
    MemTransferCreateStart,
    MemTransferCreateDone,
    MemWaitStart,
    MemWaitDone,
    MemStatus {
        ok: bool,
    },
    DmaConfigStart,
    PacDmaStart,
    DmaConfig {
        tx_len: usize,
        transfer_size: usize,
        count: usize,
        src_addr: u32,
        dst_addr: u32,
        dreq_id: u32,
        channel_id: Option<u8>,
    },
    PacDmaConfig {
        read_addr: u32,
        write_addr: u32,
        trans_count: usize,
        ctrl_base: u32,
        ctrl_before_arm: u32,
        ctrl_trig: u32,
        dreq: u32,
        chain_to: u8,
        en: bool,
        busy: bool,
        read_error: bool,
        write_error: bool,
        ahb_error: bool,
    },
    DmaTransferCreateStart,
    DmaTransferCreateDone,
    PacDmaReset {
        before: PacDmaStatus,
        after: PacDmaStatus,
    },
    PacDmaStarted,
    SmEnableStart,
    SmEnableDone,
    TxWaitStart,
    TxWaitDone,
    PacDmaWaitStart,
    PacDmaWaitDone,
    PacDmaStatus {
        busy: bool,
        read_error: bool,
        write_error: bool,
        ahb_error: bool,
    },
    PacDmaCleanup {
        final_status: PacDmaStatus,
    },
    RxDmaConfigStart,
    PacRxDmaStart,
    PacRxDmaConfig {
        read_addr: u32,
        write_addr: u32,
        trans_count: usize,
        ctrl_base: u32,
        ctrl_before_arm: u32,
        ctrl_trig: u32,
        dreq: u32,
        chain_to: u8,
        en: bool,
        busy: bool,
        read_error: bool,
        write_error: bool,
        ahb_error: bool,
    },
    RxWaitStart,
    RxWaitDone,
    PacRxDmaWaitStart,
    PacRxDmaWaitDone,
    PacRxDmaStatus {
        busy: bool,
        read_error: bool,
        write_error: bool,
        ahb_error: bool,
    },
    PacRxDmaCleanup {
        final_status: PacDmaStatus,
    },
    RxWords {
        words: [u32; 4],
        count: usize,
    },
    RxPullStart,
    RxPullDone,
    CleanupDone,
}

/// Snapshot of the PAC DMA channel status used by TX-DMA diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PacDmaStatus {
    /// Whether the DMA channel reports an active transfer.
    pub busy: bool,
    /// Whether the DMA channel latched a read error.
    pub read_error: bool,
    /// Whether the DMA channel latched a write error.
    pub write_error: bool,
    /// Whether the DMA channel latched an AHB error.
    pub ahb_error: bool,
    /// Raw CTRL_TRIG register value.
    pub ctrl_trig: u32,
}
