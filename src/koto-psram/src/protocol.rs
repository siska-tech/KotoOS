//! Protocol-level command and phase descriptions.

use crate::{addr::PsramAddr, config::TimingConfig};

/// PSRAM command opcodes used by the clean-room implementation.
pub mod command {
    /// Enter Quad I/O mode from SPI mode.
    pub const ENTER_QPI: u8 = 0x35;
    /// Exit Quad I/O mode.
    pub const EXIT_QPI: u8 = 0xF5;
    /// QPI fast read.
    pub const FAST_READ_QPI: u8 = 0xEB;
    /// QPI write.
    pub const WRITE_QPI: u8 = 0x38;
    /// Read device identity in SPI mode.
    pub const READ_ID: u8 = 0x9F;
}

/// Electrical width used for a transaction phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IoWidth {
    /// Single data pin, SPI-compatible.
    Single,
    /// Four data pins, QPI-compatible.
    Quad,
}

/// Direction driven on the data pins for a protocol phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinDirection {
    /// RP2040 drives the PSRAM pins.
    Output,
    /// RP2040 samples the PSRAM pins.
    Input,
}

/// A high-level transaction phase for PIO program generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Phase {
    /// I/O width for this phase.
    pub width: IoWidth,
    /// Pin direction for this phase.
    pub direction: PinDirection,
    /// Number of bytes represented by this phase.
    pub bytes: u8,
    /// Dummy clock cycles after this phase.
    pub dummy_cycles: u8,
}

impl Phase {
    /// SPI command phase.
    pub const SPI_COMMAND: Self = Self {
        width: IoWidth::Single,
        direction: PinDirection::Output,
        bytes: 1,
        dummy_cycles: 0,
    };

    /// QPI command phase.
    pub const QPI_COMMAND: Self = Self {
        width: IoWidth::Quad,
        direction: PinDirection::Output,
        bytes: 1,
        dummy_cycles: 0,
    };

    /// Three-byte address phase for QPI memory access.
    pub const QPI_ADDR: Self = Self {
        width: IoWidth::Quad,
        direction: PinDirection::Output,
        bytes: 3,
        dummy_cycles: 0,
    };
}

/// A QPI memory operation kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryOp {
    /// QPI fast read transaction.
    Read,
    /// QPI write transaction.
    Write,
}

impl MemoryOp {
    /// Returns the command byte for this operation.
    #[inline]
    pub const fn command(self) -> u8 {
        match self {
            Self::Read => command::FAST_READ_QPI,
            Self::Write => command::WRITE_QPI,
        }
    }

    /// Returns the data-pin direction for the payload phase.
    #[inline]
    pub const fn data_direction(self) -> PinDirection {
        match self {
            Self::Read => PinDirection::Input,
            Self::Write => PinDirection::Output,
        }
    }
}

/// Protocol description for one QPI memory transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QpiTransaction {
    /// Operation kind.
    pub op: MemoryOp,
    /// Command opcode.
    pub command: u8,
    /// Command-order 24-bit address.
    pub addr: [u8; 3],
    /// Dummy clock cycles before payload transfer.
    pub dummy_cycles: u8,
    /// Number of payload bytes transferred.
    pub len: usize,
    /// Data-pin direction during payload transfer.
    pub data_direction: PinDirection,
}

impl QpiTransaction {
    /// Builds the protocol shape for one QPI read chunk.
    #[inline]
    pub const fn read(addr: PsramAddr, len: usize, timing: TimingConfig) -> Self {
        Self::new(MemoryOp::Read, addr, len, timing.read_dummy_cycles)
    }

    /// Builds the protocol shape for one QPI write chunk.
    #[inline]
    pub const fn write(addr: PsramAddr, len: usize, timing: TimingConfig) -> Self {
        Self::new(MemoryOp::Write, addr, len, timing.write_dummy_cycles)
    }

    #[inline]
    const fn new(op: MemoryOp, addr: PsramAddr, len: usize, dummy_cycles: u8) -> Self {
        Self {
            op,
            command: op.command(),
            addr: addr24_be(addr),
            dummy_cycles,
            len,
            data_direction: op.data_direction(),
        }
    }
}

/// Converts a 24-bit PSRAM address to command-order bytes.
#[inline]
pub const fn addr24_be(addr: PsramAddr) -> [u8; 3] {
    let raw = addr.get();
    [
        ((raw >> 16) & 0xff) as u8,
        ((raw >> 8) & 0xff) as u8,
        (raw & 0xff) as u8,
    ]
}

/// Returns the two high-to-low nibbles carried by a QPI byte.
#[inline]
pub const fn qpi_nibbles(byte: u8) -> [u8; 2] {
    [(byte >> 4) & 0x0f, byte & 0x0f]
}

/// SPI-mode initialization transaction sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitStep {
    /// Broadcast QPI exit while assuming the chip might already be in QPI.
    ExitQpiAsQuad,
    /// Broadcast QPI exit while assuming the chip is in 1-bit SPI mode.
    ExitQpiAsSpi,
    /// Read device identity in SPI mode.
    ReadIdSpi,
    /// Enter QPI from SPI mode.
    EnterQpiSpi,
}

/// Documented idempotent initialization sequence.
pub const INIT_SEQUENCE: [InitStep; 4] = [
    InitStep::ExitQpiAsQuad,
    InitStep::ExitQpiAsSpi,
    InitStep::ReadIdSpi,
    InitStep::EnterQpiSpi,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_bytes_are_big_endian_24_bit() {
        assert_eq!(
            addr24_be(PsramAddr::new(0x12_3456).unwrap()),
            [0x12, 0x34, 0x56]
        );
    }

    #[test]
    fn qpi_byte_is_sent_high_nibble_first() {
        assert_eq!(qpi_nibbles(0xeb), [0x0e, 0x0b]);
    }

    #[test]
    fn init_sequence_recovers_before_probe_and_qpi_enter() {
        assert_eq!(
            INIT_SEQUENCE,
            [
                InitStep::ExitQpiAsQuad,
                InitStep::ExitQpiAsSpi,
                InitStep::ReadIdSpi,
                InitStep::EnterQpiSpi,
            ]
        );
    }

    #[test]
    fn qpi_read_transaction_uses_input_payload_and_read_dummy() {
        let timing = TimingConfig {
            read_dummy_cycles: 7,
            write_dummy_cycles: 0,
            ..TimingConfig::DEFAULT
        };
        assert_eq!(
            QpiTransaction::read(PsramAddr::new(0x12_3456).unwrap(), 64, timing),
            QpiTransaction {
                op: MemoryOp::Read,
                command: command::FAST_READ_QPI,
                addr: [0x12, 0x34, 0x56],
                dummy_cycles: 7,
                len: 64,
                data_direction: PinDirection::Input,
            }
        );
    }

    #[test]
    fn qpi_write_transaction_uses_output_payload_and_write_dummy() {
        let timing = TimingConfig {
            write_dummy_cycles: 1,
            ..TimingConfig::DEFAULT
        };
        assert_eq!(
            QpiTransaction::write(PsramAddr::new(0x00_0020).unwrap(), 3, timing),
            QpiTransaction {
                op: MemoryOp::Write,
                command: command::WRITE_QPI,
                addr: [0x00, 0x00, 0x20],
                dummy_cycles: 1,
                len: 3,
                data_direction: PinDirection::Output,
            }
        );
    }
}
