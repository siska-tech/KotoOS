//! Bounded CYW43 residency layout for KOTO-0227.
//!
//! This module fixes the concrete Pico W transport and driver-handle sizes
//! before runtime construction is wired into the Audio/Wi-Fi mode owner.

#[cfg(not(target_os = "none"))]
use core::sync::atomic::{AtomicU32, Ordering};
use core::{
    future::Future,
    mem::MaybeUninit,
    task::{Context, Poll, Waker},
};
#[cfg(target_os = "none")]
use portable_atomic::{AtomicU32, Ordering};

use crate::firmware::arena_future::{ArenaFuture, ArenaFutureError};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum WifiLifecyclePhase {
    Offline = 0,
    Initializing = 1,
    DriverReady = 2,
    RadioReady = 3,
}

static WIFI_LIFECYCLE_PHASE: AtomicU32 = AtomicU32::new(WifiLifecyclePhase::Offline as u32);

/// KOTO-0245 bounded Fetch-transport diagnostics. Per the issue's diagnostic
/// criterion these expose only fixed status, request id, byte counts, and
/// bounded timing — never URLs, header values, body bytes, or TLS material.
/// The network dispatcher records one snapshot per terminal request; UART
/// drains it as a `phase=245 fetch-diag` line.
#[cfg(any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"))]
mod fetch_diag {
    use super::AtomicU32;

    pub(super) static SEQ: AtomicU32 = AtomicU32::new(0);
    pub(super) static PRINTED: AtomicU32 = AtomicU32::new(0);
    pub(super) static REQUEST: AtomicU32 = AtomicU32::new(0);
    /// `status << 16 | error_code` (error 0 = success).
    pub(super) static RESULT: AtomicU32 = AtomicU32::new(0);
    pub(super) static BYTES: AtomicU32 = AtomicU32::new(0);
    pub(super) static ELAPSED_MS: AtomicU32 = AtomicU32::new(0);
    /// Per-request HTTP snapshot staged by the TLS stage before its decoder
    /// reset; folded into the terminal record.
    pub(super) static HTTP_STATUS: AtomicU32 = AtomicU32::new(0);
    pub(super) static HTTP_BYTES: AtomicU32 = AtomicU32::new(0);
    /// `arena_tail_bytes << 16 | session_future_bytes`: the measured size of
    /// the installed TLS session future and the stream-scratch arena tail it
    /// must fit.
    pub(super) static SESSION_LAYOUT: AtomicU32 = AtomicU32::new(0);
    /// `capacity << 16 | peak_used`: dedicated TLS crypto-stack high-water and
    /// its capacity (KOTO-0245 stack-switch headroom check). `peak >= capacity`
    /// means the crypto spilled below the dedicated stack.
    pub(super) static CRYPTO_STACK: AtomicU32 = AtomicU32::new(0);
    /// Resolved IPv4 destination (public, big-endian octets a<<24|b<<16|c<<8|d)
    /// plus a low-byte connect-error discriminant, so a `Connect` failure shows
    /// which routable address was tried. Not a URL/query/secret.
    pub(super) static RESOLVED_IP: AtomicU32 = AtomicU32::new(0);
    pub(super) static CONNECT_ERR: AtomicU32 = AtomicU32::new(0);
    /// Furthest TLS handshake milestone reached (monotonic), so a stalled or
    /// failed handshake shows where it stopped. See `record_fetch_tls_phase`.
    pub(super) static TLS_PHASE: AtomicU32 = AtomicU32::new(0);
    /// Ciphertext bytes moved across the TLS transport this request, so a stall
    /// shows whether the peer stopped sending (rx flat) or the stall is after
    /// our last write (tx advanced).
    pub(super) static TLS_RX_BYTES: AtomicU32 = AtomicU32::new(0);
    pub(super) static TLS_TX_BYTES: AtomicU32 = AtomicU32::new(0);
    /// Transport read-call count and the byte length requested by the most
    /// recent read (captured before it awaits). On a stall the request size
    /// distinguishes a record-header wait (~5), a small body wait (~tens), or a
    /// bogus oversized body wait (mis-parsed header): `count << 16 | last_req`.
    pub(super) static TLS_READ_PROBE: AtomicU32 = AtomicU32::new(0);
}

#[cfg(any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FetchTransportDiag {
    pub sequence: u32,
    pub request_raw: u32,
    /// `FetchError` discriminant; 0 means the exchange completed.
    pub error_code: u8,
    pub status: u16,
    pub bytes: u32,
    pub elapsed_ms: u32,
    pub session_future_bytes: u16,
    pub session_arena_bytes: u16,
    pub crypto_stack_peak: u16,
    pub crypto_stack_capacity: u16,
    pub resolved_ip: [u8; 4],
    pub connect_error: u8,
    pub tls_phase: u32,
    pub tls_rx_bytes: u32,
    pub tls_tx_bytes: u32,
    pub tls_read_count: u16,
    pub tls_last_read_req: u16,
}

/// Clears the per-request HTTP snapshot when the executor accepts a command.
#[cfg(any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"))]
pub(crate) fn record_fetch_command_started() {
    fetch_diag::HTTP_STATUS.store(0, Ordering::Relaxed);
    fetch_diag::HTTP_BYTES.store(0, Ordering::Relaxed);
    fetch_diag::SESSION_LAYOUT.store(0, Ordering::Relaxed);
    fetch_diag::CRYPTO_STACK.store(0, Ordering::Relaxed);
    fetch_diag::RESOLVED_IP.store(0, Ordering::Relaxed);
    fetch_diag::CONNECT_ERR.store(0, Ordering::Relaxed);
    fetch_diag::TLS_PHASE.store(0, Ordering::Relaxed);
    fetch_diag::TLS_RX_BYTES.store(0, Ordering::Relaxed);
    fetch_diag::TLS_TX_BYTES.store(0, Ordering::Relaxed);
    fetch_diag::TLS_READ_PROBE.store(0, Ordering::Relaxed);
}

/// Accumulates TLS transport ciphertext byte counts (KOTO-0245 stall triage).
#[cfg(any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"))]
pub fn record_fetch_tls_io(rx_delta: usize, tx_delta: usize) {
    if rx_delta != 0 {
        let _ = fetch_diag::TLS_RX_BYTES.fetch_add(rx_delta as u32, Ordering::Relaxed);
    }
    if tx_delta != 0 {
        let _ = fetch_diag::TLS_TX_BYTES.fetch_add(tx_delta as u32, Ordering::Relaxed);
    }
}

/// Records a transport read request (its buffer length) as it begins to await,
/// bumping the read-call count. The last such value survives a stall.
#[cfg(any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"))]
pub fn record_fetch_tls_read_request(requested: usize) {
    let count = (fetch_diag::TLS_READ_PROBE.load(Ordering::Relaxed) >> 16).wrapping_add(1);
    let req = (requested.min(0xffff)) as u32;
    fetch_diag::TLS_READ_PROBE.store((count << 16) | req, Ordering::Relaxed);
}

/// TLS handshake milestones. Recorded monotonically so the terminal snapshot
/// shows the furthest point a stalled/failed handshake reached: whether it hung
/// before the server responded (server/network), inside pin/signature checks
/// (cert or MFL/cipher mismatch), or after the handshake (HTTP exchange).
#[cfg(any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"))]
pub mod tls_phase {
    pub const SESSION_ENTERED: u32 = 1;
    pub const OPEN_STARTED: u32 = 2;
    pub const VERIFY_HOSTNAME: u32 = 10;
    pub const VERIFY_CERT_ENTERED: u32 = 11;
    pub const VERIFY_CERT_SPKI: u32 = 12;
    pub const VERIFY_CERT_PIN_OK: u32 = 13;
    pub const VERIFY_CERT_KEY_OK: u32 = 14;
    pub const VERIFY_SIG_ENTERED: u32 = 15;
    pub const VERIFY_SIG_OK: u32 = 16;
    pub const OPEN_DONE: u32 = 20;
    pub const REQUEST_SENT: u32 = 21;
    pub const RESPONSE_STARTED: u32 = 22;
    pub const COMPLETE: u32 = 23;
}

/// Advances the recorded TLS handshake milestone to `phase` if it is further
/// along than the current value.
#[cfg(any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"))]
pub fn record_fetch_tls_phase(phase: u32) {
    let _ = fetch_diag::TLS_PHASE.fetch_max(phase, Ordering::Relaxed);
}

/// Records the resolved public IPv4 destination selected for connect.
#[cfg(any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"))]
pub(crate) fn record_fetch_resolved_ip(octets: [u8; 4]) {
    fetch_diag::RESOLVED_IP.store(u32::from_be_bytes(octets), Ordering::Relaxed);
}

/// Records a transport connect-error discriminant (nonzero) for diagnostics.
#[cfg(any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"))]
pub(crate) fn record_fetch_connect_error(code: u8) {
    fetch_diag::CONNECT_ERR.store(u32::from(code), Ordering::Relaxed);
}

/// Stages the decoder outcome of the TLS session before its state is reset.
#[cfg(any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"))]
pub(crate) fn record_fetch_http_snapshot(status: u16, bytes: u32) {
    fetch_diag::HTTP_STATUS.store(u32::from(status), Ordering::Relaxed);
    fetch_diag::HTTP_BYTES.store(bytes, Ordering::Relaxed);
}

/// Records the measured TLS session future against its workspace arena tail.
#[cfg(any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"))]
pub(crate) fn record_fetch_session_layout(future_bytes: usize, arena_bytes: usize) {
    let future = future_bytes.min(u16::MAX as usize) as u32;
    let arena = arena_bytes.min(u16::MAX as usize) as u32;
    fetch_diag::SESSION_LAYOUT.store(arena << 16 | future, Ordering::Relaxed);
}

/// Records the dedicated TLS crypto-stack high-water against its capacity.
#[cfg(any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"))]
pub(crate) fn record_fetch_crypto_stack(peak_used: usize, capacity: usize) {
    let peak = peak_used.min(u16::MAX as usize) as u32;
    let cap = capacity.min(u16::MAX as usize) as u32;
    fetch_diag::CRYPTO_STACK.store(cap << 16 | peak, Ordering::Relaxed);
}

/// Publishes one terminal snapshot; the sequence increment makes it visible
/// to the UART drain.
#[cfg(any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"))]
pub(crate) fn record_fetch_terminal(
    request_raw: u32,
    error: Option<koto_core::FetchError>,
    elapsed_ms: u32,
) {
    let status = fetch_diag::HTTP_STATUS.load(Ordering::Relaxed);
    let error_code = error.map_or(0, |error| error as u32);
    fetch_diag::REQUEST.store(request_raw, Ordering::Relaxed);
    fetch_diag::RESULT.store(status << 16 | error_code, Ordering::Relaxed);
    fetch_diag::BYTES.store(
        fetch_diag::HTTP_BYTES.load(Ordering::Relaxed),
        Ordering::Relaxed,
    );
    fetch_diag::ELAPSED_MS.store(elapsed_ms, Ordering::Relaxed);
    let _ = fetch_diag::SEQ.fetch_add(1, Ordering::Release);
}

/// Drains at most one unprinted terminal snapshot. Multiple UART loops may
/// call this; the compare-exchange lets exactly one of them print it.
#[cfg(any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"))]
pub fn take_fetch_transport_diag() -> Option<FetchTransportDiag> {
    let sequence = fetch_diag::SEQ.load(Ordering::Acquire);
    if sequence == 0 {
        return None;
    }
    let printed = fetch_diag::PRINTED.load(Ordering::Acquire);
    if printed == sequence
        || fetch_diag::PRINTED
            .compare_exchange(printed, sequence, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
    {
        return None;
    }
    let result = fetch_diag::RESULT.load(Ordering::Relaxed);
    let layout = fetch_diag::SESSION_LAYOUT.load(Ordering::Relaxed);
    let crypto = fetch_diag::CRYPTO_STACK.load(Ordering::Relaxed);
    Some(FetchTransportDiag {
        sequence,
        request_raw: fetch_diag::REQUEST.load(Ordering::Relaxed),
        error_code: (result & 0xff) as u8,
        status: (result >> 16) as u16,
        bytes: fetch_diag::BYTES.load(Ordering::Relaxed),
        elapsed_ms: fetch_diag::ELAPSED_MS.load(Ordering::Relaxed),
        session_future_bytes: (layout & 0xffff) as u16,
        session_arena_bytes: (layout >> 16) as u16,
        crypto_stack_peak: (crypto & 0xffff) as u16,
        crypto_stack_capacity: (crypto >> 16) as u16,
        resolved_ip: fetch_diag::RESOLVED_IP
            .load(Ordering::Relaxed)
            .to_be_bytes(),
        connect_error: (fetch_diag::CONNECT_ERR.load(Ordering::Relaxed) & 0xff) as u8,
        tls_phase: fetch_diag::TLS_PHASE.load(Ordering::Relaxed),
        tls_rx_bytes: fetch_diag::TLS_RX_BYTES.load(Ordering::Relaxed),
        tls_tx_bytes: fetch_diag::TLS_TX_BYTES.load(Ordering::Relaxed),
        tls_read_count: (fetch_diag::TLS_READ_PROBE.load(Ordering::Relaxed) >> 16) as u16,
        tls_last_read_req: (fetch_diag::TLS_READ_PROBE.load(Ordering::Relaxed) & 0xffff) as u16,
    })
}
static WIFI_SPI_READS: AtomicU32 = AtomicU32::new(0);
static WIFI_SPI_WRITES: AtomicU32 = AtomicU32::new(0);
static WIFI_SPI_LAST_STATUS: AtomicU32 = AtomicU32::new(0);
static WIFI_SPI_LAST_WORD: AtomicU32 = AtomicU32::new(0);
static WIFI_POWER_HIGHS: AtomicU32 = AtomicU32::new(0);
static WIFI_POWER_LEVELS: AtomicU32 = AtomicU32::new(0);
static WIFI_GPIO_IN: AtomicU32 = AtomicU32::new(0);
static WIFI_PIN_FUNCS: AtomicU32 = AtomicU32::new(0);
static WIFI_PIO_CTRL: AtomicU32 = AtomicU32::new(0);
static WIFI_PIO_FSTAT: AtomicU32 = AtomicU32::new(0);
static WIFI_PIO_FDEBUG: AtomicU32 = AtomicU32::new(0);
static WIFI_PIO_PADOUT: AtomicU32 = AtomicU32::new(0);
static WIFI_PIO_PADOE: AtomicU32 = AtomicU32::new(0);
static WIFI_PIO_SM0_ADDR: AtomicU32 = AtomicU32::new(0);
/// Frames staged by the KOTO-0227 stream-soak future since its last install.
static WIFI_SOAK_TX_FRAMES: AtomicU32 = AtomicU32::new(0);

pub fn wifi_soak_tx_frames() -> u32 {
    WIFI_SOAK_TX_FRAMES.load(Ordering::Relaxed)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WifiSpiTelemetry {
    pub reads: u32,
    pub writes: u32,
    pub last_status: u32,
    pub last_word: u32,
    pub power_highs: u32,
    pub power_latch_high: bool,
    pub power_input_high: bool,
    pub gpio_in: u32,
    pub pin_funcs: u32,
    pub pio_ctrl: u32,
    pub pio_fstat: u32,
    pub pio_fdebug: u32,
    pub pio_padout: u32,
    pub pio_padoe: u32,
    pub pio_sm0_addr: u32,
}

pub fn wifi_spi_telemetry() -> WifiSpiTelemetry {
    WifiSpiTelemetry {
        reads: WIFI_SPI_READS.load(Ordering::Relaxed),
        writes: WIFI_SPI_WRITES.load(Ordering::Relaxed),
        last_status: WIFI_SPI_LAST_STATUS.load(Ordering::Relaxed),
        last_word: WIFI_SPI_LAST_WORD.load(Ordering::Relaxed),
        power_highs: WIFI_POWER_HIGHS.load(Ordering::Relaxed),
        power_latch_high: WIFI_POWER_LEVELS.load(Ordering::Relaxed) & 1 != 0,
        power_input_high: WIFI_POWER_LEVELS.load(Ordering::Relaxed) & 2 != 0,
        gpio_in: WIFI_GPIO_IN.load(Ordering::Relaxed),
        pin_funcs: WIFI_PIN_FUNCS.load(Ordering::Relaxed),
        pio_ctrl: WIFI_PIO_CTRL.load(Ordering::Relaxed),
        pio_fstat: WIFI_PIO_FSTAT.load(Ordering::Relaxed),
        pio_fdebug: WIFI_PIO_FDEBUG.load(Ordering::Relaxed),
        pio_padout: WIFI_PIO_PADOUT.load(Ordering::Relaxed),
        pio_padoe: WIFI_PIO_PADOE.load(Ordering::Relaxed),
        pio_sm0_addr: WIFI_PIO_SM0_ADDR.load(Ordering::Relaxed),
    }
}

pub fn wifi_lifecycle_phase() -> WifiLifecyclePhase {
    match WIFI_LIFECYCLE_PHASE.load(Ordering::Acquire) {
        1 => WifiLifecyclePhase::Initializing,
        2 => WifiLifecyclePhase::DriverReady,
        3 => WifiLifecyclePhase::RadioReady,
        _ => WifiLifecyclePhase::Offline,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WifiLifecycleError {
    AlreadyActive,
    StaleGeneration,
    Future(ArenaFutureError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum TlsAudioExclusionState {
    Idle = 0,
    QuiesceRequested = 1,
    Quiescing = 2,
    ExclusiveReady = 3,
    WorkspaceOwned = 4,
    RestoreRequested = 5,
    Restoring = 6,
    Complete = 7,
    CancelRequested = 8,
    Failed = 9,
}

impl TlsAudioExclusionState {
    const fn from_word(word: u32) -> Self {
        match word as u8 {
            1 => Self::QuiesceRequested,
            2 => Self::Quiescing,
            3 => Self::ExclusiveReady,
            4 => Self::WorkspaceOwned,
            5 => Self::RestoreRequested,
            6 => Self::Restoring,
            7 => Self::Complete,
            8 => Self::CancelRequested,
            9 => Self::Failed,
            _ => Self::Idle,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TlsAudioExclusionToken(u32);

impl TlsAudioExclusionToken {
    pub const fn generation(self) -> u32 {
        self.0
    }
}

/// One-word, generation-tagged handshake between the network future and the
/// CPU0 audio facade. It carries no pointer or workspace reference. State uses
/// the low byte and a wrapping nonzero 24-bit generation uses the high bytes.
pub struct TlsAudioExclusionCoordinator {
    word: AtomicU32,
}

impl TlsAudioExclusionCoordinator {
    const STATE_BITS: u32 = 8;
    const GENERATION_MASK: u32 = 0x00ff_ffff;

    pub const fn new() -> Self {
        Self {
            word: AtomicU32::new(0),
        }
    }

    pub fn state(&self) -> TlsAudioExclusionState {
        TlsAudioExclusionState::from_word(self.word.load(Ordering::Acquire))
    }

    pub fn generation(&self) -> u32 {
        self.word.load(Ordering::Acquire) >> Self::STATE_BITS
    }

    pub fn active_token(&self) -> Option<TlsAudioExclusionToken> {
        let word = self.word.load(Ordering::Acquire);
        if TlsAudioExclusionState::from_word(word) == TlsAudioExclusionState::Idle {
            None
        } else {
            Some(TlsAudioExclusionToken(word >> Self::STATE_BITS))
        }
    }

    pub fn request(&self) -> Option<TlsAudioExclusionToken> {
        let mut current = self.word.load(Ordering::Acquire);
        loop {
            if TlsAudioExclusionState::from_word(current) != TlsAudioExclusionState::Idle {
                return None;
            }
            let mut generation =
                ((current >> Self::STATE_BITS).wrapping_add(1)) & Self::GENERATION_MASK;
            if generation == 0 {
                generation = 1;
            }
            let next = Self::pack(generation, TlsAudioExclusionState::QuiesceRequested);
            match self.word.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Some(TlsAudioExclusionToken(generation)),
                Err(observed) => current = observed,
            }
        }
    }

    pub fn transition(
        &self,
        token: TlsAudioExclusionToken,
        from: TlsAudioExclusionState,
        to: TlsAudioExclusionState,
    ) -> bool {
        self.word
            .compare_exchange(
                Self::pack(token.0, from),
                Self::pack(token.0, to),
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }

    pub fn request_cancel(&self, token: TlsAudioExclusionToken) -> bool {
        for state in [
            TlsAudioExclusionState::QuiesceRequested,
            TlsAudioExclusionState::Quiescing,
            TlsAudioExclusionState::ExclusiveReady,
        ] {
            if self.transition(token, state, TlsAudioExclusionState::CancelRequested) {
                return true;
            }
        }
        false
    }

    pub fn reset_terminal(&self, token: TlsAudioExclusionToken) -> bool {
        for terminal in [
            TlsAudioExclusionState::Complete,
            TlsAudioExclusionState::Failed,
        ] {
            if self.transition(token, terminal, TlsAudioExclusionState::Idle) {
                return true;
            }
        }
        false
    }

    const fn pack(generation: u32, state: TlsAudioExclusionState) -> u32 {
        ((generation & Self::GENERATION_MASK) << Self::STATE_BITS) | state as u32
    }
}

impl Default for TlsAudioExclusionCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

const _: () = assert!(core::mem::size_of::<TlsAudioExclusionCoordinator>() == 4);

pub struct WifiLifecycleController<'storage> {
    future: Option<ArenaFuture<'storage>>,
    generation: u32,
    joined_generation: u32,
    polls: u32,
}

impl<'storage> WifiLifecycleController<'storage> {
    pub const fn new() -> Self {
        Self {
            future: None,
            generation: 0,
            joined_generation: 0,
            polls: 0,
        }
    }

    pub fn install(
        &mut self,
        generation: u32,
        future: ArenaFuture<'storage>,
    ) -> Result<(), WifiLifecycleError> {
        if self.future.is_some() {
            return Err(WifiLifecycleError::AlreadyActive);
        }
        self.generation = generation;
        self.joined_generation = 0;
        self.polls = 0;
        self.future = Some(future);
        Ok(())
    }

    pub fn try_install<F>(
        &mut self,
        generation: u32,
        storage: &'storage mut [MaybeUninit<u8>],
        future: F,
    ) -> Result<(), WifiLifecycleError>
    where
        F: Future<Output = ()> + 'storage,
    {
        let future = ArenaFuture::try_new(storage, future).map_err(WifiLifecycleError::Future)?;
        self.install(generation, future)
    }

    pub fn service(&mut self) {
        let waker = Waker::noop();
        let mut context = Context::from_waker(waker);
        self.service_with_context(&mut context);
    }

    pub fn service_with_context(&mut self, context: &mut Context<'_>) {
        let Some(future) = self.future.as_mut() else {
            return;
        };
        self.polls = self.polls.saturating_add(1);
        if future.poll_once(context) == Poll::Ready(()) {
            self.future = None;
            self.joined_generation = self.generation;
        }
    }

    pub fn cancel(&mut self, generation: u32) -> Result<(), WifiLifecycleError> {
        if generation != self.generation {
            return Err(WifiLifecycleError::StaleGeneration);
        }
        if let Some(mut future) = self.future.take() {
            future.cancel();
        }
        self.joined_generation = generation;
        Ok(())
    }

    pub const fn is_active(&self) -> bool {
        self.future.is_some()
    }

    pub const fn joined_generation(&self) -> u32 {
        self.joined_generation
    }

    pub const fn polls(&self) -> u32 {
        self.polls
    }
}

impl Default for WifiLifecycleController<'_> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"))]
mod w_board {
    use core::cell::RefCell;
    use core::{
        convert::Infallible,
        future::Future,
        mem::MaybeUninit,
        task::{Context, Poll},
    };

    use critical_section::Mutex;
    use cyw43::{Control, NetDriver, Runner, SpiBus, SpiBusCyw43, State};
    use cyw43_pio::PioSpi;
    use cyw43_pio::RM2_CLOCK_DIVIDER;
    use embassy_net_driver::{Driver, TxToken};
    use embassy_rp::gpio::Output;
    use embassy_rp::{
        dma::{self, Channel},
        gpio::Level,
        interrupt::typelevel::Binding,
        pio::{self, Pio},
    };
    use embassy_rp::{gpio::Flex, peripherals};
    use embassy_time::Timer;
    use embedded_hal::digital::{ErrorType, OutputPin};
    use koto_core::FetchTransportMailbox;

    use portable_atomic::Ordering;

    use super::TlsAudioExclusionState;
    use super::{
        TlsAudioExclusionCoordinator, WifiLifecycleController, WifiLifecycleError,
        WifiLifecyclePhase, WIFI_GPIO_IN, WIFI_LIFECYCLE_PHASE, WIFI_PIN_FUNCS, WIFI_PIO_CTRL,
        WIFI_PIO_FDEBUG, WIFI_PIO_FSTAT, WIFI_PIO_PADOE, WIFI_PIO_PADOUT, WIFI_PIO_SM0_ADDR,
        WIFI_POWER_HIGHS, WIFI_POWER_LEVELS, WIFI_RESIDENCY_BYTES, WIFI_SPI_LAST_STATUS,
        WIFI_SPI_LAST_WORD, WIFI_SPI_READS, WIFI_SPI_WRITES,
    };
    use crate::board::{PicoWRadioResources, RadioPowerPin};
    use crate::firmware::audio::WifiResidencyArena;

    pub struct RadioPowerOutput(Flex<'static>);

    pub struct CooperativePioSpi {
        inner: PioSpi<'static, peripherals::PIO0, 0>,
    }

    impl CooperativePioSpi {
        pub fn new(inner: PioSpi<'static, peripherals::PIO0, 0>) -> Self {
            Self { inner }
        }
    }

    impl SpiBusCyw43 for CooperativePioSpi {
        async fn cmd_write(&mut self, write: &[u32]) -> u32 {
            let status = SpiBusCyw43::cmd_write(&mut self.inner, write).await;
            WIFI_SPI_LAST_STATUS.store(status, Ordering::Relaxed);
            WIFI_SPI_WRITES.fetch_add(1, Ordering::Relaxed);
            status
        }

        async fn cmd_read(&mut self, write: u32, read: &mut [u32]) -> u32 {
            let status = SpiBusCyw43::cmd_read(&mut self.inner, write, read).await;
            WIFI_SPI_LAST_STATUS.store(status, Ordering::Relaxed);
            WIFI_SPI_LAST_WORD.store(read.first().copied().unwrap_or(0), Ordering::Relaxed);
            WIFI_SPI_READS.fetch_add(1, Ordering::Relaxed);
            status
        }

        async fn wait_for_event(&mut self) {
            self.inner.wait_for_event().await;
        }
    }

    impl RadioPowerOutput {
        fn new(pin: embassy_rp::Peri<'static, RadioPowerPin>) -> Self {
            let mut pin = Flex::new(pin);
            pin.set_low();
            pin.set_as_output();
            Self(pin)
        }
    }

    impl ErrorType for RadioPowerOutput {
        type Error = Infallible;
    }

    impl OutputPin for RadioPowerOutput {
        fn set_low(&mut self) -> Result<(), Self::Error> {
            self.0.set_low();
            WIFI_POWER_LEVELS.store(0, Ordering::Relaxed);
            Ok(())
        }

        fn set_high(&mut self) -> Result<(), Self::Error> {
            self.0.set_high();
            WIFI_POWER_HIGHS.fetch_add(1, Ordering::Relaxed);
            let levels = u32::from(self.0.is_set_high()) | (u32::from(self.0.is_high()) << 1);
            WIFI_POWER_LEVELS.store(levels, Ordering::Relaxed);
            Ok(())
        }
    }

    impl Drop for RadioPowerOutput {
        fn drop(&mut self) {
            self.0.set_low();
            WIFI_POWER_LEVELS.store(0, Ordering::Relaxed);
            WIFI_LIFECYCLE_PHASE.store(WifiLifecyclePhase::Offline as u32, Ordering::Release);
        }
    }

    pub type PicoWRadioSpi = CooperativePioSpi;
    pub type PicoWRadioBus = SpiBus<RadioPowerOutput, PicoWRadioSpi>;
    pub type PicoWRadioRunner = Runner<'static, PicoWRadioBus>;

    static WIFI_FIRMWARE: cyw43::Aligned<cyw43::A4, [u8; 231_077]> =
        cyw43::Aligned(*cyw43_firmware::CYW43_43439A0);

    pub fn cyw43_lifecycle_future<PioIrq, DmaTxIrq, DmaRxIrq>(
        state: &'static mut State,
        resources: PicoWRadioResources,
        pio_irq: PioIrq,
        dma_tx_irq: DmaTxIrq,
        dma_rx_irq: DmaRxIrq,
    ) -> impl Future<Output = ()> + 'static
    where
        PioIrq: Binding<
                <peripherals::PIO0 as pio::Instance>::Interrupt,
                pio::InterruptHandler<peripherals::PIO0>,
            > + 'static,
        DmaTxIrq: Binding<
                <peripherals::DMA_CH2 as dma::ChannelInstance>::Interrupt,
                dma::InterruptHandler<peripherals::DMA_CH2>,
            > + 'static,
        DmaRxIrq: Binding<
                <peripherals::DMA_CH3 as dma::ChannelInstance>::Interrupt,
                dma::InterruptHandler<peripherals::DMA_CH3>,
            > + 'static,
    {
        async move {
            WIFI_SPI_READS.store(0, Ordering::Relaxed);
            WIFI_SPI_WRITES.store(0, Ordering::Relaxed);
            WIFI_SPI_LAST_STATUS.store(0, Ordering::Relaxed);
            WIFI_SPI_LAST_WORD.store(0, Ordering::Relaxed);
            WIFI_POWER_HIGHS.store(0, Ordering::Relaxed);
            WIFI_POWER_LEVELS.store(0, Ordering::Relaxed);
            WIFI_GPIO_IN.store(0, Ordering::Relaxed);
            WIFI_PIN_FUNCS.store(0, Ordering::Relaxed);
            WIFI_PIO_CTRL.store(0, Ordering::Relaxed);
            WIFI_PIO_FSTAT.store(0, Ordering::Relaxed);
            WIFI_PIO_FDEBUG.store(0, Ordering::Relaxed);
            WIFI_PIO_PADOUT.store(0, Ordering::Relaxed);
            WIFI_PIO_PADOE.store(0, Ordering::Relaxed);
            WIFI_PIO_SM0_ADDR.store(0, Ordering::Relaxed);
            WIFI_LIFECYCLE_PHASE.store(WifiLifecyclePhase::Initializing as u32, Ordering::Release);
            let mut power = RadioPowerOutput::new(resources.power);
            let _ = power.set_low();
            // Stock ~20 ms WL_ON pre-reset interval. The diagnostic one-second
            // profile was an isolation aid only; the CS-framing fix
            // (`CooperativePioSpi` fully-qualified `SpiBusCyw43` delegation)
            // was hardware-proven with stock timing, and the 100-trip
            // acceptance soak must validate the product reset profile.
            Timer::after_millis(20).await;
            let cs = Output::new(resources.cs, Level::High);
            let mut pio = Pio::new(resources.pio, pio_irq);
            let spi = CooperativePioSpi::new(PioSpi::new(
                &mut pio.common,
                pio.sm0,
                RM2_CLOCK_DIVIDER,
                pio.irq0,
                cs,
                resources.data,
                resources.clock,
                Channel::new(resources.dma_tx, dma_tx_irq),
                Channel::new(resources.dma_rx, dma_rx_irq),
            ));
            let nvram = cyw43::aligned_bytes!("../../nvram_rp2040.bin");
            let (mut net_driver, mut control, runner) =
                cyw43::new(state, power, spi, &WIFI_FIRMWARE, nvram).await;
            WIFI_LIFECYCLE_PHASE.store(WifiLifecyclePhase::DriverReady as u32, Ordering::Release);
            let control_task = async move {
                control.init(cyw43_firmware::CYW43_43439A0_CLM).await;
                for sequence in 0..5u8 {
                    core::future::poll_fn(|context| {
                        match Driver::transmit(&mut net_driver, context) {
                            Some(token) => {
                                token.consume(64, |frame| {
                                    frame[..6].fill(0xff);
                                    frame[6..12]
                                        .copy_from_slice(&[0x02, 0x4b, 0x4f, 0x54, 0x4f, sequence]);
                                    frame[12..14].copy_from_slice(&[0x88, 0xb5]);
                                    frame[14..].fill(sequence);
                                });
                                Poll::Ready(())
                            }
                            None => Poll::Pending,
                        }
                    })
                    .await;
                }
                WIFI_LIFECYCLE_PHASE
                    .store(WifiLifecyclePhase::RadioReady as u32, Ordering::Release);
                core::future::pending::<()>().await;
                drop(net_driver);
            };
            embassy_futures::join::join(runner.run(), control_task).await;
        }
    }

    /// KOTO-0227 stream-soak lifecycle future: identical bring-up to
    /// [`cyw43_lifecycle_future`], but after CLM initialization it publishes
    /// `RadioReady` immediately and then stages one bounded 64-byte broadcast
    /// frame per second through the four-entry CYW43 TX channel for as long as
    /// the soak keeps polling, counting each staged frame in
    /// [`wifi_soak_tx_frames`].
    #[cfg(feature = "wifi_stream_soak_probe")]
    pub fn cyw43_soak_future<PioIrq, DmaTxIrq, DmaRxIrq>(
        state: &'static mut State,
        resources: PicoWRadioResources,
        pio_irq: PioIrq,
        dma_tx_irq: DmaTxIrq,
        dma_rx_irq: DmaRxIrq,
    ) -> impl Future<Output = ()> + 'static
    where
        PioIrq: Binding<
                <peripherals::PIO0 as pio::Instance>::Interrupt,
                pio::InterruptHandler<peripherals::PIO0>,
            > + 'static,
        DmaTxIrq: Binding<
                <peripherals::DMA_CH2 as dma::ChannelInstance>::Interrupt,
                dma::InterruptHandler<peripherals::DMA_CH2>,
            > + 'static,
        DmaRxIrq: Binding<
                <peripherals::DMA_CH3 as dma::ChannelInstance>::Interrupt,
                dma::InterruptHandler<peripherals::DMA_CH3>,
            > + 'static,
    {
        async move {
            super::WIFI_SOAK_TX_FRAMES.store(0, Ordering::Relaxed);
            WIFI_LIFECYCLE_PHASE.store(WifiLifecyclePhase::Initializing as u32, Ordering::Release);
            let mut power = RadioPowerOutput::new(resources.power);
            let _ = power.set_low();
            Timer::after_millis(20).await;
            let cs = Output::new(resources.cs, Level::High);
            let mut pio = Pio::new(resources.pio, pio_irq);
            let spi = CooperativePioSpi::new(PioSpi::new(
                &mut pio.common,
                pio.sm0,
                RM2_CLOCK_DIVIDER,
                pio.irq0,
                cs,
                resources.data,
                resources.clock,
                Channel::new(resources.dma_tx, dma_tx_irq),
                Channel::new(resources.dma_rx, dma_rx_irq),
            ));
            let nvram = cyw43::aligned_bytes!("../../nvram_rp2040.bin");
            let (mut net_driver, mut control, runner) =
                cyw43::new(state, power, spi, &WIFI_FIRMWARE, nvram).await;
            WIFI_LIFECYCLE_PHASE.store(WifiLifecyclePhase::DriverReady as u32, Ordering::Release);
            let control_task = async move {
                control.init(cyw43_firmware::CYW43_43439A0_CLM).await;
                WIFI_LIFECYCLE_PHASE
                    .store(WifiLifecyclePhase::RadioReady as u32, Ordering::Release);
                let mut sequence = 0u8;
                loop {
                    Timer::after_millis(1_000).await;
                    core::future::poll_fn(|context| {
                        match Driver::transmit(&mut net_driver, context) {
                            Some(token) => {
                                token.consume(64, |frame| {
                                    frame[..6].fill(0xff);
                                    frame[6..12]
                                        .copy_from_slice(&[0x02, 0x4b, 0x4f, 0x54, 0x4f, sequence]);
                                    frame[12..14].copy_from_slice(&[0x88, 0xb5]);
                                    frame[14..].fill(sequence);
                                });
                                Poll::Ready(())
                            }
                            None => Poll::Pending,
                        }
                    })
                    .await;
                    sequence = sequence.wrapping_add(1);
                    super::WIFI_SOAK_TX_FRAMES.fetch_add(1, Ordering::Relaxed);
                }
            };
            embassy_futures::join::join(runner.run(), control_task).await;
        }
    }

    /// KOTO-0239 product lifecycle future: brings up CYW43 in the residency
    /// arena and runs the [`crate::firmware::network`] command loop that owns
    /// `Control` and services `NetworkService` radio operations. Join it with
    /// the cyw43 runner; it never returns until cancelled with the arena.
    ///
    /// Like [`cyw43_lifecycle_future`], this uses the stock ~20 ms WL_ON reset
    /// pre-delay; the KOTO-0227 diagnostic one-second interval is retired.
    #[cfg(feature = "network_service")]
    pub fn cyw43_network_future<PioIrq, DmaTxIrq, DmaRxIrq>(
        state: &'static mut State,
        fetch_mailbox: &'static FetchTransportShared,
        tls_session_storage: &'static mut [MaybeUninit<u8>],
        resources: PicoWRadioResources,
        pio_irq: PioIrq,
        dma_tx_irq: DmaTxIrq,
        dma_rx_irq: DmaRxIrq,
    ) -> impl Future<Output = ()> + 'static
    where
        PioIrq: Binding<
                <peripherals::PIO0 as pio::Instance>::Interrupt,
                pio::InterruptHandler<peripherals::PIO0>,
            > + 'static,
        DmaTxIrq: Binding<
                <peripherals::DMA_CH2 as dma::ChannelInstance>::Interrupt,
                dma::InterruptHandler<peripherals::DMA_CH2>,
            > + 'static,
        DmaRxIrq: Binding<
                <peripherals::DMA_CH3 as dma::ChannelInstance>::Interrupt,
                dma::InterruptHandler<peripherals::DMA_CH3>,
            > + 'static,
    {
        async move {
            WIFI_LIFECYCLE_PHASE.store(WifiLifecyclePhase::Initializing as u32, Ordering::Release);
            let mut power = RadioPowerOutput::new(resources.power);
            let _ = power.set_low();
            Timer::after_millis(20).await;
            let cs = Output::new(resources.cs, Level::High);
            let mut pio = Pio::new(resources.pio, pio_irq);
            let spi = CooperativePioSpi::new(PioSpi::new(
                &mut pio.common,
                pio.sm0,
                RM2_CLOCK_DIVIDER,
                pio.irq0,
                cs,
                resources.data,
                resources.clock,
                Channel::new(resources.dma_tx, dma_tx_irq),
                Channel::new(resources.dma_rx, dma_rx_irq),
            ));
            let nvram = cyw43::aligned_bytes!("../../nvram_rp2040.bin");
            let (net_driver, control, runner) =
                cyw43::new(state, power, spi, &WIFI_FIRMWARE, nvram).await;
            WIFI_LIFECYCLE_PHASE.store(WifiLifecyclePhase::DriverReady as u32, Ordering::Release);
            // The bounded embassy-net stack lives in this future's frame (the
            // generation-owned arena). The app-facing NetDriver moves into it
            // and DHCP runs via `net_runner`; the `Stack` handle is retained for
            // the bounded application sockets a later step will create.
            let mut stack_storage = net_stack::NetworkStackStorage::new();
            let (stack, mut net_runner) = net_stack::build_network_stack(
                net_driver,
                &mut stack_storage.resources,
                net_stack::NETWORK_RANDOM_SEED,
            );
            let command_loop = crate::firmware::network::radio_command_loop(
                control,
                cyw43_firmware::CYW43_43439A0_CLM,
            );
            // Publish DHCP/link status so a driver can confirm IP config-up
            // after association. `Stack` is `Copy`; reads are brief and
            // cooperative with the runner on CPU0.
            let stack_monitor = async {
                loop {
                    let ipv4 = stack
                        .config_v4()
                        .map(|config| config.address.address().octets())
                        .unwrap_or([0; 4]);
                    crate::firmware::network::publish_dhcp_status(
                        stack.is_link_up(),
                        stack.is_config_up(),
                        ipv4,
                    );
                    Timer::after_millis(200).await;
                }
            };
            let time_client = net_stack::run_sntp_client(stack, &mut stack_storage.sntp);
            let fetch_control = net_stack::run_fetch_mailbox_control(
                stack,
                fetch_mailbox,
                tls_session_storage,
                &mut stack_storage.socket_rx[0],
                &mut stack_storage.socket_tx[0],
            );
            embassy_futures::join::join4(
                runner.run(),
                net_runner.run(),
                command_loop,
                embassy_futures::join::join3(stack_monitor, time_client, fetch_control),
            )
            .await;
        }
    }

    /// Scratch currently allocated in `cyw43::Runner::run`'s future frame.
    /// It is represented explicitly so the arena budget does not pretend the
    /// runner value alone accounts for its poll-time storage.
    pub const RUNNER_POLL_SCRATCH_BYTES: usize = 512;

    const DRIVER_STORAGE_BYTES: usize = core::mem::size_of::<State>()
        + core::mem::size_of::<PicoWRadioRunner>()
        + core::mem::size_of::<Control<'static>>()
        + core::mem::size_of::<NetDriver<'static>>()
        + RUNNER_POLL_SCRATCH_BYTES;
    const DRIVER_RESERVE_BYTES: usize = WIFI_RESIDENCY_BYTES - DRIVER_STORAGE_BYTES;

    /// Tail of the Wi-Fi future reservation dedicated to the type-erased TLS
    /// session. Keeping it out of audio scratch lets that scratch extend the
    /// crypto stack without adding static SRAM.
    #[cfg(feature = "app_fetch_https")]
    pub const TLS_SESSION_SLOT_BYTES: usize = 3 * 1024;
    #[cfg(not(feature = "app_fetch_https"))]
    pub const TLS_SESSION_SLOT_BYTES: usize = 0;
    const _: () = assert!(TLS_SESSION_SLOT_BYTES <= DRIVER_RESERVE_BYTES);

    /// Compile-time budget shape for the values returned by `cyw43::new`.
    ///
    /// The self-referential handles are never constructed as this ordinary
    /// Rust value. Runtime integration initializes equivalent slots in the raw
    /// generation-owned arena after its address is stable.
    #[repr(C, align(8))]
    pub struct WifiDriverResidencyLayout {
        state: MaybeUninit<State>,
        runner: MaybeUninit<PicoWRadioRunner>,
        control: MaybeUninit<Control<'static>>,
        net_driver: MaybeUninit<NetDriver<'static>>,
        runner_poll_scratch: [MaybeUninit<u8>; RUNNER_POLL_SCRATCH_BYTES],
        reserve: [MaybeUninit<u8>; DRIVER_RESERVE_BYTES],
    }

    pub const CYW43_STATE_BYTES: usize = core::mem::size_of::<State>();
    pub const CYW43_RUNNER_BYTES: usize = core::mem::size_of::<PicoWRadioRunner>();
    pub const CYW43_CONTROL_BYTES: usize = core::mem::size_of::<Control<'static>>();
    pub const CYW43_NET_DRIVER_BYTES: usize = core::mem::size_of::<NetDriver<'static>>();
    pub const CYW43_DRIVER_STORAGE_BYTES: usize = DRIVER_STORAGE_BYTES;
    pub const CYW43_DRIVER_RESERVE_BYTES: usize = DRIVER_RESERVE_BYTES;

    pub struct WifiResidencySlots<'arena> {
        pub state: &'arena mut MaybeUninit<State>,
        pub fetch_mailbox: &'arena mut MaybeUninit<FetchTransportShared>,
        pub future: &'arena mut [MaybeUninit<u8>],
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum WifiRuntimeError {
        InvalidArena,
        Lifecycle(WifiLifecycleError),
    }

    /// Short critical-section views over the arena-owned Fetch mailbox. Neither
    /// producer nor consumer can retain a reference across an async await.
    pub struct FetchTransportShared {
        mailbox: Mutex<RefCell<FetchTransportMailbox>>,
        tls_audio: TlsAudioExclusionCoordinator,
    }

    impl FetchTransportShared {
        pub const fn new() -> Self {
            Self {
                mailbox: Mutex::new(RefCell::new(FetchTransportMailbox::new())),
                tls_audio: TlsAudioExclusionCoordinator::new(),
            }
        }

        pub fn with<R>(&self, operation: impl FnOnce(&FetchTransportMailbox) -> R) -> R {
            critical_section::with(|cs| operation(&self.mailbox.borrow_ref(cs)))
        }

        pub fn with_mut<R>(&self, operation: impl FnOnce(&mut FetchTransportMailbox) -> R) -> R {
            critical_section::with(|cs| operation(&mut self.mailbox.borrow_ref_mut(cs)))
        }

        pub const fn tls_audio(&self) -> &TlsAudioExclusionCoordinator {
            &self.tls_audio
        }
    }

    pub trait WifiRuntimeArena {
        fn generation(&self) -> u32;
        fn bytes(&mut self) -> &mut [MaybeUninit<u8>];
    }

    impl WifiRuntimeArena for WifiResidencyArena {
        fn generation(&self) -> u32 {
            self.generation()
        }

        fn bytes(&mut self) -> &mut [MaybeUninit<u8>] {
            self.bytes()
        }
    }

    impl WifiRuntimeArena for &'static mut [MaybeUninit<u8>; WIFI_RESIDENCY_BYTES] {
        fn generation(&self) -> u32 {
            1
        }

        fn bytes(&mut self) -> &mut [MaybeUninit<u8>] {
            self.as_mut_slice()
        }
    }

    pub struct WifiRuntime<A: WifiRuntimeArena> {
        arena: Option<A>,
        lifecycle: WifiLifecycleController<'static>,
        generation: u32,
        future_bytes: usize,
        future_region_bytes: usize,
    }

    impl<A: WifiRuntimeArena> WifiRuntime<A> {
        pub fn try_new<F>(
            mut arena: A,
            build: impl FnOnce(
                &'static mut State,
                &'static FetchTransportShared,
                &'static mut [MaybeUninit<u8>],
            ) -> F,
        ) -> Result<Self, (WifiRuntimeError, A)>
        where
            F: Future<Output = ()> + 'static,
        {
            let generation = arena.generation();
            let future_bytes = core::mem::size_of::<F>();
            let Some(slots) = split_wifi_residency(arena.bytes()) else {
                return Err((WifiRuntimeError::InvalidArena, arena));
            };
            let state_pointer = slots.state as *mut MaybeUninit<State>;
            let fetch_mailbox_pointer =
                slots.fetch_mailbox as *mut MaybeUninit<FetchTransportShared>;
            let future_len = slots.future.len() - TLS_SESSION_SLOT_BYTES;
            let (future_region, tls_session_region) = slots.future.split_at_mut(future_len);
            let future_pointer = future_region.as_mut_ptr();
            let state = unsafe { (&mut *state_pointer).write(State::new()) };
            let fetch_mailbox =
                unsafe { (&mut *fetch_mailbox_pointer).write(FetchTransportShared::new()) };
            let future_storage =
                unsafe { core::slice::from_raw_parts_mut(future_pointer, future_len) };
            let tls_session_storage = unsafe {
                core::slice::from_raw_parts_mut(
                    tls_session_region.as_mut_ptr(),
                    tls_session_region.len(),
                )
            };
            let future = build(state, fetch_mailbox, tls_session_storage);
            let mut lifecycle = WifiLifecycleController::new();
            if let Err(error) = lifecycle.try_install(generation, future_storage, future) {
                return Err((WifiRuntimeError::Lifecycle(error), arena));
            }
            Ok(Self {
                arena: Some(arena),
                lifecycle,
                generation,
                future_bytes,
                future_region_bytes: future_len,
            })
        }

        pub async fn service(&mut self) {
            core::future::poll_fn(|context| {
                self.service_with_context(context);
                Poll::Ready(())
            })
            .await;
        }

        /// Poll one bounded unit from an outer cooperative loop. App execution
        /// uses this path so DHCP/DNS/TLS progress does not stop while the shell
        /// loop is suspended.
        pub fn service_with_context(&mut self, context: &mut Context<'_>) {
            self.lifecycle.service_with_context(context);
        }

        pub fn shutdown(mut self) -> Result<A, WifiRuntimeError> {
            self.lifecycle
                .cancel(self.generation)
                .map_err(WifiRuntimeError::Lifecycle)?;
            Ok(self.arena.take().expect("Wi-Fi runtime owns its arena"))
        }

        pub const fn is_active(&self) -> bool {
            self.lifecycle.is_active()
        }

        pub const fn joined_generation(&self) -> u32 {
            self.lifecycle.joined_generation()
        }

        pub const fn polls(&self) -> u32 {
            self.lifecycle.polls()
        }

        pub const fn future_bytes(&self) -> usize {
            self.future_bytes
        }

        /// Capacity of the arena region that must admit the lifecycle future.
        pub const fn future_region_bytes(&self) -> usize {
            self.future_region_bytes
        }

        /// OS-private mailbox placed inside the Wi-Fi driver reservation.
        pub fn fetch_mailbox(&mut self) -> &FetchTransportShared {
            let slots = split_wifi_residency(
                self.arena
                    .as_mut()
                    .expect("Wi-Fi runtime owns its arena")
                    .bytes(),
            )
            .expect("validated Wi-Fi arena");
            unsafe { slots.fetch_mailbox.assume_init_ref() }
        }

        pub fn tls_audio_exclusion_pending(&mut self) -> bool {
            self.fetch_mailbox().tls_audio().state() != TlsAudioExclusionState::Idle
        }

        /// CPU0 half of the RP2040 TLS/audio exclusion handshake. This method
        /// never lends the workspace to the network future; it only advances
        /// quiesce readiness and handles cancellation before ownership moves.
        #[cfg(feature = "mcu-rp2040")]
        pub fn service_tls_audio_exclusion(
            &mut self,
            audio: &mut crate::firmware::audio::PicoAudioBackend,
        ) {
            use crate::firmware::audio_residency::ResidencyState;

            let coordinator = self.fetch_mailbox().tls_audio();
            let Some(token) = coordinator.active_token() else {
                return;
            };
            match coordinator.state() {
                TlsAudioExclusionState::QuiesceRequested => {
                    if audio.begin_tls_audio_quiesce().is_ok() {
                        let _ = coordinator.transition(
                            token,
                            TlsAudioExclusionState::QuiesceRequested,
                            TlsAudioExclusionState::Quiescing,
                        );
                    } else {
                        let _ = coordinator.transition(
                            token,
                            TlsAudioExclusionState::QuiesceRequested,
                            TlsAudioExclusionState::Failed,
                        );
                    }
                }
                TlsAudioExclusionState::Quiescing => {
                    if audio.residency_state() == ResidencyState::TlsExclusive {
                        let _ = coordinator.transition(
                            token,
                            TlsAudioExclusionState::Quiescing,
                            TlsAudioExclusionState::ExclusiveReady,
                        );
                    }
                }
                TlsAudioExclusionState::CancelRequested => {
                    if audio.residency_state() == ResidencyState::TlsExclusive {
                        let restored = audio
                            .claim_tls_audio_workspace()
                            .and_then(|workspace| audio.release_tls_audio_workspace(workspace));
                        let _ = coordinator.transition(
                            token,
                            TlsAudioExclusionState::CancelRequested,
                            if restored.is_ok() {
                                TlsAudioExclusionState::Restoring
                            } else {
                                TlsAudioExclusionState::Failed
                            },
                        );
                    } else if audio.residency_state() == ResidencyState::WifiStreamAudio {
                        let _ = coordinator.transition(
                            token,
                            TlsAudioExclusionState::CancelRequested,
                            TlsAudioExclusionState::Complete,
                        );
                    }
                }
                TlsAudioExclusionState::Restoring => {
                    if audio.residency_state() == ResidencyState::WifiStreamAudio {
                        let _ = coordinator.transition(
                            token,
                            TlsAudioExclusionState::Restoring,
                            TlsAudioExclusionState::Complete,
                        );
                    }
                }
                TlsAudioExclusionState::Complete => {
                    if audio.residency_state() == ResidencyState::WifiStreamAudio {
                        let _ = coordinator.reset_terminal(token);
                    }
                }
                _ => {}
            }
        }
    }

    impl<A: WifiRuntimeArena> Drop for WifiRuntime<A> {
        fn drop(&mut self) {
            let _ = self.lifecycle.cancel(self.generation);
        }
    }

    pub fn split_wifi_residency(arena: &mut [MaybeUninit<u8>]) -> Option<WifiResidencySlots<'_>> {
        if arena.len() != WIFI_RESIDENCY_BYTES {
            return None;
        }
        let (driver, future) = arena.split_at_mut(DRIVER_STORAGE_BYTES);
        let state_pointer = driver.as_mut_ptr().cast::<MaybeUninit<State>>();
        if !(state_pointer as usize).is_multiple_of(core::mem::align_of::<State>()) {
            return None;
        }
        let mailbox_offset = align_up(
            core::mem::size_of::<State>(),
            core::mem::align_of::<FetchTransportShared>(),
        );
        let mailbox_end =
            mailbox_offset.checked_add(core::mem::size_of::<FetchTransportShared>())?;
        if mailbox_end > driver.len() {
            return None;
        }
        let mailbox_pointer = unsafe { driver.as_mut_ptr().add(mailbox_offset) }
            .cast::<MaybeUninit<FetchTransportShared>>();
        Some(WifiResidencySlots {
            state: unsafe { &mut *state_pointer },
            fetch_mailbox: unsafe { &mut *mailbox_pointer },
            future,
        })
    }

    const fn align_up(value: usize, alignment: usize) -> usize {
        (value + alignment - 1) & !(alignment - 1)
    }

    pub const FETCH_MAILBOX_BYTES: usize = core::mem::size_of::<FetchTransportShared>();
    pub const FETCH_MAILBOX_OFFSET: usize = align_up(
        core::mem::size_of::<State>(),
        core::mem::align_of::<FetchTransportShared>(),
    );
    pub const DRIVER_STORAGE_SPARE_BYTES: usize =
        DRIVER_STORAGE_BYTES - FETCH_MAILBOX_OFFSET - FETCH_MAILBOX_BYTES;

    const _: () = assert!(DRIVER_STORAGE_BYTES <= WIFI_RESIDENCY_BYTES);
    const _: () = assert!(FETCH_MAILBOX_OFFSET + FETCH_MAILBOX_BYTES <= DRIVER_STORAGE_BYTES);
    const _: () =
        assert!(core::mem::size_of::<WifiDriverResidencyLayout>() == WIFI_RESIDENCY_BYTES);
    const _: () = assert!(core::mem::align_of::<WifiDriverResidencyLayout>() >= 8);

    /// KOTO-0239 bounded embassy-net IP-stack residency.
    ///
    /// embassy-net 0.7 unifies onto this tree's embassy-time 0.5.1 and
    /// embassy-net-driver 0.2.0 (pulling smoltcp 0.12). This module fixes the
    /// KOTO-0224 socket capacities, exposes the compile-time SRAM cost of the IP
    /// stack for the residency budget report, and type-checks the integration of
    /// embassy-net against the concrete cyw43 `NetDriver` used by the runner.
    ///
    /// Placing the live stack in the generation-owned arena and running its
    /// command loop is the next KOTO-0239 step; this module fixes and measures
    /// the storage the arena must reserve.
    #[cfg(feature = "network_service")]
    pub mod net_stack {
        #[cfg(feature = "app_fetch_https")]
        use core::task::{Context, Poll};

        use super::NetDriver;
        use embassy_futures::select::{select, Either};
        use embassy_net::dns::DnsQueryType;
        use embassy_net::tcp::TcpSocket;
        use embassy_net::udp::{PacketMetadata, UdpSocket};
        use embassy_net::{Config, IpEndpoint, Runner, Stack, StackResources};
        use embassy_time::{with_timeout, Duration, Instant, Timer};
        use koto_core::{
            parse_fetch_url, release_ipv4_allowed, FetchError, FetchRequestId, FetchScheme,
            FetchTransportState, TimeFailure, TimeService, TimeServiceAction,
            MAX_FETCH_DURATION_MS, MAX_FETCH_URL_BYTES, SNTP_PACKET_BYTES, SNTP_TIMEOUT_MS,
        };

        /// Socket slots: one DHCP control, one DNS, and two bounded application
        /// sockets (KOTO-0224: one DHCP/DNS control socket and at most two
        /// application-internal TCP/UDP sockets; no public app socket API).
        pub const NETWORK_SOCKET_SLOTS: usize = 4;
        /// Bounded application TCP/UDP sockets served from the reserved stack.
        pub const NETWORK_APP_SOCKETS: usize = 2;

        /// Seed for embassy-net local-port and DHCP transaction randomization.
        /// A hardware entropy source (ROSC/RNG) should replace this fixed value
        /// before release; it does not affect the bounded storage budget.
        pub const NETWORK_RANDOM_SEED: u64 = 0x4b4f_544f_5749_4649;
        /// Per-application-socket receive-window bytes.
        pub const NETWORK_SOCKET_RX_BYTES: usize = 1536;
        /// Per-application-socket transmit-window bytes.
        pub const NETWORK_SOCKET_TX_BYTES: usize = 1536;

        /// Complete bounded IP-stack storage for the residency arena: the
        /// embassy-net stack resources plus the fixed application socket
        /// windows. Pointer-free bulk download bodies belong in PSRAM, never
        /// here (KOTO-0224 placement policy).
        #[repr(C, align(4))]
        pub struct NetworkStackStorage {
            pub(super) resources: StackResources<NETWORK_SOCKET_SLOTS>,
            pub(super) socket_rx: [[u8; NETWORK_SOCKET_RX_BYTES]; NETWORK_APP_SOCKETS],
            pub(super) socket_tx: [[u8; NETWORK_SOCKET_TX_BYTES]; NETWORK_APP_SOCKETS],
            pub(super) sntp: SntpSocketStorage,
        }

        /// The time client consumes one internal UDP slot and exactly one
        /// packet in each direction; it never borrows an application window.
        #[repr(C, align(4))]
        pub struct SntpSocketStorage {
            rx_meta: [PacketMetadata; 1],
            rx: [u8; SNTP_PACKET_BYTES],
            tx_meta: [PacketMetadata; 1],
            tx: [u8; SNTP_PACKET_BYTES],
        }

        impl NetworkStackStorage {
            /// Creates zeroed, uninitialized-resource stack storage.
            pub const fn new() -> Self {
                Self {
                    resources: StackResources::new(),
                    socket_rx: [[0; NETWORK_SOCKET_RX_BYTES]; NETWORK_APP_SOCKETS],
                    socket_tx: [[0; NETWORK_SOCKET_TX_BYTES]; NETWORK_APP_SOCKETS],
                    sntp: SntpSocketStorage {
                        rx_meta: [PacketMetadata::EMPTY; 1],
                        rx: [0; SNTP_PACKET_BYTES],
                        tx_meta: [PacketMetadata::EMPTY; 1],
                        tx: [0; SNTP_PACKET_BYTES],
                    },
                }
            }
        }

        impl Default for NetworkStackStorage {
            fn default() -> Self {
                Self::new()
            }
        }

        /// `size_of::<StackResources<NETWORK_SOCKET_SLOTS>>()`; the socket-set,
        /// DHCP, and DNS metadata cost, excluding the application windows.
        pub const NETWORK_STACK_RESOURCES_BYTES: usize =
            core::mem::size_of::<StackResources<NETWORK_SOCKET_SLOTS>>();
        /// Total bounded IP-stack SRAM: resources plus application windows.
        pub const NETWORK_STACK_STORAGE_BYTES: usize = core::mem::size_of::<NetworkStackStorage>();
        /// `embassy-net` runner-future value size (excludes its poll-time frame,
        /// which is captured at runtime like the cyw43 runner).
        pub const NETWORK_STACK_RUNNER_BYTES: usize =
            core::mem::size_of::<Runner<'static, NetDriver<'static>>>();

        /// Type-checks the embassy-net integration against the concrete cyw43
        /// `NetDriver`. Never called: the live stack is built inside the
        /// generation-owned arena command loop (next KOTO-0239 step). Keeping it
        /// compiled proves the stack links against this Embassy generation.
        pub fn build_network_stack<'d>(
            driver: NetDriver<'d>,
            resources: &'d mut StackResources<NETWORK_SOCKET_SLOTS>,
            random_seed: u64,
        ) -> (Stack<'d>, Runner<'d, NetDriver<'d>>) {
            embassy_net::new(
                driver,
                Config::dhcpv4(Default::default()),
                resources,
                random_seed,
            )
        }

        /// Arena-owned optional SNTP client. DNS, UDP, and timeout completion
        /// stay in this future and are never exposed to Shell or KotoConfig.
        pub async fn run_sntp_client(stack: Stack<'_>, storage: &mut SntpSocketStorage) {
            let mut socket = UdpSocket::new(
                stack,
                &mut storage.rx_meta,
                &mut storage.rx,
                &mut storage.tx_meta,
                &mut storage.tx,
            );
            if socket.bind(0).is_err() {
                return;
            }
            let mut service = TimeService::new();
            loop {
                let now_ms = Instant::now().as_millis();
                let config_up = stack.is_config_up();
                let mut server_bytes = [0u8; crate::firmware::network::SNTP_SERVER_MAX_BYTES];
                let (server_len, server_generation) =
                    crate::firmware::network::sntp_server(&mut server_bytes);
                service.set_network(config_up, server_generation, now_ms);
                if let TimeServiceAction::Send(request) = service.poll(now_ms) {
                    let mut response = [0u8; SNTP_PACKET_BYTES];
                    let exchange = with_timeout(Duration::from_millis(SNTP_TIMEOUT_MS), async {
                        let server = core::str::from_utf8(&server_bytes[..server_len])
                            .map_err(|_| TimeFailure::Dns)?;
                        let addresses = stack
                            .dns_query(server, DnsQueryType::A)
                            .await
                            .map_err(|_| TimeFailure::Dns)?;
                        let address = addresses.first().copied().ok_or(TimeFailure::Dns)?;
                        let endpoint = IpEndpoint::new(address, 123);
                        socket
                            .send_to(&request, endpoint)
                            .await
                            .map_err(|_| TimeFailure::CapabilityLost)?;
                        let (length, metadata) = socket
                            .recv_from(&mut response)
                            .await
                            .map_err(|_| TimeFailure::ResponseLength)?;
                        if metadata.endpoint != endpoint {
                            return Err(TimeFailure::RequestMismatch);
                        }
                        Ok(length)
                    })
                    .await;
                    let completed_ms = Instant::now().as_millis();
                    match exchange {
                        Ok(Ok(length)) => {
                            let _ = service.accept_response(&response[..length], completed_ms);
                        }
                        Ok(Err(TimeFailure::Dns)) => service.report_dns_failure(completed_ms),
                        Ok(Err(error)) => {
                            service.report_transport_failure(error, completed_ms);
                        }
                        Err(_) => {
                            let _ = service.poll(completed_ms);
                        }
                    }
                }
                let snapshot = service.snapshot(Instant::now().as_millis());
                if snapshot.valid {
                    crate::firmware::network::publish_sntp_utc(
                        snapshot.utc_seconds,
                        snapshot.generation,
                    );
                }
                Timer::after_millis(200).await;
            }
        }

        async fn wait_fetch_cancel(
            mailbox: &'static super::FetchTransportShared,
            request: FetchRequestId,
        ) {
            loop {
                if mailbox.with(|mailbox| mailbox.cancel_requested(request)) {
                    return;
                }
                Timer::after_millis(10).await;
            }
        }

        /// Association and DHCP completion are distinct boundaries. KotoConfig
        /// may publish Connected as soon as CYW43 joins the AP, while
        /// embassy-net still has no configured address. A Fetch submitted in
        /// that window waits cooperatively instead of failing `Unavailable` at
        /// zero milliseconds.
        async fn wait_stack_config(stack: Stack<'_>) {
            while !stack.is_config_up() {
                Timer::after_millis(10).await;
            }
        }

        /// Bounded storage for the encoded v1 GET head: the 384-byte URL
        /// maximum plus canonical host/port and the fixed header tail.
        #[cfg(feature = "app_fetch_https")]
        const FETCH_REQUEST_HEAD_BYTES: usize = 768;

        /// Executor-owned state that must survive DNS/TCP awaits. Decoder and
        /// plaintext staging are constructed only after TLS exclusion in audio
        /// scratch, reducing the Wi-Fi lifecycle future enough to reserve its
        /// tail for the TLS session future.
        #[cfg(feature = "app_fetch_https")]
        struct TlsFetchState {
            request_head: [u8; FETCH_REQUEST_HEAD_BYTES],
            head_len: usize,
        }

        #[cfg(feature = "app_fetch_https")]
        impl TlsFetchState {
            const fn new() -> Self {
                Self {
                    request_head: [0; FETCH_REQUEST_HEAD_BYTES],
                    head_len: 0,
                }
            }

            /// Encodes the complete GET head before DNS or any workspace
            /// claim, so an oversized request fails without touching audio
            /// residency or the network.
            fn prepare(&mut self, url: &str) -> Result<(), FetchError> {
                self.head_len = koto_core::encode_fetch_get_request(url, &mut self.request_head)?;
                Ok(())
            }
        }

        #[cfg(not(feature = "app_fetch_https"))]
        struct TlsFetchState;

        #[cfg(not(feature = "app_fetch_https"))]
        impl TlsFetchState {
            const fn new() -> Self {
                Self
            }

            fn prepare(&mut self, _url: &str) -> Result<(), FetchError> {
                Ok(())
            }
        }

        /// Quiesces RP2040 stream audio and takes exclusive ownership of the
        /// PCM workspace for one TLS connection lifetime.
        #[cfg(feature = "mcu-rp2040")]
        async fn acquire_tls_audio_workspace(
            mailbox: &'static super::FetchTransportShared,
            request: FetchRequestId,
        ) -> Result<
            (
                crate::firmware::audio::TlsAudioWorkspace,
                crate::firmware::wifi_residency::TlsAudioExclusionToken,
            ),
            FetchError,
        > {
            use crate::firmware::audio::{
                claim_shared_tls_audio_workspace, release_shared_tls_audio_workspace,
            };
            use crate::firmware::wifi_residency::TlsAudioExclusionState;

            const QUIESCE_TIMEOUT_MS: u64 = 2_000;
            let coordinator = mailbox.tls_audio();
            let token = coordinator.request().ok_or(FetchError::Busy)?;
            let wait_ready = async {
                loop {
                    if coordinator.active_token() != Some(token) {
                        return Err(FetchError::Unavailable);
                    }
                    match coordinator.state() {
                        TlsAudioExclusionState::ExclusiveReady => return Ok(()),
                        TlsAudioExclusionState::Failed => return Err(FetchError::Unavailable),
                        TlsAudioExclusionState::CancelRequested => {
                            return Err(FetchError::Cancelled);
                        }
                        _ => Timer::after_millis(10).await,
                    }
                }
            };
            match select(
                with_timeout(Duration::from_millis(QUIESCE_TIMEOUT_MS), wait_ready),
                wait_fetch_cancel(mailbox, request),
            )
            .await
            {
                Either::First(Ok(Ok(()))) => {}
                Either::First(Ok(Err(error))) => {
                    let _ = coordinator.request_cancel(token);
                    return Err(error);
                }
                Either::First(Err(_)) => {
                    let _ = coordinator.request_cancel(token);
                    return Err(FetchError::Timeout);
                }
                Either::Second(()) => {
                    let _ = coordinator.request_cancel(token);
                    return Err(FetchError::Cancelled);
                }
            }

            let workspace = claim_shared_tls_audio_workspace().map_err(|_| {
                let _ = coordinator.request_cancel(token);
                FetchError::Unavailable
            })?;
            if !coordinator.transition(
                token,
                TlsAudioExclusionState::ExclusiveReady,
                TlsAudioExclusionState::WorkspaceOwned,
            ) {
                let _ = release_shared_tls_audio_workspace(workspace);
                let _ = coordinator.request_cancel(token);
                return Err(FetchError::Unavailable);
            }
            Ok((workspace, token))
        }

        /// Returns the loan: overwrites all 8 KiB and starts stream-audio
        /// restoration regardless of the exchange outcome.
        #[cfg(feature = "mcu-rp2040")]
        fn release_tls_audio_workspace(
            mailbox: &'static super::FetchTransportShared,
            token: crate::firmware::wifi_residency::TlsAudioExclusionToken,
            workspace: crate::firmware::audio::TlsAudioWorkspace,
        ) {
            use crate::firmware::audio::release_shared_tls_audio_workspace;
            use crate::firmware::wifi_residency::TlsAudioExclusionState;

            let released = release_shared_tls_audio_workspace(workspace).is_ok();
            let _ = mailbox.tls_audio().transition(
                token,
                TlsAudioExclusionState::WorkspaceOwned,
                if released {
                    TlsAudioExclusionState::Restoring
                } else {
                    TlsAudioExclusionState::Failed
                },
            );
        }

        /// Polls the installed TLS session once on the dedicated crypto stack.
        /// The future state lives in its arena; only the transient per-poll
        /// call tree (handshake crypto) runs on `stack_top`.
        #[cfg(feature = "app_fetch_https")]
        fn poll_session_on_crypto_stack(
            installed: &mut crate::firmware::arena_future::ArenaFuture<'_>,
            context: &mut Context<'_>,
            stack_top: *mut u8,
        ) -> Poll<()> {
            let mut ready = false;
            let mut poll = || {
                ready = installed.poll_once(context).is_ready();
                ready
            };
            // SAFETY: `stack_top` is the aligned high end of the quiesced PCM
            // sample ring, exclusively ours while stream audio is quiesced;
            // `poll` runs to completion before the switch is undone.
            let _ = unsafe { crate::firmware::stack_switch::call_on_stack(stack_top, &mut poll) };
            if ready {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        }

        /// Product secure-exchange stage: the pinned TLS 1.3 session occupies
        /// exactly the workspace ownership interval. Deadline expiry and Fetch
        /// cancellation drop the session future before the socket is aborted
        /// and the workspace bytes are zeroized.
        #[cfg(all(feature = "mcu-rp2040", feature = "app_fetch_https"))]
        async fn stage_secure_exchange(
            mailbox: &'static super::FetchTransportShared,
            request: FetchRequestId,
            socket: &mut TcpSocket<'_>,
            hostname: &str,
            pins: koto_core::FetchPinSet,
            state: &mut TlsFetchState,
            session_storage: &mut [core::mem::MaybeUninit<u8>],
        ) -> Result<(), FetchError> {
            const MIN_CRYPTO_STACK_HEADROOM_BYTES: usize = 1536;
            let (mut workspace, token) = acquire_tls_audio_workspace(mailbox, request).await?;
            let mut outcome: Result<(), FetchError> = Err(FetchError::Unavailable);
            // Layout the audio scratch bottom-up: bounded RX record, HTTP
            // decoder, and plaintext staging. Its remaining tail, private
            // metadata, and the contiguous PCM samples form one extended
            // downward-growing crypto stack without increasing static SRAM.
            let scratch = unsafe { crate::firmware::audio_scratch::tls_workspace_bytes() };
            scratch.fill(0);
            let scratch_base = scratch.as_mut_ptr();
            let align_up =
                |value: usize, alignment: usize| (value + alignment - 1) & !(alignment - 1);
            let decoder_offset = align_up(
                crate::firmware::audio::TLS_RECORD_RX_BYTES,
                core::mem::align_of::<koto_core::HttpResponseDecoder>(),
            );
            let staging_offset = align_up(
                decoder_offset + core::mem::size_of::<koto_core::HttpResponseDecoder>(),
                core::mem::align_of::<crate::firmware::fetch_https::FetchTlsScratch>(),
            );
            let staging_end = staging_offset
                + core::mem::size_of::<crate::firmware::fetch_https::FetchTlsScratch>();
            let stack_base_address = align_up(scratch_base as usize + staging_end, 8);
            let stack_offset = stack_base_address - scratch_base as usize;
            if stack_offset >= scratch.len() {
                unsafe { crate::firmware::audio_scratch::restore_after_tls_stack() };
                release_tls_audio_workspace(mailbox, token, workspace);
                return Err(FetchError::Unavailable);
            }
            let record_rx = unsafe {
                core::slice::from_raw_parts_mut(
                    scratch_base,
                    crate::firmware::audio::TLS_RECORD_RX_BYTES,
                )
            };
            let decoder_pointer = unsafe {
                scratch_base
                    .add(decoder_offset)
                    .cast::<koto_core::HttpResponseDecoder>()
            };
            unsafe { decoder_pointer.write(koto_core::HttpResponseDecoder::new()) };
            let decoder = unsafe { &mut *decoder_pointer };
            let staging_pointer = unsafe {
                scratch_base
                    .add(staging_offset)
                    .cast::<crate::firmware::fetch_https::FetchTlsScratch>()
            };
            unsafe { staging_pointer.write(crate::firmware::fetch_https::FetchTlsScratch::new()) };
            let plaintext_scratch = unsafe { &mut *staging_pointer };
            let pcm = workspace.crypto_stack();
            let pcm_base = pcm.as_mut_ptr();
            let pcm_end_address = pcm_base as usize + pcm.len();
            let expected_pcm_base_address = scratch_base as usize
                + scratch.len()
                + crate::firmware::audio_scratch::TLS_SCRATCH_TRAILING_BYTES;
            if pcm_base as usize != expected_pcm_base_address {
                unsafe { crate::firmware::audio_scratch::restore_after_tls_stack() };
                release_tls_audio_workspace(mailbox, token, workspace);
                return Err(FetchError::Unavailable);
            }
            let stack_top = unsafe {
                crate::firmware::stack_switch::paint_raw_and_top(
                    stack_base_address,
                    pcm_end_address,
                )
            };
            let stack_top_address = stack_top as usize;
            let stack_capacity = stack_top_address - stack_base_address;
            let record_tx = unsafe { crate::firmware::audio::tls_record_tx_bytes() };
            // `session` borrows `outcome`; every assignment happens after this
            // statement ends and the install (with its borrow) is dropped.
            let override_error = {
                let arena_bytes = session_storage.len();
                let session = crate::firmware::fetch_https::run_pinned_https_session(
                    socket,
                    record_rx,
                    record_tx,
                    hostname,
                    &state.request_head[..state.head_len],
                    pins,
                    mailbox,
                    request,
                    decoder,
                    plaintext_scratch,
                    &mut outcome,
                );
                crate::firmware::wifi_residency::record_fetch_session_layout(
                    core::mem::size_of_val(&session),
                    arena_bytes,
                );
                match crate::firmware::arena_future::ArenaFuture::try_new(session_storage, session)
                {
                    Ok(mut installed) => {
                        let outcome_override = {
                            let drive = core::future::poll_fn(|context| {
                                poll_session_on_crypto_stack(&mut installed, context, stack_top)
                            });
                            match select(
                                with_timeout(
                                    Duration::from_millis(u64::from(MAX_FETCH_DURATION_MS)),
                                    drive,
                                ),
                                wait_fetch_cancel(mailbox, request),
                            )
                            .await
                            {
                                Either::First(Ok(())) => None,
                                Either::First(Err(_)) => Some(FetchError::Timeout),
                                Either::Second(()) => Some(FetchError::Cancelled),
                            }
                        };
                        installed.cancel();
                        outcome_override
                    }
                    Err(_) => Some(FetchError::Unavailable),
                }
            };
            if let Some(error) = override_error {
                outcome = Err(error);
            }
            crate::firmware::arena_future::zeroize_arena(session_storage);
            crate::firmware::wifi_residency::record_fetch_http_snapshot(
                decoder.status().unwrap_or(0),
                decoder.body_bytes(),
            );
            // Read the extended crypto-stack high-water before teardown
            // zeroizes the Wi-Fi session slot, audio scratch, TX ring, and PCM
            // loan (all held TLS or plaintext state).
            let stack_peak = unsafe {
                crate::firmware::stack_switch::high_water_raw(stack_base_address, stack_top_address)
            };
            crate::firmware::wifi_residency::record_fetch_crypto_stack(stack_peak, stack_capacity);
            if stack_capacity.saturating_sub(stack_peak) < MIN_CRYPTO_STACK_HEADROOM_BYTES {
                outcome = Err(FetchError::Tls);
            }
            unsafe {
                crate::firmware::audio::tls_record_tx_bytes();
                crate::firmware::audio_scratch::restore_after_tls_stack();
            }
            release_tls_audio_workspace(mailbox, token, workspace);
            outcome
        }

        /// RP2350A product path: audio keeps running while TLS uses a dedicated
        /// static workspace backed by the MCU's larger internal SRAM.
        #[cfg(all(feature = "mcu-rp235xa", feature = "app_fetch_https"))]
        async fn stage_secure_exchange(
            mailbox: &'static super::FetchTransportShared,
            request: FetchRequestId,
            socket: &mut TcpSocket<'_>,
            hostname: &str,
            pins: koto_core::FetchPinSet,
            state: &mut TlsFetchState,
            session_storage: &mut [core::mem::MaybeUninit<u8>],
        ) -> Result<(), FetchError> {
            const MIN_CRYPTO_STACK_HEADROOM_BYTES: usize = 4 * 1024;
            let mut workspace = crate::firmware::fetch_tls_workspace::Rp2350TlsWorkspace::claim()
                .ok_or(FetchError::Busy)?;
            let parts = workspace.prepare();
            let stack_top = crate::firmware::stack_switch::paint_and_top(&mut *parts.crypto_stack);
            let mut outcome: Result<(), FetchError> = Err(FetchError::Unavailable);
            let override_error = {
                let arena_bytes = session_storage.len();
                let session = crate::firmware::fetch_https::run_pinned_https_session(
                    socket,
                    parts.record_rx,
                    parts.record_tx,
                    hostname,
                    &state.request_head[..state.head_len],
                    pins,
                    mailbox,
                    request,
                    parts.decoder,
                    parts.plaintext,
                    &mut outcome,
                );
                crate::firmware::wifi_residency::record_fetch_session_layout(
                    core::mem::size_of_val(&session),
                    arena_bytes,
                );
                match crate::firmware::arena_future::ArenaFuture::try_new(session_storage, session)
                {
                    Ok(mut installed) => {
                        let outcome_override = {
                            let drive = core::future::poll_fn(|context| {
                                poll_session_on_crypto_stack(&mut installed, context, stack_top)
                            });
                            match select(
                                with_timeout(
                                    Duration::from_millis(u64::from(MAX_FETCH_DURATION_MS)),
                                    drive,
                                ),
                                wait_fetch_cancel(mailbox, request),
                            )
                            .await
                            {
                                Either::First(Ok(())) => None,
                                Either::First(Err(_)) => Some(FetchError::Timeout),
                                Either::Second(()) => Some(FetchError::Cancelled),
                            }
                        };
                        installed.cancel();
                        outcome_override
                    }
                    Err(_) => Some(FetchError::Unavailable),
                }
            };
            if let Some(error) = override_error {
                outcome = Err(error);
            }
            crate::firmware::arena_future::zeroize_arena(session_storage);
            crate::firmware::wifi_residency::record_fetch_http_snapshot(
                parts.decoder.status().unwrap_or(0),
                parts.decoder.body_bytes(),
            );
            let stack_peak = crate::firmware::stack_switch::high_water(parts.crypto_stack);
            let stack_capacity = parts.crypto_stack.len();
            crate::firmware::wifi_residency::record_fetch_crypto_stack(stack_peak, stack_capacity);
            if stack_capacity.saturating_sub(stack_peak) < MIN_CRYPTO_STACK_HEADROOM_BYTES {
                outcome = Err(FetchError::Tls);
            }
            // `workspace` volatile-zeroizes RX/TX, decoder, plaintext, and the
            // crypto stack after all field borrows end at this return boundary.
            outcome
        }

        /// Pre-transport placeholder: proves the ownership transfer, then
        /// immediately returns and zeroizes the loan (product TLS unlinked).
        #[cfg(all(feature = "mcu-rp2040", not(feature = "app_fetch_https")))]
        async fn stage_secure_exchange(
            mailbox: &'static super::FetchTransportShared,
            request: FetchRequestId,
            _socket: &mut TcpSocket<'_>,
            _hostname: &str,
            _pins: koto_core::FetchPinSet,
            _state: &mut TlsFetchState,
            _session_storage: &mut [core::mem::MaybeUninit<u8>],
        ) -> Result<(), FetchError> {
            let (workspace, token) = acquire_tls_audio_workspace(mailbox, request).await?;
            release_tls_audio_workspace(mailbox, token, workspace);
            Err(FetchError::Unavailable)
        }

        #[cfg(all(feature = "mcu-rp235xa", not(feature = "app_fetch_https")))]
        async fn stage_secure_exchange(
            _: &'static super::FetchTransportShared,
            _: FetchRequestId,
            _: &mut TcpSocket<'_>,
            _: &str,
            _: koto_core::FetchPinSet,
            _: &mut TlsFetchState,
            _: &mut [core::mem::MaybeUninit<u8>],
        ) -> Result<(), FetchError> {
            Err(FetchError::Unavailable)
        }

        fn finish_fetch_preflight(
            mailbox: &super::FetchTransportShared,
            request: FetchRequestId,
            result: Result<(), FetchError>,
            elapsed_ms: u32,
        ) {
            let outcome = mailbox.with_mut(|mailbox| {
                if mailbox.cancel_requested(request) {
                    let _ = mailbox.acknowledge_cancel(request);
                    Err(FetchError::Cancelled)
                } else {
                    if let Err(error) = result {
                        let _ = mailbox.fail(request, error);
                    }
                    result
                }
            });
            crate::firmware::wifi_residency::record_fetch_terminal(
                request.raw(),
                outcome.err(),
                elapsed_ms,
            );
        }

        /// Network-future side of the synchronized mailbox lifecycle. This
        /// stage owns URL copying, cancellable DNS, release destination
        /// filtering, TCP connect, and — with `app_fetch_https` — the
        /// board-specific pinned TLS session. No socket or TLS object leaves
        /// this future.
        pub async fn run_fetch_mailbox_control(
            stack: Stack<'_>,
            mailbox: &'static super::FetchTransportShared,
            tls_session_storage: &'static mut [core::mem::MaybeUninit<u8>],
            socket_rx: &mut [u8],
            socket_tx: &mut [u8],
        ) {
            let mut url = [0u8; MAX_FETCH_URL_BYTES];
            let mut tls_state = TlsFetchState::new();
            loop {
                let command = mailbox.with_mut(|mailbox| {
                    if mailbox.state() == FetchTransportState::CancelRequested {
                        if let Some(request) = mailbox.active_request() {
                            let _ = mailbox.acknowledge_cancel(request);
                        }
                        return Ok(None);
                    }
                    mailbox.take_command(&mut url)
                });
                let command = match command {
                    Ok(Some(command)) => command,
                    Ok(None) => {
                        Timer::after_millis(10).await;
                        continue;
                    }
                    Err(error) => {
                        if let Some(request) = mailbox.with(|mailbox| mailbox.active_request()) {
                            finish_fetch_preflight(mailbox, request, Err(error), 0);
                        }
                        continue;
                    }
                };
                let started = Instant::now();
                crate::firmware::wifi_residency::record_fetch_command_started();
                let elapsed_of = |started: Instant| started.elapsed().as_millis() as u32;
                let url_len = usize::from(command.url_len);
                let Some(url_str) = core::str::from_utf8(&url[..url_len]).ok() else {
                    finish_fetch_preflight(
                        mailbox,
                        command.request,
                        Err(FetchError::MalformedUrl),
                        elapsed_of(started),
                    );
                    continue;
                };
                let target = match parse_fetch_url(url_str) {
                    Ok(target) => target,
                    Err(_) => {
                        finish_fetch_preflight(
                            mailbox,
                            command.request,
                            Err(FetchError::MalformedUrl),
                            elapsed_of(started),
                        );
                        continue;
                    }
                };
                if target.scheme() != FetchScheme::Https {
                    finish_fetch_preflight(
                        mailbox,
                        command.request,
                        Err(FetchError::Denied),
                        elapsed_of(started),
                    );
                    continue;
                }
                if command.pins.is_empty() {
                    finish_fetch_preflight(
                        mailbox,
                        command.request,
                        Err(FetchError::Tls),
                        elapsed_of(started),
                    );
                    continue;
                }
                if let Err(error) = tls_state.prepare(url_str) {
                    finish_fetch_preflight(
                        mailbox,
                        command.request,
                        Err(error),
                        elapsed_of(started),
                    );
                    continue;
                }
                if !stack.is_config_up() {
                    match select(
                        with_timeout(
                            Duration::from_millis(u64::from(MAX_FETCH_DURATION_MS)),
                            wait_stack_config(stack),
                        ),
                        wait_fetch_cancel(mailbox, command.request),
                    )
                    .await
                    {
                        Either::First(Ok(())) => {}
                        Either::First(Err(_)) => {
                            finish_fetch_preflight(
                                mailbox,
                                command.request,
                                Err(FetchError::Timeout),
                                elapsed_of(started),
                            );
                            continue;
                        }
                        Either::Second(()) => {
                            finish_fetch_preflight(
                                mailbox,
                                command.request,
                                Ok(()),
                                elapsed_of(started),
                            );
                            continue;
                        }
                    }
                }

                let dns = with_timeout(
                    Duration::from_millis(u64::from(MAX_FETCH_DURATION_MS)),
                    stack.dns_query(target.hostname(), DnsQueryType::A),
                );
                let addresses = match select(dns, wait_fetch_cancel(mailbox, command.request)).await
                {
                    Either::First(Ok(Ok(addresses))) => addresses,
                    Either::First(Ok(Err(_))) => {
                        finish_fetch_preflight(
                            mailbox,
                            command.request,
                            Err(FetchError::Dns),
                            elapsed_of(started),
                        );
                        continue;
                    }
                    Either::First(Err(_)) => {
                        finish_fetch_preflight(
                            mailbox,
                            command.request,
                            Err(FetchError::Timeout),
                            elapsed_of(started),
                        );
                        continue;
                    }
                    Either::Second(()) => {
                        finish_fetch_preflight(
                            mailbox,
                            command.request,
                            Ok(()),
                            elapsed_of(started),
                        );
                        continue;
                    }
                };
                if addresses.is_empty() {
                    finish_fetch_preflight(
                        mailbox,
                        command.request,
                        Err(FetchError::Dns),
                        elapsed_of(started),
                    );
                    continue;
                }
                let all_public = addresses.iter().all(|address| match address {
                    embassy_net::IpAddress::Ipv4(address) => release_ipv4_allowed(address.octets()),
                });
                if !all_public {
                    finish_fetch_preflight(
                        mailbox,
                        command.request,
                        Err(FetchError::ForbiddenAddress),
                        elapsed_of(started),
                    );
                    continue;
                }
                let address = addresses[0];
                // Re-run the release predicate immediately before connect.
                // Later DNS refresh/retry paths must pass through this same
                // point rather than retaining a previously trusted address.
                let address_public = match address {
                    embassy_net::IpAddress::Ipv4(address) => release_ipv4_allowed(address.octets()),
                };
                if !address_public {
                    finish_fetch_preflight(
                        mailbox,
                        command.request,
                        Err(FetchError::ForbiddenAddress),
                        elapsed_of(started),
                    );
                    continue;
                }
                match address {
                    embassy_net::IpAddress::Ipv4(address) => {
                        crate::firmware::wifi_residency::record_fetch_resolved_ip(address.octets());
                    }
                }

                let endpoint = IpEndpoint::new(address, target.port());
                let connect_result = {
                    let mut socket = TcpSocket::new(stack, &mut *socket_rx, &mut *socket_tx);
                    socket.set_timeout(Some(Duration::from_millis(u64::from(
                        MAX_FETCH_DURATION_MS,
                    ))));
                    // The connect select ends (releasing its socket borrow)
                    // before the secure-exchange stage reborrows the socket.
                    let connected = {
                        let connect = with_timeout(
                            Duration::from_millis(u64::from(MAX_FETCH_DURATION_MS)),
                            socket.connect(endpoint),
                        );
                        match select(connect, wait_fetch_cancel(mailbox, command.request)).await {
                            Either::First(Ok(Ok(()))) => Ok(()),
                            Either::First(Ok(Err(error))) => {
                                // embassy-net ConnectError discriminant, so a
                                // Connect failure distinguishes refused/reset
                                // from no-route/invalid-state.
                                let code = match error {
                                    embassy_net::tcp::ConnectError::InvalidState => 1,
                                    embassy_net::tcp::ConnectError::ConnectionReset => 2,
                                    embassy_net::tcp::ConnectError::TimedOut => 3,
                                    embassy_net::tcp::ConnectError::NoRoute => 4,
                                };
                                crate::firmware::wifi_residency::record_fetch_connect_error(code);
                                Err(FetchError::Connect)
                            }
                            Either::First(Err(_)) => Err(FetchError::Timeout),
                            Either::Second(()) => Err(FetchError::Cancelled),
                        }
                    };
                    let result = match connected {
                        Ok(()) => {
                            stage_secure_exchange(
                                mailbox,
                                command.request,
                                &mut socket,
                                target.hostname(),
                                command.pins,
                                &mut tls_state,
                                &mut *tls_session_storage,
                            )
                            .await
                        }
                        Err(error) => Err(error),
                    };
                    socket.abort();
                    let _ = with_timeout(Duration::from_millis(1_000), socket.flush()).await;
                    result
                };
                finish_fetch_preflight(
                    mailbox,
                    command.request,
                    connect_result,
                    elapsed_of(started),
                );
            }
        }
    }
}

pub const WIFI_RESIDENCY_BYTES: usize = 36 * 1024;

#[cfg(any(feature = "board-picocalc-picow", feature = "board-picocalc-pico2w"))]
pub use w_board::*;

#[cfg(test)]
mod tests {
    use super::{TlsAudioExclusionCoordinator, TlsAudioExclusionState};

    #[test]
    fn tls_audio_coordinator_rejects_stale_and_out_of_order_acknowledgements() {
        let coordinator = TlsAudioExclusionCoordinator::new();
        let first = coordinator.request().unwrap();
        assert_eq!(
            coordinator.state(),
            TlsAudioExclusionState::QuiesceRequested
        );
        assert!(coordinator.request().is_none());
        assert!(coordinator.transition(
            first,
            TlsAudioExclusionState::QuiesceRequested,
            TlsAudioExclusionState::Quiescing,
        ));
        assert!(!coordinator.transition(
            first,
            TlsAudioExclusionState::ExclusiveReady,
            TlsAudioExclusionState::WorkspaceOwned,
        ));
        assert!(coordinator.transition(
            first,
            TlsAudioExclusionState::Quiescing,
            TlsAudioExclusionState::Failed,
        ));
        assert!(coordinator.reset_terminal(first));

        let second = coordinator.request().unwrap();
        assert_ne!(first, second);
        assert!(!coordinator.transition(
            first,
            TlsAudioExclusionState::QuiesceRequested,
            TlsAudioExclusionState::Quiescing,
        ));
        assert!(coordinator.transition(
            second,
            TlsAudioExclusionState::QuiesceRequested,
            TlsAudioExclusionState::CancelRequested,
        ));
    }
}
