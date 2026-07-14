// Phase 4-6E freeze: word_stream is the fastest CPU-polling write path, while
// polling_burst remains the fastest CPU-polling read path. Word-stream reads
// are currently RX-pull limited; RX FIFO join needs a different read-stream
// setup design because this qpi_read_stream still receives its count via TX.
/// Diagnostic-only payload transfer path selector for the concrete Embassy backend.
///
/// The clkdiv sweep on PicoCalc showed no meaningful throughput difference
/// between conservative clock dividers, which points at the current CPU/FIFO
/// polling loops rather than QPI wire clocking as the benchmark bottleneck.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayloadTransferPath {
    /// Known-good path: one TX FIFO push and one RX FIFO pull per byte.
    ByteFallback,
    /// Prototype path: fill/drain FIFOs opportunistically during payload only.
    PollingBurstDiagnostic,
    /// Experimental path: stream packed payload words through dedicated PIO programs.
    WordStreamPolling,
    /// Experimental path: one CPU-fed PIO program owns a full read transaction.
    TransactionPioDiagnostic,
    /// Experimental path: TX DMA feeds the full read transaction command stream.
    TransactionPioTxDmaDiagnostic,
    /// Experimental path: RX DMA drains full-transaction read payload words.
    TransactionPioRxDmaDiagnostic,
    /// Experimental path: TX DMA feeds commands while RX DMA drains payload words.
    TransactionPioTxRxDmaDiagnostic,
    /// Experimental path: CPU-fed transaction PIO with byte-granular RX FIFO pulls.
    TransactionPioRxByteFifoDiagnostic,
    /// Experimental path: CPU-fed TX stream with byte-granular RX FIFO DMA.
    TransactionPioRxByteFifoRxDmaDiagnostic,
    /// Experimental path: no-delay transaction PIO with byte-granular RX FIFO DMA.
    TransactionPioFastRxByteFifoRxDmaDiagnostic,
    /// Reserved for the later DMA-backed stream engine.
    WordStreamDma,
}

/// Diagnostic-only read-loop variant for fast transaction PIO probes.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionPioFastReadLoopVariant {
    /// Sample on the side-set phase used by the stable loop, with no delays.
    CurrentNoDelay,
    /// No-delay loop with opposite side-set polarity from the stable loop.
    OppositePolarityNoDelay,
    /// Keep the stable sampling side-set and add one delay slot to `in`.
    DelayOnIn,
    /// Keep the stable sampling side-set and add one delay slot to `jmp`.
    DelayOnJmp,
    /// Falling-edge diagnostic with one low pre-read fudge cycle.
    FallingFudgeA,
    /// Falling-edge diagnostic with an extra low jump before the first sample.
    FallingFudgeB,
    /// Falling-edge diagnostic with no pre-read fudge cycle.
    FallingNoFudge,
    /// Falling-edge diagnostic with two low pre-read fudge cycles.
    FallingFudgeExtraLow,
    /// Falling-edge diagnostic that samples one nibble before normal byte capture.
    FallingDiscardFirstNibble,
    /// Falling-edge diagnostic with one extra SCK transition before payload capture.
    FallingExtraDummyHalfCycle,
    /// Falling-edge diagnostic with two unrecorded QPI nibbles before payload capture.
    FallingExtraDummyByte,
}

impl Default for TransactionPioFastReadLoopVariant {
    fn default() -> Self {
        Self::OppositePolarityNoDelay
    }
}

/// Diagnostic timing for the most recent concrete QPI chunk.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct QpiChunkTiming {
    /// Command/address/dummy phase duration.
    pub command_addr_dummy_us: u64,
    /// Payload write loop duration.
    pub payload_write_us: u64,
    /// Payload read loop duration.
    pub payload_read_us: u64,
    /// Word-stream read wait until the first RX FIFO word is available.
    pub word_stream_rx_fifo_wait_us: u64,
    /// Word-stream read time spent pulling RX FIFO words.
    pub word_stream_rx_pull_loop_us: u64,
    /// Word-stream read time spent unpacking full 32-bit payload words.
    pub word_stream_unpack_loop_us: u64,
    /// Word-stream read time spent unpacking the final partial word.
    pub word_stream_tail_unpack_us: u64,
    /// Flush duration, when measured.
    pub flush_us: u64,
}

/// Diagnostic-only knobs for the word-stream read payload path.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WordStreamReadDiagnostics {
    /// Number of full RX FIFO words to pull before unpacking them.
    pub batch_words: usize,
    /// Whether the read stream state machine requests RX-only FIFO join.
    ///
    /// On RP2040 this doubles RX FIFO depth by disabling TX FIFO access. The
    /// current read-stream PIO program still needs TX for its payload count, so
    /// `true` is an unsupported diagnostic probe until read-stream setup stops
    /// depending on TX FIFO after `FifoJoin::RxOnly`.
    pub rx_fifo_join: bool,
}

impl Default for WordStreamReadDiagnostics {
    fn default() -> Self {
        Self {
            batch_words: 4,
            rx_fifo_join: false,
        }
    }
}
