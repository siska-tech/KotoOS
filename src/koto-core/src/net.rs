//! Portable bounded network service (KOTO-0239).
//!
//! This module implements the OS-owned `NetworkService` frozen by KOTO-0224 in
//! [`docs/architecture/KOTOCONFIG_WIFI_EXTENSION.md`]. It is `no_std`, uses no
//! general heap, and owns only fixed-capacity storage. KotoConfig and ordinary
//! applications talk to this contract and never see CYW43, Embassy tasks, an IP
//! stack, sockets, credentials, or board identity.
//!
//! The service is poll-driven. Submission methods validate and enqueue work,
//! returning a [`SubmitResult`] without waiting for radio I/O. [`NetworkService::service`]
//! is called outside painting with a bounded work budget; it owns deadlines,
//! connection lifetime, and the connect retry policy. The concrete radio and
//! secret storage are supplied through the [`WifiHal`] and [`CredentialProvider`]
//! traits so the same logic runs against a fake on host and the real CYW43 /
//! Embassy stack on device.
//!
//! Time is expressed in milliseconds. The `now_ms` argument to
//! [`NetworkService::service`] is a monotonic millisecond clock supplied by the
//! embedder (the KOTO-0224 contract names it `now_ticks`; the unit is fixed here
//! so deadlines are portable).

// ------------------------------------------------------------------ capacities

/// Maximum SSID length in octets (802.11 allows arbitrary bytes).
pub const SSID_MAX_BYTES: usize = 32;
/// BSSID length in octets.
pub const BSSID_BYTES: usize = 6;
/// Maximum retained scan results after dedup and sort.
pub const SCAN_RESULTS_MAX: usize = 16;
/// Maximum WPA2 passphrase length in octets.
pub const CREDENTIAL_MAX_BYTES: usize = 63;
/// Minimum WPA2 passphrase length in octets.
pub const CREDENTIAL_MIN_BYTES: usize = 8;
/// Retained credential profiles owned by the credential provider.
pub const RETAINED_PROFILES_MAX: usize = 4;
/// Redacted status-history depth.
pub const STATUS_HISTORY_MAX: usize = 8;
/// Fixed command FIFO depth; overflow returns [`SubmitResult::Busy`].
pub const COMMAND_QUEUE_MAX: usize = 4;
/// Fixed event FIFO depth carrying request id and state/error only.
pub const EVENT_QUEUE_MAX: usize = 8;
/// Upper bound on raw scan entries ingested from the HAL per scan.
const RAW_SCAN_MAX: usize = 32;

// -------------------------------------------------------------------- deadlines

/// Radio enable/disable deadline (10 s). No automatic retry.
pub const RADIO_DEADLINE_MS: u64 = 10_000;
/// Scan deadline (15 s). No automatic retry.
pub const SCAN_DEADLINE_MS: u64 = 15_000;
/// Connect deadline (30 s). Bounded association retry inside.
pub const CONNECT_DEADLINE_MS: u64 = 30_000;
/// Disconnect deadline (5 s). Timeout forces the owner toward offline.
pub const DISCONNECT_DEADLINE_MS: u64 = 5_000;
/// Forget deadline (5 s). Success requires a commit acknowledgement.
pub const FORGET_DEADLINE_MS: u64 = 5_000;
/// Association retry delays after a transient connect failure.
const CONNECT_RETRY_DELAYS_MS: [u64; 2] = [1_000, 2_000];
/// Maximum association retries for transient radio/link errors.
const CONNECT_RETRY_MAX: u8 = 2;

// ------------------------------------------------------------------- basic enums

/// A regulatory domain (ISO 3166-1 alpha-2, or `XX` worldwide) selected as a
/// signed release/product policy for the destination market (KOTO-0224). It is
/// never a KotoConfig field, a credential attribute, an SSID-derived value, or a
/// user override. An absent or unsupported policy must prevent radio
/// initialization (keeping `WIFI_CONFIG` false) without blocking offline boot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RegulatoryRegion {
    code: [u8; 2],
}

/// Why a regulatory policy did not resolve to a usable region.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegionError {
    /// No region policy was provided by the release/product build.
    Absent,
    /// The provided code is not in the supported set.
    Unsupported,
}

impl RegulatoryRegion {
    /// Worldwide/unrestricted default (matches the cyw43 `WORLD_WIDE_XX` domain).
    pub const WORLDWIDE: Self = Self { code: *b"XX" };
    pub const JAPAN: Self = Self { code: *b"JP" };
    pub const UNITED_STATES: Self = Self { code: *b"US" };

    const SUPPORTED: [[u8; 2]; 3] = [*b"XX", *b"JP", *b"US"];

    /// Validates a raw release region policy. `None` (absent) and unsupported
    /// codes are rejected; radio initialization must not proceed for either.
    pub fn resolve(policy: Option<[u8; 2]>) -> Result<Self, RegionError> {
        let code = policy.ok_or(RegionError::Absent)?;
        if Self::SUPPORTED.contains(&code) {
            Ok(Self { code })
        } else {
            Err(RegionError::Unsupported)
        }
    }

    /// Whether a release region policy permits radio initialization. Absent or
    /// unsupported policies never permit it.
    pub fn permits_radio_enable(policy: Option<[u8; 2]>) -> bool {
        Self::resolve(policy).is_ok()
    }

    pub const fn code(&self) -> [u8; 2] {
        self.code
    }

    /// The two-letter region code as text (`"XX"`, `"JP"`, `"US"`).
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.code).unwrap_or("??")
    }
}

/// Supported 802.11 security modes. WEP, enterprise/EAP, WPA1, WPA3/SAE, WPS,
/// and captive-portal setup are out of scope for v1.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Security {
    Open,
    Wpa2PersonalAes,
}

/// Radio power state reported in the snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RadioState {
    Disabled,
    Enabling,
    Enabled,
    Disabling,
    Unavailable,
}

/// Overall operation state. Names align with the KOTO-0224 fixture strings so
/// the KotoSim fake service (KOTO-0242) and the `network.wifi` page (KOTO-0241)
/// can map to the same vocabulary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationState {
    RadioUnavailable,
    Disabled,
    RadioEnabling,
    RadioDisabling,
    Scanning,
    Results,
    Connecting,
    Connected,
    Disconnecting,
    Forgetting,
    Failed,
}

/// The single active operation kind. Operations are serialized; only one is in
/// flight and the rest wait in the command queue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ActiveKind {
    RadioEnable,
    RadioDisable,
    Scan,
    Connect,
    Disconnect,
    Forget,
}

/// Fixed redacted error enum. Values carry no driver strings, SSIDs,
/// credentials, keys, packet bytes, or addresses.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetworkError {
    Busy,
    InvalidInput,
    UnsupportedSecurity,
    RadioUnavailable,
    FirmwareUnavailable,
    CredentialStoreUnavailable,
    AuthenticationFailed,
    NetworkNotFound,
    LinkLost,
    Timeout,
    Cancelled,
    StorageCorrupt,
    Internal,
}

/// Result of a submission method. Returned immediately without radio I/O.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubmitResult {
    Accepted(RequestId),
    Busy,
    InvalidInput,
    Unavailable,
    StaleGeneration,
}

/// Monotonically wrapping, nonzero request id.
pub type RequestId = u32;
/// Lifecycle generation token. Advances on radio/capability loss and re-init.
pub type Generation = u32;

// -------------------------------------------------------------------- data types

/// An SSID stored as raw octets plus an explicit length. Invalid UTF-8 must be
/// displayed escaped by the UI, never treated as a C string.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Ssid {
    bytes: [u8; SSID_MAX_BYTES],
    len: u8,
}

impl Ssid {
    /// An empty SSID (zero length). Used as a placeholder for inactive staging.
    pub const EMPTY: Self = Self {
        bytes: [0; SSID_MAX_BYTES],
        len: 0,
    };

    /// Builds an SSID from raw bytes, truncating to [`SSID_MAX_BYTES`].
    pub fn from_bytes(src: &[u8]) -> Self {
        let mut bytes = [0u8; SSID_MAX_BYTES];
        let len = src.len().min(SSID_MAX_BYTES);
        bytes[..len].copy_from_slice(&src[..len]);
        Self {
            bytes,
            len: len as u8,
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..usize::from(self.len)]
    }

    pub fn len(&self) -> usize {
        usize::from(self.len)
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// A raw scan observation supplied by the HAL before dedup/sort.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RawScanResult {
    pub ssid: Ssid,
    pub bssid: [u8; BSSID_BYTES],
    pub rssi_dbm: i8,
    pub security: Security,
}

/// A retained, deduplicated scan result with a stable identity for connect.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScanResult {
    pub result_id: u16,
    pub ssid: Ssid,
    pub bssid: [u8; BSSID_BYTES],
    pub rssi_dbm: i8,
    pub security: Security,
}

/// A borrowed, operation-scoped credential view passed to `connect`. The
/// service copies it into fixed staging and zeroizes that staging at every
/// terminal boundary; it never retains a heap copy.
#[derive(Clone, Copy)]
pub struct CredentialView<'a> {
    pub security: Security,
    pub secret: &'a [u8],
}

/// An event drained from the fixed event FIFO. Carries only a request id and a
/// redacted state/error, never SSID or credential bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetworkEvent {
    pub request_id: RequestId,
    pub state: OperationState,
    pub error: Option<NetworkError>,
}

/// The public snapshot copied by KotoConfig at most once per frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetworkSnapshot {
    pub generation: Generation,
    pub request_id: RequestId,
    pub radio: RadioState,
    pub state: OperationState,
    pub connected_result_id: Option<u16>,
    pub result_count: u8,
    pub retry_count: u8,
    pub deadline_ms_remaining: u64,
    pub last_error: Option<NetworkError>,
    pub command_depth: u8,
    pub event_depth: u8,
}

/// Progress returned by [`NetworkService::service`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServiceProgress {
    /// Whether the observable snapshot changed during this call.
    pub changed: bool,
    /// Whether an operation is still active after this call.
    pub op_active: bool,
    /// Number of events waiting to be drained.
    pub events_pending: u8,
}

// ----------------------------------------------------------------------- HAL

/// Non-blocking poll result from a radio operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HalPoll {
    Pending,
    Ready,
    /// The available scan-result count is reported through `Ready`; call
    /// [`WifiHal::scan_result`] to read each entry.
    ReadyCount(u8),
    Failed(HalFault),
}

/// Fixed radio fault taxonomy the HAL reports. The service maps these to
/// [`NetworkError`] and decides retry eligibility.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HalFault {
    /// Authentication rejected. Never auto-retried.
    Auth,
    /// The requested network was not found. Never auto-retried.
    NotFound,
    /// Association/link dropped. Eligible for bounded connect retry.
    LinkLost,
    /// Generic transient radio error. Eligible for bounded connect retry.
    Transient,
    /// Radio powered down or hardware/capability lost.
    RadioLost,
    /// Radio firmware unavailable.
    Firmware,
    /// Unexpected internal driver failure.
    Internal,
}

impl HalFault {
    fn to_error(self) -> NetworkError {
        match self {
            HalFault::Auth => NetworkError::AuthenticationFailed,
            HalFault::NotFound => NetworkError::NetworkNotFound,
            HalFault::LinkLost => NetworkError::LinkLost,
            HalFault::Transient => NetworkError::Internal,
            HalFault::RadioLost => NetworkError::RadioUnavailable,
            HalFault::Firmware => NetworkError::FirmwareUnavailable,
            HalFault::Internal => NetworkError::Internal,
        }
    }

    fn is_retryable(self) -> bool {
        matches!(self, HalFault::LinkLost | HalFault::Transient)
    }
}

/// The concrete radio boundary. Every method is non-blocking: `begin_*` starts
/// an operation and `poll_*` advances it. The service calls exactly one
/// operation family at a time and never blocks in `service`.
pub trait WifiHal {
    /// Whether radio hardware/firmware is present and usable this instant.
    /// A transition to `false` while enabled forces `RadioUnavailable`.
    fn radio_present(&self) -> bool;

    fn begin_set_radio(&mut self, enabled: bool);
    fn poll_set_radio(&mut self) -> HalPoll;

    fn begin_scan(&mut self);
    /// Returns [`HalPoll::ReadyCount`] with the number of raw results available.
    fn poll_scan(&mut self) -> HalPoll;
    /// Reads a raw scan result by index in `0..count`.
    fn scan_result(&self, index: u8) -> RawScanResult;

    /// Begins association. `ssid` is the network name a driver joins by; `bssid`
    /// identifies the chosen result for drivers that pin it. `secret` is the
    /// operation-scoped credential (empty for `Open`).
    fn begin_connect(
        &mut self,
        ssid: &Ssid,
        bssid: &[u8; BSSID_BYTES],
        security: Security,
        secret: &[u8],
    );
    fn poll_connect(&mut self) -> HalPoll;

    fn begin_disconnect(&mut self);
    fn poll_disconnect(&mut self) -> HalPoll;

    /// Requests cancellation of any in-flight radio operation. Idempotent.
    fn cancel(&mut self);
}

/// Bounded, redacted result of a credential-provider commit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ForgetOutcome {
    Committed,
    StoreUnavailable,
    Corrupt,
}

/// The secret-storage boundary. The service never sees stored bytes; it only
/// asks whether the provider is available and requests a bounded forget commit.
pub trait CredentialProvider {
    /// Whether the credential provider initialized its secret namespace.
    fn available(&self) -> bool;
    /// Commits erasure of a retained profile and acknowledges the result.
    fn forget(&mut self, profile_id: u16) -> ForgetOutcome;
}

// ------------------------------------------------------------- internal active op

/// State of the single in-flight operation.
struct ActiveOp {
    kind: ActiveKind,
    request_id: RequestId,
    /// Absolute deadline, set on the first `service` advance from the real
    /// clock. Zero until the transport is kicked.
    deadline_ms: u64,
    /// Whether the HAL `begin_*` transport has been started. Submission never
    /// performs radio I/O, so the first `service` advance kicks the transport.
    kicked: bool,
    /// Connect-only: number of association retries already spent.
    retry_count: u8,
    /// Connect-only: absolute time of the next scheduled retry begin, if any.
    retry_at_ms: Option<u64>,
    /// Connect-only: chosen target from the retained results.
    connect_target: Option<usize>,
}

/// A queued submission waiting behind the active operation.
#[derive(Clone, Copy)]
struct QueuedCommand {
    kind: ActiveKind,
    request_id: RequestId,
    /// Connect-only: index into retained results.
    connect_target: usize,
}

// ------------------------------------------------------------- ring buffers

struct EventQueue {
    slots: [Option<NetworkEvent>; EVENT_QUEUE_MAX],
    head: usize,
    len: usize,
}

impl EventQueue {
    const fn new() -> Self {
        Self {
            slots: [None; EVENT_QUEUE_MAX],
            head: 0,
            len: 0,
        }
    }

    /// Pushes an event, dropping the oldest when full (bounded FIFO).
    fn push(&mut self, event: NetworkEvent) {
        let tail = (self.head + self.len) % EVENT_QUEUE_MAX;
        if self.len == EVENT_QUEUE_MAX {
            // Overwrite oldest.
            self.slots[self.head] = Some(event);
            self.head = (self.head + 1) % EVENT_QUEUE_MAX;
        } else {
            self.slots[tail] = Some(event);
            self.len += 1;
        }
    }

    fn pop(&mut self) -> Option<NetworkEvent> {
        if self.len == 0 {
            return None;
        }
        let event = self.slots[self.head].take();
        self.head = (self.head + 1) % EVENT_QUEUE_MAX;
        self.len -= 1;
        event
    }

    fn depth(&self) -> u8 {
        self.len as u8
    }
}

// ------------------------------------------------------------- NetworkService

/// The bounded, poll-driven network service.
pub struct NetworkService {
    generation: Generation,
    next_request: RequestId,
    radio: RadioState,
    state: OperationState,
    last_error: Option<NetworkError>,
    connected_result_id: Option<u16>,

    results: [Option<ScanResult>; SCAN_RESULTS_MAX],
    result_count: usize,

    active: Option<ActiveOp>,
    commands: [Option<QueuedCommand>; COMMAND_QUEUE_MAX],
    command_head: usize,
    command_len: usize,

    events: EventQueue,
    status_history: [Option<NetworkError>; STATUS_HISTORY_MAX],
    status_head: usize,
    status_len: usize,

    /// Fixed connect credential staging, zeroized at every terminal boundary.
    staging: [u8; CREDENTIAL_MAX_BYTES],
    staging_len: u8,
    staging_security: Security,

    /// Most recent `service` clock, used to report the remaining deadline.
    last_now_ms: u64,

    /// Test/diagnostic: set true after a completed forget commit.
    last_forget_committed: bool,
    /// Test/diagnostic: incremented when a late completion is discarded.
    late_completions_discarded: u32,
}

impl NetworkService {
    /// Constructs a service starting at generation 1 with the radio disabled and
    /// no results. Radio availability is decided by the HAL at `service` time.
    pub fn new() -> Self {
        Self {
            generation: 1,
            next_request: 1,
            radio: RadioState::Disabled,
            state: OperationState::Disabled,
            last_error: None,
            connected_result_id: None,
            results: [None; SCAN_RESULTS_MAX],
            result_count: 0,
            active: None,
            commands: [None; COMMAND_QUEUE_MAX],
            command_head: 0,
            command_len: 0,
            events: EventQueue::new(),
            status_history: [None; STATUS_HISTORY_MAX],
            status_head: 0,
            status_len: 0,
            staging: [0; CREDENTIAL_MAX_BYTES],
            staging_len: 0,
            staging_security: Security::Open,
            last_now_ms: 0,
            last_forget_committed: false,
            late_completions_discarded: 0,
        }
    }

    // ------------------------------------------------------------- observation

    pub fn generation(&self) -> Generation {
        self.generation
    }

    /// Copies the public snapshot. Cheap; safe to call once per frame.
    pub fn snapshot(&self) -> NetworkSnapshot {
        let (request_id, retry_count, deadline_remaining) = match &self.active {
            Some(op) if op.kicked => (
                op.request_id,
                op.retry_count,
                op.deadline_ms.saturating_sub(self.last_now_ms),
            ),
            Some(op) => (op.request_id, op.retry_count, deadline_for(op.kind)),
            None => (0, 0, 0),
        };
        NetworkSnapshot {
            generation: self.generation,
            request_id,
            radio: self.radio,
            state: self.state,
            connected_result_id: self.connected_result_id,
            result_count: self.result_count as u8,
            retry_count,
            deadline_ms_remaining: deadline_remaining,
            last_error: self.last_error,
            command_depth: self.command_len as u8,
            event_depth: self.events.depth(),
        }
    }

    /// Drains the next event from the fixed event FIFO.
    pub fn poll_event(&mut self) -> Option<NetworkEvent> {
        self.events.pop()
    }

    /// Retained scan results after dedup and sort.
    pub fn results(&self) -> impl Iterator<Item = &ScanResult> {
        self.results[..self.result_count]
            .iter()
            .filter_map(Option::as_ref)
    }

    /// Test/diagnostic: whether the connect credential staging is fully zeroed.
    pub fn credential_staging_zeroized(&self) -> bool {
        self.staging_len == 0 && self.staging.iter().all(|&b| b == 0)
    }

    /// Test/diagnostic: whether the most recent forget committed erasure.
    pub fn last_forget_committed(&self) -> bool {
        self.last_forget_committed
    }

    /// Test/diagnostic: number of late driver completions discarded.
    pub fn late_completions_discarded(&self) -> u32 {
        self.late_completions_discarded
    }

    // ------------------------------------------------------------- submissions

    fn next_request_id(&mut self) -> RequestId {
        let id = self.next_request;
        // Monotonic, wrapping, nonzero.
        self.next_request = self.next_request.wrapping_add(1);
        if self.next_request == 0 {
            self.next_request = 1;
        }
        id
    }

    fn radio_usable(&self) -> bool {
        !matches!(self.radio, RadioState::Unavailable)
    }

    /// Enqueues an operation, or starts nothing when one is already active. The
    /// caller must ensure the operation is valid for the current radio state.
    fn submit(&mut self, kind: ActiveKind, connect_target: usize) -> SubmitResult {
        let request_id = self.next_request_id();
        if self.active.is_none() && self.command_len == 0 {
            self.begin(kind, request_id, connect_target);
            return SubmitResult::Accepted(request_id);
        }
        if self.command_len == COMMAND_QUEUE_MAX {
            return SubmitResult::Busy;
        }
        let tail = (self.command_head + self.command_len) % COMMAND_QUEUE_MAX;
        self.commands[tail] = Some(QueuedCommand {
            kind,
            request_id,
            connect_target,
        });
        self.command_len += 1;
        SubmitResult::Accepted(request_id)
    }

    /// Requests radio power. Deadline 10 s, no auto retry.
    pub fn set_radio(&mut self, enabled: bool) -> SubmitResult {
        if !self.radio_usable() {
            return SubmitResult::Unavailable;
        }
        let kind = if enabled {
            ActiveKind::RadioEnable
        } else {
            ActiveKind::RadioDisable
        };
        self.submit(kind, 0)
    }

    /// Requests a scan. Requires the radio enabled. Deadline 15 s, no auto retry.
    pub fn scan(&mut self) -> SubmitResult {
        if !self.radio_usable() {
            return SubmitResult::Unavailable;
        }
        if !matches!(self.radio, RadioState::Enabled) {
            return SubmitResult::InvalidInput;
        }
        self.submit(ActiveKind::Scan, 0)
    }

    /// Requests association with a scanned result using a borrowed secret view.
    /// The secret is copied into fixed staging and validated before submission.
    pub fn connect(&mut self, result_id: u16, view: CredentialView<'_>) -> SubmitResult {
        if !self.radio_usable() {
            return SubmitResult::Unavailable;
        }
        if !matches!(self.radio, RadioState::Enabled) {
            return SubmitResult::InvalidInput;
        }
        let Some(target) = self.result_index(result_id) else {
            return SubmitResult::InvalidInput;
        };
        // The result's advertised security must match the requested mode.
        let result_security = self.results[target].as_ref().unwrap().security;
        if result_security != view.security {
            return SubmitResult::InvalidInput;
        }
        if !credential_valid(view.security, view.secret) {
            return SubmitResult::InvalidInput;
        }
        // Stage the secret for the connect operation's retry window.
        self.stage_credential(view);
        self.submit(ActiveKind::Connect, target)
    }

    /// Requests disconnection. Deadline 5 s, no retry.
    pub fn disconnect(&mut self) -> SubmitResult {
        if !self.radio_usable() {
            return SubmitResult::Unavailable;
        }
        self.submit(ActiveKind::Disconnect, 0)
    }

    /// Requests that a retained profile be forgotten. Requires a credential
    /// provider commit acknowledgement. Deadline 5 s, no retry.
    pub fn forget(&mut self, _profile_id: u16) -> SubmitResult {
        if !self.radio_usable() {
            return SubmitResult::Unavailable;
        }
        // The profile id is carried implicitly by the forget request; the
        // provider owns slot identity. We keep the last requested id in the
        // active op for the commit call.
        self.submit(ActiveKind::Forget, usize::from(_profile_id))
    }

    /// Cancels the current request. Idempotent for the active request. Stops
    /// retries, asks the driver to cancel, zeroizes staging, and emits one
    /// terminal `Cancelled` event.
    pub fn cancel(&mut self, request_id: RequestId, hal: &mut dyn WifiHal) -> SubmitResult {
        // Cancel a queued (not yet started) command outright.
        if self.cancel_queued(request_id) {
            self.emit(
                request_id,
                OperationState::Failed,
                Some(NetworkError::Cancelled),
            );
            return SubmitResult::Accepted(request_id);
        }
        let Some(op) = self.active.as_ref() else {
            return SubmitResult::InvalidInput;
        };
        if op.request_id != request_id {
            return SubmitResult::InvalidInput;
        }
        hal.cancel();
        self.finish_failure(NetworkError::Cancelled);
        SubmitResult::Accepted(request_id)
    }

    // ------------------------------------------------------------- service loop

    /// Advances the active operation with a bounded work budget. Never loops to
    /// completion. Owns deadlines, connect retries, and radio-loss handling.
    pub fn service(
        &mut self,
        now_ms: u64,
        work_budget: u32,
        hal: &mut dyn WifiHal,
        creds: &mut dyn CredentialProvider,
    ) -> ServiceProgress {
        self.last_now_ms = now_ms;
        let before = self.snapshot_key();

        // Radio/capability loss preempts everything else.
        if !hal.radio_present() && self.radio_usable() {
            self.on_radio_lost(hal);
            return self.progress(before);
        }

        let mut budget = work_budget;
        // Start the next queued command if idle.
        if self.active.is_none() {
            self.start_next_command(now_ms);
        }

        while budget > 0 && self.active.is_some() {
            budget -= 1;
            let advanced = self.advance_active(now_ms, hal, creds);
            if !advanced {
                break; // pending; nothing more to do this call
            }
            if self.active.is_none() {
                self.start_next_command(now_ms);
            }
        }

        self.progress(before)
    }

    /// Advances the active op by one step. Returns true if the op reached a
    /// terminal state (freeing the slot) or a retry was rescheduled; false when
    /// the op is still pending on the driver.
    fn advance_active(
        &mut self,
        now_ms: u64,
        hal: &mut dyn WifiHal,
        creds: &mut dyn CredentialProvider,
    ) -> bool {
        let kind = match self.active.as_ref() {
            Some(op) => op.kind,
            None => return false,
        };

        // Submission never performs radio I/O; the first advance kicks the
        // transport and arms the deadline from the real clock.
        if !self.active.as_ref().map(|op| op.kicked).unwrap_or(true) {
            if let Some(op) = self.active.as_mut() {
                op.kicked = true;
                op.deadline_ms = now_ms + deadline_for(kind);
            }
            self.kick_transport(hal);
        }

        // Deadline check applies to every operation.
        if now_ms
            >= self
                .active
                .as_ref()
                .map(|op| op.deadline_ms)
                .unwrap_or(u64::MAX)
        {
            self.finish_failure(NetworkError::Timeout);
            return true;
        }

        let request_id = self.active.as_ref().map(|op| op.request_id).unwrap_or(0);

        match kind {
            ActiveKind::RadioEnable | ActiveKind::RadioDisable => match hal.poll_set_radio() {
                HalPoll::Pending => false,
                HalPoll::Ready | HalPoll::ReadyCount(_) => {
                    let enabled = matches!(kind, ActiveKind::RadioEnable);
                    self.radio = if enabled {
                        RadioState::Enabled
                    } else {
                        RadioState::Disabled
                    };
                    if !enabled {
                        self.clear_results();
                        self.connected_result_id = None;
                    }
                    self.finish_success(OperationState::Disabled, request_id);
                    true
                }
                HalPoll::Failed(fault) => {
                    self.apply_fault(fault, hal);
                    true
                }
            },
            ActiveKind::Scan => match hal.poll_scan() {
                HalPoll::Pending => false,
                HalPoll::ReadyCount(count) => {
                    self.ingest_scan(count, &*hal);
                    self.finish_success(OperationState::Results, request_id);
                    true
                }
                HalPoll::Ready => {
                    self.finish_success(OperationState::Results, request_id);
                    true
                }
                HalPoll::Failed(fault) => {
                    self.apply_fault(fault, hal);
                    true
                }
            },
            ActiveKind::Connect => self.advance_connect(now_ms, hal),
            ActiveKind::Disconnect => match hal.poll_disconnect() {
                HalPoll::Pending => false,
                HalPoll::Ready | HalPoll::ReadyCount(_) => {
                    self.connected_result_id = None;
                    self.finish_success(OperationState::Results, request_id);
                    true
                }
                HalPoll::Failed(fault) => {
                    self.apply_fault(fault, hal);
                    true
                }
            },
            ActiveKind::Forget => {
                let profile_id = self
                    .active
                    .as_ref()
                    .and_then(|op| op.connect_target)
                    .unwrap_or(0) as u16;
                match creds.forget(profile_id) {
                    ForgetOutcome::Committed => {
                        self.last_forget_committed = true;
                        self.finish_success(OperationState::Results, request_id);
                        true
                    }
                    ForgetOutcome::StoreUnavailable => {
                        self.finish_failure(NetworkError::CredentialStoreUnavailable);
                        true
                    }
                    ForgetOutcome::Corrupt => {
                        self.finish_failure(NetworkError::StorageCorrupt);
                        true
                    }
                }
            }
        }
    }

    fn advance_connect(&mut self, now_ms: u64, hal: &mut dyn WifiHal) -> bool {
        // A scheduled retry re-begins association, then falls through to poll
        // the fresh attempt in the same step. Until the retry window opens the
        // op simply waits.
        if let Some(at) = self.active.as_ref().and_then(|op| op.retry_at_ms) {
            if now_ms < at {
                return false; // waiting for retry window
            }
            self.begin_connect_transport(hal);
            if let Some(op) = self.active.as_mut() {
                op.retry_at_ms = None;
            }
        }

        match hal.poll_connect() {
            HalPoll::Pending => false,
            HalPoll::Ready | HalPoll::ReadyCount(_) => {
                let (request_id, target) = {
                    let op = self.active.as_ref().unwrap();
                    (op.request_id, op.connect_target)
                };
                self.connected_result_id = target
                    .and_then(|i| self.results[i].as_ref())
                    .map(|r| r.result_id);
                self.finish_success(OperationState::Connected, request_id);
                true
            }
            HalPoll::Failed(fault) => {
                if fault == HalFault::RadioLost || fault == HalFault::Firmware {
                    self.apply_fault(fault, hal);
                    return true;
                }
                let can_retry = fault.is_retryable()
                    && self
                        .active
                        .as_ref()
                        .map(|op| op.retry_count < CONNECT_RETRY_MAX)
                        .unwrap_or(false);
                if can_retry {
                    let delay = {
                        let op = self.active.as_mut().unwrap();
                        let idx = usize::from(op.retry_count);
                        op.retry_count += 1;
                        CONNECT_RETRY_DELAYS_MS[idx.min(CONNECT_RETRY_DELAYS_MS.len() - 1)]
                    };
                    if let Some(op) = self.active.as_mut() {
                        op.retry_at_ms = Some(now_ms + delay);
                    }
                    false
                } else {
                    self.finish_failure(fault.to_error());
                    true
                }
            }
        }
    }

    // ------------------------------------------------------------- transitions

    /// Begins an operation, moving to its running state and emitting the entry
    /// event. The HAL transport is kicked and the deadline armed on the first
    /// `service` advance so no submission performs radio I/O.
    fn begin(&mut self, kind: ActiveKind, request_id: RequestId, connect_target: usize) {
        // Connect carries the chosen result index; Forget carries the profile id.
        let target = if matches!(kind, ActiveKind::Connect | ActiveKind::Forget) {
            Some(connect_target)
        } else {
            None
        };
        self.active = Some(ActiveOp {
            kind,
            request_id,
            deadline_ms: 0,
            kicked: false,
            retry_count: 0,
            retry_at_ms: None,
            connect_target: target,
        });
        self.last_error = None;
        self.enter_running_state(kind);
        self.emit(request_id, self.state, None);
    }

    /// Starts the next queued command if the active slot is free.
    fn start_next_command(&mut self, _now_ms: u64) {
        if self.active.is_some() || self.command_len == 0 {
            return;
        }
        let cmd = self.commands[self.command_head].take().unwrap();
        self.command_head = (self.command_head + 1) % COMMAND_QUEUE_MAX;
        self.command_len -= 1;
        self.begin(cmd.kind, cmd.request_id, cmd.connect_target);
    }

    fn enter_running_state(&mut self, kind: ActiveKind) {
        self.state = match kind {
            ActiveKind::RadioEnable => {
                self.radio = RadioState::Enabling;
                OperationState::RadioEnabling
            }
            ActiveKind::RadioDisable => {
                self.radio = RadioState::Disabling;
                OperationState::RadioDisabling
            }
            ActiveKind::Scan => OperationState::Scanning,
            ActiveKind::Connect => OperationState::Connecting,
            ActiveKind::Disconnect => OperationState::Disconnecting,
            ActiveKind::Forget => OperationState::Forgetting,
        };
    }

    /// Emits the transport `begin_*` for the active op. Called lazily on the
    /// first `service` advance so a submission never performs radio I/O.
    fn kick_transport(&mut self, hal: &mut dyn WifiHal) {
        let Some(op) = self.active.as_ref() else {
            return;
        };
        match op.kind {
            ActiveKind::RadioEnable => hal.begin_set_radio(true),
            ActiveKind::RadioDisable => hal.begin_set_radio(false),
            ActiveKind::Scan => hal.begin_scan(),
            ActiveKind::Connect => self.begin_connect_transport(hal),
            ActiveKind::Disconnect => hal.begin_disconnect(),
            ActiveKind::Forget => {} // handled synchronously via provider
        }
    }

    fn begin_connect_transport(&mut self, hal: &mut dyn WifiHal) {
        let Some(op) = self.active.as_ref() else {
            return;
        };
        let Some(idx) = op.connect_target else {
            return;
        };
        let Some(result) = self.results[idx].as_ref() else {
            return;
        };
        let ssid = result.ssid;
        let bssid = result.bssid;
        let security = self.staging_security;
        let secret_len = usize::from(self.staging_len);
        hal.begin_connect(&ssid, &bssid, security, &self.staging[..secret_len]);
    }

    fn finish_success(&mut self, state: OperationState, request_id: RequestId) {
        self.zeroize_staging();
        self.state = state;
        self.last_error = None;
        self.active = None;
        self.emit(request_id, state, None);
    }

    fn finish_failure(&mut self, error: NetworkError) {
        let request_id = self.active.as_ref().map(|op| op.request_id).unwrap_or(0);
        self.zeroize_staging();
        self.active = None;
        self.state = OperationState::Failed;
        self.last_error = Some(error);
        self.record_status(error);
        self.emit(request_id, OperationState::Failed, Some(error));
    }

    fn apply_fault(&mut self, fault: HalFault, hal: &mut dyn WifiHal) {
        match fault {
            HalFault::RadioLost | HalFault::Firmware => self.on_radio_lost(hal),
            other => self.finish_failure(other.to_error()),
        }
    }

    /// Radio/capability loss: advance generation, drop active op and queue,
    /// zeroize staging, discard late completions, and enter `RadioUnavailable`.
    fn on_radio_lost(&mut self, hal: &mut dyn WifiHal) {
        hal.cancel();
        self.advance_generation();
        self.active = None;
        self.clear_commands();
        self.clear_results();
        self.connected_result_id = None;
        self.zeroize_staging();
        self.radio = RadioState::Unavailable;
        self.state = OperationState::RadioUnavailable;
        self.last_error = Some(NetworkError::RadioUnavailable);
        self.record_status(NetworkError::RadioUnavailable);
        self.emit(
            0,
            OperationState::RadioUnavailable,
            Some(NetworkError::RadioUnavailable),
        );
    }

    /// Prepares the service for lifecycle shutdown. It cancels any active
    /// request, drains the command and event queues, clears results, zeroizes
    /// credential staging, and advances the generation so any outstanding handle
    /// is rejected as stale. It lands in the safe offline `RadioUnavailable`
    /// state. The driver runner and radio power-down are torn down separately by
    /// the lifecycle owner after this returns.
    pub fn quiesce_offline(&mut self) {
        self.advance_generation();
        self.active = None;
        self.clear_commands();
        self.clear_results();
        self.connected_result_id = None;
        self.zeroize_staging();
        while self.events.pop().is_some() {}
        self.radio = RadioState::Unavailable;
        self.state = OperationState::RadioUnavailable;
        self.last_error = None;
    }

    /// Re-declares the radio available after a loss, advancing the generation so
    /// prior handles are stale. Leaves the radio disabled and idle.
    pub fn reinitialize(&mut self) {
        self.advance_generation();
        self.radio = RadioState::Disabled;
        self.state = OperationState::Disabled;
        self.last_error = None;
        self.active = None;
        self.clear_commands();
        self.clear_results();
        self.connected_result_id = None;
        self.zeroize_staging();
    }

    /// Reports a late driver completion carrying an old generation/request id.
    /// It is discarded and counted; the observable state is unchanged.
    pub fn note_late_completion(&mut self, generation: Generation, request_id: RequestId) -> bool {
        let is_current = self.generation == generation
            && self
                .active
                .as_ref()
                .map(|op| op.request_id == request_id)
                .unwrap_or(false);
        if is_current {
            return false;
        }
        self.late_completions_discarded += 1;
        true
    }

    fn advance_generation(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        if self.generation == 0 {
            self.generation = 1;
        }
    }

    // ------------------------------------------------------------- scan ingest

    fn ingest_scan(&mut self, count: u8, hal: &dyn WifiHal) {
        self.clear_results();
        let count = usize::from(count).min(RAW_SCAN_MAX);
        let mut deduped: [Option<RawScanResult>; RAW_SCAN_MAX] = [None; RAW_SCAN_MAX];
        let mut deduped_len = 0usize;
        for i in 0..count {
            let raw = hal.scan_result(i as u8);
            // Dedup by BSSID, keeping the strongest RSSI.
            let mut found = false;
            for slot in deduped[..deduped_len].iter_mut() {
                let existing = slot.as_mut().unwrap();
                if existing.bssid == raw.bssid {
                    if raw.rssi_dbm > existing.rssi_dbm {
                        *existing = raw;
                    }
                    found = true;
                    break;
                }
            }
            if !found && deduped_len < RAW_SCAN_MAX {
                deduped[deduped_len] = Some(raw);
                deduped_len += 1;
            }
        }
        // Sort by RSSI descending, then BSSID ascending (deterministic).
        let mut list = deduped;
        insertion_sort_scan(&mut list[..deduped_len]);
        let keep = deduped_len.min(SCAN_RESULTS_MAX);
        for (i, raw) in list[..keep].iter().enumerate() {
            let raw = raw.unwrap();
            self.results[i] = Some(ScanResult {
                result_id: (i + 1) as u16,
                ssid: raw.ssid,
                bssid: raw.bssid,
                rssi_dbm: raw.rssi_dbm,
                security: raw.security,
            });
        }
        self.result_count = keep;
    }

    fn result_index(&self, result_id: u16) -> Option<usize> {
        self.results[..self.result_count]
            .iter()
            .position(|r| r.as_ref().map(|r| r.result_id) == Some(result_id))
    }

    fn clear_results(&mut self) {
        for slot in self.results.iter_mut() {
            *slot = None;
        }
        self.result_count = 0;
    }

    // ------------------------------------------------------------- queue helpers

    fn cancel_queued(&mut self, request_id: RequestId) -> bool {
        for i in 0..self.command_len {
            let idx = (self.command_head + i) % COMMAND_QUEUE_MAX;
            if self.commands[idx].map(|c| c.request_id) == Some(request_id) {
                // Compact by shifting subsequent entries down.
                for j in i..self.command_len - 1 {
                    let from = (self.command_head + j + 1) % COMMAND_QUEUE_MAX;
                    let to = (self.command_head + j) % COMMAND_QUEUE_MAX;
                    self.commands[to] = self.commands[from];
                }
                let last = (self.command_head + self.command_len - 1) % COMMAND_QUEUE_MAX;
                self.commands[last] = None;
                self.command_len -= 1;
                return true;
            }
        }
        false
    }

    fn clear_commands(&mut self) {
        for slot in self.commands.iter_mut() {
            *slot = None;
        }
        self.command_head = 0;
        self.command_len = 0;
    }

    // ------------------------------------------------------------- staging

    fn stage_credential(&mut self, view: CredentialView<'_>) {
        self.zeroize_staging();
        let len = view.secret.len().min(CREDENTIAL_MAX_BYTES);
        self.staging[..len].copy_from_slice(&view.secret[..len]);
        self.staging_len = len as u8;
        self.staging_security = view.security;
    }

    fn zeroize_staging(&mut self) {
        // Volatile zeroization: overwrite every byte, unconditionally.
        for byte in self.staging.iter_mut() {
            unsafe {
                core::ptr::write_volatile(byte, 0);
            }
        }
        self.staging_len = 0;
    }

    // ------------------------------------------------------------- status/events

    fn emit(&mut self, request_id: RequestId, state: OperationState, error: Option<NetworkError>) {
        self.events.push(NetworkEvent {
            request_id,
            state,
            error,
        });
    }

    fn record_status(&mut self, error: NetworkError) {
        let tail = (self.status_head + self.status_len) % STATUS_HISTORY_MAX;
        if self.status_len == STATUS_HISTORY_MAX {
            self.status_history[self.status_head] = Some(error);
            self.status_head = (self.status_head + 1) % STATUS_HISTORY_MAX;
        } else {
            self.status_history[tail] = Some(error);
            self.status_len += 1;
        }
    }

    /// Redacted status history, oldest first.
    pub fn status_history(&self) -> impl Iterator<Item = NetworkError> + '_ {
        (0..self.status_len).filter_map(move |i| {
            let idx = (self.status_head + i) % STATUS_HISTORY_MAX;
            self.status_history[idx]
        })
    }

    // ------------------------------------------------------------- change detect

    fn snapshot_key(
        &self,
    ) -> (
        Generation,
        OperationState,
        RadioState,
        Option<NetworkError>,
        u8,
        u8,
    ) {
        (
            self.generation,
            self.state,
            self.radio,
            self.last_error,
            self.result_count as u8,
            self.events.depth(),
        )
    }

    fn progress(
        &self,
        before: (
            Generation,
            OperationState,
            RadioState,
            Option<NetworkError>,
            u8,
            u8,
        ),
    ) -> ServiceProgress {
        ServiceProgress {
            changed: before != self.snapshot_key(),
            op_active: self.active.is_some(),
            events_pending: self.events.depth(),
        }
    }
}

impl Default for NetworkService {
    fn default() -> Self {
        Self::new()
    }
}

// ------------------------------------------------------------- free functions

fn deadline_for(kind: ActiveKind) -> u64 {
    match kind {
        ActiveKind::RadioEnable | ActiveKind::RadioDisable => RADIO_DEADLINE_MS,
        ActiveKind::Scan => SCAN_DEADLINE_MS,
        ActiveKind::Connect => CONNECT_DEADLINE_MS,
        ActiveKind::Disconnect => DISCONNECT_DEADLINE_MS,
        ActiveKind::Forget => FORGET_DEADLINE_MS,
    }
}

/// Validates a credential against the security mode per the KOTO-0224 rules:
/// Open requires zero bytes; WPA2 requires 8..=63 printable ASCII bytes.
fn credential_valid(security: Security, secret: &[u8]) -> bool {
    match security {
        Security::Open => secret.is_empty(),
        Security::Wpa2PersonalAes => {
            (CREDENTIAL_MIN_BYTES..=CREDENTIAL_MAX_BYTES).contains(&secret.len())
                && secret.iter().all(|&b| (0x20..=0x7e).contains(&b))
        }
    }
}

/// Deterministic in-place sort: RSSI descending, then BSSID ascending.
fn insertion_sort_scan(list: &mut [Option<RawScanResult>]) {
    for i in 1..list.len() {
        let mut j = i;
        while j > 0 {
            let a = list[j - 1].unwrap();
            let b = list[j].unwrap();
            let swap = b.rssi_dbm > a.rssi_dbm || (b.rssi_dbm == a.rssi_dbm && b.bssid < a.bssid);
            if swap {
                list.swap(j - 1, j);
                j -= 1;
            } else {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scriptable fake radio HAL. Every operation is driven step by step so a
    /// test can inject pending, ready, and fault sequences deterministically.
    struct FakeHal {
        present: bool,
        set_radio_poll: HalPoll,
        set_radio_begins: u32,
        scan: Vec<RawScanResult>,
        scan_pending: u32,
        scan_begins: u32,
        connect_script: Vec<HalPoll>,
        connect_begins: u32,
        last_secret: Vec<u8>,
        last_security: Option<Security>,
        last_ssid: Option<Ssid>,
        last_bssid: Option<[u8; BSSID_BYTES]>,
        disconnect_poll: HalPoll,
        cancels: u32,
    }

    impl FakeHal {
        fn new() -> Self {
            Self {
                present: true,
                set_radio_poll: HalPoll::Ready,
                set_radio_begins: 0,
                scan: Vec::new(),
                scan_pending: 0,
                scan_begins: 0,
                connect_script: Vec::new(),
                connect_begins: 0,
                last_secret: Vec::new(),
                last_security: None,
                last_ssid: None,
                last_bssid: None,
                disconnect_poll: HalPoll::Ready,
                cancels: 0,
            }
        }
    }

    impl WifiHal for FakeHal {
        fn radio_present(&self) -> bool {
            self.present
        }
        fn begin_set_radio(&mut self, _enabled: bool) {
            self.set_radio_begins += 1;
        }
        fn poll_set_radio(&mut self) -> HalPoll {
            self.set_radio_poll
        }
        fn begin_scan(&mut self) {
            self.scan_begins += 1;
        }
        fn poll_scan(&mut self) -> HalPoll {
            if self.scan_pending > 0 {
                self.scan_pending -= 1;
                return HalPoll::Pending;
            }
            HalPoll::ReadyCount(self.scan.len() as u8)
        }
        fn scan_result(&self, index: u8) -> RawScanResult {
            self.scan[usize::from(index)]
        }
        fn begin_connect(
            &mut self,
            ssid: &Ssid,
            bssid: &[u8; BSSID_BYTES],
            security: Security,
            secret: &[u8],
        ) {
            self.connect_begins += 1;
            self.last_ssid = Some(*ssid);
            self.last_bssid = Some(*bssid);
            self.last_security = Some(security);
            self.last_secret = secret.to_vec();
        }
        fn poll_connect(&mut self) -> HalPoll {
            if self.connect_script.is_empty() {
                return HalPoll::Pending;
            }
            self.connect_script.remove(0)
        }
        fn begin_disconnect(&mut self) {}
        fn poll_disconnect(&mut self) -> HalPoll {
            self.disconnect_poll
        }
        fn cancel(&mut self) {
            self.cancels += 1;
        }
    }

    struct FakeCreds {
        available: bool,
        outcome: ForgetOutcome,
    }

    impl FakeCreds {
        fn ok() -> Self {
            Self {
                available: true,
                outcome: ForgetOutcome::Committed,
            }
        }
    }

    impl CredentialProvider for FakeCreds {
        fn available(&self) -> bool {
            self.available
        }
        fn forget(&mut self, _profile_id: u16) -> ForgetOutcome {
            self.outcome
        }
    }

    fn raw(bssid_last: u8, rssi: i8, security: Security) -> RawScanResult {
        RawScanResult {
            ssid: Ssid::from_bytes(b"KotoLab"),
            bssid: [0x02, 0, 0, 0, 0, bssid_last],
            rssi_dbm: rssi,
            security,
        }
    }

    fn pump(svc: &mut NetworkService, hal: &mut FakeHal, creds: &mut FakeCreds, now: u64) {
        svc.service(now, 8, hal, creds);
    }

    fn enable_radio(svc: &mut NetworkService, hal: &mut FakeHal, creds: &mut FakeCreds) {
        assert!(matches!(svc.set_radio(true), SubmitResult::Accepted(_)));
        pump(svc, hal, creds, 0);
        assert_eq!(svc.snapshot().radio, RadioState::Enabled);
    }

    fn scan_with(
        svc: &mut NetworkService,
        hal: &mut FakeHal,
        creds: &mut FakeCreds,
        results: Vec<RawScanResult>,
    ) {
        hal.scan = results;
        assert!(matches!(svc.scan(), SubmitResult::Accepted(_)));
        pump(svc, hal, creds, 1);
        assert_eq!(svc.snapshot().state, OperationState::Results);
    }

    fn ready_to_connect(
        svc: &mut NetworkService,
        hal: &mut FakeHal,
        creds: &mut FakeCreds,
        security: Security,
    ) {
        enable_radio(svc, hal, creds);
        scan_with(svc, hal, creds, vec![raw(1, -40, security)]);
    }

    #[test]
    fn radio_enable_disable_round_trip() {
        let mut svc = NetworkService::new();
        let mut hal = FakeHal::new();
        let mut creds = FakeCreds::ok();
        enable_radio(&mut svc, &mut hal, &mut creds);
        assert_eq!(hal.set_radio_begins, 1);

        assert!(matches!(svc.set_radio(false), SubmitResult::Accepted(_)));
        pump(&mut svc, &mut hal, &mut creds, 2);
        assert_eq!(svc.snapshot().radio, RadioState::Disabled);
    }

    #[test]
    fn submission_performs_no_radio_io() {
        let mut svc = NetworkService::new();
        let hal = FakeHal::new();
        assert!(matches!(svc.set_radio(true), SubmitResult::Accepted(_)));
        assert_eq!(hal.set_radio_begins, 0);
        assert_eq!(svc.snapshot().state, OperationState::RadioEnabling);
    }

    #[test]
    fn request_ids_are_monotonic_nonzero_and_wrap() {
        let mut svc = NetworkService::new();
        svc.next_request = u32::MAX;
        let a = svc.next_request_id();
        let b = svc.next_request_id();
        assert_eq!(a, u32::MAX);
        assert_eq!(b, 1);
        assert_ne!(b, 0);
    }

    #[test]
    fn generation_is_monotonic_nonzero_and_wraps() {
        let mut svc = NetworkService::new();
        svc.generation = u32::MAX;
        svc.advance_generation();
        assert_eq!(svc.generation(), 1);
        svc.advance_generation();
        assert_eq!(svc.generation(), 2);
    }

    #[test]
    fn scan_dedups_by_bssid_and_sorts_rssi_then_bssid() {
        let mut svc = NetworkService::new();
        let mut hal = FakeHal::new();
        let mut creds = FakeCreds::ok();
        enable_radio(&mut svc, &mut hal, &mut creds);
        scan_with(
            &mut svc,
            &mut hal,
            &mut creds,
            vec![
                raw(3, -70, Security::Open),
                raw(1, -50, Security::Wpa2PersonalAes),
                raw(1, -40, Security::Wpa2PersonalAes),
                raw(2, -50, Security::Open),
            ],
        );
        let ordered: Vec<(u8, i8)> = svc.results().map(|r| (r.bssid[5], r.rssi_dbm)).collect();
        assert_eq!(ordered, vec![(1, -40), (2, -50), (3, -70)]);
        assert_eq!(svc.snapshot().result_count, 3);
    }

    #[test]
    fn scan_caps_at_sixteen_strongest() {
        let mut svc = NetworkService::new();
        let mut hal = FakeHal::new();
        let mut creds = FakeCreds::ok();
        enable_radio(&mut svc, &mut hal, &mut creds);
        let mut results = Vec::new();
        for i in 0..20u8 {
            results.push(raw(i, -80 + i as i8, Security::Open));
        }
        scan_with(&mut svc, &mut hal, &mut creds, results);
        assert_eq!(svc.snapshot().result_count, SCAN_RESULTS_MAX as u8);
        let strongest = svc.results().next().unwrap();
        assert_eq!(strongest.rssi_dbm, -61);
        assert_eq!(strongest.bssid[5], 19);
    }

    #[test]
    fn empty_scan_is_a_valid_results_snapshot() {
        let mut svc = NetworkService::new();
        let mut hal = FakeHal::new();
        let mut creds = FakeCreds::ok();
        enable_radio(&mut svc, &mut hal, &mut creds);
        scan_with(&mut svc, &mut hal, &mut creds, Vec::new());
        assert_eq!(svc.snapshot().state, OperationState::Results);
        assert_eq!(svc.snapshot().result_count, 0);
        assert_eq!(svc.results().count(), 0);
    }

    #[test]
    fn connect_success_sets_connected_and_zeroizes_staging() {
        let mut svc = NetworkService::new();
        let mut hal = FakeHal::new();
        let mut creds = FakeCreds::ok();
        ready_to_connect(&mut svc, &mut hal, &mut creds, Security::Wpa2PersonalAes);
        hal.connect_script = vec![HalPoll::Ready];
        let view = CredentialView {
            security: Security::Wpa2PersonalAes,
            secret: b"supersecret",
        };
        assert!(matches!(svc.connect(1, view), SubmitResult::Accepted(_)));
        pump(&mut svc, &mut hal, &mut creds, 100);
        let snap = svc.snapshot();
        assert_eq!(snap.state, OperationState::Connected);
        assert_eq!(snap.connected_result_id, Some(1));
        assert_eq!(hal.last_secret, b"supersecret");
        assert_eq!(hal.last_security, Some(Security::Wpa2PersonalAes));
        assert_eq!(
            hal.last_ssid.map(|s| s.as_bytes().to_vec()),
            Some(b"KotoLab".to_vec())
        );
        assert_eq!(hal.last_bssid, Some([0x02, 0, 0, 0, 0, 1]));
        assert!(svc.credential_staging_zeroized());
    }

    #[test]
    fn connect_rejects_invalid_credentials_at_submit() {
        let mut svc = NetworkService::new();
        let mut hal = FakeHal::new();
        let mut creds = FakeCreds::ok();
        ready_to_connect(&mut svc, &mut hal, &mut creds, Security::Wpa2PersonalAes);

        let short = CredentialView {
            security: Security::Wpa2PersonalAes,
            secret: b"1234",
        };
        assert_eq!(svc.connect(1, short), SubmitResult::InvalidInput);

        let open = CredentialView {
            security: Security::Open,
            secret: b"",
        };
        assert_eq!(svc.connect(1, open), SubmitResult::InvalidInput);

        let valid = CredentialView {
            security: Security::Wpa2PersonalAes,
            secret: b"validpass",
        };
        assert_eq!(svc.connect(99, valid), SubmitResult::InvalidInput);
        assert!(svc.credential_staging_zeroized());
    }

    #[test]
    fn open_network_requires_empty_credential() {
        let mut svc = NetworkService::new();
        let mut hal = FakeHal::new();
        let mut creds = FakeCreds::ok();
        ready_to_connect(&mut svc, &mut hal, &mut creds, Security::Open);
        let bad = CredentialView {
            security: Security::Open,
            secret: b"x",
        };
        assert_eq!(svc.connect(1, bad), SubmitResult::InvalidInput);
        let good = CredentialView {
            security: Security::Open,
            secret: b"",
        };
        hal.connect_script = vec![HalPoll::Ready];
        assert!(matches!(svc.connect(1, good), SubmitResult::Accepted(_)));
        pump(&mut svc, &mut hal, &mut creds, 100);
        assert_eq!(svc.snapshot().state, OperationState::Connected);
    }

    #[test]
    fn connect_authentication_failure_does_not_retry() {
        let mut svc = NetworkService::new();
        let mut hal = FakeHal::new();
        let mut creds = FakeCreds::ok();
        ready_to_connect(&mut svc, &mut hal, &mut creds, Security::Wpa2PersonalAes);
        hal.connect_script = vec![HalPoll::Failed(HalFault::Auth)];
        let view = CredentialView {
            security: Security::Wpa2PersonalAes,
            secret: b"wrongpass",
        };
        assert!(matches!(svc.connect(1, view), SubmitResult::Accepted(_)));
        pump(&mut svc, &mut hal, &mut creds, 100);
        let snap = svc.snapshot();
        assert_eq!(snap.state, OperationState::Failed);
        assert_eq!(snap.last_error, Some(NetworkError::AuthenticationFailed));
        assert_eq!(hal.connect_begins, 1);
        assert!(svc.credential_staging_zeroized());
    }

    #[test]
    fn connect_transient_failure_retries_twice_then_succeeds() {
        let mut svc = NetworkService::new();
        let mut hal = FakeHal::new();
        let mut creds = FakeCreds::ok();
        ready_to_connect(&mut svc, &mut hal, &mut creds, Security::Wpa2PersonalAes);
        hal.connect_script = vec![
            HalPoll::Failed(HalFault::LinkLost),
            HalPoll::Failed(HalFault::Transient),
            HalPoll::Ready,
        ];
        let view = CredentialView {
            security: Security::Wpa2PersonalAes,
            secret: b"validpass",
        };
        assert!(matches!(svc.connect(1, view), SubmitResult::Accepted(_)));
        // Attempt 1 fails transiently -> retry scheduled 1s later.
        pump(&mut svc, &mut hal, &mut creds, 1_000);
        assert_eq!(svc.snapshot().state, OperationState::Connecting);
        // Retry #1 (re-begin + poll) fails -> retry scheduled 2s later.
        pump(&mut svc, &mut hal, &mut creds, 2_000);
        // Retry #2 (re-begin + poll) succeeds.
        pump(&mut svc, &mut hal, &mut creds, 4_000);
        let snap = svc.snapshot();
        assert_eq!(snap.state, OperationState::Connected);
        assert_eq!(hal.connect_begins, 3);
    }

    #[test]
    fn connect_transient_failure_exhausts_retries() {
        let mut svc = NetworkService::new();
        let mut hal = FakeHal::new();
        let mut creds = FakeCreds::ok();
        ready_to_connect(&mut svc, &mut hal, &mut creds, Security::Wpa2PersonalAes);
        hal.connect_script = vec![
            HalPoll::Failed(HalFault::LinkLost),
            HalPoll::Failed(HalFault::LinkLost),
            HalPoll::Failed(HalFault::LinkLost),
        ];
        let view = CredentialView {
            security: Security::Wpa2PersonalAes,
            secret: b"validpass",
        };
        assert!(matches!(svc.connect(1, view), SubmitResult::Accepted(_)));
        pump(&mut svc, &mut hal, &mut creds, 1_000); // attempt 1 fails
        pump(&mut svc, &mut hal, &mut creds, 2_000); // retry #1 fails
        pump(&mut svc, &mut hal, &mut creds, 4_000); // retry #2 fails -> give up
        let snap = svc.snapshot();
        assert_eq!(snap.state, OperationState::Failed);
        assert_eq!(snap.last_error, Some(NetworkError::LinkLost));
        assert_eq!(hal.connect_begins, 3);
    }

    #[test]
    fn connect_times_out_after_deadline() {
        let mut svc = NetworkService::new();
        let mut hal = FakeHal::new();
        let mut creds = FakeCreds::ok();
        ready_to_connect(&mut svc, &mut hal, &mut creds, Security::Wpa2PersonalAes);
        let view = CredentialView {
            security: Security::Wpa2PersonalAes,
            secret: b"validpass",
        };
        assert!(matches!(svc.connect(1, view), SubmitResult::Accepted(_)));
        pump(&mut svc, &mut hal, &mut creds, 1_000);
        assert_eq!(svc.snapshot().state, OperationState::Connecting);
        pump(&mut svc, &mut hal, &mut creds, 1_000 + CONNECT_DEADLINE_MS);
        let snap = svc.snapshot();
        assert_eq!(snap.state, OperationState::Failed);
        assert_eq!(snap.last_error, Some(NetworkError::Timeout));
        assert!(svc.credential_staging_zeroized());
    }

    #[test]
    fn disconnect_returns_to_results() {
        let mut svc = NetworkService::new();
        let mut hal = FakeHal::new();
        let mut creds = FakeCreds::ok();
        ready_to_connect(&mut svc, &mut hal, &mut creds, Security::Open);
        hal.connect_script = vec![HalPoll::Ready];
        let view = CredentialView {
            security: Security::Open,
            secret: b"",
        };
        svc.connect(1, view);
        pump(&mut svc, &mut hal, &mut creds, 5);
        assert_eq!(svc.snapshot().state, OperationState::Connected);

        assert!(matches!(svc.disconnect(), SubmitResult::Accepted(_)));
        pump(&mut svc, &mut hal, &mut creds, 6);
        let snap = svc.snapshot();
        assert_eq!(snap.state, OperationState::Results);
        assert_eq!(snap.connected_result_id, None);
    }

    #[test]
    fn forget_commit_acknowledges_and_returns_to_results() {
        let mut svc = NetworkService::new();
        let mut hal = FakeHal::new();
        let mut creds = FakeCreds::ok();
        enable_radio(&mut svc, &mut hal, &mut creds);
        assert!(matches!(svc.forget(1), SubmitResult::Accepted(_)));
        pump(&mut svc, &mut hal, &mut creds, 1);
        assert_eq!(svc.snapshot().state, OperationState::Results);
        assert!(svc.last_forget_committed());
        assert!(svc.credential_staging_zeroized());
    }

    #[test]
    fn forget_store_errors_are_redacted() {
        for (outcome, expected) in [
            (
                ForgetOutcome::StoreUnavailable,
                NetworkError::CredentialStoreUnavailable,
            ),
            (ForgetOutcome::Corrupt, NetworkError::StorageCorrupt),
        ] {
            let mut svc = NetworkService::new();
            let mut hal = FakeHal::new();
            let mut creds = FakeCreds {
                available: true,
                outcome,
            };
            enable_radio(&mut svc, &mut hal, &mut creds);
            svc.forget(1);
            pump(&mut svc, &mut hal, &mut creds, 1);
            assert_eq!(svc.snapshot().last_error, Some(expected));
        }
    }

    #[test]
    fn command_queue_overflow_returns_busy() {
        let mut svc = NetworkService::new();
        let mut hal = FakeHal::new();
        let mut creds = FakeCreds::ok();
        hal.set_radio_poll = HalPoll::Pending;
        assert!(matches!(svc.set_radio(true), SubmitResult::Accepted(_)));
        pump(&mut svc, &mut hal, &mut creds, 0);

        for _ in 0..COMMAND_QUEUE_MAX {
            assert!(matches!(svc.set_radio(false), SubmitResult::Accepted(_)));
        }
        assert_eq!(svc.set_radio(false), SubmitResult::Busy);
        assert_eq!(svc.snapshot().command_depth, COMMAND_QUEUE_MAX as u8);
    }

    #[test]
    fn cancel_active_operation_emits_cancelled_and_zeroizes() {
        let mut svc = NetworkService::new();
        let mut hal = FakeHal::new();
        let mut creds = FakeCreds::ok();
        ready_to_connect(&mut svc, &mut hal, &mut creds, Security::Wpa2PersonalAes);
        let view = CredentialView {
            security: Security::Wpa2PersonalAes,
            secret: b"validpass",
        };
        let SubmitResult::Accepted(req) = svc.connect(1, view) else {
            panic!("expected accept");
        };
        pump(&mut svc, &mut hal, &mut creds, 1);
        assert!(matches!(
            svc.cancel(req, &mut hal),
            SubmitResult::Accepted(_)
        ));
        let snap = svc.snapshot();
        assert_eq!(snap.state, OperationState::Failed);
        assert_eq!(snap.last_error, Some(NetworkError::Cancelled));
        assert_eq!(hal.cancels, 1);
        assert!(svc.credential_staging_zeroized());
    }

    #[test]
    fn cancel_queued_command_removes_it() {
        let mut svc = NetworkService::new();
        let mut hal = FakeHal::new();
        let mut creds = FakeCreds::ok();
        hal.set_radio_poll = HalPoll::Pending;
        svc.set_radio(true);
        pump(&mut svc, &mut hal, &mut creds, 0);
        let SubmitResult::Accepted(queued) = svc.set_radio(false) else {
            panic!("expected accept");
        };
        assert_eq!(svc.snapshot().command_depth, 1);
        assert!(matches!(
            svc.cancel(queued, &mut hal),
            SubmitResult::Accepted(_)
        ));
        assert_eq!(svc.snapshot().command_depth, 0);
    }

    #[test]
    fn radio_loss_advances_generation_and_zeroizes() {
        let mut svc = NetworkService::new();
        let mut hal = FakeHal::new();
        let mut creds = FakeCreds::ok();
        ready_to_connect(&mut svc, &mut hal, &mut creds, Security::Wpa2PersonalAes);
        let view = CredentialView {
            security: Security::Wpa2PersonalAes,
            secret: b"validpass",
        };
        svc.connect(1, view);
        pump(&mut svc, &mut hal, &mut creds, 1);
        let gen_before = svc.generation();

        hal.present = false;
        pump(&mut svc, &mut hal, &mut creds, 2);
        let snap = svc.snapshot();
        assert_eq!(snap.state, OperationState::RadioUnavailable);
        assert_eq!(snap.radio, RadioState::Unavailable);
        assert_eq!(snap.last_error, Some(NetworkError::RadioUnavailable));
        assert_eq!(svc.generation(), gen_before + 1);
        assert!(svc.credential_staging_zeroized());

        assert_eq!(svc.scan(), SubmitResult::Unavailable);
    }

    #[test]
    fn late_completion_with_old_generation_is_discarded() {
        let mut svc = NetworkService::new();
        let mut hal = FakeHal::new();
        let mut creds = FakeCreds::ok();
        ready_to_connect(&mut svc, &mut hal, &mut creds, Security::Wpa2PersonalAes);
        let view = CredentialView {
            security: Security::Wpa2PersonalAes,
            secret: b"validpass",
        };
        let SubmitResult::Accepted(req) = svc.connect(1, view) else {
            panic!("expected accept");
        };
        pump(&mut svc, &mut hal, &mut creds, 1);
        let stale_gen = svc.generation();
        hal.present = false;
        pump(&mut svc, &mut hal, &mut creds, 2);

        // The pre-loss generation/request is stale and discarded.
        assert!(svc.note_late_completion(stale_gen, req));
        assert_eq!(svc.late_completions_discarded(), 1);

        // A completion matching the current generation and active request is
        // not a late completion.
        let mut svc2 = NetworkService::new();
        let mut hal2 = FakeHal::new();
        let mut creds2 = FakeCreds::ok();
        ready_to_connect(&mut svc2, &mut hal2, &mut creds2, Security::Wpa2PersonalAes);
        let view2 = CredentialView {
            security: Security::Wpa2PersonalAes,
            secret: b"validpass",
        };
        let SubmitResult::Accepted(req2) = svc2.connect(1, view2) else {
            panic!("expected accept");
        };
        pump(&mut svc2, &mut hal2, &mut creds2, 1); // active + pending
        assert!(!svc2.note_late_completion(svc2.generation(), req2));
        assert_eq!(svc2.late_completions_discarded(), 0);
    }

    #[test]
    fn event_queue_is_bounded() {
        let mut svc = NetworkService::new();
        let mut hal = FakeHal::new();
        let mut creds = FakeCreds::ok();
        for _ in 0..(EVENT_QUEUE_MAX + 4) {
            enable_radio(&mut svc, &mut hal, &mut creds);
            svc.set_radio(false);
            pump(&mut svc, &mut hal, &mut creds, 0);
        }
        let mut drained = 0;
        while svc.poll_event().is_some() {
            drained += 1;
        }
        assert!(drained <= EVENT_QUEUE_MAX);
    }

    #[test]
    fn regulatory_region_resolves_only_supported_codes() {
        assert_eq!(
            RegulatoryRegion::resolve(Some(*b"JP")),
            Ok(RegulatoryRegion::JAPAN)
        );
        assert_eq!(
            RegulatoryRegion::resolve(Some(*b"US")),
            Ok(RegulatoryRegion::UNITED_STATES)
        );
        assert_eq!(
            RegulatoryRegion::resolve(Some(*b"XX")),
            Ok(RegulatoryRegion::WORLDWIDE)
        );
        assert_eq!(RegulatoryRegion::resolve(None), Err(RegionError::Absent));
        assert_eq!(
            RegulatoryRegion::resolve(Some(*b"ZZ")),
            Err(RegionError::Unsupported)
        );
    }

    #[test]
    fn region_gate_blocks_absent_or_invalid_policy() {
        assert!(RegulatoryRegion::permits_radio_enable(Some(*b"JP")));
        assert!(RegulatoryRegion::permits_radio_enable(Some(*b"XX")));
        assert!(!RegulatoryRegion::permits_radio_enable(None));
        assert!(!RegulatoryRegion::permits_radio_enable(Some(*b"ZZ")));
        assert_eq!(RegulatoryRegion::JAPAN.as_str(), "JP");
        assert_eq!(RegulatoryRegion::JAPAN.code(), *b"JP");
    }

    #[test]
    fn quiesce_offline_clears_state_and_advances_generation() {
        let mut svc = NetworkService::new();
        let mut hal = FakeHal::new();
        let mut creds = FakeCreds::ok();
        ready_to_connect(&mut svc, &mut hal, &mut creds, Security::Wpa2PersonalAes);
        let view = CredentialView {
            security: Security::Wpa2PersonalAes,
            secret: b"validpass",
        };
        svc.connect(1, view);
        pump(&mut svc, &mut hal, &mut creds, 1); // active + pending, staging held
        let gen_before = svc.generation();

        svc.quiesce_offline();

        let snap = svc.snapshot();
        assert_eq!(snap.state, OperationState::RadioUnavailable);
        assert_eq!(snap.radio, RadioState::Unavailable);
        assert_eq!(snap.command_depth, 0);
        assert_eq!(snap.result_count, 0);
        assert_eq!(svc.generation(), gen_before + 1);
        assert!(svc.credential_staging_zeroized());
        // A pre-quiesce handle is now stale.
        assert!(svc.note_late_completion(gen_before, 1));
    }

    #[test]
    fn reinitialize_advances_generation_and_clears_state() {
        let mut svc = NetworkService::new();
        let mut hal = FakeHal::new();
        let mut creds = FakeCreds::ok();
        enable_radio(&mut svc, &mut hal, &mut creds);
        let gen = svc.generation();
        svc.reinitialize();
        assert_eq!(svc.generation(), gen + 1);
        assert_eq!(svc.snapshot().state, OperationState::Disabled);
        assert_eq!(svc.snapshot().radio, RadioState::Disabled);
    }
}
