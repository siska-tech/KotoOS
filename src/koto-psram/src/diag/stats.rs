//! Diagnostic counters.

/// Transfer counters suitable for key-value diagnostic logs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TransferStats {
    /// Successful read operations.
    pub reads: u32,
    /// Successful write operations.
    pub writes: u32,
    /// Timeout count.
    pub timeouts: u32,
    /// Mismatch count.
    pub mismatches: u32,
}

impl TransferStats {
    /// Records a successful read.
    pub fn record_read(&mut self) {
        self.reads = self.reads.saturating_add(1);
    }

    /// Records a successful write.
    pub fn record_write(&mut self) {
        self.writes = self.writes.saturating_add(1);
    }
}
