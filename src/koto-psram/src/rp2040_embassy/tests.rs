use crate::{
    addr::PsramAddr,
    bus::PsramBus,
    config::{Pins, TimingConfig},
    device::DeviceId,
    error::PsramError,
    pio::blocking::{BlockingDriver, BlockingPio},
    protocol::QpiTransaction,
};

use super::{boundary::qpi_dummy_transfer_bytes, Rp2040QpiBackend, Rp2040QpiExecutor};

struct Resources {
    memory: [u8; 1024],
}

impl Default for Resources {
    fn default() -> Self {
        Self { memory: [0; 1024] }
    }
}

#[derive(Default)]
struct Executor {
    last_read: Option<QpiTransaction>,
    last_write: Option<QpiTransaction>,
    fail_after_write: bool,
}

impl Rp2040QpiExecutor<Resources> for Executor {
    type Error = PsramError;

    fn configure(
        &mut self,
        _resources: &mut Resources,
        _pins: Pins,
        _timing: TimingConfig,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn exit_qpi_quad(
        &mut self,
        _resources: &mut Resources,
        _timing: TimingConfig,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn exit_qpi_spi(
        &mut self,
        _resources: &mut Resources,
        _timing: TimingConfig,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn read_id_spi(
        &mut self,
        _resources: &mut Resources,
        _timing: TimingConfig,
    ) -> Result<DeviceId, Self::Error> {
        Ok(DeviceId::new([0x0d, 0x5d, 0x5d]))
    }

    fn enter_qpi_spi(
        &mut self,
        _resources: &mut Resources,
        _timing: TimingConfig,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn write_qpi_chunk(
        &mut self,
        resources: &mut Resources,
        transaction: QpiTransaction,
        data: &[u8],
        _timing: TimingConfig,
    ) -> Result<(), Self::Error> {
        self.last_write = Some(transaction);
        let start = transaction_addr(transaction) as usize;
        let end = start + data.len();
        resources.memory[start..end].copy_from_slice(data);
        if self.fail_after_write {
            Err(PsramError::HardwareFault)
        } else {
            Ok(())
        }
    }

    fn read_qpi_chunk(
        &mut self,
        resources: &mut Resources,
        transaction: QpiTransaction,
        buf: &mut [u8],
        _timing: TimingConfig,
    ) -> Result<(), Self::Error> {
        self.last_read = Some(transaction);
        let start = transaction_addr(transaction) as usize;
        let end = start + buf.len();
        buf.copy_from_slice(&resources.memory[start..end]);
        Ok(())
    }
}

fn transaction_addr(transaction: QpiTransaction) -> u32 {
    ((transaction.addr[0] as u32) << 16)
        | ((transaction.addr[1] as u32) << 8)
        | transaction.addr[2] as u32
}

#[test]
fn backend_consumes_one_transaction_per_driver_chunk() {
    let timing = TimingConfig {
        max_chunk_len: 256,
        ..TimingConfig::DEFAULT
    };
    let backend = Rp2040QpiBackend::new(Resources::default(), Executor::default());
    let mut driver = BlockingDriver::with_config(backend, Pins::PICOCALC, timing);
    driver.init().unwrap();

    let data = [0x5a; 257];
    driver
        .write_all(PsramAddr::new(0x20).unwrap(), &data)
        .unwrap();
    let mut out = [0; 257];
    driver
        .read_exact(PsramAddr::new(0x20).unwrap(), &mut out)
        .unwrap();

    assert_eq!(out, data);
    let (_resources, executor) = driver.into_inner().into_parts();
    assert_eq!(
        executor.last_write,
        Some(QpiTransaction::write(
            PsramAddr::new(0x120).unwrap(),
            1,
            timing
        ))
    );
    assert_eq!(
        executor.last_read,
        Some(QpiTransaction::read(
            PsramAddr::new(0x120).unwrap(),
            1,
            timing
        ))
    );
}

#[test]
fn backend_rejects_unbounded_chunk() {
    let timing = TimingConfig {
        max_chunk_len: 4,
        ..TimingConfig::DEFAULT
    };
    let mut backend = Rp2040QpiBackend::new(Resources::default(), Executor::default());
    backend.configure(Pins::PICOCALC, timing).unwrap();
    let tx = QpiTransaction::write(PsramAddr::zero(), 5, timing);
    assert_eq!(
        backend.write_qpi_chunk(tx, &[0; 5]),
        Err(PsramError::InvalidState)
    );
}

#[test]
fn backend_rejects_unbounded_read_chunk() {
    let timing = TimingConfig {
        max_chunk_len: 4,
        ..TimingConfig::DEFAULT
    };
    let mut backend = Rp2040QpiBackend::new(Resources::default(), Executor::default());
    backend.configure(Pins::PICOCALC, timing).unwrap();
    let tx = QpiTransaction::read(PsramAddr::zero(), 5, timing);
    let mut out = [0; 5];

    assert_eq!(
        backend.read_qpi_chunk(tx, &mut out),
        Err(PsramError::InvalidState)
    );

    let (_resources, executor) = backend.into_parts();
    assert_eq!(executor.last_read, None);
}

#[test]
fn backend_rejects_invalid_write_op_before_executor() {
    let timing = TimingConfig::DEFAULT;
    let mut backend = Rp2040QpiBackend::new(Resources::default(), Executor::default());
    backend.configure(Pins::PICOCALC, timing).unwrap();
    let tx = QpiTransaction::read(PsramAddr::zero(), 4, timing);

    assert_eq!(
        backend.write_qpi_chunk(tx, &[0; 4]),
        Err(PsramError::InvalidState)
    );

    let (_resources, executor) = backend.into_parts();
    assert_eq!(executor.last_write, None);
}

#[test]
fn backend_rejects_invalid_write_length_before_executor() {
    let timing = TimingConfig::DEFAULT;
    let mut backend = Rp2040QpiBackend::new(Resources::default(), Executor::default());
    backend.configure(Pins::PICOCALC, timing).unwrap();
    let tx = QpiTransaction::write(PsramAddr::zero(), 4, timing);

    assert_eq!(
        backend.write_qpi_chunk(tx, &[0; 3]),
        Err(PsramError::InvalidState)
    );

    let (_resources, executor) = backend.into_parts();
    assert_eq!(executor.last_write, None);
}

#[test]
fn backend_rejects_invalid_read_op_before_executor() {
    let timing = TimingConfig::DEFAULT;
    let mut backend = Rp2040QpiBackend::new(Resources::default(), Executor::default());
    backend.configure(Pins::PICOCALC, timing).unwrap();
    let tx = QpiTransaction::write(PsramAddr::zero(), 4, timing);
    let mut out = [0; 4];

    assert_eq!(
        backend.read_qpi_chunk(tx, &mut out),
        Err(PsramError::InvalidState)
    );

    let (_resources, executor) = backend.into_parts();
    assert_eq!(executor.last_read, None);
}

#[test]
fn backend_rejects_invalid_read_length_before_executor() {
    let timing = TimingConfig::DEFAULT;
    let mut backend = Rp2040QpiBackend::new(Resources::default(), Executor::default());
    backend.configure(Pins::PICOCALC, timing).unwrap();
    let tx = QpiTransaction::read(PsramAddr::zero(), 4, timing);
    let mut out = [0; 3];

    assert_eq!(
        backend.read_qpi_chunk(tx, &mut out),
        Err(PsramError::InvalidState)
    );

    let (_resources, executor) = backend.into_parts();
    assert_eq!(executor.last_read, None);
}

#[test]
fn qpi_dummy_cycles_are_counted_as_quad_clocks_not_bytes() {
    assert_eq!(qpi_dummy_transfer_bytes(0), 0);
    assert_eq!(qpi_dummy_transfer_bytes(1), 1);
    assert_eq!(qpi_dummy_transfer_bytes(2), 1);
    assert_eq!(qpi_dummy_transfer_bytes(6), 3);
    assert_eq!(qpi_dummy_transfer_bytes(7), 4);
}
