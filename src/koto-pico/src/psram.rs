//! PIO-backed PicoCalc PSRAM block-transfer backend.
//!
//! The PIO protocol is adapted from Ian Scott's MIT-licensed
//! `rp2040-psram` implementation. This backend intentionally exposes only
//! copies between PSRAM and caller-owned SRAM buffers.

#[cfg(feature = "psram_qpi_backend")]
use embassy_rp::gpio::Level;
use embassy_rp::{
    gpio::{Drive, SlewRate},
    pac, peripherals,
    pio::{Common, Config, Direction, LoadedProgram, Pin, ShiftDirection, StateMachine},
};
use embassy_time::{block_for, Duration};
use koto_core::hal::{HalError, PsramHal};
#[cfg(feature = "psram_dma_read_code_window")]
use koto_core::psram::PsramError;

#[cfg(all(
    feature = "psram_dma_read_code_window",
    feature = "psram_qpi_safe_read_code_window"
))]
compile_error!(
    "features `psram_dma_read_code_window` and `psram_qpi_safe_read_code_window` are mutually exclusive"
);

#[cfg(all(
    feature = "psram_dma_read_code_window",
    feature = "psram_dma_read_code_window_diag"
))]
use core::sync::atomic::{AtomicU32, AtomicU8, Ordering};

pub const PSRAM_CAPACITY: u32 = 8 * 1024 * 1024;
pub const PSRAM_PROD_READ_CHUNK_BYTES: usize = 16;
pub const PSRAM_FAST_READ_DUMMY_CYCLES: u8 = 8;
pub const PSRAM_PIO_SYS_HZ: u32 = 133_000_000;
#[cfg(feature = "psram_dma_read_code_window")]
pub const PSRAM_PIO_CLOCK_DIVIDER: u32 = 3;
#[cfg(not(feature = "psram_dma_read_code_window"))]
pub const PSRAM_PIO_CLOCK_DIVIDER: u32 = 4;
pub const PSRAM_PIO_SM_HZ: u32 = PSRAM_PIO_SYS_HZ / PSRAM_PIO_CLOCK_DIVIDER;
pub const PSRAM_PIO_CYCLES_PER_BIT: u8 = 2;
#[cfg(feature = "psram_qpi_backend")]
pub const PSRAM_QPI_CHUNK_BYTES: usize = 64;
#[cfg(feature = "psram_qpi_backend")]
pub const PSRAM_QPI_SAFE_READ_CHUNK_BYTES: usize = 120;
#[cfg(feature = "psram_qpi_backend")]
pub const PSRAM_QPI_LARGE_CHUNK_BYTES: usize = 120;
#[cfg(feature = "psram_qpi_backend")]
pub const PSRAM_QPI_EXT_READ_MAX_CHUNK_BYTES: usize = 4096;
#[cfg(feature = "psram_qpi_backend")]
pub const PSRAM_QPI_CLOCK_DIVIDER: u32 = 16;
#[cfg(feature = "psram_qpi_backend")]
pub const PSRAM_QPI_SM_HZ: u32 = PSRAM_PIO_SYS_HZ / PSRAM_QPI_CLOCK_DIVIDER;
#[cfg(feature = "psram_qpi_backend")]
pub const PSRAM_QPI_DUMMY_NIBBLES: u8 = 6;
#[cfg(feature = "psram_qpi_backend")]
pub const PSRAM_QPI_SAFE_READ_CLOCK_DIVIDER: u32 = 6;
#[cfg(feature = "psram_qpi_backend")]
pub const PSRAM_QPI_SAFE_READ_SM_HZ: u32 = PSRAM_PIO_SYS_HZ / PSRAM_QPI_SAFE_READ_CLOCK_DIVIDER;
#[cfg(feature = "psram_qpi_backend")]
pub const PSRAM_QPI_SAFE_WRITE_CLOCK_DIVIDER: u32 = 6;
#[cfg(feature = "psram_qpi_backend")]
pub const PSRAM_QPI_SAFE_WRITE_CHUNK_BYTES: usize = 120;
#[cfg(feature = "psram_qpi_backend")]
pub const PSRAM_QPI_V2_READ_CLOCK_DIVIDER: u32 = 8;
#[cfg(feature = "psram_qpi_backend")]
pub const PSRAM_QPI_V2_WRITE_CLOCK_DIVIDER: u32 = 2;
#[cfg(feature = "psram_qpi_backend")]
pub const PSRAM_QPI_V2_CHUNK_BYTES: usize = 120;

#[cfg(feature = "psram_qpi_backend")]
const PSRAM_QPI_FIFO_TIMEOUT_ITERS: u32 = 2_000_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PicoCalcPsramDiagState {
    pub clkdiv: u32,
    pub clkdiv_frac: u8,
    pub sm_hz: u32,
    pub cycles_per_bit: u8,
    pub flevel: u32,
    pub fstat: u32,
    pub fdebug: u32,
    pub rx_autopush: bool,
    pub rx_fjoin: bool,
    pub rx_threshold: u8,
    pub tx_autopull: bool,
    pub tx_fjoin: bool,
    pub tx_threshold: u8,
    pub qpi_input_sync_bypass: bool,
}

#[cfg(feature = "psram_dma_read_code_window")]
const DMA_TRACE_MODE_LEGACY: u8 = 1;
#[cfg(all(
    feature = "psram_dma_read_code_window",
    feature = "psram_dma_read_code_window_diag"
))]
const DMA_TRACE_MODE_DMA: u8 = 2;
#[allow(dead_code)]
const DMA_TRACE_MODE_DMA_FALLBACK: u8 = 3;
#[cfg(feature = "psram_dma_read_code_window")]
const DMA_TRACE_MODE_PHASE_EDGE_FUDGE: u8 = 4;

#[cfg(feature = "psram_dma_read_code_window")]
const DMA_ERR_NONE: u8 = 0;
#[cfg(all(
    feature = "psram_dma_read_code_window",
    feature = "psram_dma_read_code_window_diag"
))]
const DMA_ERR_UNAVAILABLE: u8 = 1;
#[cfg(all(
    feature = "psram_dma_read_code_window",
    feature = "psram_dma_read_code_window_diag"
))]
const DMA_ERR_OUT_OF_RANGE: u8 = 2;
#[cfg(all(
    feature = "psram_dma_read_code_window",
    feature = "psram_dma_read_code_window_diag"
))]
const DMA_ERR_BLOCK_SIZE: u8 = 3;
#[cfg(all(
    feature = "psram_dma_read_code_window",
    feature = "psram_dma_read_code_window_diag"
))]
const DMA_ERR_INVALID_ARG: u8 = 4;
#[cfg(all(
    feature = "psram_dma_read_code_window",
    feature = "psram_dma_read_code_window_diag"
))]
const DMA_ERR_UNSUPPORTED: u8 = 5;
#[cfg(all(
    feature = "psram_dma_read_code_window",
    feature = "psram_dma_read_code_window_diag"
))]
const DMA_ERR_OTHER_HAL: u8 = 6;

#[cfg(all(
    feature = "psram_dma_read_code_window",
    feature = "psram_dma_read_code_window_diag"
))]
static DMA_TRACE_LAST_ADDR: AtomicU32 = AtomicU32::new(0);
#[cfg(all(
    feature = "psram_dma_read_code_window",
    feature = "psram_dma_read_code_window_diag"
))]
static DMA_TRACE_LAST_LEN: AtomicU32 = AtomicU32::new(0);
#[cfg(all(
    feature = "psram_dma_read_code_window",
    feature = "psram_dma_read_code_window_diag"
))]
static DMA_TRACE_LAST_MODE: AtomicU8 = AtomicU8::new(0);
#[cfg(all(
    feature = "psram_dma_read_code_window",
    feature = "psram_dma_read_code_window_diag"
))]
static DMA_TRACE_LAST_ERROR: AtomicU8 = AtomicU8::new(0);
#[cfg(all(
    feature = "psram_dma_read_code_window",
    feature = "psram_dma_read_code_window_diag"
))]
static DMA_TRACE_DMA_ATTEMPTS: AtomicU32 = AtomicU32::new(0);
#[cfg(all(
    feature = "psram_dma_read_code_window",
    feature = "psram_dma_read_code_window_diag"
))]
static DMA_TRACE_DMA_SUCCESSES: AtomicU32 = AtomicU32::new(0);
#[cfg(all(
    feature = "psram_dma_read_code_window",
    feature = "psram_dma_read_code_window_diag"
))]
static DMA_TRACE_DMA_FALLBACKS: AtomicU32 = AtomicU32::new(0);

#[cfg(all(
    feature = "psram_dma_read_code_window",
    feature = "psram_dma_read_code_window_diag"
))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DmaReadTraceMode {
    None,
    Legacy,
    Dma,
    DmaFallback,
    PhaseEdgeFudge,
}

#[cfg(all(
    feature = "psram_dma_read_code_window",
    feature = "psram_dma_read_code_window_diag"
))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DmaCodeWindowReadTrace {
    pub last_addr: u32,
    pub last_len: u32,
    pub last_mode: DmaReadTraceMode,
    pub last_dma_error: u8,
    pub dma_attempts: u32,
    pub dma_successes: u32,
    pub dma_fallbacks: u32,
}

#[cfg(all(
    feature = "psram_dma_read_code_window",
    feature = "psram_dma_read_code_window_diag"
))]
pub fn dma_code_window_read_trace_snapshot() -> DmaCodeWindowReadTrace {
    let last_mode = match DMA_TRACE_LAST_MODE.load(Ordering::Relaxed) {
        DMA_TRACE_MODE_LEGACY => DmaReadTraceMode::Legacy,
        DMA_TRACE_MODE_DMA => DmaReadTraceMode::Dma,
        DMA_TRACE_MODE_DMA_FALLBACK => DmaReadTraceMode::DmaFallback,
        DMA_TRACE_MODE_PHASE_EDGE_FUDGE => DmaReadTraceMode::PhaseEdgeFudge,
        _ => DmaReadTraceMode::None,
    };
    DmaCodeWindowReadTrace {
        last_addr: DMA_TRACE_LAST_ADDR.load(Ordering::Relaxed),
        last_len: DMA_TRACE_LAST_LEN.load(Ordering::Relaxed),
        last_mode,
        last_dma_error: DMA_TRACE_LAST_ERROR.load(Ordering::Relaxed),
        dma_attempts: DMA_TRACE_DMA_ATTEMPTS.load(Ordering::Relaxed),
        dma_successes: DMA_TRACE_DMA_SUCCESSES.load(Ordering::Relaxed),
        dma_fallbacks: DMA_TRACE_DMA_FALLBACKS.load(Ordering::Relaxed),
    }
}

#[cfg(all(
    feature = "psram_dma_read_code_window",
    feature = "psram_dma_read_code_window_diag"
))]
#[inline]
fn classify_dma_error(err: PsramError) -> u8 {
    match err {
        PsramError::Unavailable => DMA_ERR_UNAVAILABLE,
        PsramError::OutOfRange => DMA_ERR_OUT_OF_RANGE,
        PsramError::BlockSizeMismatch => DMA_ERR_BLOCK_SIZE,
        PsramError::Hal(HalError::InvalidArgument) => DMA_ERR_INVALID_ARG,
        PsramError::Hal(HalError::Unsupported) => DMA_ERR_UNSUPPORTED,
        PsramError::Hal(_) => DMA_ERR_OTHER_HAL,
    }
}

#[cfg(all(
    feature = "psram_dma_read_code_window",
    feature = "psram_dma_read_code_window_diag"
))]
#[inline]
fn update_dma_trace(address: u32, len: usize, mode: u8, err: u8) {
    DMA_TRACE_LAST_ADDR.store(address, Ordering::Relaxed);
    DMA_TRACE_LAST_LEN.store(len as u32, Ordering::Relaxed);
    DMA_TRACE_LAST_MODE.store(mode, Ordering::Relaxed);
    DMA_TRACE_LAST_ERROR.store(err, Ordering::Relaxed);
}

#[cfg(all(
    feature = "psram_dma_read_code_window",
    feature = "psram_dma_read_code_window_diag"
))]
#[inline]
fn trace_inc(counter: &AtomicU32) {
    let value = counter.load(Ordering::Relaxed);
    counter.store(value.wrapping_add(1), Ordering::Relaxed);
}

#[cfg(all(
    feature = "psram_dma_read_code_window",
    not(feature = "psram_dma_read_code_window_diag")
))]
#[inline]
fn update_dma_trace(_address: u32, _len: usize, _mode: u8, _err: u8) {}

#[cfg(feature = "psram_dma_read_code_window")]
pub struct DmaCodeWindowPsram<'d> {
    inner: PicoCalcPsram<'d>,
}

#[cfg(feature = "psram_dma_read_code_window")]
impl<'d> DmaCodeWindowPsram<'d> {
    pub const fn new(inner: PicoCalcPsram<'d>) -> Self {
        Self { inner }
    }

    pub fn read_legacy_for_diag(&mut self, address: u32, dst: &mut [u8]) -> Result<(), HalError> {
        self.inner.read(address, dst)
    }

    pub fn read_dma_for_diag(&mut self, address: u32, dst: &mut [u8]) -> Result<(), PsramError> {
        crate::psram_dma::read_phase_edge_fudge_clkdiv3(&mut self.inner, address, dst)
    }
}

#[cfg(feature = "psram_dma_read_code_window")]
impl PsramHal for DmaCodeWindowPsram<'_> {
    fn available(&self) -> bool {
        true
    }

    fn read(&mut self, address: u32, dst: &mut [u8]) -> Result<(), HalError> {
        // For the clkdiv=3 experiment, keep CodeWindow reads on the proven
        // SM0 PIO path and only change timing (no DMA path).
        update_dma_trace(address, dst.len(), DMA_TRACE_MODE_LEGACY, DMA_ERR_NONE);
        self.inner.read(address, dst)
    }

    fn write(&mut self, address: u32, src: &[u8]) -> Result<(), HalError> {
        self.inner.write(address, src)
    }
}

#[cfg(feature = "psram_qpi_safe_read_code_window")]
pub struct QpiCodeWindowPsram<'d> {
    #[cfg(feature = "psram_qpi_backend_v2")]
    inner: PicoCalcPsramQpiV2<'d>,
    #[cfg(not(feature = "psram_qpi_backend_v2"))]
    inner: PicoCalcQpiPsram<'d>,
}

#[cfg(all(
    feature = "psram_qpi_safe_read_code_window",
    not(feature = "psram_qpi_code_window_counters")
))]
impl<'d> QpiCodeWindowPsram<'d> {
    #[cfg(feature = "psram_qpi_backend_v2")]
    pub fn new(inner: PicoCalcQpiPsram<'d>) -> Result<Self, HalError> {
        let inner = PicoCalcPsramQpiV2::new(inner)?;
        Ok(Self { inner })
    }

    #[cfg(not(feature = "psram_qpi_backend_v2"))]
    pub fn new(mut inner: PicoCalcQpiPsram<'d>) -> Result<Self, HalError> {
        inner.set_qpi_input_sync_bypass_for_diag(true);
        inner.set_clock_divider_for_diag(PSRAM_QPI_SAFE_READ_CLOCK_DIVIDER)?;
        Ok(Self { inner })
    }
}

#[cfg(all(
    feature = "psram_qpi_safe_read_code_window",
    feature = "psram_qpi_code_window_counters"
))]
impl<'d> QpiCodeWindowPsram<'d> {
    #[cfg(feature = "psram_qpi_backend_v2")]
    pub fn new(inner: PicoCalcQpiPsram<'d>) -> Result<Self, HalError> {
        let inner = PicoCalcPsramQpiV2::new(inner)?;
        Ok(Self { inner })
    }

    #[cfg(not(feature = "psram_qpi_backend_v2"))]
    pub fn new(mut inner: PicoCalcQpiPsram<'d>) -> Result<Self, HalError> {
        inner.set_qpi_input_sync_bypass_for_diag(true);
        inner.set_clock_divider_for_diag(PSRAM_QPI_SAFE_READ_CLOCK_DIVIDER)?;
        Ok(Self { inner })
    }

    pub fn read_qpi_for_verify(&mut self, address: u32, dst: &mut [u8]) -> Result<(), HalError> {
        #[cfg(feature = "psram_qpi_backend_v2")]
        {
            self.inner.read_qpi_for_verify(address, dst)
        }

        #[cfg(not(feature = "psram_qpi_backend_v2"))]
        {
            self.inner.set_qpi_input_sync_bypass_for_diag(true);
            self.inner
                .set_clock_divider_for_diag(PSRAM_QPI_SAFE_READ_CLOCK_DIVIDER)?;
            self.inner
                .read_cpu_for_diag(address, dst, PSRAM_QPI_SAFE_READ_CHUNK_BYTES)
        }
    }

    #[cfg(feature = "psram_qpi_backend_v2")]
    pub fn mode_for_diag(&self) -> PsramMode {
        self.inner.mode()
    }

    pub fn write_qpi_stage_chunk(&mut self, address: u32, src: &[u8]) -> Result<(), HalError> {
        #[cfg(feature = "psram_qpi_backend_v2")]
        {
            return self.inner.write(address, src);
        }

        #[cfg(not(feature = "psram_qpi_backend_v2"))]
        {
            self.inner.write_legacy_spi_for_stage(address, src)
        }
    }

    pub fn read_qpi_stage_for_verify(
        &mut self,
        address: u32,
        dst: &mut [u8],
    ) -> Result<(), HalError> {
        self.read_qpi_for_verify(address, dst)
    }

    pub fn begin_legacy_stage_mode(&mut self) -> Result<(), HalError> {
        #[cfg(feature = "psram_qpi_backend_v2")]
        {
            self.inner.begin_legacy_stage_mode()
        }

        #[cfg(not(feature = "psram_qpi_backend_v2"))]
        {
            self.inner.begin_legacy_stage_mode()
        }
    }

    pub fn write_legacy_stage_chunk(&mut self, address: u32, src: &[u8]) -> Result<(), HalError> {
        #[cfg(feature = "psram_qpi_backend_v2")]
        {
            self.inner.write_legacy_stage_chunk(address, src)
        }

        #[cfg(not(feature = "psram_qpi_backend_v2"))]
        {
            self.inner.write_legacy_stage_chunk(address, src)
        }
    }

    pub fn read_legacy_same_mode_for_verify(
        &mut self,
        address: u32,
        dst: &mut [u8],
    ) -> Result<(), HalError> {
        #[cfg(feature = "psram_qpi_backend_v2")]
        {
            self.inner.read_legacy_same_mode_for_verify(address, dst)
        }

        #[cfg(not(feature = "psram_qpi_backend_v2"))]
        {
            self.inner.read_legacy_same_mode_for_verify(address, dst)
        }
    }

    pub fn finish_legacy_stage_mode(&mut self) -> Result<(), HalError> {
        #[cfg(feature = "psram_qpi_backend_v2")]
        {
            self.inner.finish_legacy_stage_mode()
        }

        #[cfg(not(feature = "psram_qpi_backend_v2"))]
        {
            self.inner.finish_legacy_stage_mode()
        }
    }

    pub fn read_serial_for_verify(&mut self, address: u32, dst: &mut [u8]) -> Result<(), HalError> {
        #[cfg(feature = "psram_qpi_backend_v2")]
        {
            self.inner.read_serial_for_verify(address, dst)
        }

        #[cfg(not(feature = "psram_qpi_backend_v2"))]
        {
            self.inner.read_serial_for_verify(address, dst)
        }
    }
}

#[cfg(feature = "psram_qpi_safe_read_code_window")]
impl PsramHal for QpiCodeWindowPsram<'_> {
    fn available(&self) -> bool {
        true
    }

    fn read(&mut self, address: u32, dst: &mut [u8]) -> Result<(), HalError> {
        #[cfg(feature = "psram_qpi_backend_v2")]
        {
            self.inner.read(address, dst)
        }

        #[cfg(not(feature = "psram_qpi_backend_v2"))]
        {
            self.inner
                .set_clock_divider_for_diag(PSRAM_QPI_SAFE_READ_CLOCK_DIVIDER)?;
            self.inner
                .read_cpu_for_diag(address, dst, PSRAM_QPI_SAFE_READ_CHUNK_BYTES)
        }
    }

    fn write(&mut self, address: u32, src: &[u8]) -> Result<(), HalError> {
        #[cfg(feature = "psram_qpi_backend_v2")]
        {
            self.inner.write(address, src)
        }

        #[cfg(not(feature = "psram_qpi_backend_v2"))]
        {
            self.inner.write_legacy_spi_for_stage(address, src)
        }
    }
}

#[cfg(feature = "psram_qpi_backend_v2")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PsramMode {
    Unknown,
    QpiRw,
    QpiWriteOnly,
    RecoverSerial,
}

#[cfg(feature = "psram_qpi_backend_v2")]
pub struct PicoCalcPsramQpiV2<'d> {
    inner: PicoCalcQpiPsram<'d>,
    mode: PsramMode,
    read_clkdiv: u32,
    write_clkdiv: u32,
    chunk_bytes: usize,
}

#[cfg(feature = "psram_qpi_backend_v2")]
impl<'d> PicoCalcPsramQpiV2<'d> {
    pub fn new(inner: PicoCalcQpiPsram<'d>) -> Result<Self, HalError> {
        let mut this = Self {
            inner,
            mode: PsramMode::Unknown,
            read_clkdiv: PSRAM_QPI_V2_READ_CLOCK_DIVIDER,
            write_clkdiv: PSRAM_QPI_V2_WRITE_CLOCK_DIVIDER,
            chunk_bytes: PSRAM_QPI_V2_CHUNK_BYTES,
        };
        this.ensure_qpi_rw()?;
        Ok(this)
    }

    pub fn mode(&self) -> PsramMode {
        self.mode
    }

    pub fn ensure_qpi_rw(&mut self) -> Result<(), HalError> {
        match self.mode {
            PsramMode::QpiRw => {}
            PsramMode::RecoverSerial => {
                self.enter_qpi()?;
            }
            PsramMode::Unknown | PsramMode::QpiWriteOnly => {
                self.inner.apply_rw_sm_config();
                self.inner.set_qpi_input_sync_bypass_for_diag(true);
                self.inner.set_clock_divider_for_diag(self.read_clkdiv)?;
            }
        }
        self.mode = PsramMode::QpiRw;
        Ok(())
    }

    pub fn ensure_qpi_write_only(&mut self) -> Result<(), HalError> {
        self.ensure_qpi_rw()?;
        self.inner.set_clock_divider_for_diag(self.write_clkdiv)?;
        self.mode = PsramMode::QpiWriteOnly;
        Ok(())
    }

    pub fn recover_to_serial(&mut self) -> Result<(), HalError> {
        self.inner.recover_to_serial_bus()?;
        self.mode = PsramMode::RecoverSerial;
        Ok(())
    }

    pub fn enter_qpi(&mut self) -> Result<(), HalError> {
        self.inner.enter_qpi_bus()?;
        self.inner.set_clock_divider_for_diag(self.read_clkdiv)?;
        self.mode = PsramMode::QpiRw;
        Ok(())
    }

    pub fn read(&mut self, address: u32, dst: &mut [u8]) -> Result<(), HalError> {
        self.ensure_qpi_rw()?;
        self.inner.read_cpu_for_diag(address, dst, self.chunk_bytes)
    }

    pub fn write(&mut self, address: u32, src: &[u8]) -> Result<(), HalError> {
        self.ensure_qpi_write_only()?;
        self.inner
            .write_4wire_for_diag(address, src, self.chunk_bytes)
    }

    #[cfg(feature = "psram_qpi_code_window_counters")]
    pub fn read_qpi_for_verify(&mut self, address: u32, dst: &mut [u8]) -> Result<(), HalError> {
        self.read(address, dst)
    }

    #[cfg(feature = "psram_qpi_code_window_counters")]
    pub fn begin_legacy_stage_mode(&mut self) -> Result<(), HalError> {
        self.recover_to_serial()
    }

    #[cfg(feature = "psram_qpi_code_window_counters")]
    pub fn write_legacy_stage_chunk(&mut self, address: u32, src: &[u8]) -> Result<(), HalError> {
        if self.mode != PsramMode::RecoverSerial {
            self.recover_to_serial()?;
        }
        self.inner.write_legacy_stage_chunk(address, src)
    }

    #[cfg(feature = "psram_qpi_code_window_counters")]
    pub fn read_legacy_same_mode_for_verify(
        &mut self,
        address: u32,
        dst: &mut [u8],
    ) -> Result<(), HalError> {
        if self.mode != PsramMode::RecoverSerial {
            self.recover_to_serial()?;
        }
        self.inner.read_legacy_same_mode_for_verify(address, dst)
    }

    #[cfg(feature = "psram_qpi_code_window_counters")]
    pub fn finish_legacy_stage_mode(&mut self) -> Result<(), HalError> {
        self.enter_qpi()
    }

    #[cfg(feature = "psram_qpi_code_window_counters")]
    pub fn read_serial_for_verify(&mut self, address: u32, dst: &mut [u8]) -> Result<(), HalError> {
        self.inner.read_serial_for_verify(address, dst)?;
        self.mode = PsramMode::QpiRw;
        Ok(())
    }
}

#[cfg(feature = "psram_qpi_safe_read_code_window")]
pub type FirmwarePsramHal<'d> = QpiCodeWindowPsram<'d>;
#[cfg(feature = "psram_dma_read_code_window")]
pub type FirmwarePsramHal<'d> = DmaCodeWindowPsram<'d>;

// Default config: the extracted `koto-psram` crate adapter is the default
// firmware backend. Enabling `legacy_psram` reverts to the in-tree
// `PicoCalcPsram` SM0 PIO backend below.
#[cfg(all(
    not(feature = "psram_dma_read_code_window"),
    not(feature = "psram_qpi_safe_read_code_window"),
    not(feature = "legacy_psram")
))]
pub type FirmwarePsramHal<'d> = crate::psram_ext::KotoPsram<'d>;

#[cfg(all(
    not(feature = "psram_dma_read_code_window"),
    not(feature = "psram_qpi_safe_read_code_window"),
    feature = "legacy_psram"
))]
pub type FirmwarePsramHal<'d> = PicoCalcPsram<'d>;

#[cfg(feature = "psram_qpi_backend")]
pub struct PicoCalcQpiPsram<'d> {
    sm: StateMachine<'d, peripherals::PIO1, 0>,
    _program: LoadedProgram<'d, peripherals::PIO1>,
    _read16_program: LoadedProgram<'d, peripherals::PIO1>,
    #[cfg(feature = "psram_qpi_safe_read_code_window")]
    _serial_program: LoadedProgram<'d, peripherals::PIO1>,
    _write4_program: LoadedProgram<'d, peripherals::PIO1>,
    _cs: Pin<'d, peripherals::PIO1>,
    _sck: Pin<'d, peripherals::PIO1>,
    _sio0: Pin<'d, peripherals::PIO1>,
    _sio1: Pin<'d, peripherals::PIO1>,
    _sio2: Pin<'d, peripherals::PIO1>,
    _sio3: Pin<'d, peripherals::PIO1>,
}

#[cfg(feature = "psram_qpi_backend")]
impl<'d> PicoCalcQpiPsram<'d> {
    pub fn new(
        common: &mut Common<'d, peripherals::PIO1>,
        mut sm: StateMachine<'d, peripherals::PIO1, 0>,
        cs: embassy_rp::Peri<'d, peripherals::PIN_20>,
        sck: embassy_rp::Peri<'d, peripherals::PIN_21>,
        sio0: embassy_rp::Peri<'d, peripherals::PIN_2>,
        sio1: embassy_rp::Peri<'d, peripherals::PIN_3>,
        sio2: embassy_rp::Peri<'d, peripherals::PIN_4>,
        sio3: embassy_rp::Peri<'d, peripherals::PIN_5>,
    ) -> Result<Self, HalError> {
        sm.set_enable(false);
        sm.clear_fifos();
        sm.restart();

        // Matches Picoware `psram_send_spi_command`: the PSRAM powers up in
        // 1-bit SPI mode, so reset and enter-QPI are sent via SIO bit-bang
        // before the pins are handed to PIO.
        bitbang_spi_command(0x66);
        block_for(Duration::from_micros(50));
        bitbang_spi_command(0x99);
        block_for(Duration::from_micros(100));
        bitbang_spi_command(0x35);
        block_for(Duration::from_micros(50));

        let program = pio::pio_asm!(
            r#"
                ; Port of docs/Reference/PicoCalc/picoware_psram/psram_qspi.pio
                ; qspi_psram_rw. First byte = output nibble count, second byte
                ; = input nibble loop count. Counts are u8, so qpi_cpu64 keeps
                ; 64B chunks to stay below the reference limits.
                ;
                ; Intentional difference from Picoware:
                ; Picoware inserts `readloop_entry: jmp readloop_mid side 0b00`
                ; before the first `in pins, 4`. On this KotoOS/Embassy SM0
                ; path that drops the first nibble (`0d...` becomes `d...`).
                ; Sample immediately after switching SIO0-3 to inputs and pass
                ; input_nibbles_minus_1 from Rust instead.
                .side_set 2
                .wrap_target
                begin:
                    out x, 8            side 0b01
                    out y, 8            side 0b01
                    jmp x--, writeloop  side 0b00
                writeloop:
                    out pins, 4         side 0b00
                    jmp x--, writeloop  side 0b10
                    jmp !y, begin       side 0b00
                    set pindirs, 0      side 0b10
                readloop:
                    in pins, 4          side 0b00
                    jmp y--, readloop   side 0b10
                    set pindirs, 0xF    side 0b01
                .wrap
            "#,
            options(max_program_size = 32)
        );
        let loaded = common.load_program(&program.program);
        let read16_program = pio::pio_asm!(
            r#"
                ; qspi_psram_read16: read-only QPI prototype with a 16-bit
                ; input nibble loop count. Command format:
                ;   u8 output_nibbles, u16 input_nibbles_minus_1, opcode/address/dummy.
                ; Uses the same KotoOS read-phase correction as qspi_psram_rw.
                .side_set 2
                .wrap_target
                begin:
                    out x, 8            side 0b01
                    out y, 16           side 0b01
                    jmp x--, writeloop  side 0b00
                writeloop:
                    out pins, 4         side 0b00
                    jmp x--, writeloop  side 0b10
                    set pindirs, 0      side 0b10
                readloop:
                    in pins, 4          side 0b00
                    jmp y--, readloop   side 0b10
                    set pindirs, 0xF    side 0b01
                .wrap
            "#,
            options(max_program_size = 32)
        );
        let read16_loaded = common.load_program(&read16_program.program);
        #[cfg(feature = "psram_qpi_safe_read_code_window")]
        let serial_program = pio::pio_asm!(
            r#"
                .side_set 2
                .wrap_target
                begin:
                    out x, 8            side 0b01
                    out y, 8            side 0b01
                    jmp x--, writeloop  side 0b01
                writeloop:
                    out pins, 1         side 0b00
                    jmp x--, writeloop  side 0b10
                    jmp !y, begin       side 0b00
                readloop:
                    in pins, 1          side 0b10
                    jmp y--, readloop   side 0b00
                .wrap
            "#,
            options(max_program_size = 32)
        );
        #[cfg(feature = "psram_qpi_safe_read_code_window")]
        let serial_loaded = common.load_program(&serial_program.program);
        let write4_program = pio::pio_asm!(
            r#"
                ; Port of Picoware qspi_4wire_write. This program only clocks
                ; 4-bit output data; CS is controlled by the diagnostic wrapper.
                .side_set 1 opt
                .wrap_target
                    out pins, 4        side 0
                    nop                side 1
                .wrap
            "#,
            options(max_program_size = 8)
        );
        let write4_loaded = common.load_program(&write4_program.program);

        let mut cs = common.make_pio_pin(cs);
        let mut sck = common.make_pio_pin(sck);
        let mut sio0 = common.make_pio_pin(sio0);
        let mut sio1 = common.make_pio_pin(sio1);
        let mut sio2 = common.make_pio_pin(sio2);
        let mut sio3 = common.make_pio_pin(sio3);
        for pin in [
            &mut cs, &mut sck, &mut sio0, &mut sio1, &mut sio2, &mut sio3,
        ] {
            pin.set_drive_strength(Drive::_8mA);
            pin.set_slew_rate(SlewRate::Fast);
        }
        sio0.set_input_sync_bypass(true);
        sio1.set_input_sync_bypass(true);
        sio2.set_input_sync_bypass(true);
        sio3.set_input_sync_bypass(true);

        let mut config = Config::default();
        config.use_program(&loaded, &[&cs, &sck]);
        config.set_out_pins(&[&sio0, &sio1, &sio2, &sio3]);
        config.set_in_pins(&[&sio0, &sio1, &sio2, &sio3]);
        config.set_set_pins(&[&sio0, &sio1, &sio2, &sio3]);
        config.shift_out.auto_fill = true;
        config.shift_out.direction = ShiftDirection::Left;
        config.shift_out.threshold = 8;
        config.shift_in.auto_fill = true;
        config.shift_in.direction = ShiftDirection::Left;
        config.shift_in.threshold = 8;
        config.clock_divider = (PSRAM_QPI_CLOCK_DIVIDER as u8).into();

        sm.set_config(&config);
        sm.set_pin_dirs(Direction::Out, &[&cs, &sck, &sio0, &sio1, &sio2, &sio3]);
        sm.clear_fifos();
        sm.restart();
        sm.clkdiv_restart();
        sm.set_enable(true);

        Ok(Self {
            sm,
            _program: loaded,
            _read16_program: read16_loaded,
            #[cfg(feature = "psram_qpi_safe_read_code_window")]
            _serial_program: serial_loaded,
            _write4_program: write4_loaded,
            _cs: cs,
            _sck: sck,
            _sio0: sio0,
            _sio1: sio1,
            _sio2: sio2,
            _sio3: sio3,
        })
    }

    pub fn recover_exit_qpi(&mut self) -> Result<(), HalError> {
        // Picoware deinit sends QPI exit command 0xF5 as one command byte:
        // output nibble count 2, input nibble count 0.
        self.write_only(&[2, 0, 0xF5])
    }

    fn recover_to_serial_bus(&mut self) -> Result<(), HalError> {
        self.recover_exit_qpi()?;
        self.sm.set_enable(false);
        self.sm.clear_fifos();
        self.sm.restart();
        self.sm.set_pins(Level::High, &[&self._cs]);
        self.set_qpi_input_sync_bypass_for_diag(false);
        Ok(())
    }

    fn enter_qpi_bus(&mut self) -> Result<(), HalError> {
        bitbang_spi_command(0x35);
        block_for(Duration::from_micros(50));
        self.apply_rw_sm_config();
        self.set_qpi_input_sync_bypass_for_diag(true);
        self.set_clock_divider_for_diag(PSRAM_QPI_SAFE_READ_CLOCK_DIVIDER)?;
        Ok(())
    }

    #[cfg(feature = "psram_qpi_code_window_counters")]
    pub fn begin_legacy_stage_mode(&mut self) -> Result<(), HalError> {
        self.recover_to_serial_bus()
    }

    #[cfg(feature = "psram_qpi_code_window_counters")]
    pub fn write_legacy_stage_chunk(&mut self, address: u32, src: &[u8]) -> Result<(), HalError> {
        Self::check_range(address, src.len())?;
        bitbang_spi_write(address, src)
    }

    #[cfg(feature = "psram_qpi_code_window_counters")]
    pub fn read_legacy_same_mode_for_verify(
        &mut self,
        address: u32,
        dst: &mut [u8],
    ) -> Result<(), HalError> {
        Self::check_range(address, dst.len())?;
        bitbang_spi_read(address, dst)
    }

    #[cfg(feature = "psram_qpi_code_window_counters")]
    pub fn finish_legacy_stage_mode(&mut self) -> Result<(), HalError> {
        self.enter_qpi_bus()
    }

    #[cfg(feature = "psram_qpi_code_window_counters")]
    pub fn read_serial_for_verify(&mut self, address: u32, dst: &mut [u8]) -> Result<(), HalError> {
        self.recover_to_serial_bus()?;
        self.apply_serial_sm_config();
        for (offset, chunk) in dst.chunks_mut(PSRAM_PROD_READ_CHUNK_BYTES).enumerate() {
            let chunk_address = address + (offset * PSRAM_PROD_READ_CHUNK_BYTES) as u32;
            self.read_chunk_serial(chunk_address, chunk)?;
        }
        self.enter_qpi_bus()
    }

    pub fn read_rx_dma64_for_diag(&mut self, address: u32, dst: &mut [u8]) -> Result<(), HalError> {
        self.read_rx_dma_for_diag(address, dst, PSRAM_QPI_CHUNK_BYTES)
    }

    pub fn read_large_cpu_for_diag(
        &mut self,
        address: u32,
        dst: &mut [u8],
        chunk_bytes: usize,
    ) -> Result<(), HalError> {
        Self::check_qpi_ext_read_chunk(chunk_bytes)?;
        Self::check_range(address, dst.len())?;
        self.apply_read16_sm_config();
        let mut result = Ok(());
        for (offset, chunk) in dst.chunks_mut(chunk_bytes).enumerate() {
            let chunk_address = address + (offset * chunk_bytes) as u32;
            if let Err(err) = self.read_chunk_cpu16(chunk_address, chunk) {
                result = Err(err);
                break;
            }
        }
        self.apply_rw_sm_config();
        result
    }

    pub fn read_cpu_for_diag(
        &mut self,
        address: u32,
        dst: &mut [u8],
        chunk_bytes: usize,
    ) -> Result<(), HalError> {
        Self::check_qpi_read_chunk(chunk_bytes)?;
        Self::check_range(address, dst.len())?;
        for (offset, chunk) in dst.chunks_mut(chunk_bytes).enumerate() {
            let chunk_address = address + (offset * chunk_bytes) as u32;
            self.read_chunk_cpu(chunk_address, chunk)?;
        }
        Ok(())
    }

    pub fn read_rx_dma_for_diag(
        &mut self,
        address: u32,
        dst: &mut [u8],
        chunk_bytes: usize,
    ) -> Result<(), HalError> {
        Self::check_qpi_read_chunk(chunk_bytes)?;
        Self::check_range(address, dst.len())?;
        for (offset, chunk) in dst.chunks_mut(chunk_bytes).enumerate() {
            let chunk_address = address + (offset * chunk_bytes) as u32;
            self.read_chunk_rx_dma_ch1(chunk_address, chunk)?;
        }
        Ok(())
    }

    pub fn write_for_diag(
        &mut self,
        address: u32,
        src: &[u8],
        chunk_bytes: usize,
    ) -> Result<(), HalError> {
        Self::check_qpi_write_chunk(chunk_bytes)?;
        Self::check_range(address, src.len())?;
        for (offset, chunk) in src.chunks(chunk_bytes).enumerate() {
            let chunk_address = address + (offset * chunk_bytes) as u32;
            self.write_chunk_cpu(chunk_address, chunk)?;
        }
        Ok(())
    }

    pub fn write_4wire_for_diag(
        &mut self,
        address: u32,
        src: &[u8],
        chunk_bytes: usize,
    ) -> Result<(), HalError> {
        if chunk_bytes == 0 {
            return Err(HalError::InvalidArgument);
        }
        Self::check_range(address, src.len())?;
        for (offset, chunk) in src.chunks(chunk_bytes).enumerate() {
            let chunk_address = address + (offset * chunk_bytes) as u32;
            self.write_chunk_4wire(chunk_address, chunk)?;
        }
        Ok(())
    }

    pub fn set_clock_divider_for_diag(&mut self, divider: u32) -> Result<(), HalError> {
        if divider == 0 || divider > u16::MAX as u32 {
            return Err(HalError::InvalidArgument);
        }
        self.sm.set_enable(false);
        self.sm.clear_fifos();
        self.sm.restart();
        pac::PIO1.sm(0).clkdiv().write(|w| {
            w.0 = divider << 16;
        });
        self.sm.clkdiv_restart();
        self.sm.set_enable(true);
        Ok(())
    }

    pub fn set_qpi_input_sync_bypass_for_diag(&mut self, enabled: bool) {
        if enabled {
            self._sio0.set_input_sync_bypass(true);
            self._sio1.set_input_sync_bypass(true);
            self._sio2.set_input_sync_bypass(true);
            self._sio3.set_input_sync_bypass(true);
        } else {
            self._sio0.set_input_sync_bypass(false);
            self._sio1.set_input_sync_bypass(false);
            self._sio2.set_input_sync_bypass(false);
            self._sio3.set_input_sync_bypass(false);
        }
    }

    pub fn diag_state(&self) -> PicoCalcPsramDiagState {
        let pio1 = &pac::PIO1;
        let sm0 = pio1.sm(0);
        let clkdiv_raw = sm0.clkdiv().read().0;
        let clkdiv = ((clkdiv_raw >> 16) & 0xffff).max(1);
        let clkdiv_frac = ((clkdiv_raw >> 8) & 0xff) as u8;
        let divider_fp = (u64::from(clkdiv) << 8) | u64::from(clkdiv_frac);
        let shiftctrl = sm0.shiftctrl().read();
        let qpi_bypass_mask = 0x0fu32 << 2;
        PicoCalcPsramDiagState {
            clkdiv,
            clkdiv_frac,
            sm_hz: ((u64::from(PSRAM_PIO_SYS_HZ) * 256) / divider_fp) as u32,
            cycles_per_bit: 2,
            flevel: pio1.flevel().read().0,
            fstat: pio1.fstat().read().0,
            fdebug: pio1.fdebug().read().0,
            rx_autopush: shiftctrl.autopush(),
            rx_fjoin: shiftctrl.fjoin_rx(),
            rx_threshold: shiftctrl.push_thresh(),
            tx_autopull: shiftctrl.autopull(),
            tx_fjoin: shiftctrl.fjoin_tx(),
            tx_threshold: shiftctrl.pull_thresh(),
            qpi_input_sync_bypass: (pio1.input_sync_bypass().read() & qpi_bypass_mask)
                == qpi_bypass_mask,
        }
    }

    fn apply_rw_sm_config(&mut self) {
        let divider = self.diag_state().clkdiv as u8;
        let mut config = Config::default();
        config.use_program(&self._program, &[&self._cs, &self._sck]);
        config.set_out_pins(&[&self._sio0, &self._sio1, &self._sio2, &self._sio3]);
        config.set_in_pins(&[&self._sio0, &self._sio1, &self._sio2, &self._sio3]);
        config.set_set_pins(&[&self._sio0, &self._sio1, &self._sio2, &self._sio3]);
        config.shift_out.auto_fill = true;
        config.shift_out.direction = ShiftDirection::Left;
        config.shift_out.threshold = 8;
        config.shift_in.auto_fill = true;
        config.shift_in.direction = ShiftDirection::Left;
        config.shift_in.threshold = 8;
        config.clock_divider = divider.into();
        self.sm.set_enable(false);
        self.sm.clear_fifos();
        self.sm.restart();
        self.sm.set_config(&config);
        self.sm.set_pin_dirs(
            Direction::Out,
            &[
                &self._cs,
                &self._sck,
                &self._sio0,
                &self._sio1,
                &self._sio2,
                &self._sio3,
            ],
        );
        self.sm.clkdiv_restart();
        self.sm.set_enable(true);
    }

    #[cfg(feature = "psram_qpi_safe_read_code_window")]
    fn apply_serial_sm_config(&mut self) {
        let mut config = Config::default();
        config.use_program(&self._serial_program, &[&self._cs, &self._sck]);
        config.set_out_pins(&[&self._sio0]);
        config.set_in_pins(&[&self._sio1]);
        config.shift_out.auto_fill = true;
        config.shift_out.direction = ShiftDirection::Left;
        config.shift_out.threshold = 8;
        config.shift_in.auto_fill = true;
        config.shift_in.direction = ShiftDirection::Left;
        config.shift_in.threshold = 8;
        config.clock_divider = (PSRAM_PIO_CLOCK_DIVIDER as u8).into();
        self.sm.set_enable(false);
        self.sm.clear_fifos();
        self.sm.restart();
        self.sm.set_config(&config);
        self.sm
            .set_pin_dirs(Direction::Out, &[&self._cs, &self._sck, &self._sio0]);
        self.sm.set_pin_dirs(Direction::In, &[&self._sio1]);
        self.sm.set_pins(Level::High, &[&self._cs]);
        self.sm.clkdiv_restart();
        self.sm.set_enable(true);
    }

    fn apply_read16_sm_config(&mut self) {
        let divider = self.diag_state().clkdiv as u8;
        let mut config = Config::default();
        config.use_program(&self._read16_program, &[&self._cs, &self._sck]);
        config.set_out_pins(&[&self._sio0, &self._sio1, &self._sio2, &self._sio3]);
        config.set_in_pins(&[&self._sio0, &self._sio1, &self._sio2, &self._sio3]);
        config.set_set_pins(&[&self._sio0, &self._sio1, &self._sio2, &self._sio3]);
        config.shift_out.auto_fill = true;
        config.shift_out.direction = ShiftDirection::Left;
        config.shift_out.threshold = 8;
        config.shift_in.auto_fill = true;
        config.shift_in.direction = ShiftDirection::Left;
        config.shift_in.threshold = 8;
        config.clock_divider = divider.into();
        self.sm.set_enable(false);
        self.sm.clear_fifos();
        self.sm.restart();
        self.sm.set_config(&config);
        self.sm.set_pin_dirs(
            Direction::Out,
            &[
                &self._cs,
                &self._sck,
                &self._sio0,
                &self._sio1,
                &self._sio2,
                &self._sio3,
            ],
        );
        self.sm.clkdiv_restart();
        self.sm.set_enable(true);
    }

    fn apply_write4_sm_config(&mut self) {
        let divider = self.diag_state().clkdiv as u8;
        let mut config = Config::default();
        config.use_program(&self._write4_program, &[&self._sck]);
        config.set_out_pins(&[&self._sio0, &self._sio1, &self._sio2, &self._sio3]);
        config.shift_out.auto_fill = true;
        config.shift_out.direction = ShiftDirection::Left;
        config.shift_out.threshold = 8;
        config.clock_divider = divider.into();
        self.sm.set_enable(false);
        self.sm.clear_fifos();
        self.sm.restart();
        self.sm.set_config(&config);
        self.sm.set_pin_dirs(
            Direction::Out,
            &[
                &self._cs,
                &self._sck,
                &self._sio0,
                &self._sio1,
                &self._sio2,
                &self._sio3,
            ],
        );
        self.sm.set_pins(Level::High, &[&self._cs]);
        self.sm.clkdiv_restart();
        self.sm.set_enable(true);
    }

    fn push_byte_timeout(&mut self, value: u8) -> Result<(), HalError> {
        let word = u32::from_be_bytes([value, 0, 0, 0]);
        for _ in 0..PSRAM_QPI_FIFO_TIMEOUT_ITERS {
            if self.sm.tx().try_push(word) {
                return Ok(());
            }
        }
        Err(HalError::Timeout)
    }

    fn push_u16_timeout(&mut self, value: u16) -> Result<(), HalError> {
        let word = u32::from_be_bytes([(value >> 8) as u8, value as u8, 0, 0]);
        for _ in 0..PSRAM_QPI_FIFO_TIMEOUT_ITERS {
            if self.sm.tx().try_push(word) {
                return Ok(());
            }
        }
        Err(HalError::Timeout)
    }

    fn push_byte_mmio8_timeout(&mut self, value: u8) -> Result<(), HalError> {
        let tx_fifo = self.sm.tx_fifo_ptr() as *mut u8;
        for _ in 0..PSRAM_QPI_FIFO_TIMEOUT_ITERS {
            if !self.sm.tx().full() {
                // SAFETY: TXF0 accepts byte writes; this mirrors Picoware's
                // DMA_SIZE_8 / io_rw_8 FIFO feed for qspi_4wire_write.
                unsafe {
                    core::ptr::write_volatile(tx_fifo, value);
                }
                return Ok(());
            }
        }
        Err(HalError::Timeout)
    }

    fn pull_byte_timeout(&mut self) -> Result<u8, HalError> {
        for _ in 0..PSRAM_QPI_FIFO_TIMEOUT_ITERS {
            if !self.sm.rx().empty() {
                return Ok(self.sm.rx().pull() as u8);
            }
        }
        Err(HalError::Timeout)
    }

    fn wait_write_idle(&mut self) -> Result<(), HalError> {
        for _ in 0..PSRAM_QPI_FIFO_TIMEOUT_ITERS {
            if self.sm.tx().empty() && self.sm.tx().stalled() {
                return Ok(());
            }
        }
        Err(HalError::Timeout)
    }

    fn write_only(&mut self, bytes: &[u8]) -> Result<(), HalError> {
        for byte in bytes {
            self.push_byte_timeout(*byte)?;
        }
        self.wait_write_idle()
    }

    fn transfer(&mut self, command: &[u8], read: &mut [u8]) -> Result<(), HalError> {
        for byte in command {
            self.push_byte_timeout(*byte)?;
        }
        for byte in read {
            *byte = self.pull_byte_timeout()?;
        }
        Ok(())
    }

    fn read_chunk_cpu(&mut self, address: u32, dst: &mut [u8]) -> Result<(), HalError> {
        if dst.is_empty() {
            return Ok(());
        }
        if dst.len() > 127 {
            return Err(HalError::InvalidArgument);
        }
        let read_nibbles = (dst.len() * 2) as u8;
        let command = [
            14,
            read_nibbles.saturating_sub(1),
            0xEB,
            (address >> 16) as u8,
            (address >> 8) as u8,
            address as u8,
            0,
            0,
            0,
        ];
        self.transfer(&command, dst)
    }

    fn read_chunk_cpu16(&mut self, address: u32, dst: &mut [u8]) -> Result<(), HalError> {
        if dst.is_empty() {
            return Ok(());
        }
        if dst.len() > PSRAM_QPI_EXT_READ_MAX_CHUNK_BYTES {
            return Err(HalError::InvalidArgument);
        }
        let read_nibbles = (dst.len() * 2) as u16;
        self.push_byte_timeout(14)?;
        self.push_u16_timeout(read_nibbles.saturating_sub(1))?;
        for byte in [
            0xEB,
            (address >> 16) as u8,
            (address >> 8) as u8,
            address as u8,
            0,
            0,
            0,
        ] {
            self.push_byte_timeout(byte)?;
        }
        for byte in dst {
            *byte = self.pull_byte_timeout()?;
        }
        Ok(())
    }

    #[cfg(feature = "psram_qpi_code_window_counters")]
    fn read_chunk_serial(&mut self, address: u32, dst: &mut [u8]) -> Result<(), HalError> {
        if dst.is_empty() {
            return Ok(());
        }
        if dst.len() > PSRAM_PROD_READ_CHUNK_BYTES {
            return Err(HalError::InvalidArgument);
        }
        let command = [
            40,
            (dst.len() * 8 - 1) as u8,
            0x0b,
            (address >> 16) as u8,
            (address >> 8) as u8,
            address as u8,
            0,
        ];
        for byte in command {
            self.push_byte_timeout(byte)?;
        }
        for byte in dst {
            *byte = self.pull_byte_timeout()?;
        }
        Ok(())
    }

    #[cfg(feature = "psram_qpi_safe_read_code_window")]
    pub fn write_legacy_spi_for_stage(&mut self, address: u32, src: &[u8]) -> Result<(), HalError> {
        Self::check_range(address, src.len())?;
        self.recover_to_serial_bus()?;
        let mut result = Ok(());
        for (offset, chunk) in src.chunks(PSRAM_PROD_READ_CHUNK_BYTES).enumerate() {
            let chunk_address = address + (offset * PSRAM_PROD_READ_CHUNK_BYTES) as u32;
            if let Err(err) = bitbang_spi_write(chunk_address, chunk) {
                result = Err(err);
                break;
            }
        }
        self.enter_qpi_bus()?;
        result
    }

    fn read_chunk_rx_dma_ch1(&mut self, address: u32, dst: &mut [u8]) -> Result<(), HalError> {
        if dst.is_empty() || dst.len() > 127 {
            return Err(HalError::InvalidArgument);
        }
        let read_nibbles = (dst.len() * 2) as u8;
        let command = [
            14,
            read_nibbles.saturating_sub(1),
            0xEB,
            (address >> 16) as u8,
            (address >> 8) as u8,
            address as u8,
            0,
            0,
            0,
        ];

        while !self.sm.rx().empty() {
            let _ = self.sm.rx().pull();
        }

        let dma = &pac::DMA;
        let ch = dma.ch(1);
        use embassy_rp::pac::dma::regs::CtrlTrig;
        ch.ctrl_trig().write_value(CtrlTrig(0));
        ch.read_addr().write_value(self.sm.rx_fifo_ptr() as u32);
        ch.write_addr().write_value(dst.as_mut_ptr() as u32);
        ch.trans_count().write_value(dst.len() as u32);
        ch.ctrl_trig().write(|w| {
            use embassy_rp::pac::dma::vals::DataSize;
            w.set_data_size(DataSize::SIZE_BYTE);
            w.set_incr_read(false);
            w.set_incr_write(true);
            w.set_treq_sel(self.sm.rx_treq());
            w.set_irq_quiet(true);
            w.set_en(true);
        });

        for byte in command {
            self.push_byte_timeout(byte)?;
        }

        let mut saw_busy = false;
        for _ in 0..PSRAM_QPI_FIFO_TIMEOUT_ITERS {
            let ctrl = ch.ctrl_trig().read();
            if ctrl.busy() {
                saw_busy = true;
            } else if ctrl.ahb_error() || ctrl.read_error() || ctrl.write_error() {
                ch.ctrl_trig().write_value(CtrlTrig(0));
                return Err(HalError::Io);
            } else if saw_busy || ch.trans_count().read() == 0 {
                return self.wait_write_idle();
            }
        }

        ch.ctrl_trig().write_value(CtrlTrig(0));
        Err(HalError::Timeout)
    }

    fn write_chunk_cpu(&mut self, address: u32, src: &[u8]) -> Result<(), HalError> {
        if src.is_empty() {
            return Ok(());
        }
        if src.len() > 123 {
            return Err(HalError::InvalidArgument);
        }
        let command = [
            ((4 + src.len()) * 2) as u8,
            0,
            0x38,
            (address >> 16) as u8,
            (address >> 8) as u8,
            address as u8,
        ];
        for byte in command.iter().chain(src) {
            self.push_byte_timeout(*byte)?;
        }
        self.wait_write_idle()
    }

    fn write_chunk_4wire(&mut self, address: u32, src: &[u8]) -> Result<(), HalError> {
        if src.is_empty() {
            return Ok(());
        }

        self.apply_write4_sm_config();
        self.sm.set_pins(Level::Low, &[&self._cs]);

        let command = [
            0x38,
            (address >> 16) as u8,
            (address >> 8) as u8,
            address as u8,
        ];
        let mut result = Ok(());
        for byte in command.iter().chain(src) {
            if let Err(err) = self.push_byte_mmio8_timeout(*byte) {
                result = Err(err);
                break;
            }
        }
        if result.is_ok() {
            result = self.wait_write_idle();
        }

        self.sm.set_pins(Level::High, &[&self._cs]);
        self.apply_rw_sm_config();
        result
    }

    fn check_qpi_read_chunk(chunk_bytes: usize) -> Result<(), HalError> {
        if chunk_bytes == 0 || chunk_bytes > 127 {
            Err(HalError::InvalidArgument)
        } else {
            Ok(())
        }
    }

    fn check_qpi_write_chunk(chunk_bytes: usize) -> Result<(), HalError> {
        if chunk_bytes == 0 || chunk_bytes > 123 {
            Err(HalError::InvalidArgument)
        } else {
            Ok(())
        }
    }

    fn check_qpi_ext_read_chunk(chunk_bytes: usize) -> Result<(), HalError> {
        if chunk_bytes == 0 || chunk_bytes > PSRAM_QPI_EXT_READ_MAX_CHUNK_BYTES {
            Err(HalError::InvalidArgument)
        } else {
            Ok(())
        }
    }

    fn check_range(address: u32, len: usize) -> Result<(), HalError> {
        let len = u32::try_from(len).map_err(|_| HalError::InvalidArgument)?;
        let end = address.checked_add(len).ok_or(HalError::InvalidArgument)?;
        if end > PSRAM_CAPACITY {
            return Err(HalError::InvalidArgument);
        }
        Ok(())
    }
}

#[cfg(feature = "psram_qpi_backend")]
impl PsramHal for PicoCalcQpiPsram<'_> {
    fn available(&self) -> bool {
        true
    }

    fn read(&mut self, address: u32, dst: &mut [u8]) -> Result<(), HalError> {
        self.read_cpu_for_diag(address, dst, PSRAM_QPI_CHUNK_BYTES)
    }

    fn write(&mut self, address: u32, src: &[u8]) -> Result<(), HalError> {
        self.write_for_diag(address, src, PSRAM_QPI_CHUNK_BYTES)
    }
}

#[cfg(feature = "psram_qpi_backend")]
fn bitbang_spi_command(command: u8) {
    const CS: u8 = 20;
    const SCK: u8 = 21;
    const SIO0: u8 = 2;
    let mask = (1u32 << CS) | (1u32 << SCK) | (1u32 << SIO0);

    for pin in [CS, SCK, SIO0] {
        pac::IO_BANK0
            .gpio(pin as usize)
            .ctrl()
            .write(|w| w.set_funcsel(pac::io::vals::Gpio0ctrlFuncsel::SIO_0 as _));
    }
    let sio = pac::SIO;
    sio.gpio_oe(0).value_set().write(|w| *w = mask);
    sio.gpio_out(0)
        .value_clr()
        .write(|w| *w = (1u32 << SCK) | (1u32 << SIO0));
    sio.gpio_out(0).value_set().write(|w| *w = 1u32 << CS);
    block_for(Duration::from_micros(1));

    sio.gpio_out(0).value_clr().write(|w| *w = 1u32 << CS);
    for bit in (0..8).rev() {
        if ((command >> bit) & 1) != 0 {
            sio.gpio_out(0).value_set().write(|w| *w = 1u32 << SIO0);
        } else {
            sio.gpio_out(0).value_clr().write(|w| *w = 1u32 << SIO0);
        }
        block_for(Duration::from_micros(1));
        sio.gpio_out(0).value_set().write(|w| *w = 1u32 << SCK);
        block_for(Duration::from_micros(1));
        sio.gpio_out(0).value_clr().write(|w| *w = 1u32 << SCK);
    }
    sio.gpio_out(0).value_set().write(|w| *w = 1u32 << CS);
    block_for(Duration::from_micros(1));
}

#[cfg(feature = "psram_qpi_backend")]
fn bitbang_spi_write(address: u32, src: &[u8]) -> Result<(), HalError> {
    const CS: u8 = 20;
    const SCK: u8 = 21;
    const SIO0: u8 = 2;
    let mask = (1u32 << CS) | (1u32 << SCK) | (1u32 << SIO0);

    for pin in [CS, SCK, SIO0] {
        pac::IO_BANK0
            .gpio(pin as usize)
            .ctrl()
            .write(|w| w.set_funcsel(pac::io::vals::Gpio0ctrlFuncsel::SIO_0 as _));
    }

    let sio = pac::SIO;
    sio.gpio_oe(0).value_set().write(|w| *w = mask);
    sio.gpio_out(0)
        .value_clr()
        .write(|w| *w = (1u32 << SCK) | (1u32 << SIO0));
    sio.gpio_out(0).value_set().write(|w| *w = 1u32 << CS);
    block_for(Duration::from_micros(1));

    let mut bytes = [0u8; 6];
    bytes[0] = 0x02;
    bytes[1] = (address >> 16) as u8;
    bytes[2] = (address >> 8) as u8;
    bytes[3] = address as u8;

    sio.gpio_out(0).value_clr().write(|w| *w = 1u32 << CS);
    for byte in bytes[..4].iter().chain(src.iter()) {
        for bit in (0..8).rev() {
            if ((byte >> bit) & 1) != 0 {
                sio.gpio_out(0).value_set().write(|w| *w = 1u32 << SIO0);
            } else {
                sio.gpio_out(0).value_clr().write(|w| *w = 1u32 << SIO0);
            }
            block_for(Duration::from_micros(1));
            sio.gpio_out(0).value_set().write(|w| *w = 1u32 << SCK);
            block_for(Duration::from_micros(1));
            sio.gpio_out(0).value_clr().write(|w| *w = 1u32 << SCK);
        }
    }
    sio.gpio_out(0).value_set().write(|w| *w = 1u32 << CS);
    block_for(Duration::from_micros(1));
    Ok(())
}

#[cfg(feature = "psram_qpi_backend")]
fn bitbang_spi_read(address: u32, dst: &mut [u8]) -> Result<(), HalError> {
    const CS: u8 = 20;
    const SCK: u8 = 21;
    const SIO0: u8 = 2;
    const SIO1: u8 = 3;
    let oe_mask = (1u32 << CS) | (1u32 << SCK) | (1u32 << SIO0);

    for pin in [CS, SCK, SIO0, SIO1] {
        pac::IO_BANK0
            .gpio(pin as usize)
            .ctrl()
            .write(|w| w.set_funcsel(pac::io::vals::Gpio0ctrlFuncsel::SIO_0 as _));
    }

    let sio = pac::SIO;
    sio.gpio_oe(0).value_clr().write(|w| *w = 1u32 << SIO1);
    sio.gpio_oe(0).value_set().write(|w| *w = oe_mask);
    sio.gpio_out(0)
        .value_clr()
        .write(|w| *w = (1u32 << SCK) | (1u32 << SIO0));
    sio.gpio_out(0).value_set().write(|w| *w = 1u32 << CS);
    block_for(Duration::from_micros(1));

    let header = [
        0x0b,
        (address >> 16) as u8,
        (address >> 8) as u8,
        address as u8,
        0,
    ];
    sio.gpio_out(0).value_clr().write(|w| *w = 1u32 << CS);
    for byte in header {
        for bit in (0..8).rev() {
            if ((byte >> bit) & 1) != 0 {
                sio.gpio_out(0).value_set().write(|w| *w = 1u32 << SIO0);
            } else {
                sio.gpio_out(0).value_clr().write(|w| *w = 1u32 << SIO0);
            }
            block_for(Duration::from_micros(1));
            sio.gpio_out(0).value_set().write(|w| *w = 1u32 << SCK);
            block_for(Duration::from_micros(1));
            sio.gpio_out(0).value_clr().write(|w| *w = 1u32 << SCK);
        }
    }

    for byte in dst.iter_mut() {
        let mut value = 0u8;
        for _ in 0..8 {
            sio.gpio_out(0).value_set().write(|w| *w = 1u32 << SCK);
            block_for(Duration::from_micros(1));
            value <<= 1;
            let pin_in = sio.gpio_in(0).read();
            if (pin_in & (1u32 << SIO1)) != 0 {
                value |= 1;
            }
            sio.gpio_out(0).value_clr().write(|w| *w = 1u32 << SCK);
            block_for(Duration::from_micros(1));
        }
        *byte = value;
    }

    sio.gpio_out(0).value_set().write(|w| *w = 1u32 << CS);
    block_for(Duration::from_micros(1));
    Ok(())
}

pub struct PicoCalcPsram<'d> {
    sm: StateMachine<'d, peripherals::PIO1, 0>,
    _program: LoadedProgram<'d, peripherals::PIO1>,
    _cs: Pin<'d, peripherals::PIO1>,
    _sck: Pin<'d, peripherals::PIO1>,
    _mosi: Pin<'d, peripherals::PIO1>,
    _miso: Pin<'d, peripherals::PIO1>,
}

impl<'d> PicoCalcPsram<'d> {
    fn apply_default_sm0_config(&mut self) {
        let mut config = Config::default();
        config.use_program(&self._program, &[&self._cs, &self._sck]);
        config.set_out_pins(&[&self._mosi]);
        config.set_in_pins(&[&self._miso]);
        config.shift_out.auto_fill = true;
        config.shift_out.direction = ShiftDirection::Left;
        config.shift_out.threshold = 8;
        config.shift_in.auto_fill = true;
        config.shift_in.direction = ShiftDirection::Left;
        config.shift_in.threshold = 8;
        config.clock_divider = (PSRAM_PIO_CLOCK_DIVIDER as u8).into();

        self.sm.set_config(&config);
        self.sm
            .set_pin_dirs(Direction::Out, &[&self._cs, &self._sck, &self._mosi]);
        self.sm.set_pin_dirs(Direction::In, &[&self._miso]);
    }

    // Implementation notes / protocol summary:
    //
    // The PIO program below implements a simple 1-bit serial PSRAM transfer
    // adapted from the rp2040-psram reference. The host-side command layout
    // used by `read()` / `write()` is:
    //
    // Read command (bytes pushed to the PIO TX FIFO):
    //   [out_bits_count, in_bits_count, opcode, addr_hi, addr_mid, addr_lo, 0]
    //
    // Where:
    // - `out_bits_count` is the number of command/address/dummy bits the PIO
    //   will drive onto the MOSI pin before switching to input. For the
    //   fast-read opcode (0x0B) this is 8 (opcode) + 24 (addr) + 8 (dummy) =
    //   40 bits (hence the hard-coded 40 in `read()`).
    // - `in_bits_count` is the number of data bits to clock in from MISO
    //   (typically `chunk.len() * 8 - 1` to account for the PIO loop
    //   decrement semantics).
    //
    // Important limits and practical constraints:
    // - Both `out_bits_count` and `in_bits_count` are loaded into PIO
    //   registers with `out x, 8` / `out y, 8`, so they are 8-bit values and
    //   limited to 0..=255. This yields a theoretical maximum read chunk of
    //   32 bytes because `chunk.len() * 8 - 1 <= 255` -> `chunk.len() <= 32`.
    // - In practice the implementation is sensitive to FIFO/OSR/ISR timing and
    //   alignment: the PIO state machine uses 1-bit `in`/`out` loops while the
    //   CPU-side code pushes/pulls whole bytes via the 32-bit PIO FIFOs. The
    //   RX/TX FIFO depth, `shift_*` thresholds (8 bits) and the `auto_fill`
    //   behaviour interact such that larger bursts can overflow or misalign
    //   the stream if the producer/consumer pattern does not match the SM's
    //   expectations exactly.
    //
    // Observed behaviour on hardware:
    // - 16B chunks are the known-good production baseline (used by
    //   `PsramHal::read`).
    // - 32B chunks fit the 8-bit length field but have shown corruption in
    //   practice (KBC verifier `StackUnderflow`), indicating subtle timing or
    //   alignment races between the SM and CPU-side FIFO handling.
    // - 128B chunks overflow the 8-bit length field and will hang or
    //   otherwise misbehave.
    //
    // Conclusion: the protocol's encoded `in_bits_count` permits up to 32B in
    // theory, but the PIO/FIFO/auto-fill interplay makes anything above 16B
    // unreliable on current firmware. The safe production path is therefore
    // to keep the `read()` chunk size at 16B and pursue DMA/QPI/protocol
    // redesign for higher sustained bandwidth.

    pub fn new(
        common: &mut Common<'d, peripherals::PIO1>,
        mut sm: StateMachine<'d, peripherals::PIO1, 0>,
        cs: embassy_rp::Peri<'d, peripherals::PIN_20>,
        sck: embassy_rp::Peri<'d, peripherals::PIN_21>,
        mosi: embassy_rp::Peri<'d, peripherals::PIN_2>,
        miso: embassy_rp::Peri<'d, peripherals::PIN_3>,
    ) -> Self {
        let program = pio::pio_asm!(
            r#"
                .side_set 2
                .wrap_target
                begin:
                    out x, 8            side 0b01
                    out y, 8            side 0b01
                    jmp x--, writeloop  side 0b01
                writeloop:
                    out pins, 1         side 0b00
                    jmp x--, writeloop  side 0b10
                    jmp !y, begin       side 0b00
                readloop:
                    in pins, 1          side 0b10
                    jmp y--, readloop   side 0b00
                .wrap
            "#
        );
        let loaded = common.load_program(&program.program);

        let mut cs = common.make_pio_pin(cs);
        let mut sck = common.make_pio_pin(sck);
        let mut mosi = common.make_pio_pin(mosi);
        let mut miso = common.make_pio_pin(miso);
        for pin in [&mut cs, &mut sck, &mut mosi] {
            pin.set_drive_strength(Drive::_4mA);
            pin.set_slew_rate(SlewRate::Fast);
        }
        miso.set_input_sync_bypass(true);

        let mut config = Config::default();
        config.use_program(&loaded, &[&cs, &sck]);
        config.set_out_pins(&[&mosi]);
        config.set_in_pins(&[&miso]);
        config.shift_out.auto_fill = true;
        config.shift_out.direction = ShiftDirection::Left;
        config.shift_out.threshold = 8;
        config.shift_in.auto_fill = true;
        config.shift_in.direction = ShiftDirection::Left;
        config.shift_in.threshold = 8;
        // Two PIO instructions per serial bit. Divider comes from
        // PSRAM_PIO_CLOCK_DIVIDER (feature-dependent experiment knob).
        config.clock_divider = (PSRAM_PIO_CLOCK_DIVIDER as u8).into();

        sm.set_config(&config);
        sm.set_pin_dirs(Direction::Out, &[&cs, &sck, &mosi]);
        sm.set_pin_dirs(Direction::In, &[&miso]);
        sm.set_enable(true);

        let mut psram = Self {
            sm,
            _program: loaded,
            _cs: cs,
            _sck: sck,
            _mosi: mosi,
            _miso: miso,
        };
        psram.write_only(&[8, 0, 0x66]);
        block_for(Duration::from_micros(50));
        psram.write_only(&[8, 0, 0x99]);
        block_for(Duration::from_micros(100));
        psram
    }

    // Observed DMA channel usage notes (diagnostic planning):
    // - LCD scanline SPI transfers and some firmware SPI paths use `DMA_CH0`.
    //   See `src/koto-pico/src/bin/probe_lcd.rs` and `src/koto-pico/src/bin/probe_lcd.rs`.
    // - PWM/audio probe uses `DMA_CH0` as well (`src/koto-pico/src/bin/probe_audio.rs`).
    // When adding a DMA-assisted PSRAM diagnostic, avoid colliding with
    // these consumers on test hardware or temporarily quiesce them while the
    // diagnostic runs.

    fn push_byte(&mut self, value: u8) {
        let word = u32::from_be_bytes([value, 0, 0, 0]);
        while !self.sm.tx().try_push(word) {}
    }

    fn pull_byte(&mut self) -> u8 {
        while self.sm.rx().empty() {}
        self.sm.rx().pull() as u8
    }

    /// Internal helper for pushing bytes (used by diagnostics).
    #[cfg(feature = "psram_pio_word_diag")]
    pub fn push_byte_internal(&mut self, value: u8) {
        let word = u32::from_be_bytes([value, 0, 0, 0]);
        while !self.sm.tx().try_push(word) {}
    }

    fn write_only(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.push_byte(*byte);
        }
        while !self.sm.tx().empty() {}
        while !self.sm.tx().stalled() {}
    }

    fn transfer(&mut self, command: &[u8], read: &mut [u8]) {
        for byte in command {
            self.push_byte(*byte);
        }
        for byte in read {
            *byte = self.pull_byte();
        }
    }

    fn check_range(address: u32, len: usize) -> Result<(), HalError> {
        let len = u32::try_from(len).map_err(|_| HalError::InvalidArgument)?;
        let end = address.checked_add(len).ok_or(HalError::InvalidArgument)?;
        if end > PSRAM_CAPACITY {
            return Err(HalError::InvalidArgument);
        }
        Ok(())
    }

    pub fn set_clock_divider_for_diag(&mut self, divider: u32) -> Result<(), HalError> {
        if divider == 0 || divider > u16::MAX as u32 {
            return Err(HalError::InvalidArgument);
        }
        self.set_clock_divider_parts_for_diag(divider as u16, 0)
    }

    pub fn set_clock_divider_parts_for_diag(
        &mut self,
        divider_int: u16,
        divider_frac: u8,
    ) -> Result<(), HalError> {
        if divider_int == 0 {
            return Err(HalError::InvalidArgument);
        }
        // Keep SM0 in a deterministic idle state before changing timing.
        // Retuning while the SM is mid-transaction can leave stale FIFO/PC state
        // and corrupt subsequent diagnostics.
        self.sm.set_enable(false);
        self.sm.clear_fifos();
        self.sm.restart();
        pac::PIO1.sm(0).clkdiv().write(|w| {
            // RP2040 PIO CLKDIV uses INT in bits [31:16] and FRAC in [15:8].
            w.0 = (u32::from(divider_int) << 16) | (u32::from(divider_frac) << 8);
        });
        self.sm.clkdiv_restart();
        self.sm.set_enable(true);
        Ok(())
    }

    pub fn recover_after_diag_failure(&mut self) -> Result<(), HalError> {
        let pio1 = &pac::PIO1;

        self.sm.set_enable(false);
        self.sm.clear_fifos();
        pio1.fdebug().write(|w| {
            w.0 = 0xffff_ffff;
        });

        self.apply_default_sm0_config();
        self.sm.restart();
        // `embassy_rp::pio::Pin` does not expose direct output-level forcing,
        // so recovery re-enters a known idle bus state by restoring the SM0
        // config and replaying the PSRAM reset sequence below.
        pac::PIO1.sm(0).clkdiv().write(|w| {
            w.0 = (PSRAM_PIO_CLOCK_DIVIDER << 16) as u32;
        });
        self.sm.clkdiv_restart();
        self.sm.set_enable(true);

        self.write_only(&[8, 0, 0x66]);
        block_for(Duration::from_micros(50));
        self.write_only(&[8, 0, 0x99]);
        block_for(Duration::from_micros(100));
        Ok(())
    }

    pub fn read_with_dummy_bits_for_diag(
        &mut self,
        address: u32,
        dst: &mut [u8],
        dummy_bits: u8,
    ) -> Result<(), HalError> {
        self.read_with_diag_tuning_for_diag(address, dst, dummy_bits, 0)
    }

    pub fn read_with_diag_tuning_for_diag(
        &mut self,
        address: u32,
        dst: &mut [u8],
        dummy_bits: u8,
        inter_tx_delay_us: u16,
    ) -> Result<(), HalError> {
        if dummy_bits == 0 || dummy_bits > 16 {
            return Err(HalError::InvalidArgument);
        }
        Self::check_range(address, dst.len())?;
        let total_chunks = dst.len().div_ceil(PSRAM_PROD_READ_CHUNK_BYTES);
        for (offset, chunk) in dst.chunks_mut(PSRAM_PROD_READ_CHUNK_BYTES).enumerate() {
            let chunk_address = address + (offset * PSRAM_PROD_READ_CHUNK_BYTES) as u32;
            let out_bits = 32u8.saturating_add(dummy_bits);
            let dummy_bytes = usize::from(dummy_bits.div_ceil(8));
            let mut command = [0u8; 8];
            command[0] = out_bits;
            command[1] = (chunk.len() * 8 - 1) as u8;
            command[2] = 0x0b;
            command[3] = (chunk_address >> 16) as u8;
            command[4] = (chunk_address >> 8) as u8;
            command[5] = chunk_address as u8;
            self.transfer(&command[..(2 + 4 + dummy_bytes)], chunk);
            if inter_tx_delay_us > 0 && (offset + 1) < total_chunks {
                block_for(Duration::from_micros(inter_tx_delay_us.into()));
            }
        }
        Ok(())
    }

    pub fn diag_state(&self) -> PicoCalcPsramDiagState {
        let pio1 = &pac::PIO1;
        let sm0 = pio1.sm(0);
        let clkdiv_raw = sm0.clkdiv().read().0;
        let clkdiv = ((clkdiv_raw >> 16) & 0xffff).max(1);
        let clkdiv_frac = ((clkdiv_raw >> 8) & 0xff) as u8;
        let divider_fp = (u64::from(clkdiv) << 8) | u64::from(clkdiv_frac);
        let shiftctrl = sm0.shiftctrl().read();
        PicoCalcPsramDiagState {
            clkdiv,
            clkdiv_frac,
            sm_hz: ((u64::from(PSRAM_PIO_SYS_HZ) * 256) / divider_fp) as u32,
            cycles_per_bit: PSRAM_PIO_CYCLES_PER_BIT,
            flevel: pio1.flevel().read().0,
            fstat: pio1.fstat().read().0,
            fdebug: pio1.fdebug().read().0,
            rx_autopush: shiftctrl.autopush(),
            rx_fjoin: shiftctrl.fjoin_rx(),
            rx_threshold: shiftctrl.push_thresh(),
            tx_autopull: shiftctrl.autopull(),
            tx_fjoin: shiftctrl.fjoin_tx(),
            tx_threshold: shiftctrl.pull_thresh(),
            qpi_input_sync_bypass: false,
        }
    }
}

impl PsramHal for PicoCalcPsram<'_> {
    fn available(&self) -> bool {
        true
    }

    fn read(&mut self, address: u32, dst: &mut [u8]) -> Result<(), HalError> {
        Self::check_range(address, dst.len())?;
        for (offset, chunk) in dst.chunks_mut(PSRAM_PROD_READ_CHUNK_BYTES).enumerate() {
            let chunk_address = address + (offset * PSRAM_PROD_READ_CHUNK_BYTES) as u32;
            let command = [
                40,
                (chunk.len() * 8 - 1) as u8,
                0x0b,
                (chunk_address >> 16) as u8,
                (chunk_address >> 8) as u8,
                chunk_address as u8,
                0,
            ];
            self.transfer(&command, chunk);
        }
        Ok(())
    }

    fn write(&mut self, address: u32, src: &[u8]) -> Result<(), HalError> {
        Self::check_range(address, src.len())?;
        for (offset, chunk) in src.chunks(16).enumerate() {
            let chunk_address = address + (offset * 16) as u32;
            let command = [
                ((4 + chunk.len()) * 8) as u8,
                0,
                0x02,
                (chunk_address >> 16) as u8,
                (chunk_address >> 8) as u8,
                chunk_address as u8,
            ];
            for byte in command.iter().chain(chunk) {
                self.push_byte(*byte);
            }
            while !self.sm.tx().empty() {}
            while !self.sm.tx().stalled() {}
        }
        Ok(())
    }
}

impl<'d> PicoCalcPsram<'d> {
    /// Diagnostic helper: issue a 16B read command and collect the raw
    /// 32-bit RX FIFO words into `words`.
    ///
    /// This is only intended for diagnostic use. It mirrors the command
    /// sequence used by `read()` but returns the 32-bit FIFO words so a
    /// diagnostic harness can perform word-based reassembly (DMA-like).
    pub fn diag_read_words_32(&mut self, address: u32, words: &mut [u32]) -> Result<(), HalError> {
        // Support reading a variable number of RX FIFO entries produced by
        // a single 16-byte PSRAM read command. The PIO state machine will
        // present 16 bytes for a 16-byte read; callers may request 4 words
        // (word-packed) or 16 words (one u32 per valid byte) depending on
        // the diagnostic reconstruction approach.
        if words.len() == 0 || words.len() > 16 {
            return Err(HalError::InvalidArgument);
        }
        // Always perform a 16-byte read command at the PSRAM level.
        Self::check_range(address, 16)?;
        let command = [
            40,
            (16 * 8 - 1) as u8,
            0x0b,
            (address >> 16) as u8,
            (address >> 8) as u8,
            address as u8,
            0,
        ];
        for byte in &command {
            self.push_byte(*byte);
        }
        // Collect `words.len()` entries from the RX FIFO.
        for i in 0..words.len() {
            while self.sm.rx().empty() {}
            words[i] = self.sm.rx().pull();
        }
        Ok(())
    }
}
