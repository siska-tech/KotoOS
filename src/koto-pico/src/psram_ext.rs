//! KotoOS adapter over the extracted `koto-psram` driver crate.
//!
//! This is the default firmware PSRAM backend. It wraps `koto-psram`'s safe
//! blocking `BlockingDriver` + `EmbassyRpQpiBackend` and exposes the in-tree
//! [`koto_core::hal::PsramHal`] shape (`available` / `read` / `write` /
//! `read_code_window`) that `PsramBlocks` and `PsramCodeWindow` consume.
//!
//! The ordinary `read`/`write` path always uses the hardware-validated safe
//! profile: `Pins::PICOCALC` + `TimingConfig::PICOCALC_SAFE`. The legacy in-tree
//! backend remains available behind the `legacy_psram` feature.
//!
//! Behind the `psram_fast_code_window` feature (default-on since KOTO-0171),
//! the [`PsramHal::read_code_window`] override attempts `koto-psram`'s
//! hardware-validated `FastFallingClkdiv2` read for PsramCodeWindow refills
//! (read_clkdiv=2.0, falling-edge sampling, extra dummy byte, RX byte-FIFO + RX
//! DMA), with an automatic safe fallback on any unsupported address/length or
//! read error. Without it (`--no-default-features`) CodeWindow refills use the
//! same safe read as everything else.

#[cfg(all(
    feature = "psram_fast_code_window",
    any(
        feature = "legacy_psram",
        feature = "psram_dma_read_code_window",
        feature = "psram_qpi_safe_read_code_window"
    )
))]
compile_error!(
    "`psram_fast_code_window` requires the default koto-psram backend and is mutually exclusive with `legacy_psram`, `psram_dma_read_code_window`, and `psram_qpi_safe_read_code_window` — build those profiles with `--no-default-features` (psram_fast_code_window is default-on since KOTO-0171)"
);

use embassy_rp::Peri;
use koto_core::hal::{HalError, PsramHal};
use koto_psram::bus::PsramBus;
use koto_psram::config::{Pins, TimingConfig};
use koto_psram::device::DeviceId;
use koto_psram::error::PsramError;
use koto_psram::pio::blocking::BlockingDriver;
use koto_psram::rp2040_embassy::{EmbassyRpQpiBackend, EmbassyRpQpiError};
use koto_psram::PsramAddr;

use crate::board::{
    PsramCsPin, PsramPio, PsramSckPin, PsramSio0Pin, PsramSio1Pin, PsramSio2Pin, PsramSio3Pin,
};

#[cfg(feature = "psram_fast_code_window")]
use embassy_rp::dma;
#[cfg(feature = "psram_fast_code_window")]
use koto_psram::rp2040_embassy::PayloadTransferPath;

#[cfg(feature = "psram_fast_code_window")]
pub use fast_counters::{
    koto_psram_fast_code_window_snapshot, KotoPsramFastReadMode, KotoPsramFastReadSnapshot,
    PSRAM_FAST_CODE_WINDOW_CHUNK_BYTES,
};

/// Concrete `koto-psram` Embassy backend bound to the PicoCalc PIO1/SM0 wiring
/// (SIO0..3 on GP2-5, CS on GP20, SCK on GP21).
type Backend<'d> = EmbassyRpQpiBackend<
    'd,
    PsramPio,
    0,
    PsramSio0Pin,
    PsramSio1Pin,
    PsramSio2Pin,
    PsramSio3Pin,
    PsramCsPin,
    PsramSckPin,
>;

/// PAC DMA channel `koto-psram`'s fast RX-DMA path drives directly. The firmware
/// must reserve `DMA_CH1` and pass it to [`KotoPsram::new`] so the embassy
/// channel ownership matches the channel the fast read manipulates via PAC.
#[cfg(feature = "psram_fast_code_window")]
const FAST_RX_DMA_PAC_CH: u8 = 1;

/// Hardware-validated `FastFallingClkdiv2` read timing: read clock divider 2.0,
/// 4 KiB chunks, and the shorter RX byte-FIFO poll budget exercised by
/// `rp2040_embassy_fast_clkdiv2_validation`.
#[cfg(feature = "psram_fast_code_window")]
const FAST_TIMING: TimingConfig = TimingConfig {
    read_clkdiv: 2.0,
    fallback_read_clkdiv: 8.0,
    max_chunk_len: PSRAM_FAST_CODE_WINDOW_CHUNK_BYTES,
    timeout_polls: 10_000,
    ..TimingConfig::PICOCALC_FAST_CANDIDATE
};

/// Active read profile of the driver. Ordinary `read`/`write` always run on the
/// safe profile; only CodeWindow refills switch to the fast profile, and
/// consecutive refills keep it without reconfiguring.
#[cfg(feature = "psram_fast_code_window")]
#[derive(Clone, Copy, PartialEq, Eq)]
enum ReadMode {
    Safe,
    Fast,
}

/// Default firmware PSRAM HAL backed by the `koto-psram` safe blocking driver.
pub struct KotoPsram<'d> {
    driver: BlockingDriver<Backend<'d>>,
    device_id: Option<DeviceId>,
    ready: bool,
    /// Active driver read profile (safe vs fast CodeWindow refill).
    #[cfg(feature = "psram_fast_code_window")]
    mode: ReadMode,
}

impl<'d> KotoPsram<'d> {
    /// Brings up the PSRAM through `koto-psram`'s documented init flow using the
    /// PicoCalc-safe pin and timing profiles. Returns an error if the init
    /// sequence (QPI exit/enter, ID probe) does not reach a ready state.
    #[cfg(not(feature = "psram_fast_code_window"))]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        common: embassy_rp::pio::Common<'d, PsramPio>,
        sm0: embassy_rp::pio::StateMachine<'d, PsramPio, 0>,
        cs: Peri<'d, PsramCsPin>,
        sck: Peri<'d, PsramSckPin>,
        sio0: Peri<'d, PsramSio0Pin>,
        sio1: Peri<'d, PsramSio1Pin>,
        sio2: Peri<'d, PsramSio2Pin>,
        sio3: Peri<'d, PsramSio3Pin>,
    ) -> Result<Self, HalError> {
        let backend = EmbassyRpQpiBackend::with_timing(
            common,
            sm0,
            sio0,
            sio1,
            sio2,
            sio3,
            cs,
            sck,
            TimingConfig::PICOCALC_SAFE,
        );
        let mut driver =
            BlockingDriver::with_config(backend, Pins::PICOCALC, TimingConfig::PICOCALC_SAFE);
        let device_id = driver.init().map_err(map_err)?;
        Ok(Self {
            driver,
            device_id: Some(device_id),
            ready: true,
        })
    }

    /// Fast-CodeWindow variant of [`KotoPsram::new`]: identical safe init, but
    /// also attaches the RX DMA channel (PAC channel 1) the validated
    /// `FastFallingClkdiv2` read drives. Ordinary reads/writes still run on the
    /// safe profile; only CodeWindow refills opt into the fast path.
    #[cfg(feature = "psram_fast_code_window")]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        common: embassy_rp::pio::Common<'d, PsramPio>,
        sm0: embassy_rp::pio::StateMachine<'d, PsramPio, 0>,
        cs: Peri<'d, PsramCsPin>,
        sck: Peri<'d, PsramSckPin>,
        sio0: Peri<'d, PsramSio0Pin>,
        sio1: Peri<'d, PsramSio1Pin>,
        sio2: Peri<'d, PsramSio2Pin>,
        sio3: Peri<'d, PsramSio3Pin>,
        rx_dma: dma::Channel<'d>,
    ) -> Result<Self, HalError> {
        let backend = EmbassyRpQpiBackend::with_timing(
            common,
            sm0,
            sio0,
            sio1,
            sio2,
            sio3,
            cs,
            sck,
            TimingConfig::PICOCALC_SAFE,
        )
        .with_rx_dma_channel_id(rx_dma, FAST_RX_DMA_PAC_CH);
        let mut driver =
            BlockingDriver::with_config(backend, Pins::PICOCALC, TimingConfig::PICOCALC_SAFE);
        let device_id = driver.init().map_err(map_err)?;
        Ok(Self {
            driver,
            device_id: Some(device_id),
            ready: true,
            mode: ReadMode::Safe,
        })
    }

    /// Returns the device identity captured during init, for bring-up logging.
    pub fn device_id(&self) -> Option<DeviceId> {
        self.device_id
    }

    /// Switch the driver to the safe read profile if it is not already there.
    #[cfg(feature = "psram_fast_code_window")]
    fn ensure_safe_mode(&mut self) -> Result<(), HalError> {
        if self.mode != ReadMode::Safe {
            self.driver
                .configure_timing(TimingConfig::PICOCALC_SAFE)
                .map_err(map_err)?;
            self.driver
                .backend_mut_for_diagnostics()
                .set_payload_transfer_path_for_diagnostics(PayloadTransferPath::ByteFallback);
            self.mode = ReadMode::Safe;
        }
        Ok(())
    }

    /// Switch the driver to the fast `FastFallingClkdiv2` read profile if needed.
    #[cfg(feature = "psram_fast_code_window")]
    fn ensure_fast_mode(&mut self) -> Result<(), HalError> {
        if self.mode != ReadMode::Fast {
            self.driver.configure_timing(FAST_TIMING).map_err(map_err)?;
            self.mode = ReadMode::Fast;
        }
        Ok(())
    }

    /// Attempt a CodeWindow refill through the validated `FastFallingClkdiv2`
    /// read, falling back to the safe read (and recording the fallback) on any
    /// unsupported address/length, config error, or read error. Never panics and
    /// never returns partially-read data: the safe read fully re-reads on
    /// fallback.
    #[cfg(feature = "psram_fast_code_window")]
    fn read_code_window_fast_or_fallback(
        &mut self,
        address: u32,
        dst: &mut [u8],
    ) -> Result<(), HalError> {
        // The fast RX-DMA read requires a 16-byte-aligned, non-empty range.
        if dst.is_empty() || !address.is_multiple_of(16) {
            fast_counters::record_fallback(fast_counters::REASON_UNSUPPORTED);
            return self.read(address, dst);
        }
        let Some(addr) = PsramAddr::new(address) else {
            fast_counters::record_fallback(fast_counters::REASON_UNSUPPORTED);
            return self.read(address, dst);
        };
        if self.ensure_fast_mode().is_err() {
            fast_counters::record_fallback(fast_counters::REASON_CONFIG);
            return self.read(address, dst);
        }
        match self
            .driver
            .backend_mut_for_diagnostics()
            .read_code_window_fast_falling_clkdiv2(addr, dst)
        {
            Ok(()) => {
                fast_counters::record_fast_success();
                Ok(())
            }
            Err(_error) => {
                fast_counters::record_fallback(fast_counters::REASON_READ_ERROR);
                // `read` switches the driver back to the safe profile and fully
                // re-reads the same range, so the window is never left corrupt.
                self.read(address, dst)
            }
        }
    }
}

impl PsramHal for KotoPsram<'_> {
    fn available(&self) -> bool {
        self.ready
    }

    fn read(&mut self, address: u32, dst: &mut [u8]) -> Result<(), HalError> {
        #[cfg(feature = "psram_fast_code_window")]
        self.ensure_safe_mode()?;
        let addr = PsramAddr::new(address).ok_or(HalError::InvalidArgument)?;
        self.driver.read_exact(addr, dst).map_err(map_err)
    }

    fn write(&mut self, address: u32, src: &[u8]) -> Result<(), HalError> {
        #[cfg(feature = "psram_fast_code_window")]
        self.ensure_safe_mode()?;
        let addr = PsramAddr::new(address).ok_or(HalError::InvalidArgument)?;
        self.driver.write_all(addr, src).map_err(map_err)
    }

    /// Opt-in fast CodeWindow refill. Without `psram_fast_code_window` this is
    /// the defaulted trait method (identical to [`PsramHal::read`]); with it, the
    /// refill attempts the validated fast read and falls back to the safe read.
    #[cfg(feature = "psram_fast_code_window")]
    fn read_code_window(&mut self, address: u32, dst: &mut [u8]) -> Result<(), HalError> {
        self.read_code_window_fast_or_fallback(address, dst)
    }
}

/// Maps a `koto-psram` backend error onto the in-tree HAL error space.
fn map_err(error: EmbassyRpQpiError) -> HalError {
    match error {
        EmbassyRpQpiError::Timeout | EmbassyRpQpiError::Core(PsramError::Timeout) => {
            HalError::Timeout
        }
        EmbassyRpQpiError::Unsupported => HalError::Unsupported,
        EmbassyRpQpiError::Core(PsramError::OutOfRange)
        | EmbassyRpQpiError::Core(PsramError::InvalidState)
        | EmbassyRpQpiError::StreamLength => HalError::InvalidArgument,
        EmbassyRpQpiError::Core(_)
        | EmbassyRpQpiError::InvalidResources
        | EmbassyRpQpiError::ProgramLoad => HalError::Io,
    }
}

/// Fast CodeWindow refill counters, mirrored into static atomics so the app
/// runtime can log fast/fallback usage without reaching into the concrete HAL
/// type (mirrors the in-tree DMA-experiment trace pattern). RP2040 (thumbv6m)
/// has no atomic RMW, so counts use a load/store increment like the DMA trace.
#[cfg(feature = "psram_fast_code_window")]
mod fast_counters {
    use core::sync::atomic::{AtomicU32, AtomicU8, Ordering};

    /// Bytes per fast read chunk (also the configured `max_chunk_len`).
    pub const PSRAM_FAST_CODE_WINDOW_CHUNK_BYTES: usize = 4096;

    const MODE_NONE: u8 = 0;
    const MODE_FAST: u8 = 1;
    const MODE_SAFE_FALLBACK: u8 = 2;

    pub(super) const REASON_NONE: u8 = 0;
    pub(super) const REASON_UNSUPPORTED: u8 = 1;
    pub(super) const REASON_CONFIG: u8 = 2;
    pub(super) const REASON_READ_ERROR: u8 = 3;

    static FAST_SUCCESS: AtomicU32 = AtomicU32::new(0);
    static FAST_FALLBACK: AtomicU32 = AtomicU32::new(0);
    static LAST_MODE: AtomicU8 = AtomicU8::new(MODE_NONE);
    static LAST_REASON: AtomicU8 = AtomicU8::new(REASON_NONE);

    fn inc(counter: &AtomicU32) {
        let value = counter.load(Ordering::Relaxed);
        counter.store(value.wrapping_add(1), Ordering::Relaxed);
    }

    pub(super) fn record_fast_success() {
        inc(&FAST_SUCCESS);
        LAST_MODE.store(MODE_FAST, Ordering::Relaxed);
        LAST_REASON.store(REASON_NONE, Ordering::Relaxed);
    }

    pub(super) fn record_fallback(reason: u8) {
        inc(&FAST_FALLBACK);
        LAST_MODE.store(MODE_SAFE_FALLBACK, Ordering::Relaxed);
        LAST_REASON.store(reason, Ordering::Relaxed);
    }

    /// Last CodeWindow refill read mode reported by [`KotoPsramFastReadSnapshot`].
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum KotoPsramFastReadMode {
        /// No CodeWindow refill has happened yet.
        None,
        /// The last refill used the fast `FastFallingClkdiv2` read.
        FastClkdiv2,
        /// The last refill fell back to the safe read.
        SafeFallback,
    }

    /// Snapshot of fast CodeWindow refill counters for diagnostics logging.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct KotoPsramFastReadSnapshot {
        /// Refills served by the fast read since boot.
        pub fast_success_count: u32,
        /// Refills that fell back to the safe read since boot.
        pub fast_fallback_count: u32,
        /// Read mode of the most recent refill.
        pub last_mode: KotoPsramFastReadMode,
        /// Reason for the most recent fallback (`"none"` if none).
        pub last_fallback_reason: &'static str,
    }

    /// Snapshot the fast CodeWindow refill counters.
    pub fn koto_psram_fast_code_window_snapshot() -> KotoPsramFastReadSnapshot {
        let last_mode = match LAST_MODE.load(Ordering::Relaxed) {
            MODE_FAST => KotoPsramFastReadMode::FastClkdiv2,
            MODE_SAFE_FALLBACK => KotoPsramFastReadMode::SafeFallback,
            _ => KotoPsramFastReadMode::None,
        };
        let last_fallback_reason = match LAST_REASON.load(Ordering::Relaxed) {
            REASON_UNSUPPORTED => "unsupported_addr_or_len",
            REASON_CONFIG => "fast_config_error",
            REASON_READ_ERROR => "fast_read_error",
            _ => "none",
        };
        KotoPsramFastReadSnapshot {
            fast_success_count: FAST_SUCCESS.load(Ordering::Relaxed),
            fast_fallback_count: FAST_FALLBACK.load(Ordering::Relaxed),
            last_mode,
            last_fallback_reason,
        }
    }
}
