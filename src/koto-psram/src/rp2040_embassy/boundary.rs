use crate::{
    config::{Pins, TimingConfig},
    device::DeviceId,
    error::PsramError,
    pio::blocking::BlockingPio,
    protocol::{MemoryOp, QpiTransaction},
};

/// Owned RP2040 resources needed by the blocking QPI backend.
#[derive(Debug)]
pub struct Rp2040QpiResources<Pio, StateMachine, Sio0, Sio1, Sio2, Sio3, Cs, Sck> {
    /// PIO peripheral ownership.
    pub pio: Pio,
    /// PIO state-machine ownership.
    pub state_machine: StateMachine,
    /// QPI SIO0 / SPI MOSI pin.
    pub sio0: Sio0,
    /// QPI SIO1 / SPI MISO pin.
    pub sio1: Sio1,
    /// QPI SIO2 pin.
    pub sio2: Sio2,
    /// QPI SIO3 pin.
    pub sio3: Sio3,
    /// Active-low chip-select pin.
    pub cs: Cs,
    /// Serial clock pin.
    pub sck: Sck,
}

impl<Pio, StateMachine, Sio0, Sio1, Sio2, Sio3, Cs, Sck>
    Rp2040QpiResources<Pio, StateMachine, Sio0, Sio1, Sio2, Sio3, Cs, Sck>
{
    /// Creates an owned resource bundle.
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        pio: Pio,
        state_machine: StateMachine,
        sio0: Sio0,
        sio1: Sio1,
        sio2: Sio2,
        sio3: Sio3,
        cs: Cs,
        sck: Sck,
    ) -> Self {
        Self {
            pio,
            state_machine,
            sio0,
            sio1,
            sio2,
            sio3,
            cs,
            sck,
        }
    }
}

/// Low-level blocking operations supplied by an RP2040 Embassy board adapter.
pub trait Rp2040QpiExecutor<R> {
    /// Adapter-specific error type.
    type Error: From<PsramError>;

    /// Configures GPIO direction, PIO programs, state-machine pin mapping, and
    /// clocking for the supplied pins and timing.
    fn configure(
        &mut self,
        resources: &mut R,
        pins: Pins,
        timing: TimingConfig,
    ) -> Result<(), Self::Error>;

    /// Sends `EXIT_QPI` while assuming the chip is already in QPI mode.
    fn exit_qpi_quad(&mut self, resources: &mut R, timing: TimingConfig)
        -> Result<(), Self::Error>;

    /// Sends `EXIT_QPI` while assuming the chip is in SPI mode.
    fn exit_qpi_spi(&mut self, resources: &mut R, timing: TimingConfig) -> Result<(), Self::Error>;

    /// Reads the three-byte device identity in SPI mode.
    fn read_id_spi(
        &mut self,
        resources: &mut R,
        timing: TimingConfig,
    ) -> Result<DeviceId, Self::Error>;

    /// Sends `ENTER_QPI` in SPI mode.
    fn enter_qpi_spi(&mut self, resources: &mut R, timing: TimingConfig)
        -> Result<(), Self::Error>;

    /// Executes exactly one QPI write transaction.
    fn write_qpi_chunk(
        &mut self,
        resources: &mut R,
        transaction: QpiTransaction,
        data: &[u8],
        timing: TimingConfig,
    ) -> Result<(), Self::Error>;

    /// Executes exactly one QPI read transaction.
    fn read_qpi_chunk(
        &mut self,
        resources: &mut R,
        transaction: QpiTransaction,
        buf: &mut [u8],
        timing: TimingConfig,
    ) -> Result<(), Self::Error>;
}

/// Blocking RP2040 QPI backend using externally owned Embassy-compatible
/// resources.
#[derive(Debug)]
pub struct Rp2040QpiBackend<R, E> {
    resources: R,
    executor: E,
    pins: Option<Pins>,
    timing: TimingConfig,
}

impl<R, E> Rp2040QpiBackend<R, E> {
    /// Creates a backend from owned resources and a low-level executor.
    pub const fn new(resources: R, executor: E) -> Self {
        Self {
            resources,
            executor,
            pins: None,
            timing: TimingConfig::DEFAULT,
        }
    }

    /// Returns the configured pins, if [`BlockingPio::configure`] has run.
    pub const fn configured_pins(&self) -> Option<Pins> {
        self.pins
    }

    /// Returns the currently configured timing.
    pub const fn timing(&self) -> TimingConfig {
        self.timing
    }

    /// Releases the resource bundle and executor.
    pub fn into_parts(self) -> (R, E) {
        (self.resources, self.executor)
    }
}

impl<R, E> BlockingPio for Rp2040QpiBackend<R, E>
where
    E: Rp2040QpiExecutor<R>,
{
    type Error = E::Error;

    fn configure(&mut self, pins: Pins, timing: TimingConfig) -> Result<(), Self::Error> {
        if !pins.validate() || !timing.validate() {
            return Err(PsramError::InvalidState.into());
        }

        self.executor.configure(&mut self.resources, pins, timing)?;
        self.pins = Some(pins);
        self.timing = timing;
        Ok(())
    }

    fn exit_qpi_quad(&mut self) -> Result<(), Self::Error> {
        self.executor
            .exit_qpi_quad(&mut self.resources, self.timing)
    }

    fn exit_qpi_spi(&mut self) -> Result<(), Self::Error> {
        self.executor.exit_qpi_spi(&mut self.resources, self.timing)
    }

    fn read_id_spi(&mut self) -> Result<DeviceId, Self::Error> {
        self.executor.read_id_spi(&mut self.resources, self.timing)
    }

    fn enter_qpi_spi(&mut self) -> Result<(), Self::Error> {
        self.executor
            .enter_qpi_spi(&mut self.resources, self.timing)
    }

    fn read_qpi_chunk(
        &mut self,
        transaction: QpiTransaction,
        buf: &mut [u8],
    ) -> Result<(), Self::Error> {
        validate_chunk(transaction, MemoryOp::Read, buf.len(), self.timing)?;
        self.executor
            .read_qpi_chunk(&mut self.resources, transaction, buf, self.timing)
    }

    fn write_qpi_chunk(
        &mut self,
        transaction: QpiTransaction,
        data: &[u8],
    ) -> Result<(), Self::Error> {
        validate_chunk(transaction, MemoryOp::Write, data.len(), self.timing)?;
        self.executor
            .write_qpi_chunk(&mut self.resources, transaction, data, self.timing)
    }
}

pub(super) fn validate_chunk<E>(
    transaction: QpiTransaction,
    op: MemoryOp,
    slice_len: usize,
    timing: TimingConfig,
) -> Result<(), E>
where
    E: From<PsramError>,
{
    if transaction.op != op
        || transaction.command != op.command()
        || transaction.data_direction != op.data_direction()
        || transaction.len != slice_len
        || transaction.len > timing.max_chunk_len
    {
        return Err(PsramError::InvalidState.into());
    }

    Ok(())
}

#[cfg(any(test, target_os = "none"))]
pub(super) fn qpi_dummy_transfer_bytes(dummy_cycles: u8) -> usize {
    (usize::from(dummy_cycles) + 1) / 2
}
