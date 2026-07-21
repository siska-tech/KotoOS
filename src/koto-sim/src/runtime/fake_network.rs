//! Deterministic fake NetworkService for KotoSim (KOTO-0242).
//!
//! This turns the KOTO-0224 fixture (`koto.fake-network-service.v1`) into an
//! executable test double. It parses and strictly validates the fixture, then
//! **drives the real [`koto_core::net::NetworkService`]** through deterministic
//! driver doubles (a scriptable fake radio HAL and a fake credential provider),
//! asserting that the service's own snapshots match the fixture's recorded
//! ordered snapshots. No wall clock, RNG, DNS, radio, socket, or host-network
//! API is on the execution path: time is the fixture's integer ticks and every
//! driver completion is a fixed, tick-counted latency.
//!
//! The service requires a radio-enabled, scanned precondition before connect and
//! several scenarios record only the interesting sub-sequence. For any scenario
//! whose first action is not `set_radio`, the runner applies a deterministic
//! prologue (enable the radio, then scan the fixture `networks`) before replaying
//! the scenario's actions at their stated ticks.

use koto_core::net::{
    CredentialProvider, CredentialView, ForgetOutcome, HalFault, HalPoll, NetworkError,
    NetworkService, NetworkSnapshot, OperationState, RadioState, RawScanResult, ScanResult,
    Security, Ssid, SubmitResult, WifiHal, BSSID_BYTES, COMMAND_QUEUE_MAX, CREDENTIAL_MAX_BYTES,
    EVENT_QUEUE_MAX, SCAN_RESULTS_MAX, SSID_MAX_BYTES, STATUS_HISTORY_MAX,
};
use koto_core::WifiIntent;
use serde::Deserialize;
use std::collections::VecDeque;

/// The only fixture schema this runner accepts.
pub const FAKE_NETWORK_SCHEMA: &str = "koto.fake-network-service.v1";

// Fixed driver-completion latencies, in ticks, chosen so the v1 fixture replays
// exactly. Each `begin_*` re-arms the countdown; each poll consumes one tick.
const RADIO_LATENCY: u32 = 1;
const SCAN_LATENCY: u32 = 2;
const CONNECT_LATENCY: u32 = 3;
const CONNECT_FAIL_LATENCY: u32 = 2;
const DISCONNECT_LATENCY: u32 = 1;

/// A service work budget large enough to advance one op per `service` call
/// without ever looping to completion inside a single tick.
const WORK_BUDGET: u32 = 8;

// --------------------------------------------------------------------- errors

/// A fixture-validation or replay failure. Carries a human-readable reason with
/// no secret material.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeNetworkError {
    pub scenario: Option<String>,
    pub reason: String,
}

/// One redacted service observation captured at a fixture snapshot boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplayObservation {
    pub scenario: String,
    pub tick: u64,
    pub snapshot: NetworkSnapshot,
    pub results: Vec<ScanResult>,
}

/// Command-driven KotoConfig development session backed by observations that
/// were first produced by replaying the real portable NetworkService.
pub struct FakeNetworkUiSession {
    trace: Vec<ReplayObservation>,
    pending: VecDeque<ReplayObservation>,
    snapshot: NetworkSnapshot,
    results: Vec<ScanResult>,
}

impl FakeNetworkUiSession {
    pub fn new(json: &str) -> Result<Self, FakeNetworkError> {
        let trace = replay_trace(json)?;
        Ok(Self {
            trace,
            pending: VecDeque::new(),
            snapshot: NetworkSnapshot {
                generation: 1,
                request_id: 0,
                radio: RadioState::Disabled,
                state: OperationState::Disabled,
                connected_result_id: None,
                result_count: 0,
                retry_count: 0,
                deadline_ms_remaining: 0,
                last_error: None,
                command_depth: 0,
                event_depth: 0,
            },
            results: Vec::new(),
        })
    }

    pub fn snapshot(&self) -> NetworkSnapshot {
        self.snapshot
    }

    pub fn results(&self) -> impl Iterator<Item = ScanResult> + '_ {
        self.results.iter().copied()
    }

    /// Advances at most one recorded service boundary per frame.
    pub fn service_frame(&mut self) -> bool {
        let Some(observation) = self.pending.pop_front() else {
            return false;
        };
        self.snapshot = observation.snapshot;
        self.results = observation.results;
        true
    }

    /// Submits one bounded page intent. Credential bytes are inspected only to
    /// select the fixture's success/auth-failure branch and are never retained.
    pub fn submit(&mut self, intent: WifiIntent, credential: &[u8]) -> bool {
        if self.snapshot.state == OperationState::RadioUnavailable {
            return false;
        }
        let selection = match intent {
            WifiIntent::EnableRadio => Some(("scan-connect-disconnect", &[0usize, 1][..])),
            WifiIntent::Scan => Some(("scan-connect-disconnect", &[2usize, 3][..])),
            WifiIntent::Connect { security, .. } => {
                let valid = match security {
                    Security::Open => credential.is_empty(),
                    Security::Wpa2PersonalAes => credential == b"password1",
                };
                if valid {
                    Some(("scan-connect-disconnect", &[4usize, 5][..]))
                } else {
                    Some(("authentication-failure", &[0usize, 1][..]))
                }
            }
            WifiIntent::Disconnect => Some(("scan-connect-disconnect", &[6usize, 7][..])),
            WifiIntent::Cancel => Some(("cancel-scan", &[1usize][..])),
            WifiIntent::Forget { .. } => Some(("forget-commit", &[0usize][..])),
            WifiIntent::None | WifiIntent::Exit => None,
        };
        let Some((scenario, indices)) = selection else {
            return false;
        };
        self.pending.clear();
        for &index in indices {
            if let Some(observation) = self
                .trace
                .iter()
                .filter(|item| item.scenario == scenario)
                .nth(index)
            {
                self.pending.push_back(observation.clone());
            }
        }
        !self.pending.is_empty()
    }

    pub fn lose_capability(&mut self, error: NetworkError) {
        self.pending.clear();
        self.results.clear();
        self.snapshot.generation = self.snapshot.generation.wrapping_add(1).max(1);
        self.snapshot.request_id = 0;
        self.snapshot.radio = RadioState::Unavailable;
        self.snapshot.state = OperationState::RadioUnavailable;
        self.snapshot.result_count = 0;
        self.snapshot.last_error = Some(error);
    }
}

impl FakeNetworkError {
    fn top(reason: impl Into<String>) -> Self {
        Self {
            scenario: None,
            reason: reason.into(),
        }
    }

    fn scoped(scenario: &str, reason: impl Into<String>) -> Self {
        Self {
            scenario: Some(scenario.to_string()),
            reason: reason.into(),
        }
    }
}

impl core::fmt::Display for FakeNetworkError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match &self.scenario {
            Some(name) => write!(f, "[{name}] {}", self.reason),
            None => write!(f, "{}", self.reason),
        }
    }
}

// --------------------------------------------------------------------- schema

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FakeNetworkFixture {
    schema: String,
    tick_unit_ms: u64,
    limits: Limits,
    networks: Vec<NetworkDef>,
    scenarios: Vec<Scenario>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Limits {
    ssid_bytes: usize,
    scan_results: usize,
    credential_bytes: usize,
    status_records: usize,
    command_queue: usize,
    event_queue: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NetworkDef {
    result_id: u16,
    #[serde(default)]
    ssid_utf8: Option<String>,
    #[serde(default)]
    ssid_hex: Option<String>,
    bssid: String,
    rssi_dbm: i8,
    security: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Scenario {
    name: String,
    capability_inputs: Vec<bool>,
    #[serde(default)]
    actions: Vec<RawAction>,
    snapshots: Vec<RawSnapshot>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAction {
    tick: u64,
    op: String,
    #[serde(default)]
    request_id: Option<u32>,
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    result_id: Option<u16>,
    #[serde(default)]
    credential_case: Option<String>,
    #[serde(default)]
    profile_id: Option<u16>,
    #[serde(default)]
    generation: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSnapshot {
    tick: u64,
    #[serde(default)]
    generation: Option<u32>,
    #[serde(default)]
    request_id: Option<u32>,
    state: String,
    #[serde(default)]
    radio: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    result_id: Option<u16>,
    #[serde(default)]
    result_ids: Option<Vec<u16>>,
    #[serde(default)]
    retry: Option<u8>,
    #[serde(default)]
    credential_staging_zeroized: Option<bool>,
    #[serde(default)]
    late_completion_discarded: Option<bool>,
    #[serde(default)]
    secret_commit_acknowledged: Option<bool>,
}

// ----------------------------------------------------------------- fake driver

/// A scriptable fake radio HAL driven only by tick-counted latencies.
struct FakeRadio {
    present: bool,
    networks: Vec<RawScanResult>,
    radio_remaining: u32,
    scan_remaining: u32,
    connect_remaining: u32,
    disconnect_remaining: u32,
    connect_terminal: HalPoll,
}

impl FakeRadio {
    fn new(networks: Vec<RawScanResult>) -> Self {
        Self {
            present: true,
            networks,
            radio_remaining: 0,
            scan_remaining: 0,
            connect_remaining: 0,
            disconnect_remaining: 0,
            connect_terminal: HalPoll::Ready,
        }
    }

    /// Configures the next connect's terminal outcome and latency.
    fn arm_connect(&mut self, valid: bool) {
        self.connect_terminal = if valid {
            HalPoll::Ready
        } else {
            HalPoll::Failed(HalFault::Auth)
        };
    }

    fn connect_latency(&self) -> u32 {
        match self.connect_terminal {
            HalPoll::Ready => CONNECT_LATENCY,
            _ => CONNECT_FAIL_LATENCY,
        }
    }
}

impl WifiHal for FakeRadio {
    fn radio_present(&self) -> bool {
        self.present
    }

    fn begin_set_radio(&mut self, _enabled: bool) {
        self.radio_remaining = RADIO_LATENCY;
    }

    fn poll_set_radio(&mut self) -> HalPoll {
        if self.radio_remaining > 0 {
            self.radio_remaining -= 1;
            HalPoll::Pending
        } else {
            HalPoll::Ready
        }
    }

    fn begin_scan(&mut self) {
        self.scan_remaining = SCAN_LATENCY;
    }

    fn poll_scan(&mut self) -> HalPoll {
        if self.scan_remaining > 0 {
            self.scan_remaining -= 1;
            HalPoll::Pending
        } else {
            HalPoll::ReadyCount(self.networks.len() as u8)
        }
    }

    fn scan_result(&self, index: u8) -> RawScanResult {
        self.networks[usize::from(index)]
    }

    fn begin_connect(
        &mut self,
        _ssid: &Ssid,
        _bssid: &[u8; BSSID_BYTES],
        _security: Security,
        _secret: &[u8],
    ) {
        self.connect_remaining = self.connect_latency();
    }

    fn poll_connect(&mut self) -> HalPoll {
        if self.connect_remaining > 0 {
            self.connect_remaining -= 1;
            HalPoll::Pending
        } else {
            self.connect_terminal
        }
    }

    fn begin_disconnect(&mut self) {
        self.disconnect_remaining = DISCONNECT_LATENCY;
    }

    fn poll_disconnect(&mut self) -> HalPoll {
        if self.disconnect_remaining > 0 {
            self.disconnect_remaining -= 1;
            HalPoll::Pending
        } else {
            HalPoll::Ready
        }
    }

    fn cancel(&mut self) {}
}

/// A fake credential provider that acknowledges forget commits.
struct FakeStore {
    available: bool,
}

impl CredentialProvider for FakeStore {
    fn available(&self) -> bool {
        self.available
    }

    fn forget(&mut self, _profile_id: u16) -> ForgetOutcome {
        ForgetOutcome::Committed
    }
}

// ------------------------------------------------------------------ validation

/// Parses and strictly validates a fixture without running any scenario.
pub fn parse_fixture(json: &str) -> Result<FakeNetworkFixture, FakeNetworkError> {
    let fixture: FakeNetworkFixture =
        serde_json::from_str(json).map_err(|e| FakeNetworkError::top(format!("parse: {e}")))?;
    fixture.validate()?;
    Ok(fixture)
}

impl FakeNetworkFixture {
    fn validate(&self) -> Result<(), FakeNetworkError> {
        if self.schema != FAKE_NETWORK_SCHEMA {
            return Err(FakeNetworkError::top(format!(
                "unknown schema {:?}",
                self.schema
            )));
        }
        if self.tick_unit_ms == 0 {
            return Err(FakeNetworkError::top("tick_unit_ms must be nonzero"));
        }
        self.limits.validate()?;
        self.validate_networks()?;
        // Reject duplicate scenario names and validate each scenario.
        let mut seen: Vec<&str> = Vec::new();
        for scenario in &self.scenarios {
            if seen.contains(&scenario.name.as_str()) {
                return Err(FakeNetworkError::top(format!(
                    "duplicate scenario name {:?}",
                    scenario.name
                )));
            }
            seen.push(&scenario.name);
            self.validate_scenario(scenario)?;
        }
        Ok(())
    }

    fn validate_networks(&self) -> Result<(), FakeNetworkError> {
        if self.networks.len() > self.limits.scan_results {
            return Err(FakeNetworkError::top("networks exceed scan_results limit"));
        }
        for (index, net) in self.networks.iter().enumerate() {
            net.decode(&self.limits)
                .map_err(|reason| FakeNetworkError::top(format!("network[{index}]: {reason}")))?;
            // Unique result ids and BSSIDs.
            if self.networks[..index]
                .iter()
                .any(|other| other.result_id == net.result_id)
            {
                return Err(FakeNetworkError::top(format!(
                    "duplicate network result_id {}",
                    net.result_id
                )));
            }
            if self.networks[..index]
                .iter()
                .any(|other| other.bssid == net.bssid)
            {
                return Err(FakeNetworkError::top("duplicate network bssid"));
            }
        }
        Ok(())
    }

    fn validate_scenario(&self, scenario: &Scenario) -> Result<(), FakeNetworkError> {
        let name = scenario.name.as_str();
        if scenario.capability_inputs.len() != 4 {
            return Err(FakeNetworkError::scoped(
                name,
                "capability_inputs must have 4 entries",
            ));
        }
        if scenario.snapshots.is_empty() {
            return Err(FakeNetworkError::scoped(name, "no snapshots"));
        }
        if scenario
            .capability_inputs
            .iter()
            .any(|&available| !available)
            && !scenario.actions.is_empty()
        {
            return Err(FakeNetworkError::scoped(
                name,
                "capability-absent scenario must have no actions",
            ));
        }

        // Actions: non-decreasing ticks, known ops, correct fields, request ids
        // present where required, valid credential cases.
        let mut last_tick = 0u64;
        let mut first = true;
        let mut submitted: Vec<(u32, u64)> = Vec::new();
        for action in &scenario.actions {
            if !first && action.tick < last_tick {
                return Err(FakeNetworkError::scoped(name, "action ticks decrease"));
            }
            first = false;
            last_tick = action.tick;
            action.validate(name, &self.networks)?;
            match action.op.as_str() {
                "set_radio" | "scan" | "connect" | "disconnect" | "forget" => {
                    let request_id = action.request_id.unwrap();
                    if request_id == 0 {
                        return Err(FakeNetworkError::scoped(
                            name,
                            "submitted request_id must be nonzero",
                        ));
                    }
                    if submitted.iter().any(|&(id, _)| id == request_id) {
                        return Err(FakeNetworkError::scoped(
                            name,
                            format!("duplicate submitted request_id {request_id}"),
                        ));
                    }
                    submitted.push((request_id, action.tick));
                }
                "cancel" | "driver_completion" => {
                    let request_id = action.request_id.unwrap();
                    if !submitted.iter().any(|&(id, _)| id == request_id) {
                        return Err(FakeNetworkError::scoped(
                            name,
                            format!("{} references unknown request_id {request_id}", action.op),
                        ));
                    }
                }
                _ => {}
            }
        }

        // Snapshots: non-decreasing ticks, known states/errors.
        let mut last_snap = 0u64;
        let mut sfirst = true;
        for snapshot in &scenario.snapshots {
            if !sfirst && snapshot.tick < last_snap {
                return Err(FakeNetworkError::scoped(name, "snapshot ticks decrease"));
            }
            sfirst = false;
            last_snap = snapshot.tick;
            if snapshot.generation == Some(0) {
                return Err(FakeNetworkError::scoped(name, "generation must be nonzero"));
            }
            if let Some(request_id) = snapshot.request_id {
                let known_by_tick = request_id == 0
                    || submitted.iter().any(|&(id, submitted_tick)| {
                        id == request_id && submitted_tick <= snapshot.tick
                    });
                if !known_by_tick {
                    return Err(FakeNetworkError::scoped(
                        name,
                        format!(
                            "snapshot at tick {} references stale request_id {request_id}",
                            snapshot.tick
                        ),
                    ));
                }
            }
            parse_state(&snapshot.state).map_err(|r| FakeNetworkError::scoped(name, r))?;
            if let Some(err) = &snapshot.error {
                parse_error(err).map_err(|r| FakeNetworkError::scoped(name, r))?;
            }
            if let Some(radio) = &snapshot.radio {
                parse_radio(radio).map_err(|r| FakeNetworkError::scoped(name, r))?;
            }
            if let Some(ids) = &snapshot.result_ids {
                if ids.len() > self.limits.scan_results {
                    return Err(FakeNetworkError::scoped(name, "result_ids over capacity"));
                }
                for (index, result_id) in ids.iter().enumerate() {
                    if !self
                        .networks
                        .iter()
                        .any(|network| network.result_id == *result_id)
                    {
                        return Err(FakeNetworkError::scoped(
                            name,
                            format!("snapshot result_id {result_id} unknown"),
                        ));
                    }
                    if ids[..index].contains(result_id) {
                        return Err(FakeNetworkError::scoped(
                            name,
                            format!("duplicate snapshot result_id {result_id}"),
                        ));
                    }
                }
            }
            if let Some(result_id) = snapshot.result_id {
                if !self
                    .networks
                    .iter()
                    .any(|network| network.result_id == result_id)
                {
                    return Err(FakeNetworkError::scoped(
                        name,
                        format!("snapshot result_id {result_id} unknown"),
                    ));
                }
            }
        }
        Ok(())
    }
}

impl Limits {
    fn validate(&self) -> Result<(), FakeNetworkError> {
        let expected = [
            ("ssid_bytes", self.ssid_bytes, SSID_MAX_BYTES),
            ("scan_results", self.scan_results, SCAN_RESULTS_MAX),
            (
                "credential_bytes",
                self.credential_bytes,
                CREDENTIAL_MAX_BYTES,
            ),
            ("status_records", self.status_records, STATUS_HISTORY_MAX),
            ("command_queue", self.command_queue, COMMAND_QUEUE_MAX),
            ("event_queue", self.event_queue, EVENT_QUEUE_MAX),
        ];
        for (field, actual, want) in expected {
            if actual != want {
                return Err(FakeNetworkError::top(format!(
                    "limit {field}={actual} must equal contract {want}"
                )));
            }
        }
        Ok(())
    }
}

impl NetworkDef {
    /// Decodes and validates the network into a `RawScanResult`.
    fn decode(&self, limits: &Limits) -> Result<RawScanResult, String> {
        if self.result_id == 0 {
            return Err("result_id must be nonzero".into());
        }
        let ssid_bytes = match (&self.ssid_utf8, &self.ssid_hex) {
            (Some(_), Some(_)) => return Err("both ssid_utf8 and ssid_hex present".into()),
            (Some(text), None) => text.as_bytes().to_vec(),
            (None, Some(hex)) => decode_hex(hex)?,
            (None, None) => return Err("missing ssid".into()),
        };
        if ssid_bytes.is_empty() || ssid_bytes.len() > limits.ssid_bytes {
            return Err("ssid length out of range".into());
        }
        let bssid = decode_bssid(&self.bssid)?;
        let security = parse_security(&self.security)?;
        Ok(RawScanResult {
            ssid: Ssid::from_bytes(&ssid_bytes),
            bssid,
            rssi_dbm: self.rssi_dbm,
            security,
        })
    }
}

impl RawAction {
    fn validate(&self, name: &str, networks: &[NetworkDef]) -> Result<(), FakeNetworkError> {
        let scoped = |r: String| FakeNetworkError::scoped(name, r);
        match self.op.as_str() {
            "set_radio" => {
                self.require_request_id(name)?;
                if self.enabled.is_none() {
                    return Err(scoped("set_radio missing enabled".into()));
                }
                self.forbid(
                    name,
                    &["result_id", "credential_case", "profile_id", "generation"],
                )?;
            }
            "scan" | "disconnect" | "cancel" => {
                self.require_request_id(name)?;
                self.forbid(
                    name,
                    &[
                        "enabled",
                        "result_id",
                        "credential_case",
                        "profile_id",
                        "generation",
                    ],
                )?;
            }
            "connect" => {
                self.require_request_id(name)?;
                let Some(result_id) = self.result_id else {
                    return Err(scoped("connect missing result_id".into()));
                };
                if !networks.iter().any(|n| n.result_id == result_id) {
                    return Err(scoped(format!("connect result_id {result_id} unknown")));
                }
                match self.credential_case.as_deref() {
                    Some("valid") | Some("invalid") => {}
                    _ => {
                        return Err(scoped(
                            "connect credential_case must be valid/invalid".into(),
                        ));
                    }
                }
                self.forbid(name, &["enabled", "profile_id", "generation"])?;
            }
            "forget" => {
                self.require_request_id(name)?;
                if self.profile_id.is_none() {
                    return Err(scoped("forget missing profile_id".into()));
                }
                self.forbid(
                    name,
                    &["enabled", "result_id", "credential_case", "generation"],
                )?;
            }
            "lose_radio" => {
                self.forbid(
                    name,
                    &[
                        "request_id",
                        "enabled",
                        "result_id",
                        "credential_case",
                        "profile_id",
                        "generation",
                    ],
                )?;
            }
            "driver_completion" => {
                if self.generation.is_none() || self.request_id.is_none() {
                    return Err(scoped(
                        "driver_completion needs generation+request_id".into(),
                    ));
                }
                self.forbid(
                    name,
                    &["enabled", "result_id", "credential_case", "profile_id"],
                )?;
            }
            other => return Err(scoped(format!("unknown op {other:?}"))),
        }
        Ok(())
    }

    fn require_request_id(&self, name: &str) -> Result<(), FakeNetworkError> {
        if self.request_id.is_none() {
            return Err(FakeNetworkError::scoped(
                name,
                format!("{} missing request_id", self.op),
            ));
        }
        Ok(())
    }

    fn forbid(&self, name: &str, fields: &[&str]) -> Result<(), FakeNetworkError> {
        for field in fields {
            let present = match *field {
                "request_id" => self.request_id.is_some(),
                "enabled" => self.enabled.is_some(),
                "result_id" => self.result_id.is_some(),
                "credential_case" => self.credential_case.is_some(),
                "profile_id" => self.profile_id.is_some(),
                "generation" => self.generation.is_some(),
                _ => false,
            };
            if present {
                return Err(FakeNetworkError::scoped(
                    name,
                    format!("{} must not carry {field}", self.op),
                ));
            }
        }
        Ok(())
    }
}

// -------------------------------------------------------------------- replay

/// Parses, validates, and replays every scenario, returning the scenario names
/// that were exercised. Any mismatch is a hard error.
pub fn replay_all(json: &str) -> Result<Vec<String>, FakeNetworkError> {
    let fixture = parse_fixture(json)?;
    let mut names = Vec::new();
    for scenario in &fixture.scenarios {
        fixture.replay(scenario)?;
        names.push(scenario.name.clone());
    }
    Ok(names)
}

/// Replays every scenario and returns the real service observations captured at
/// each declared snapshot boundary. This is the KotoConfig development hook:
/// callers receive only public snapshots and retained scan results.
pub fn replay_trace(json: &str) -> Result<Vec<ReplayObservation>, FakeNetworkError> {
    let fixture = parse_fixture(json)?;
    let mut observations = Vec::new();
    for scenario in &fixture.scenarios {
        observations.extend(fixture.replay(scenario)?);
    }
    Ok(observations)
}

impl FakeNetworkFixture {
    fn replay(&self, scenario: &Scenario) -> Result<Vec<ReplayObservation>, FakeNetworkError> {
        let name = scenario.name.as_str();
        // A false capability input gates the whole page: the service is not
        // usable and the page shows RadioUnavailable with a mapped reason.
        if let Some(index) = scenario.capability_inputs.iter().position(|ok| !ok) {
            return self.replay_capability_gate(scenario, index);
        }

        let networks: Vec<RawScanResult> = self
            .networks
            .iter()
            .map(|n| n.decode(&self.limits).unwrap())
            .collect();
        let mut svc = NetworkService::new();
        let mut hal = FakeRadio::new(networks);
        let mut store = FakeStore { available: true };

        // Prologue: unless the scenario enables the radio itself, bring the radio
        // up and seed a scan so connect/forget preconditions hold.
        let self_starts = scenario
            .actions
            .first()
            .map(|a| a.op == "set_radio")
            .unwrap_or(false);
        if !self_starts {
            prologue(&mut svc, &mut hal, &mut store)
                .map_err(|r| FakeNetworkError::scoped(name, r))?;
        }
        // Drop any prologue events so the scenario starts from a clean stream.
        drain_last(&mut svc);

        let max_tick = scenario
            .actions
            .iter()
            .map(|a| a.tick)
            .chain(scenario.snapshots.iter().map(|s| s.tick))
            .max()
            .unwrap_or(0);

        // Maps a fixture request id to the service-assigned id at submit time,
        // since a prologue consumes ids and the two counters can differ.
        let mut request_map: Vec<(u32, u32)> = Vec::new();
        // The request id of the most recent emitted event, which the fixture's
        // per-snapshot `request_id` describes (the live snapshot field is zero
        // once the op completes).
        let mut last_request_id = 0u32;

        let mut action_cursor = 0usize;
        let mut snapshot_cursor = 0usize;
        let mut observations = Vec::new();
        for tick in 0..=max_tick {
            let now_ms = tick * self.tick_unit_ms;
            while action_cursor < scenario.actions.len()
                && scenario.actions[action_cursor].tick == tick
            {
                let action = &scenario.actions[action_cursor];
                apply_action(&mut svc, &mut hal, action, &mut request_map)
                    .map_err(|r| FakeNetworkError::scoped(name, r))?;
                action_cursor += 1;
            }

            svc.service(now_ms, WORK_BUDGET, &mut hal, &mut store);
            if let Some(rid) = drain_last(&mut svc) {
                last_request_id = rid;
            }

            while snapshot_cursor < scenario.snapshots.len()
                && scenario.snapshots[snapshot_cursor].tick == tick
            {
                let expect = &scenario.snapshots[snapshot_cursor];
                if let Some(want) = expect.request_id {
                    if want == 0 {
                        let actual = svc.snapshot().request_id;
                        if actual != 0 {
                            return Err(FakeNetworkError::scoped(
                                name,
                                format!("tick {tick}: active request_id {actual} != 0"),
                            ));
                        }
                    } else {
                        let want = map_request(&request_map, want)
                            .map_err(|r| FakeNetworkError::scoped(name, r))?;
                        if last_request_id != want {
                            return Err(FakeNetworkError::scoped(
                                name,
                                format!("tick {tick}: request_id {last_request_id} != {want}"),
                            ));
                        }
                    }
                }
                assert_snapshot(&svc, &store, expect)
                    .map_err(|r| FakeNetworkError::scoped(name, format!("tick {tick}: {r}")))?;
                observations.push(ReplayObservation {
                    scenario: scenario.name.clone(),
                    tick,
                    snapshot: svc.snapshot(),
                    results: svc.results().copied().collect(),
                });
                snapshot_cursor += 1;
            }
        }

        if action_cursor != scenario.actions.len() {
            return Err(FakeNetworkError::scoped(name, "unconsumed actions"));
        }
        if snapshot_cursor != scenario.snapshots.len() {
            return Err(FakeNetworkError::scoped(name, "unchecked snapshots"));
        }
        Ok(observations)
    }

    fn replay_capability_gate(
        &self,
        scenario: &Scenario,
        false_index: usize,
    ) -> Result<Vec<ReplayObservation>, FakeNetworkError> {
        let name = scenario.name.as_str();
        if !scenario.actions.is_empty() {
            return Err(FakeNetworkError::scoped(
                name,
                "capability-absent scenario must have no actions",
            ));
        }
        let expected_error = match false_index {
            1 => "FirmwareUnavailable",
            3 => "CredentialStoreUnavailable",
            _ => "RadioUnavailable",
        };
        let mut observations = Vec::new();
        for snapshot in &scenario.snapshots {
            if snapshot.state != "RadioUnavailable" {
                return Err(FakeNetworkError::scoped(
                    name,
                    "capability gate must stay RadioUnavailable",
                ));
            }
            match snapshot.error.as_deref() {
                Some(err) if err == expected_error => {}
                other => {
                    return Err(FakeNetworkError::scoped(
                        name,
                        format!("capability gate error {other:?} != {expected_error}"),
                    ));
                }
            }
            observations.push(ReplayObservation {
                scenario: scenario.name.clone(),
                tick: snapshot.tick,
                snapshot: NetworkSnapshot {
                    generation: snapshot.generation.unwrap_or(1),
                    request_id: snapshot.request_id.unwrap_or(0),
                    radio: RadioState::Unavailable,
                    state: OperationState::RadioUnavailable,
                    connected_result_id: None,
                    result_count: 0,
                    retry_count: 0,
                    deadline_ms_remaining: 0,
                    last_error: Some(parse_error(expected_error).unwrap()),
                    command_depth: 0,
                    event_depth: 0,
                },
                results: Vec::new(),
            });
        }
        Ok(observations)
    }
}

/// Enables the radio and performs one scan so the service reaches `Results`.
fn prologue(
    svc: &mut NetworkService,
    hal: &mut FakeRadio,
    store: &mut FakeStore,
) -> Result<(), String> {
    if !matches!(svc.set_radio(true), SubmitResult::Accepted(_)) {
        return Err("prologue set_radio rejected".into());
    }
    pump_until(svc, hal, store, |s| {
        s.snapshot().radio == RadioState::Enabled
    })?;
    if !matches!(svc.scan(), SubmitResult::Accepted(_)) {
        return Err("prologue scan rejected".into());
    }
    pump_until(svc, hal, store, |s| {
        s.snapshot().state == OperationState::Results
    })?;
    Ok(())
}

fn pump_until(
    svc: &mut NetworkService,
    hal: &mut FakeRadio,
    store: &mut FakeStore,
    done: impl Fn(&NetworkService) -> bool,
) -> Result<(), String> {
    for _ in 0..32 {
        svc.service(0, WORK_BUDGET, hal, store);
        if done(svc) {
            return Ok(());
        }
    }
    Err("prologue did not settle".into())
}

/// Applies one fixture action to the service or driver.
fn apply_action(
    svc: &mut NetworkService,
    hal: &mut FakeRadio,
    action: &RawAction,
    request_map: &mut Vec<(u32, u32)>,
) -> Result<(), String> {
    match action.op.as_str() {
        "set_radio" => {
            let actual = expect_accepted(svc.set_radio(action.enabled.unwrap()), "set_radio")?;
            record_request(request_map, action.request_id.unwrap(), actual)?;
        }
        "scan" => {
            let actual = expect_accepted(svc.scan(), "scan")?;
            record_request(request_map, action.request_id.unwrap(), actual)?;
        }
        "connect" => {
            let valid = action.credential_case.as_deref() == Some("valid");
            let result_id = action.result_id.unwrap();
            // Match the credential to the target's advertised security so the
            // service accepts submission; the fake HAL decides the outcome.
            let security = svc
                .results()
                .find(|r| r.result_id == result_id)
                .map(|r| r.security)
                .ok_or_else(|| format!("connect result_id {result_id} not in scan results"))?;
            hal.arm_connect(valid);
            let secret: &[u8] = match security {
                Security::Open => &[],
                Security::Wpa2PersonalAes => b"password1",
            };
            let view = CredentialView { security, secret };
            let actual = expect_accepted(svc.connect(result_id, view), "connect")?;
            record_request(request_map, action.request_id.unwrap(), actual)?;
        }
        "disconnect" => {
            let actual = expect_accepted(svc.disconnect(), "disconnect")?;
            record_request(request_map, action.request_id.unwrap(), actual)?;
        }
        "cancel" => {
            let request_id = map_request(request_map, action.request_id.unwrap())?;
            expect_accepted(svc.cancel(request_id, hal), "cancel")?;
        }
        "forget" => {
            let actual = expect_accepted(svc.forget(action.profile_id.unwrap()), "forget")?;
            record_request(request_map, action.request_id.unwrap(), actual)?;
        }
        "lose_radio" => {
            hal.present = false;
        }
        "driver_completion" => {
            let request_id = map_request(request_map, action.request_id.unwrap())?;
            svc.note_late_completion(action.generation.unwrap(), request_id);
        }
        other => return Err(format!("cannot apply op {other:?}")),
    }
    Ok(())
}

fn expect_accepted(result: SubmitResult, op: &str) -> Result<u32, String> {
    match result {
        SubmitResult::Accepted(request_id) => Ok(request_id),
        other => Err(format!("{op} not accepted: {other:?}")),
    }
}

fn record_request(
    request_map: &mut Vec<(u32, u32)>,
    fixture_id: u32,
    actual_id: u32,
) -> Result<(), String> {
    if request_map.iter().any(|&(id, _)| id == fixture_id) {
        return Err(format!("duplicate submitted request_id {fixture_id}"));
    }
    request_map.push((fixture_id, actual_id));
    Ok(())
}

fn map_request(request_map: &[(u32, u32)], fixture_id: u32) -> Result<u32, String> {
    request_map
        .iter()
        .find_map(|&(fixture, actual)| (fixture == fixture_id).then_some(actual))
        .ok_or_else(|| format!("request_id {fixture_id} has not been submitted"))
}

/// Drains the fixed event FIFO and returns the most recently emitted request.
fn drain_last(svc: &mut NetworkService) -> Option<u32> {
    let mut last = None;
    while let Some(event) = svc.poll_event() {
        last = Some(event.request_id);
    }
    last
}

/// Asserts the live service snapshot matches a fixture snapshot.
fn assert_snapshot(
    svc: &NetworkService,
    store: &FakeStore,
    expect: &RawSnapshot,
) -> Result<(), String> {
    let snap = svc.snapshot();

    let want_state = parse_state(&expect.state)?;
    if snap.state != want_state {
        return Err(format!("state {:?} != {:?}", snap.state, want_state));
    }
    if let Some(generation) = expect.generation {
        if snap.generation != generation {
            return Err(format!("generation {} != {generation}", snap.generation));
        }
    }
    if let Some(radio) = &expect.radio {
        let want = parse_radio(radio)?;
        if snap.radio != want {
            return Err(format!("radio {:?} != {:?}", snap.radio, want));
        }
    }
    match &expect.error {
        Some(err) => {
            let want = parse_error(err)?;
            if snap.last_error != Some(want) {
                return Err(format!("error {:?} != {:?}", snap.last_error, Some(want)));
            }
        }
        None => {
            // `error: null` asserts no error is surfaced for this state.
            if snap.last_error.is_some() && snap.state != OperationState::Failed {
                return Err(format!("unexpected error {:?}", snap.last_error));
            }
        }
    }
    if let Some(result_id) = expect.result_id {
        if snap.connected_result_id != Some(result_id) {
            return Err(format!(
                "connected_result_id {:?} != {:?}",
                snap.connected_result_id,
                Some(result_id)
            ));
        }
    }
    if let Some(ids) = &expect.result_ids {
        let actual: Vec<u16> = svc.results().map(|r| r.result_id).collect();
        if &actual != ids {
            return Err(format!("result_ids {actual:?} != {ids:?}"));
        }
    }
    if let Some(retry) = expect.retry {
        if snap.retry_count != retry {
            return Err(format!("retry {} != {retry}", snap.retry_count));
        }
    }
    if expect.credential_staging_zeroized == Some(true) && !svc.credential_staging_zeroized() {
        return Err("credential staging not zeroized".into());
    }
    if expect.secret_commit_acknowledged == Some(true) && !svc.last_forget_committed() {
        return Err("forget commit not acknowledged".into());
    }
    if expect.late_completion_discarded == Some(true) && svc.late_completions_discarded() == 0 {
        return Err("late completion not discarded".into());
    }
    let _ = store;
    Ok(())
}

// ------------------------------------------------------------------ enum maps

fn parse_state(text: &str) -> Result<OperationState, String> {
    Ok(match text {
        "RadioUnavailable" => OperationState::RadioUnavailable,
        "Disabled" => OperationState::Disabled,
        "RadioEnabling" => OperationState::RadioEnabling,
        "RadioDisabling" => OperationState::RadioDisabling,
        "Scanning" => OperationState::Scanning,
        "Results" => OperationState::Results,
        "Connecting" => OperationState::Connecting,
        "Connected" => OperationState::Connected,
        "Disconnecting" => OperationState::Disconnecting,
        "Forgetting" => OperationState::Forgetting,
        "Failed" => OperationState::Failed,
        other => return Err(format!("unknown state {other:?}")),
    })
}

fn parse_radio(text: &str) -> Result<RadioState, String> {
    Ok(match text {
        "Disabled" => RadioState::Disabled,
        "Enabling" => RadioState::Enabling,
        "Enabled" => RadioState::Enabled,
        "Disabling" => RadioState::Disabling,
        "Unavailable" => RadioState::Unavailable,
        other => return Err(format!("unknown radio {other:?}")),
    })
}

fn parse_error(text: &str) -> Result<koto_core::net::NetworkError, String> {
    use koto_core::net::NetworkError as E;
    Ok(match text {
        "Busy" => E::Busy,
        "InvalidInput" => E::InvalidInput,
        "UnsupportedSecurity" => E::UnsupportedSecurity,
        "RadioUnavailable" => E::RadioUnavailable,
        "FirmwareUnavailable" => E::FirmwareUnavailable,
        "CredentialStoreUnavailable" => E::CredentialStoreUnavailable,
        "AuthenticationFailed" => E::AuthenticationFailed,
        "NetworkNotFound" => E::NetworkNotFound,
        "LinkLost" => E::LinkLost,
        "Timeout" => E::Timeout,
        "Cancelled" => E::Cancelled,
        "StorageCorrupt" => E::StorageCorrupt,
        "Internal" => E::Internal,
        other => return Err(format!("unknown error {other:?}")),
    })
}

fn parse_security(text: &str) -> Result<Security, String> {
    match text {
        "Open" => Ok(Security::Open),
        "Wpa2PersonalAes" => Ok(Security::Wpa2PersonalAes),
        other => Err(format!("unsupported security {other:?}")),
    }
}

fn decode_hex(hex: &str) -> Result<Vec<u8>, String> {
    if !hex.len().is_multiple_of(2) {
        return Err("hex length odd".into());
    }
    let mut out = Vec::with_capacity(hex.len() / 2);
    let bytes = hex.as_bytes();
    for pair in bytes.chunks(2) {
        let hi = hex_nibble(pair[0])?;
        let lo = hex_nibble(pair[1])?;
        out.push((hi << 4) | lo);
    }
    Ok(out)
}

fn hex_nibble(byte: u8) -> Result<u8, String> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err("invalid hex digit".into()),
    }
}

fn decode_bssid(text: &str) -> Result<[u8; BSSID_BYTES], String> {
    let parts: Vec<&str> = text.split(':').collect();
    if parts.len() != BSSID_BYTES {
        return Err("bssid must be 6 octets".into());
    }
    let mut out = [0u8; BSSID_BYTES];
    for (index, part) in parts.iter().enumerate() {
        if part.len() != 2 {
            return Err("bssid octet must be 2 hex digits".into());
        }
        let bytes = part.as_bytes();
        out[index] = (hex_nibble(bytes[0])? << 4) | hex_nibble(bytes[1])?;
    }
    Ok(out)
}
