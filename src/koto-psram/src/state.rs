//! Driver state machine.

use crate::error::PsramError;

/// Logical state of the PSRAM device and driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PsramState {
    /// GPIO/PIO resources may exist, but the chip mode is unknown.
    Uninitialized,
    /// 1-bit SPI mode is established.
    SpiMode,
    /// QPI command mode is established.
    QpiMode,
    /// QPI byte read/write is ready for production access.
    QpiReadWriteReady,
    /// A transfer failed and slower timing should be used before retrying.
    Degraded,
    /// Recovery failed.
    Failed,
}

impl PsramState {
    /// Returns whether production read/write access is allowed.
    #[inline]
    pub const fn can_access(self) -> bool {
        matches!(self, Self::QpiReadWriteReady | Self::Degraded)
    }
}

/// Small state-machine wrapper used by concrete backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateMachine {
    state: PsramState,
}

impl StateMachine {
    /// Creates a state machine in the uninitialized state.
    #[inline]
    pub const fn new() -> Self {
        Self {
            state: PsramState::Uninitialized,
        }
    }

    /// Returns the current state.
    #[inline]
    pub const fn state(self) -> PsramState {
        self.state
    }

    /// Moves to SPI mode after the idempotent exit sequence.
    pub fn enter_spi(&mut self) -> Result<(), PsramError> {
        match self.state {
            PsramState::Uninitialized
            | PsramState::SpiMode
            | PsramState::QpiMode
            | PsramState::QpiReadWriteReady
            | PsramState::Degraded => {
                self.state = PsramState::SpiMode;
                Ok(())
            }
            PsramState::Failed => Err(PsramError::InvalidState),
        }
    }

    /// Moves from SPI mode to QPI command mode.
    pub fn enter_qpi(&mut self) -> Result<(), PsramError> {
        match self.state {
            PsramState::SpiMode => {
                self.state = PsramState::QpiMode;
                Ok(())
            }
            _ => Err(PsramError::InvalidState),
        }
    }

    /// Marks production read/write as ready.
    pub fn mark_ready(&mut self) -> Result<(), PsramError> {
        match self.state {
            PsramState::QpiMode | PsramState::Degraded => {
                self.state = PsramState::QpiReadWriteReady;
                Ok(())
            }
            _ => Err(PsramError::InvalidState),
        }
    }

    /// Marks the bus as degraded after a recoverable transfer failure.
    pub fn mark_degraded(&mut self) {
        self.state = PsramState::Degraded;
    }

    /// Marks the bus as failed after unrecoverable recovery failure.
    pub fn mark_failed(&mut self) {
        self.state = PsramState::Failed;
    }
}

impl Default for StateMachine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_init_to_ready_path() {
        let mut sm = StateMachine::new();
        sm.enter_spi().unwrap();
        sm.enter_qpi().unwrap();
        sm.mark_ready().unwrap();
        assert_eq!(sm.state(), PsramState::QpiReadWriteReady);
        assert!(sm.state().can_access());
    }

    #[test]
    fn rejects_qpi_before_spi() {
        let mut sm = StateMachine::new();
        assert_eq!(sm.enter_qpi(), Err(PsramError::InvalidState));
    }
}
