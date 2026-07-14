//! Blocking PIO backend abstraction.

use crate::{
    addr::PsramAddr,
    bus::{check_access, PsramBus},
    config::{Pins, TimingConfig},
    device::DeviceId,
    error::PsramError,
    protocol::QpiTransaction,
    state::StateMachine,
};

/// Low-level operations required from an RP2040 PIO implementation.
pub trait BlockingPio {
    /// Error returned by the PIO implementation.
    type Error: From<PsramError>;

    /// Initializes GPIO ownership, PIO programs, and FIFO state.
    fn configure(&mut self, pins: Pins, timing: TimingConfig) -> Result<(), Self::Error>;

    /// Sends QPI exit while assuming the chip might already be in QPI mode.
    fn exit_qpi_quad(&mut self) -> Result<(), Self::Error>;

    /// Sends QPI exit while assuming the chip is in 1-bit SPI mode.
    fn exit_qpi_spi(&mut self) -> Result<(), Self::Error>;

    /// Reads the device identity in SPI mode.
    fn read_id_spi(&mut self) -> Result<DeviceId, Self::Error>;

    /// Sends the SPI command that enters QPI mode.
    fn enter_qpi_spi(&mut self) -> Result<(), Self::Error>;

    /// Executes one bounded QPI read transaction.
    fn read_qpi_chunk(
        &mut self,
        transaction: QpiTransaction,
        buf: &mut [u8],
    ) -> Result<(), Self::Error> {
        if transaction.op != crate::protocol::MemoryOp::Read || transaction.len != buf.len() {
            return Err(PsramError::InvalidState.into());
        }

        self.qpi_read(transaction_addr(transaction)?, buf)
    }

    /// Executes one bounded QPI write transaction.
    fn write_qpi_chunk(
        &mut self,
        transaction: QpiTransaction,
        data: &[u8],
    ) -> Result<(), Self::Error> {
        if transaction.op != crate::protocol::MemoryOp::Write || transaction.len != data.len() {
            return Err(PsramError::InvalidState.into());
        }

        self.qpi_write(transaction_addr(transaction)?, data)
    }

    /// Performs a blocking QPI read.
    fn qpi_read(&mut self, _addr: PsramAddr, _buf: &mut [u8]) -> Result<(), Self::Error> {
        Err(PsramError::InvalidState.into())
    }

    /// Performs a blocking QPI write.
    fn qpi_write(&mut self, _addr: PsramAddr, _data: &[u8]) -> Result<(), Self::Error> {
        Err(PsramError::InvalidState.into())
    }
}

fn transaction_addr<E>(transaction: QpiTransaction) -> Result<PsramAddr, E>
where
    E: From<PsramError>,
{
    PsramAddr::new(
        ((transaction.addr[0] as u32) << 16)
            | ((transaction.addr[1] as u32) << 8)
            | transaction.addr[2] as u32,
    )
    .ok_or(PsramError::OutOfRange.into())
}

/// Production blocking PSRAM driver backed by PIO.
pub struct BlockingDriver<P> {
    pio: P,
    pins: Pins,
    timing: TimingConfig,
    state: StateMachine,
}

impl<P> BlockingDriver<P> {
    /// Creates a driver with default PicoCalc pins and conservative timing.
    pub fn new(pio: P) -> Self {
        Self::with_config(pio, Pins::PICOCALC, TimingConfig::DEFAULT)
    }

    /// Creates a driver with explicit pins and timing.
    pub const fn with_config(pio: P, pins: Pins, timing: TimingConfig) -> Self {
        Self {
            pio,
            pins,
            timing,
            state: StateMachine::new(),
        }
    }

    /// Returns the configured pins.
    pub const fn pins(&self) -> Pins {
        self.pins
    }

    /// Returns the configured timing.
    pub const fn timing(&self) -> TimingConfig {
        self.timing
    }

    /// Returns the current state machine value.
    pub const fn state(&self) -> crate::state::PsramState {
        self.state.state()
    }

    /// Releases the backend.
    pub fn into_inner(self) -> P {
        self.pio
    }

    /// Returns the backend for diagnostic examples that need concrete knobs.
    ///
    /// This is intentionally outside the stable public bus surface; production
    /// code should use [`PsramBus`] instead.
    #[doc(hidden)]
    pub fn backend_mut_for_diagnostics(&mut self) -> &mut P {
        &mut self.pio
    }
}

impl<P: BlockingPio> BlockingDriver<P> {
    /// Runs the documented idempotent init flow through QPI ready state.
    pub fn init(&mut self) -> Result<DeviceId, P::Error> {
        if !self.pins.validate() || !self.timing.validate() {
            return Err(PsramError::InvalidState.into());
        }

        self.pio.configure(self.pins, self.timing)?;
        self.pio.exit_qpi_quad()?;
        self.pio.exit_qpi_spi()?;
        self.state.enter_spi()?;

        let id = self.pio.read_id_spi()?;
        if !id.looks_present() {
            self.state.mark_failed();
            return Err(PsramError::UnsupportedDevice.into());
        }

        self.pio.enter_qpi_spi()?;
        self.state.enter_qpi()?;
        self.state.mark_ready()?;
        Ok(id)
    }

    /// Reconfigures backend timing while preserving the current driver state.
    ///
    /// This is intended for diagnostics and benchmark sweeps that need to vary
    /// bounded transfer parameters after the driver has already reached ready
    /// state. It does not rerun the PSRAM mode-entry sequence.
    pub fn configure_timing(&mut self, timing: TimingConfig) -> Result<(), P::Error> {
        if !timing.validate() {
            return Err(PsramError::InvalidState.into());
        }

        self.pio.configure(self.pins, timing)?;
        self.timing = timing;
        Ok(())
    }
}

impl<P: BlockingPio> PsramBus for BlockingDriver<P> {
    type Error = P::Error;

    fn read_exact(&mut self, addr: PsramAddr, buf: &mut [u8]) -> Result<(), Self::Error> {
        check_access(addr, buf.len())?;
        if !self.state.state().can_access() {
            return Err(PsramError::InvalidState.into());
        }

        let mut addr = addr;
        let mut remaining = buf;
        while !remaining.is_empty() {
            let chunk_len = remaining.len().min(self.timing.max_chunk_len);
            let (chunk, next) = remaining.split_at_mut(chunk_len);
            let transaction = QpiTransaction::read(addr, chunk_len, self.timing);
            match self.pio.read_qpi_chunk(transaction, chunk) {
                Ok(()) => {}
                Err(err) => {
                    self.state.mark_degraded();
                    return Err(err);
                }
            }

            let advance = u32::try_from(chunk_len).map_err(|_| PsramError::OutOfRange)?;
            addr = addr.checked_add(advance).ok_or(PsramError::OutOfRange)?;
            remaining = next;
        }

        Ok(())
    }

    fn write_all(&mut self, addr: PsramAddr, data: &[u8]) -> Result<(), Self::Error> {
        check_access(addr, data.len())?;
        if !self.state.state().can_access() {
            return Err(PsramError::InvalidState.into());
        }

        let mut addr = addr;
        let mut remaining = data;
        while !remaining.is_empty() {
            let chunk_len = remaining.len().min(self.timing.max_chunk_len);
            let (chunk, next) = remaining.split_at(chunk_len);
            let transaction = QpiTransaction::write(addr, chunk_len, self.timing);
            match self.pio.write_qpi_chunk(transaction, chunk) {
                Ok(()) => {}
                Err(err) => {
                    self.state.mark_degraded();
                    return Err(err);
                }
            }

            let advance = u32::try_from(chunk_len).map_err(|_| PsramError::OutOfRange)?;
            addr = addr.checked_add(advance).ok_or(PsramError::OutOfRange)?;
            remaining = next;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::InitStep;

    #[derive(Default)]
    struct FakePio {
        memory: [u8; 32],
        configured: bool,
        timing: TimingConfig,
        log: [Option<InitStep>; 4],
        log_len: usize,
        read_chunks: [Option<(u32, usize)>; 8],
        read_chunks_len: usize,
        write_chunks: [Option<(u32, usize)>; 8],
        write_chunks_len: usize,
        qpi: bool,
    }

    impl FakePio {
        fn push_log(&mut self, step: InitStep) {
            self.log[self.log_len] = Some(step);
            self.log_len += 1;
        }

        fn push_read_chunk(&mut self, addr: PsramAddr, len: usize) {
            self.read_chunks[self.read_chunks_len] = Some((addr.get(), len));
            self.read_chunks_len += 1;
        }

        fn push_write_chunk(&mut self, addr: PsramAddr, len: usize) {
            self.write_chunks[self.write_chunks_len] = Some((addr.get(), len));
            self.write_chunks_len += 1;
        }
    }

    impl BlockingPio for FakePio {
        type Error = PsramError;

        fn configure(&mut self, _pins: Pins, _timing: TimingConfig) -> Result<(), Self::Error> {
            self.configured = true;
            self.timing = _timing;
            Ok(())
        }

        fn exit_qpi_quad(&mut self) -> Result<(), Self::Error> {
            self.push_log(InitStep::ExitQpiAsQuad);
            self.qpi = false;
            Ok(())
        }

        fn exit_qpi_spi(&mut self) -> Result<(), Self::Error> {
            self.push_log(InitStep::ExitQpiAsSpi);
            self.qpi = false;
            Ok(())
        }

        fn read_id_spi(&mut self) -> Result<DeviceId, Self::Error> {
            self.push_log(InitStep::ReadIdSpi);
            if self.configured {
                Ok(DeviceId::new([0x0d, 0x5d, 0x5d]))
            } else {
                Err(PsramError::InvalidState)
            }
        }

        fn enter_qpi_spi(&mut self) -> Result<(), Self::Error> {
            self.push_log(InitStep::EnterQpiSpi);
            self.qpi = true;
            Ok(())
        }

        fn read_qpi_chunk(
            &mut self,
            transaction: QpiTransaction,
            buf: &mut [u8],
        ) -> Result<(), Self::Error> {
            if !self.qpi {
                return Err(PsramError::InvalidState);
            }
            if transaction.len != buf.len() || transaction.op != crate::protocol::MemoryOp::Read {
                return Err(PsramError::InvalidState);
            }
            let addr = PsramAddr::new(
                ((transaction.addr[0] as u32) << 16)
                    | ((transaction.addr[1] as u32) << 8)
                    | transaction.addr[2] as u32,
            )
            .ok_or(PsramError::OutOfRange)?;
            self.push_read_chunk(addr, buf.len());
            let start = addr.get() as usize;
            let end = start + buf.len();
            buf.copy_from_slice(self.memory.get(start..end).ok_or(PsramError::OutOfRange)?);
            Ok(())
        }

        fn write_qpi_chunk(
            &mut self,
            transaction: QpiTransaction,
            data: &[u8],
        ) -> Result<(), Self::Error> {
            if !self.qpi {
                return Err(PsramError::InvalidState);
            }
            if transaction.len != data.len() || transaction.op != crate::protocol::MemoryOp::Write {
                return Err(PsramError::InvalidState);
            }
            let addr = PsramAddr::new(
                ((transaction.addr[0] as u32) << 16)
                    | ((transaction.addr[1] as u32) << 8)
                    | transaction.addr[2] as u32,
            )
            .ok_or(PsramError::OutOfRange)?;
            self.push_write_chunk(addr, data.len());
            let start = addr.get() as usize;
            let end = start + data.len();
            self.memory
                .get_mut(start..end)
                .ok_or(PsramError::OutOfRange)?
                .copy_from_slice(data);
            Ok(())
        }
    }

    #[test]
    fn init_enables_blocking_access() {
        let mut driver = BlockingDriver::new(FakePio::default());
        assert_eq!(driver.init().unwrap().raw, [0x0d, 0x5d, 0x5d]);
        let addr = PsramAddr::new(4).unwrap();
        driver.write_all(addr, &[1, 2, 3]).unwrap();
        let mut out = [0; 3];
        driver.read_exact(addr, &mut out).unwrap();
        assert_eq!(out, [1, 2, 3]);
    }

    #[test]
    fn init_uses_documented_spi_recovery_sequence() {
        let mut driver = BlockingDriver::new(FakePio::default());
        driver.init().unwrap();
        let pio = driver.into_inner();
        assert_eq!(
            pio.log,
            [
                Some(InitStep::ExitQpiAsQuad),
                Some(InitStep::ExitQpiAsSpi),
                Some(InitStep::ReadIdSpi),
                Some(InitStep::EnterQpiSpi),
            ]
        );
    }

    #[test]
    fn access_before_init_is_rejected() {
        let mut driver = BlockingDriver::new(FakePio::default());
        let mut out = [0; 1];
        assert_eq!(
            driver.read_exact(PsramAddr::zero(), &mut out),
            Err(PsramError::InvalidState)
        );
    }

    #[test]
    fn impl_rejects_absent_device_id() {
        struct AbsentIdPio(FakePio);

        impl BlockingPio for AbsentIdPio {
            type Error = PsramError;

            fn configure(&mut self, pins: Pins, timing: TimingConfig) -> Result<(), Self::Error> {
                self.0.configure(pins, timing)
            }

            fn exit_qpi_quad(&mut self) -> Result<(), Self::Error> {
                self.0.exit_qpi_quad()
            }

            fn exit_qpi_spi(&mut self) -> Result<(), Self::Error> {
                self.0.exit_qpi_spi()
            }

            fn read_id_spi(&mut self) -> Result<DeviceId, Self::Error> {
                Ok(DeviceId::new([0xff; 3]))
            }

            fn enter_qpi_spi(&mut self) -> Result<(), Self::Error> {
                self.0.enter_qpi_spi()
            }

            fn read_qpi_chunk(
                &mut self,
                transaction: QpiTransaction,
                buf: &mut [u8],
            ) -> Result<(), Self::Error> {
                self.0.read_qpi_chunk(transaction, buf)
            }

            fn write_qpi_chunk(
                &mut self,
                transaction: QpiTransaction,
                data: &[u8],
            ) -> Result<(), Self::Error> {
                self.0.write_qpi_chunk(transaction, data)
            }
        }

        let mut driver = BlockingDriver::new(AbsentIdPio(FakePio::default()));
        assert_eq!(driver.init(), Err(PsramError::UnsupportedDevice));
        assert_eq!(driver.state(), crate::state::PsramState::Failed);
    }

    #[test]
    fn read_and_write_are_split_into_configured_chunks() {
        let timing = TimingConfig {
            max_chunk_len: 4,
            ..TimingConfig::DEFAULT
        };
        let mut driver = BlockingDriver::with_config(FakePio::default(), Pins::PICOCALC, timing);
        driver.init().unwrap();

        let addr = PsramAddr::new(2).unwrap();
        driver
            .write_all(addr, &[1, 2, 3, 4, 5, 6, 7, 8, 9])
            .unwrap();
        let mut out = [0; 9];
        driver.read_exact(addr, &mut out).unwrap();
        assert_eq!(out, [1, 2, 3, 4, 5, 6, 7, 8, 9]);

        let pio = driver.into_inner();
        assert_eq!(
            pio.write_chunks,
            [
                Some((2, 4)),
                Some((6, 4)),
                Some((10, 1)),
                None,
                None,
                None,
                None,
                None,
            ]
        );
        assert_eq!(
            pio.read_chunks,
            [
                Some((2, 4)),
                Some((6, 4)),
                Some((10, 1)),
                None,
                None,
                None,
                None,
                None,
            ]
        );
    }

    #[test]
    fn configure_timing_preserves_ready_state_and_updates_backend() {
        let mut driver = BlockingDriver::new(FakePio::default());
        driver.init().unwrap();

        let timing = TimingConfig {
            max_chunk_len: 8,
            ..TimingConfig::DEFAULT
        };
        driver.configure_timing(timing).unwrap();

        assert_eq!(driver.state(), crate::state::PsramState::QpiReadWriteReady);
        assert_eq!(driver.timing(), timing);
        assert_eq!(driver.into_inner().timing, timing);
    }

    #[test]
    fn backend_chunk_failure_marks_driver_degraded() {
        struct FailingPio(FakePio);

        impl BlockingPio for FailingPio {
            type Error = PsramError;

            fn configure(&mut self, pins: Pins, timing: TimingConfig) -> Result<(), Self::Error> {
                self.0.configure(pins, timing)
            }

            fn exit_qpi_quad(&mut self) -> Result<(), Self::Error> {
                self.0.exit_qpi_quad()
            }

            fn exit_qpi_spi(&mut self) -> Result<(), Self::Error> {
                self.0.exit_qpi_spi()
            }

            fn read_id_spi(&mut self) -> Result<DeviceId, Self::Error> {
                self.0.read_id_spi()
            }

            fn enter_qpi_spi(&mut self) -> Result<(), Self::Error> {
                self.0.enter_qpi_spi()
            }

            fn read_qpi_chunk(
                &mut self,
                _transaction: QpiTransaction,
                _buf: &mut [u8],
            ) -> Result<(), Self::Error> {
                Err(PsramError::HardwareFault)
            }

            fn write_qpi_chunk(
                &mut self,
                transaction: QpiTransaction,
                data: &[u8],
            ) -> Result<(), Self::Error> {
                self.0.write_qpi_chunk(transaction, data)
            }
        }

        let mut driver = BlockingDriver::new(FailingPio(FakePio::default()));
        driver.init().unwrap();
        let mut out = [0; 1];
        assert_eq!(
            driver.read_exact(PsramAddr::zero(), &mut out),
            Err(PsramError::HardwareFault)
        );
        assert_eq!(driver.state(), crate::state::PsramState::Degraded);
    }

    #[test]
    fn backend_write_chunk_failure_marks_driver_degraded() {
        struct FailingWritePio(FakePio);

        impl BlockingPio for FailingWritePio {
            type Error = PsramError;

            fn configure(&mut self, pins: Pins, timing: TimingConfig) -> Result<(), Self::Error> {
                self.0.configure(pins, timing)
            }

            fn exit_qpi_quad(&mut self) -> Result<(), Self::Error> {
                self.0.exit_qpi_quad()
            }

            fn exit_qpi_spi(&mut self) -> Result<(), Self::Error> {
                self.0.exit_qpi_spi()
            }

            fn read_id_spi(&mut self) -> Result<DeviceId, Self::Error> {
                self.0.read_id_spi()
            }

            fn enter_qpi_spi(&mut self) -> Result<(), Self::Error> {
                self.0.enter_qpi_spi()
            }

            fn read_qpi_chunk(
                &mut self,
                transaction: QpiTransaction,
                buf: &mut [u8],
            ) -> Result<(), Self::Error> {
                self.0.read_qpi_chunk(transaction, buf)
            }

            fn write_qpi_chunk(
                &mut self,
                _transaction: QpiTransaction,
                _data: &[u8],
            ) -> Result<(), Self::Error> {
                Err(PsramError::HardwareFault)
            }
        }

        let mut driver = BlockingDriver::new(FailingWritePio(FakePio::default()));
        driver.init().unwrap();
        assert_eq!(
            driver.write_all(PsramAddr::zero(), &[0xad]),
            Err(PsramError::HardwareFault)
        );
        assert_eq!(driver.state(), crate::state::PsramState::Degraded);
    }
}
