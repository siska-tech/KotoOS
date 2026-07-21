//! Bounded application network credential vault (KOTO-0248).
//!
//! This module implements the OS-owned credential store and grant model frozen
//! by [`docs/architecture/APP_CREDENTIAL_VAULT.md`]. It keeps at most
//! [`MAX_GRANTS`] application service credentials in a **separate**, versioned,
//! checksummed secret namespace (`KAV1`) and fails closed on missing, corrupt,
//! torn, or unsupported data. It is `no_std`, uses no general heap, and owns
//! only fixed-capacity storage.
//!
//! ## What an application can and cannot do
//!
//! An application never sees a secret byte. It holds only an opaque
//! [`CredentialHandle`], a generation-tagged token minted by the OS for one
//! grant. The vault has no read, enumerate, copy, or export call: the only way a
//! secret leaves this module is as an operation-scoped [`CredentialInjection`]
//! borrow handed to the OS-private Fetch/MQTT transport, which copies it into a
//! transient buffer and zeroizes that. Secrets never enter VM memory, an app
//! read buffer, a response, or a diagnostic.
//!
//! ## Scope binding (KOTO-0248 threat model)
//!
//! A grant binds a secret to an exact triple: the canonical package
//! [`app_id`](crate::package::validate_app_id), a [`ServiceKind`], and a
//! canonical TLS [`VaultEndpoint`]. A package update, an app-id change, or an
//! origin change does not match an existing grant, so it cannot silently
//! broaden one. Resolving a handle re-checks the requesting app id, the service,
//! and the exact destination endpoint before any secret is touched, and refuses
//! a plain-HTTP / non-TLS request outright.
//!
//! ## Disclosure limits (no hardware-backed confidentiality)
//!
//! The applicable Pico W / Pico 2 W boards provide no KotoOS-managed,
//! non-exportable key, so this store does not claim confidentiality of stored
//! bytes. Secrets are held at rest without encryption; **physical access to the
//! SD card or flash can recover credentials** despite volatile zeroization of
//! RAM and logical two-slot erasure. The FNV-1a checksum detects accidental
//! corruption and torn writes; it is not an authentication tag against a
//! deliberate offline forger.
//!
//! ## Two-slot commit
//!
//! The whole grant set serializes to one fixed-size record
//! ([`VAULT_RECORD_BYTES`]). Two medium slots ([`VaultSlot::A`],
//! [`VaultSlot::B`]) hold successive generations; a write always targets the
//! slot that is *not* currently newest-valid, so an interrupted (torn) write
//! leaves the previous good slot intact. Load decodes both slots, verifies
//! magic/version/length/checksum/generation, and selects the newest valid one
//! with wrap-aware serial-number comparison. Corruption disables the vault.

use crate::package::{validate_app_id, MAX_APP_ID_LEN};

// ------------------------------------------------------------------ capacities

/// Maximum retained grants.
pub const MAX_GRANTS: usize = 4;
/// Maximum canonical endpoint host length in octets.
pub const MAX_ENDPOINT_HOST_BYTES: usize = 128;
/// Maximum secret length in octets (bearer token, API key, or MQTT password).
pub const MAX_SECRET_BYTES: usize = 192;
/// Maximum API-key header name length in octets.
pub const MAX_HEADER_NAME_BYTES: usize = 32;
/// Maximum MQTT username length in octets.
pub const MAX_USERNAME_BYTES: usize = 48;
/// Shared auxiliary field width (header name or username).
const AUX_FIELD: usize = MAX_USERNAME_BYTES; // 48, >= MAX_HEADER_NAME_BYTES

/// The `service` selector an application passes to the `vault_handle` host call
/// (Host ABI minor 22, KOTO-0248), surfaced to bytecode as the `VAULT_SERVICE_*`
/// SDK constants. The values equal the on-wire [`ServiceKind`] tags so the SDK
/// cannot drift from the store's encoding.
pub mod app_vault {
    /// Fetch (HTTPS) credential scope.
    pub const SERVICE_FETCH: i32 = super::SERVICE_FETCH as i32;
    /// MQTT (MQTTS) credential scope.
    pub const SERVICE_MQTT: i32 = super::SERVICE_MQTT as i32;
}

// ------------------------------------------------------------------ format

/// Vault-record magic. Distinct from `KWS1` (Wi-Fi), `KCF1` (settings), and
/// `KUC1` so no decoder can confuse the namespaces.
pub const VAULT_MAGIC: [u8; 4] = *b"KAV1";
const FORMAT_MAJOR: u16 = 1;
const FORMAT_MINOR: u16 = 0;

/// Record header length in octets.
const HEADER_LEN: usize = 32;

// Per-grant field offsets (relative to a grant record start).
const G_ID: usize = 0; // u16
const G_GENERATION: usize = 2; // u16
const G_SERVICE: usize = 4; // u8
const G_KIND: usize = 5; // u8
const G_APP_ID_LEN: usize = 6; // u8
const G_HOST_LEN: usize = 7; // u8
const G_AUX_LEN: usize = 8; // u8
const G_SECRET_LEN: usize = 9; // u8
const G_PORT: usize = 10; // u16
const G_RESERVED: usize = 12; // 4 bytes, must be zero
const G_APP_ID: usize = 16;
const G_HOST: usize = G_APP_ID + MAX_APP_ID_LEN; // 80
const G_AUX: usize = G_HOST + MAX_ENDPOINT_HOST_BYTES; // 208
const G_SECRET: usize = G_AUX + AUX_FIELD; // 256
/// Fixed per-grant record stride in octets.
const GRANT_STRIDE: usize = G_SECRET + MAX_SECRET_BYTES; // 448

/// Exact serialized size of a complete vault record, and therefore the exact
/// read/write size of a medium slot.
pub const VAULT_RECORD_BYTES: usize = HEADER_LEN + MAX_GRANTS * GRANT_STRIDE;

// Record header field offsets.
const OFF_MAGIC: usize = 0; // 4
const OFF_MAJOR: usize = 4; // u16
const OFF_MINOR: usize = 6; // u16
const OFF_TOTAL_LEN: usize = 8; // u32
const OFF_COUNT: usize = 12; // u8
const OFF_RESERVED0: usize = 13; // u8, must be zero
const OFF_STRIDE: usize = 14; // u16
const OFF_GENERATION: usize = 16; // u32
const OFF_CHECKSUM: usize = 20; // u32
const OFF_RESERVED: usize = 24; // 8 bytes, must be zero

const SERVICE_FETCH: u8 = 0;
const SERVICE_MQTT: u8 = 1;

const KIND_BEARER: u8 = 0;
const KIND_API_KEY: u8 = 1;
const KIND_MQTT_LOGIN: u8 = 2;

// ------------------------------------------------------------------ errors

/// Why a vault operation failed. Values are fixed enums carrying no app id,
/// host, header, username, secret, or address bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VaultError {
    /// A supplied buffer was smaller than [`VAULT_RECORD_BYTES`].
    BufferTooSmall,
    /// The record did not begin with [`VAULT_MAGIC`].
    BadMagic,
    /// The format major/minor was not understood.
    UnsupportedVersion,
    /// The record was truncated, carried trailing bytes, or declared a bad size.
    InvalidLength,
    /// The stored checksum did not match the record contents.
    InvalidChecksum,
    /// The record generation was zero (reserved as "never written").
    InvalidGeneration,
    /// The record declared more than [`MAX_GRANTS`] grants.
    TooManyGrants,
    /// Two stored grants shared the same identity (app id + service + endpoint).
    DuplicateGrant,
    /// A reserved field, padding byte, or empty-slot region was non-zero, or a
    /// populated grant appeared after an empty one.
    InvalidRecord,
    /// The app id was empty, too long, or not a canonical package id.
    InvalidAppId,
    /// The service byte was not one of the supported services.
    UnsupportedService,
    /// The credential kind was not valid, or not valid for the service.
    InvalidKind,
    /// The endpoint host or port was empty, too long, or non-canonical.
    InvalidEndpoint,
    /// The auxiliary field (header name or username) was invalid for the kind.
    InvalidAux,
    /// The secret was empty, too long, or carried invalid bytes for the kind.
    InvalidSecret,
    /// No free grant slot remained for a new identity.
    StoreFull,
    /// The vault namespace is unavailable (corrupt or not yet loaded).
    Unavailable,
    /// The requested grant identity or handle grant id was not present.
    NotFound,
    /// The handle's generation did not match the current grant generation.
    StaleHandle,
    /// The requesting app id did not own the grant.
    ForeignApp,
    /// The grant is for a different service than the request.
    ServiceMismatch,
    /// The request destination did not exactly equal the grant endpoint.
    EndpointMismatch,
    /// The request was not over an authenticated (TLS) transport.
    InsecureEndpoint,
    /// The backing medium reported a read/write/erase failure.
    MediumError,
}

// ------------------------------------------------------------------ public model

/// Which network service a credential is scoped to. A Fetch grant can never be
/// used on MQTT or vice versa.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceKind {
    Fetch,
    Mqtt,
}

impl ServiceKind {
    const fn to_byte(self) -> u8 {
        match self {
            ServiceKind::Fetch => SERVICE_FETCH,
            ServiceKind::Mqtt => SERVICE_MQTT,
        }
    }

    const fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            SERVICE_FETCH => Some(ServiceKind::Fetch),
            SERVICE_MQTT => Some(ServiceKind::Mqtt),
            _ => None,
        }
    }
}

/// How the OS injects the secret. Determines the transport format, never what
/// the application can observe.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CredentialKind {
    /// Fetch: `Authorization: Bearer <secret>`. No auxiliary field.
    BearerToken,
    /// Fetch: `<name>: <secret>` header; the auxiliary field holds `name`.
    ApiKeyHeader,
    /// MQTT: username (clear) + password (secret) in CONNECT; the auxiliary
    /// field holds the username.
    MqttLogin,
}

impl CredentialKind {
    const fn to_byte(self) -> u8 {
        match self {
            CredentialKind::BearerToken => KIND_BEARER,
            CredentialKind::ApiKeyHeader => KIND_API_KEY,
            CredentialKind::MqttLogin => KIND_MQTT_LOGIN,
        }
    }

    const fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            KIND_BEARER => Some(CredentialKind::BearerToken),
            KIND_API_KEY => Some(CredentialKind::ApiKeyHeader),
            KIND_MQTT_LOGIN => Some(CredentialKind::MqttLogin),
            _ => None,
        }
    }

    /// Whether this kind is valid for the given service.
    const fn allows(self, service: ServiceKind) -> bool {
        matches!(
            (self, service),
            (CredentialKind::BearerToken, ServiceKind::Fetch)
                | (CredentialKind::ApiKeyHeader, ServiceKind::Fetch)
                | (CredentialKind::MqttLogin, ServiceKind::Mqtt)
        )
    }
}

/// A canonical `(host, port)` endpoint. The scheme is implied by the grant's
/// [`ServiceKind`] and is always the authenticated one (`https` / `mqtts`); a
/// non-TLS request is refused at injection. The host array beyond `host_len` is
/// always zero, so the derived equality is exact.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct VaultEndpoint {
    host: [u8; MAX_ENDPOINT_HOST_BYTES],
    host_len: u8,
    port: u16,
}

impl VaultEndpoint {
    /// Builds a canonical endpoint from a lower-ASCII DNS host and a nonzero
    /// port. Wildcards, upper case, empty labels, IP-literal brackets, and
    /// over-length hosts fail closed.
    pub fn new(host: &str, port: u16) -> Result<Self, VaultError> {
        if port == 0 {
            return Err(VaultError::InvalidEndpoint);
        }
        if host.is_empty() || host.len() > MAX_ENDPOINT_HOST_BYTES {
            return Err(VaultError::InvalidEndpoint);
        }
        if !is_canonical_hostname(host) {
            return Err(VaultError::InvalidEndpoint);
        }
        let mut bytes = [0u8; MAX_ENDPOINT_HOST_BYTES];
        bytes[..host.len()].copy_from_slice(host.as_bytes());
        Ok(Self {
            host: bytes,
            host_len: host.len() as u8,
            port,
        })
    }

    pub fn host(&self) -> &str {
        core::str::from_utf8(&self.host[..usize::from(self.host_len)]).unwrap_or("")
    }

    pub const fn port(&self) -> u16 {
        self.port
    }

    fn host_bytes(&self) -> &[u8] {
        &self.host[..usize::from(self.host_len)]
    }
}

impl core::fmt::Debug for VaultEndpoint {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // Never print the host bytes; they are threat-model scope, not a secret,
        // but the redaction discipline is uniform across the vault.
        f.debug_struct("VaultEndpoint")
            .field("host_len", &self.host_len)
            .field("port", &self.port)
            .finish()
    }
}

/// An opaque, generation-tagged handle to one grant. Meaningless to bytecode: it
/// cannot be reversed to bytes, enumerated, or used by another app. The low 16
/// bits are the grant id; the high 16 bits are the grant generation at mint.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct CredentialHandle(u32);

impl CredentialHandle {
    /// Reconstruct an untrusted VM-supplied handle. The vault still checks its
    /// generation, the owning app id, the service, and the endpoint before use.
    pub const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> u32 {
        self.0
    }

    const fn new(generation: u16, grant_id: u16) -> Self {
        Self((generation as u32) << 16 | grant_id as u32)
    }

    const fn grant_id(self) -> u16 {
        (self.0 & 0xffff) as u16
    }

    const fn generation(self) -> u16 {
        (self.0 >> 16) as u16
    }
}

impl core::fmt::Debug for CredentialHandle {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CredentialHandle")
            .field("grant_id", &self.grant_id())
            .field("generation", &self.generation())
            .finish()
    }
}

/// An operation-scoped secret borrow handed to the OS-private transport. The
/// transport copies the bytes into a transient buffer and zeroizes that; the
/// borrow never crosses into VM memory or a diagnostic.
#[derive(Clone, Copy)]
pub enum CredentialInjection<'a> {
    /// Fetch: send `Authorization: Bearer <token>`.
    BearerToken { token: &'a [u8] },
    /// Fetch: send `<name>: <value>`.
    ApiKeyHeader { name: &'a [u8], value: &'a [u8] },
    /// MQTT: send `username` and `password` in CONNECT.
    MqttLogin {
        username: &'a [u8],
        password: &'a [u8],
    },
}

impl core::fmt::Debug for CredentialInjection<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // Redact every value byte; expose only the shape.
        match self {
            CredentialInjection::BearerToken { .. } => f.write_str("BearerToken(..)"),
            CredentialInjection::ApiKeyHeader { .. } => f.write_str("ApiKeyHeader(..)"),
            CredentialInjection::MqttLogin { .. } => f.write_str("MqttLogin(..)"),
        }
    }
}

/// Redacted, secret-free grant metadata for the management page and
/// diagnostics. Carries no secret, header value, username, or host bytes.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct GrantInfo {
    pub grant_id: u16,
    pub generation: u16,
    pub service: ServiceKind,
    pub kind: CredentialKind,
    pub app_id_len: u8,
    pub endpoint_host_len: u8,
    pub endpoint_port: u16,
    pub secret_len: u8,
}

impl core::fmt::Debug for GrantInfo {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GrantInfo")
            .field("grant_id", &self.grant_id)
            .field("generation", &self.generation)
            .field("service", &self.service)
            .field("kind", &self.kind)
            .field("app_id_len", &self.app_id_len)
            .field("endpoint_host_len", &self.endpoint_host_len)
            .field("endpoint_port", &self.endpoint_port)
            .field("secret_len", &self.secret_len)
            .finish()
    }
}

// ------------------------------------------------------------------ internal grant

/// A retained grant including its secret. Never leaves this module by value; its
/// `Debug` is redacted and its `Drop` volatile-zeroizes the secret and aux.
struct Grant {
    grant_id: u16,
    generation: u16,
    service: ServiceKind,
    kind: CredentialKind,
    app_id: [u8; MAX_APP_ID_LEN],
    app_id_len: u8,
    endpoint: VaultEndpoint,
    aux: [u8; AUX_FIELD],
    aux_len: u8,
    secret: [u8; MAX_SECRET_BYTES],
    secret_len: u8,
}

impl Grant {
    fn app_id(&self) -> &[u8] {
        &self.app_id[..usize::from(self.app_id_len)]
    }

    fn aux(&self) -> &[u8] {
        &self.aux[..usize::from(self.aux_len)]
    }

    fn secret(&self) -> &[u8] {
        &self.secret[..usize::from(self.secret_len)]
    }

    fn identity_matches(
        &self,
        app_id: &[u8],
        service: ServiceKind,
        endpoint: &VaultEndpoint,
    ) -> bool {
        self.service == service && self.app_id() == app_id && self.endpoint == *endpoint
    }

    fn info(&self) -> GrantInfo {
        GrantInfo {
            grant_id: self.grant_id,
            generation: self.generation,
            service: self.service,
            kind: self.kind,
            app_id_len: self.app_id_len,
            endpoint_host_len: self.endpoint.host_len,
            endpoint_port: self.endpoint.port,
            secret_len: self.secret_len,
        }
    }

    fn injection(&self) -> CredentialInjection<'_> {
        match self.kind {
            CredentialKind::BearerToken => CredentialInjection::BearerToken {
                token: self.secret(),
            },
            CredentialKind::ApiKeyHeader => CredentialInjection::ApiKeyHeader {
                name: self.aux(),
                value: self.secret(),
            },
            CredentialKind::MqttLogin => CredentialInjection::MqttLogin {
                username: self.aux(),
                password: self.secret(),
            },
        }
    }
}

impl core::fmt::Debug for Grant {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Grant")
            .field("grant_id", &self.grant_id)
            .field("generation", &self.generation)
            .field("service", &self.service)
            .field("kind", &self.kind)
            .field("app_id_len", &self.app_id_len)
            .field("secret_len", &self.secret_len)
            .finish()
    }
}

impl Drop for Grant {
    fn drop(&mut self) {
        volatile_zeroize(&mut self.secret);
        volatile_zeroize(&mut self.aux);
        self.secret_len = 0;
        self.aux_len = 0;
    }
}

// ------------------------------------------------------------------ staging

/// Volatile edit staging: a candidate grant assembled by the consent flow before
/// it is committed. Zeroized at every terminal boundary.
struct EditStaging {
    active: bool,
    grant_id_hint: Option<u16>,
    service: ServiceKind,
    kind: CredentialKind,
    app_id: [u8; MAX_APP_ID_LEN],
    app_id_len: u8,
    endpoint: VaultEndpoint,
    aux: [u8; AUX_FIELD],
    aux_len: u8,
    secret: [u8; MAX_SECRET_BYTES],
    secret_len: u8,
}

impl EditStaging {
    fn new() -> Self {
        Self {
            active: false,
            grant_id_hint: None,
            service: ServiceKind::Fetch,
            kind: CredentialKind::BearerToken,
            app_id: [0; MAX_APP_ID_LEN],
            app_id_len: 0,
            endpoint: EMPTY_ENDPOINT,
            aux: [0; AUX_FIELD],
            aux_len: 0,
            secret: [0; MAX_SECRET_BYTES],
            secret_len: 0,
        }
    }

    fn zeroize(&mut self) {
        volatile_zeroize(&mut self.secret);
        volatile_zeroize(&mut self.aux);
        volatile_zeroize(&mut self.app_id);
        self.secret_len = 0;
        self.aux_len = 0;
        self.app_id_len = 0;
        self.active = false;
        self.grant_id_hint = None;
        self.service = ServiceKind::Fetch;
        self.kind = CredentialKind::BearerToken;
        self.endpoint = EMPTY_ENDPOINT;
    }
}

impl Drop for EditStaging {
    fn drop(&mut self) {
        volatile_zeroize(&mut self.secret);
        volatile_zeroize(&mut self.aux);
        volatile_zeroize(&mut self.app_id);
        self.secret_len = 0;
        self.aux_len = 0;
        self.app_id_len = 0;
    }
}

const EMPTY_ENDPOINT: VaultEndpoint = VaultEndpoint {
    host: [0; MAX_ENDPOINT_HOST_BYTES],
    host_len: 0,
    port: 0,
};

// ------------------------------------------------------------------ load outcome

/// Whether the vault namespace initialized usably.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoadOutcome {
    /// Usable: a valid record was selected, or both slots were absent (a fresh,
    /// never-provisioned device).
    Loaded,
    /// Data was present but unusable in every slot (corrupt, torn, or
    /// unsupported). Grants are disabled and the vault reports unavailable.
    Corrupt,
}

// ------------------------------------------------------------------ medium

/// Which of the two commit slots a medium operation addresses.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VaultSlot {
    A,
    B,
}

impl VaultSlot {
    /// The opposite slot, used to pick a write target for two-slot commit.
    pub const fn other(self) -> Self {
        match self {
            VaultSlot::A => VaultSlot::B,
            VaultSlot::B => VaultSlot::A,
        }
    }
}

/// A medium read/write/erase failure. Opaque: no driver text, address, or bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MediumFault;

/// Result of reading a slot from the medium.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SlotRead {
    /// The slot has never been written (fresh device / after erase).
    Absent,
    /// The slot held exactly [`VAULT_RECORD_BYTES`] bytes, now in `dst`.
    Present,
}

/// The raw two-slot storage boundary. Platform adapters back this with a
/// location kept **separate** from public settings, the Wi-Fi secrets, and app
/// files. The medium sees only opaque fixed-size blocks; it never parses or logs
/// them. A partial/interrupted write must either leave the prior bytes intact or
/// produce a block that fails checksum on read-back.
pub trait VaultMedium {
    /// Reads a slot into `dst` (exactly [`VAULT_RECORD_BYTES`]). Returns
    /// [`SlotRead::Absent`] when the slot has never been written; `dst` is left
    /// untouched in that case.
    fn read_slot(&self, slot: VaultSlot, dst: &mut [u8; VAULT_RECORD_BYTES]) -> SlotRead;

    /// Writes a full record to a slot. Returns `Err` on any medium failure.
    fn write_slot(
        &mut self,
        slot: VaultSlot,
        src: &[u8; VAULT_RECORD_BYTES],
    ) -> Result<(), MediumFault>;

    /// Erases a slot so a subsequent [`VaultMedium::read_slot`] returns
    /// [`SlotRead::Absent`]. Returns `Err` on any medium failure.
    fn erase_slot(&mut self, slot: VaultSlot) -> Result<(), MediumFault>;
}

// ------------------------------------------------------------------ store

/// The bounded application credential vault. Owns the medium, up to
/// [`MAX_GRANTS`] grants, and one volatile edit-staging buffer.
pub struct VaultStore<M: VaultMedium> {
    medium: M,
    grants: [Option<Grant>; MAX_GRANTS],
    generation: u32,
    available: bool,
    current_slot: Option<VaultSlot>,
    edit: EditStaging,
}

impl<M: VaultMedium> VaultStore<M> {
    /// Loads both slots and selects the newest valid record. Always returns a
    /// store; inspect [`VaultStore::available`] or the returned [`LoadOutcome`]
    /// to learn whether the namespace is usable. Never blocks boot and never
    /// falls back to public settings or the Wi-Fi store.
    pub fn load(medium: M) -> (Self, LoadOutcome) {
        let mut store = Self {
            medium,
            grants: core::array::from_fn(|_| None),
            generation: 0,
            available: false,
            current_slot: None,
            edit: EditStaging::new(),
        };
        let outcome = store.reload();
        (store, outcome)
    }

    /// Re-reads both slots from the medium and rebuilds the in-RAM model.
    fn reload(&mut self) -> LoadOutcome {
        self.clear_grants();

        let mut scratch = [0u8; VAULT_RECORD_BYTES];
        let a = self.decode_slot(VaultSlot::A, &mut scratch);
        let b = self.decode_slot(VaultSlot::B, &mut scratch);
        volatile_zeroize(&mut scratch);

        let outcome = match (a, b) {
            (SlotState::Valid(ga), SlotState::Valid(gb)) => {
                let slot = if generation_is_newer(gb, ga) {
                    VaultSlot::B
                } else {
                    VaultSlot::A
                };
                self.adopt_slot(slot, &mut scratch);
                LoadOutcome::Loaded
            }
            (SlotState::Valid(_), _) => {
                self.adopt_slot(VaultSlot::A, &mut scratch);
                LoadOutcome::Loaded
            }
            (_, SlotState::Valid(_)) => {
                self.adopt_slot(VaultSlot::B, &mut scratch);
                LoadOutcome::Loaded
            }
            (SlotState::Absent, SlotState::Absent) => {
                // Fresh device: never provisioned. Usable but empty.
                self.available = true;
                self.generation = 0;
                self.current_slot = None;
                LoadOutcome::Loaded
            }
            _ => {
                // Present in at least one slot but valid in none: corruption.
                self.available = false;
                self.generation = 0;
                self.current_slot = None;
                LoadOutcome::Corrupt
            }
        };
        volatile_zeroize(&mut scratch);
        outcome
    }

    fn decode_slot(&self, slot: VaultSlot, scratch: &mut [u8; VAULT_RECORD_BYTES]) -> SlotState {
        match self.medium.read_slot(slot, scratch) {
            SlotRead::Absent => SlotState::Absent,
            SlotRead::Present => match decode_generation(scratch) {
                Ok(generation) => SlotState::Valid(generation),
                Err(_) => SlotState::Invalid,
            },
        }
    }

    fn adopt_slot(&mut self, slot: VaultSlot, scratch: &mut [u8; VAULT_RECORD_BYTES]) {
        if self.medium.read_slot(slot, scratch) != SlotRead::Present {
            self.available = false;
            return;
        }
        match decode_record(scratch, &mut self.grants) {
            Ok(generation) => {
                self.generation = generation;
                self.available = true;
                self.current_slot = Some(slot);
            }
            Err(_) => {
                self.clear_grants();
                self.available = false;
            }
        }
    }

    fn clear_grants(&mut self) {
        for slot in self.grants.iter_mut() {
            // Dropping the Some(_) volatile-zeroizes the secret and aux.
            *slot = None;
        }
    }

    // -------------------------------------------------------------- observation

    /// Whether the vault initialized its bounded secret namespace.
    pub fn available(&self) -> bool {
        self.available
    }

    /// The current record generation (zero means never provisioned / unusable).
    pub fn generation(&self) -> u32 {
        self.generation
    }

    /// Number of retained grants.
    pub fn len(&self) -> usize {
        self.grants.iter().filter(|slot| slot.is_some()).count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Redacted metadata for the grant at `index` in `0..len()`.
    pub fn info(&self, index: usize) -> Option<GrantInfo> {
        self.grants.iter().flatten().nth(index).map(Grant::info)
    }

    /// Whether a grant exists for the exact identity.
    pub fn contains(&self, app_id: &[u8], service: ServiceKind, endpoint: &VaultEndpoint) -> bool {
        self.grant_ref(app_id, service, endpoint).is_some()
    }

    /// Mints the opaque handle for a grant matching the exact identity, if one is
    /// retained. This is the OS-side lookup that turns a running app's
    /// destination into a handle; the application never derives it itself.
    pub fn handle_for(
        &self,
        app_id: &[u8],
        service: ServiceKind,
        endpoint: &VaultEndpoint,
    ) -> Option<CredentialHandle> {
        self.grant_ref(app_id, service, endpoint)
            .map(|g| CredentialHandle::new(g.generation, g.grant_id))
    }

    fn grant_ref(
        &self,
        app_id: &[u8],
        service: ServiceKind,
        endpoint: &VaultEndpoint,
    ) -> Option<&Grant> {
        self.grants
            .iter()
            .flatten()
            .find(|g| g.identity_matches(app_id, service, endpoint))
    }

    // -------------------------------------------------------------- injection

    /// Resolves a handle to an operation-scoped [`CredentialInjection`] after
    /// re-checking the grant generation, the requesting app id, the service, the
    /// exact destination endpoint, and that the request is over TLS. Any
    /// mismatch fails closed with a fixed error before a secret byte is touched.
    ///
    /// `request_is_tls` reflects the transport the OS is about to use; a
    /// non-TLS request is refused so a secret can never ride plain HTTP.
    pub fn injection_for(
        &self,
        handle: CredentialHandle,
        app_id: &[u8],
        service: ServiceKind,
        endpoint: &VaultEndpoint,
        request_is_tls: bool,
    ) -> Result<CredentialInjection<'_>, VaultError> {
        if !self.available {
            return Err(VaultError::Unavailable);
        }
        let grant = self
            .grants
            .iter()
            .flatten()
            .find(|g| g.grant_id == handle.grant_id())
            .ok_or(VaultError::NotFound)?;
        if grant.generation != handle.generation() {
            return Err(VaultError::StaleHandle);
        }
        if grant.app_id() != app_id {
            return Err(VaultError::ForeignApp);
        }
        if grant.service != service {
            return Err(VaultError::ServiceMismatch);
        }
        if !request_is_tls {
            return Err(VaultError::InsecureEndpoint);
        }
        if grant.endpoint != *endpoint {
            return Err(VaultError::EndpointMismatch);
        }
        Ok(grant.injection())
    }

    // -------------------------------------------------------------- staging

    /// Whether the volatile edit staging is fully zeroized.
    pub fn staging_zeroized(&self) -> bool {
        !self.edit.active
            && self.edit.secret_len == 0
            && self.edit.aux_len == 0
            && self.edit.app_id_len == 0
            && self.edit.secret.iter().all(|byte| *byte == 0)
            && self.edit.aux.iter().all(|byte| *byte == 0)
            && self.edit.app_id.iter().all(|byte| *byte == 0)
    }

    /// Validates a candidate grant and holds it in volatile edit staging.
    /// Rejects invalid app ids, service/kind mismatches, non-canonical
    /// endpoints, and invalid aux/secret bytes before any byte is retained. On
    /// any validation failure the staging is left zeroized.
    #[allow(clippy::too_many_arguments)]
    pub fn stage(
        &mut self,
        app_id: &[u8],
        service: ServiceKind,
        endpoint: VaultEndpoint,
        kind: CredentialKind,
        aux: &[u8],
        secret: &[u8],
    ) -> Result<(), VaultError> {
        self.edit.zeroize();
        validate_app_id_bytes(app_id)?;
        if !kind.allows(service) {
            return Err(VaultError::InvalidKind);
        }
        validate_aux(kind, aux)?;
        validate_secret(kind, secret)?;

        self.edit.app_id[..app_id.len()].copy_from_slice(app_id);
        self.edit.app_id_len = app_id.len() as u8;
        self.edit.service = service;
        self.edit.endpoint = endpoint;
        self.edit.kind = kind;
        self.edit.aux[..aux.len()].copy_from_slice(aux);
        self.edit.aux_len = aux.len() as u8;
        self.edit.secret[..secret.len()].copy_from_slice(secret);
        self.edit.secret_len = secret.len() as u8;
        self.edit.active = true;
        Ok(())
    }

    /// Cancels a staged edit, zeroizing the staging. Idempotent (page exit,
    /// Escape, and capability loss all funnel here).
    pub fn cancel_staged(&mut self) {
        self.edit.zeroize();
    }

    /// Commits the staged grant as a new or replaced grant and persists the whole
    /// record to the other slot. Replacing an existing `(app_id, service,
    /// endpoint)` grant advances that grant's generation, invalidating its prior
    /// handle. On success the staging is zeroized and the grant's handle is
    /// returned; on any failure the staging is zeroized and the in-RAM model is
    /// restored from the medium.
    pub fn commit(&mut self) -> Result<CredentialHandle, VaultError> {
        if !self.edit.active {
            return Err(VaultError::InvalidRecord);
        }
        if !self.available {
            self.edit.zeroize();
            return Err(VaultError::Unavailable);
        }

        let app_id_len = usize::from(self.edit.app_id_len);
        let mut app_id_buf = [0u8; MAX_APP_ID_LEN];
        app_id_buf[..app_id_len].copy_from_slice(&self.edit.app_id[..app_id_len]);
        let service = self.edit.service;
        let endpoint = self.edit.endpoint;
        let kind = self.edit.kind;

        // Stamp the grant with the record generation this commit will persist.
        // The record generation is monotonic, so every write — including a
        // revoke-then-re-add that reuses a grant id — produces a distinct,
        // nonzero stamp, and any handle minted from an earlier stamp is stale.
        let stamp = grant_generation_stamp(next_generation(self.generation));
        let (index, grant_id, generation) =
            match self.slot_for_identity(&app_id_buf[..app_id_len], service, &endpoint) {
                Some((index, existing_id)) => (index, existing_id, stamp),
                None => {
                    let Some(index) = self.free_index() else {
                        self.edit.zeroize();
                        return Err(VaultError::StoreFull);
                    };
                    (index, self.next_grant_id(), stamp)
                }
            };

        let mut aux = [0u8; AUX_FIELD];
        let aux_len = self.edit.aux_len;
        aux[..usize::from(aux_len)].copy_from_slice(&self.edit.aux[..usize::from(aux_len)]);
        let mut secret = [0u8; MAX_SECRET_BYTES];
        let secret_len = self.edit.secret_len;
        secret[..usize::from(secret_len)]
            .copy_from_slice(&self.edit.secret[..usize::from(secret_len)]);

        // Installing the new grant drops any previous occupant (zeroizing it).
        self.grants[index] = Some(Grant {
            grant_id,
            generation,
            service,
            kind,
            app_id: app_id_buf,
            app_id_len: app_id_len as u8,
            endpoint,
            aux,
            aux_len,
            secret,
            secret_len,
        });

        match self.persist() {
            Ok(()) => {
                self.edit.zeroize();
                Ok(CredentialHandle::new(generation, grant_id))
            }
            Err(err) => {
                self.reload();
                self.edit.zeroize();
                Err(err)
            }
        }
    }

    /// Revokes the grant with the given id: zeroizes its RAM secret immediately
    /// and commits the smaller record. A missing id is reported as success
    /// (idempotent revoke). Any outstanding handle for a revoked grant then
    /// fails closed.
    pub fn revoke(&mut self, grant_id: u16) -> Result<(), VaultError> {
        if !self.available {
            return Err(VaultError::Unavailable);
        }
        let Some(index) = self
            .grants
            .iter()
            .position(|slot| slot.as_ref().is_some_and(|g| g.grant_id == grant_id))
        else {
            return Ok(());
        };

        // Dropping the slot volatile-zeroizes the secret before the durable
        // commit. Compact so the encoding stays dense.
        self.grants[index] = None;
        self.compact();

        match self.persist() {
            Ok(()) => Ok(()),
            Err(err) => {
                self.reload();
                Err(err)
            }
        }
    }

    fn slot_for_identity(
        &self,
        app_id: &[u8],
        service: ServiceKind,
        endpoint: &VaultEndpoint,
    ) -> Option<(usize, u16)> {
        self.grants.iter().enumerate().find_map(|(index, slot)| {
            slot.as_ref()
                .filter(|g| g.identity_matches(app_id, service, endpoint))
                .map(|g| (index, g.grant_id))
        })
    }

    fn free_index(&self) -> Option<usize> {
        self.grants.iter().position(Option::is_none)
    }

    /// Picks the smallest unused nonzero grant id in `1..=MAX_GRANTS`.
    fn next_grant_id(&self) -> u16 {
        for candidate in 1..=(MAX_GRANTS as u16) {
            if !self
                .grants
                .iter()
                .flatten()
                .any(|g| g.grant_id == candidate)
            {
                return candidate;
            }
        }
        MAX_GRANTS as u16
    }

    // -------------------------------------------------------------- persistence

    fn persist(&mut self) -> Result<(), VaultError> {
        let next_generation = next_generation(self.generation);
        let mut record = [0u8; VAULT_RECORD_BYTES];
        encode_record(&self.grants, next_generation, &mut record);

        let target = self
            .current_slot
            .map(VaultSlot::other)
            .unwrap_or(VaultSlot::A);
        let result = self.medium.write_slot(target, &record);
        volatile_zeroize(&mut record);

        result.map_err(|MediumFault| VaultError::MediumError)?;
        self.generation = next_generation;
        self.current_slot = Some(target);
        Ok(())
    }

    /// Moves populated grants to the front so empty slots trail.
    fn compact(&mut self) {
        let mut write = 0;
        for read in 0..MAX_GRANTS {
            if self.grants[read].is_some() {
                if read != write {
                    self.grants.swap(read, write);
                }
                write += 1;
            }
        }
    }

    /// Erases both slots, verifies both read back absent/invalid, clears RAM
    /// staging and grants, and resets the generation. Does not claim secure
    /// physical erasure of the underlying flash/SD.
    pub fn factory_reset(&mut self) -> Result<(), VaultError> {
        self.edit.zeroize();
        self.clear_grants();

        let ea = self.medium.erase_slot(VaultSlot::A);
        let eb = self.medium.erase_slot(VaultSlot::B);
        ea.map_err(|MediumFault| VaultError::MediumError)?;
        eb.map_err(|MediumFault| VaultError::MediumError)?;

        let mut scratch = [0u8; VAULT_RECORD_BYTES];
        let a = self.decode_slot(VaultSlot::A, &mut scratch);
        let b = self.decode_slot(VaultSlot::B, &mut scratch);
        volatile_zeroize(&mut scratch);
        if matches!(a, SlotState::Valid(_)) || matches!(b, SlotState::Valid(_)) {
            return Err(VaultError::MediumError);
        }

        self.generation = 0;
        self.current_slot = None;
        self.available = true;
        Ok(())
    }

    /// Volatile-zeroizes all RAM staging and retained secrets. Called on arena
    /// teardown and capability loss. The durable medium is untouched.
    pub fn zeroize_ram(&mut self) {
        self.edit.zeroize();
        self.clear_grants();
        self.available = false;
        self.current_slot = None;
    }
}

impl<M: VaultMedium> Drop for VaultStore<M> {
    fn drop(&mut self) {
        self.edit.zeroize();
        self.clear_grants();
    }
}

// ------------------------------------------------------------------ codec

#[derive(Clone, Copy, Eq, PartialEq)]
enum SlotState {
    Absent,
    Invalid,
    Valid(u32),
}

fn decode_generation(src: &[u8; VAULT_RECORD_BYTES]) -> Result<u32, VaultError> {
    validate_header(src)?;
    let mut sink: [Option<Grant>; MAX_GRANTS] = core::array::from_fn(|_| None);
    let generation = decode_record(src, &mut sink)?;
    // sink drops here, zeroizing any secrets it briefly held.
    Ok(generation)
}

fn validate_header(src: &[u8; VAULT_RECORD_BYTES]) -> Result<u32, VaultError> {
    if src[OFF_MAGIC..OFF_MAGIC + 4] != VAULT_MAGIC {
        return Err(VaultError::BadMagic);
    }
    if get_u16(src, OFF_MAJOR) != FORMAT_MAJOR || get_u16(src, OFF_MINOR) != FORMAT_MINOR {
        return Err(VaultError::UnsupportedVersion);
    }
    if get_u32(src, OFF_TOTAL_LEN) as usize != VAULT_RECORD_BYTES {
        return Err(VaultError::InvalidLength);
    }
    if usize::from(get_u16(src, OFF_STRIDE)) != GRANT_STRIDE {
        return Err(VaultError::InvalidLength);
    }
    if src[OFF_RESERVED0] != 0 || src[OFF_RESERVED..OFF_RESERVED + 8].iter().any(|b| *b != 0) {
        return Err(VaultError::InvalidRecord);
    }
    let generation = get_u32(src, OFF_GENERATION);
    if generation == 0 {
        return Err(VaultError::InvalidGeneration);
    }
    if get_u32(src, OFF_CHECKSUM) != record_checksum(src) {
        return Err(VaultError::InvalidChecksum);
    }
    Ok(generation)
}

fn decode_record(
    src: &[u8; VAULT_RECORD_BYTES],
    grants: &mut [Option<Grant>; MAX_GRANTS],
) -> Result<u32, VaultError> {
    for slot in grants.iter_mut() {
        *slot = None;
    }

    let generation = validate_header(src)?;
    let count = usize::from(src[OFF_COUNT]);
    if count > MAX_GRANTS {
        return Err(VaultError::TooManyGrants);
    }

    for index in 0..MAX_GRANTS {
        let base = HEADER_LEN + index * GRANT_STRIDE;
        let record = &src[base..base + GRANT_STRIDE];
        if index < count {
            let grant = decode_grant(record)?;
            if grants[..index]
                .iter()
                .flatten()
                .any(|g| g.identity_matches(grant.app_id(), grant.service, &grant.endpoint))
            {
                clear(grants);
                return Err(VaultError::DuplicateGrant);
            }
            grants[index] = Some(grant);
        } else if record.iter().any(|b| *b != 0) {
            clear(grants);
            return Err(VaultError::InvalidRecord);
        }
    }
    Ok(generation)
}

fn clear(grants: &mut [Option<Grant>; MAX_GRANTS]) {
    for slot in grants.iter_mut() {
        *slot = None;
    }
}

fn decode_grant(record: &[u8]) -> Result<Grant, VaultError> {
    let grant_id = get_u16(record, G_ID);
    if grant_id == 0 {
        return Err(VaultError::InvalidRecord);
    }
    let generation = get_u16(record, G_GENERATION);
    if generation == 0 {
        return Err(VaultError::InvalidRecord);
    }
    if record[G_RESERVED..G_RESERVED + 4].iter().any(|b| *b != 0) {
        return Err(VaultError::InvalidRecord);
    }

    let service =
        ServiceKind::from_byte(record[G_SERVICE]).ok_or(VaultError::UnsupportedService)?;
    let kind = CredentialKind::from_byte(record[G_KIND]).ok_or(VaultError::InvalidKind)?;
    if !kind.allows(service) {
        return Err(VaultError::InvalidKind);
    }

    let app_id_len = usize::from(record[G_APP_ID_LEN]);
    if app_id_len == 0 || app_id_len > MAX_APP_ID_LEN {
        return Err(VaultError::InvalidAppId);
    }
    let app_id_bytes = &record[G_APP_ID..G_APP_ID + app_id_len];
    validate_app_id_bytes(app_id_bytes)?;
    if record[G_APP_ID + app_id_len..G_APP_ID + MAX_APP_ID_LEN]
        .iter()
        .any(|b| *b != 0)
    {
        return Err(VaultError::InvalidRecord);
    }

    let host_len = usize::from(record[G_HOST_LEN]);
    if host_len == 0 || host_len > MAX_ENDPOINT_HOST_BYTES {
        return Err(VaultError::InvalidEndpoint);
    }
    let host = core::str::from_utf8(&record[G_HOST..G_HOST + host_len])
        .map_err(|_| VaultError::InvalidEndpoint)?;
    let port = get_u16(record, G_PORT);
    let endpoint = VaultEndpoint::new(host, port)?;
    if record[G_HOST + host_len..G_HOST + MAX_ENDPOINT_HOST_BYTES]
        .iter()
        .any(|b| *b != 0)
    {
        return Err(VaultError::InvalidRecord);
    }

    let aux_len = usize::from(record[G_AUX_LEN]);
    if aux_len > AUX_FIELD {
        return Err(VaultError::InvalidAux);
    }
    let aux_bytes = &record[G_AUX..G_AUX + aux_len];
    validate_aux(kind, aux_bytes)?;
    if record[G_AUX + aux_len..G_AUX + AUX_FIELD]
        .iter()
        .any(|b| *b != 0)
    {
        return Err(VaultError::InvalidRecord);
    }

    let secret_len = usize::from(record[G_SECRET_LEN]);
    if secret_len == 0 || secret_len > MAX_SECRET_BYTES {
        return Err(VaultError::InvalidSecret);
    }
    let secret_bytes = &record[G_SECRET..G_SECRET + secret_len];
    validate_secret(kind, secret_bytes)?;
    if record[G_SECRET + secret_len..G_SECRET + MAX_SECRET_BYTES]
        .iter()
        .any(|b| *b != 0)
    {
        return Err(VaultError::InvalidRecord);
    }

    let mut app_id = [0u8; MAX_APP_ID_LEN];
    app_id[..app_id_len].copy_from_slice(app_id_bytes);
    let mut aux = [0u8; AUX_FIELD];
    aux[..aux_len].copy_from_slice(aux_bytes);
    let mut secret = [0u8; MAX_SECRET_BYTES];
    secret[..secret_len].copy_from_slice(secret_bytes);

    Ok(Grant {
        grant_id,
        generation,
        service,
        kind,
        app_id,
        app_id_len: app_id_len as u8,
        endpoint,
        aux,
        aux_len: aux_len as u8,
        secret,
        secret_len: secret_len as u8,
    })
}

fn encode_record(
    grants: &[Option<Grant>; MAX_GRANTS],
    generation: u32,
    dst: &mut [u8; VAULT_RECORD_BYTES],
) {
    dst.fill(0);
    dst[OFF_MAGIC..OFF_MAGIC + 4].copy_from_slice(&VAULT_MAGIC);
    put_u16(dst, OFF_MAJOR, FORMAT_MAJOR);
    put_u16(dst, OFF_MINOR, FORMAT_MINOR);
    put_u32(dst, OFF_TOTAL_LEN, VAULT_RECORD_BYTES as u32);
    let count = grants.iter().filter(|slot| slot.is_some()).count();
    dst[OFF_COUNT] = count as u8;
    put_u16(dst, OFF_STRIDE, GRANT_STRIDE as u16);
    put_u32(dst, OFF_GENERATION, generation);

    for (index, grant) in grants.iter().flatten().enumerate() {
        let base = HEADER_LEN + index * GRANT_STRIDE;
        put_u16(dst, base + G_ID, grant.grant_id);
        put_u16(dst, base + G_GENERATION, grant.generation);
        dst[base + G_SERVICE] = grant.service.to_byte();
        dst[base + G_KIND] = grant.kind.to_byte();
        dst[base + G_APP_ID_LEN] = grant.app_id_len;
        dst[base + G_HOST_LEN] = grant.endpoint.host_len;
        dst[base + G_AUX_LEN] = grant.aux_len;
        dst[base + G_SECRET_LEN] = grant.secret_len;
        put_u16(dst, base + G_PORT, grant.endpoint.port);
        let app_id = grant.app_id();
        dst[base + G_APP_ID..base + G_APP_ID + app_id.len()].copy_from_slice(app_id);
        let host = grant.endpoint.host_bytes();
        dst[base + G_HOST..base + G_HOST + host.len()].copy_from_slice(host);
        let aux = grant.aux();
        dst[base + G_AUX..base + G_AUX + aux.len()].copy_from_slice(aux);
        let secret = grant.secret();
        dst[base + G_SECRET..base + G_SECRET + secret.len()].copy_from_slice(secret);
    }

    let checksum = record_checksum(dst);
    put_u32(dst, OFF_CHECKSUM, checksum);
}

// ------------------------------------------------------------------ validation

/// Validates an app id given as raw bytes: canonical UTF-8 package id.
fn validate_app_id_bytes(app_id: &[u8]) -> Result<(), VaultError> {
    if app_id.is_empty() || app_id.len() > MAX_APP_ID_LEN {
        return Err(VaultError::InvalidAppId);
    }
    let text = core::str::from_utf8(app_id).map_err(|_| VaultError::InvalidAppId)?;
    validate_app_id(text).map_err(|_| VaultError::InvalidAppId)
}

/// Validates the auxiliary field for its kind. `BearerToken` requires it empty;
/// `ApiKeyHeader` requires a nonempty HTTP token name up to
/// [`MAX_HEADER_NAME_BYTES`]; `MqttLogin` requires a nonempty printable-ASCII
/// username up to [`MAX_USERNAME_BYTES`].
fn validate_aux(kind: CredentialKind, aux: &[u8]) -> Result<(), VaultError> {
    match kind {
        CredentialKind::BearerToken => {
            if aux.is_empty() {
                Ok(())
            } else {
                Err(VaultError::InvalidAux)
            }
        }
        CredentialKind::ApiKeyHeader => {
            if aux.is_empty() || aux.len() > MAX_HEADER_NAME_BYTES {
                return Err(VaultError::InvalidAux);
            }
            if aux.iter().all(|b| is_http_token_byte(*b)) {
                Ok(())
            } else {
                Err(VaultError::InvalidAux)
            }
        }
        CredentialKind::MqttLogin => {
            if aux.is_empty() || aux.len() > MAX_USERNAME_BYTES {
                return Err(VaultError::InvalidAux);
            }
            if aux.iter().all(|b| (0x20..=0x7e).contains(b)) {
                Ok(())
            } else {
                Err(VaultError::InvalidAux)
            }
        }
    }
}

/// Validates a secret for its kind. Header-bound kinds (bearer, API key) must be
/// nonempty header-safe visible ASCII (`0x21..=0x7e`) so no control byte can be
/// injected into an HTTP header. An MQTT password may be any nonempty bytes up
/// to [`MAX_SECRET_BYTES`].
fn validate_secret(kind: CredentialKind, secret: &[u8]) -> Result<(), VaultError> {
    if secret.is_empty() || secret.len() > MAX_SECRET_BYTES {
        return Err(VaultError::InvalidSecret);
    }
    match kind {
        CredentialKind::BearerToken | CredentialKind::ApiKeyHeader => {
            if secret.iter().all(|b| (0x21..=0x7e).contains(b)) {
                Ok(())
            } else {
                Err(VaultError::InvalidSecret)
            }
        }
        CredentialKind::MqttLogin => Ok(()),
    }
}

/// RFC 7230 token characters (a header field name).
fn is_http_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'!' | b'#'
                | b'$'
                | b'%'
                | b'&'
                | b'\''
                | b'*'
                | b'+'
                | b'-'
                | b'.'
                | b'^'
                | b'_'
                | b'`'
                | b'|'
                | b'~'
        )
}

/// Lower-case DNS hostname acceptance shared with the Fetch/MQTT origins:
/// lower letters, digits, `-`, and `.` label separators, no empty labels or
/// leading/trailing dots or hyphens.
fn is_canonical_hostname(host: &str) -> bool {
    if host.starts_with('.') || host.ends_with('.') || host.contains("..") {
        return false;
    }
    host.bytes().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'.'
    }) && host
        .split('.')
        .all(|label| !label.is_empty() && !label.starts_with('-') && !label.ends_with('-'))
}

// ------------------------------------------------------------------ helpers

const fn generation_is_newer(candidate: u32, current: u32) -> bool {
    candidate != current && candidate.wrapping_sub(current) < 0x8000_0000
}

const fn next_generation(current: u32) -> u32 {
    let next = current.wrapping_add(1);
    if next == 0 {
        1
    } else {
        next
    }
}

/// The per-grant handle stamp derived from the monotonic record generation.
/// The low 16 bits identify the write; zero is reserved so a stamp is never
/// zero (which decode treats as an invalid grant).
const fn grant_generation_stamp(record_generation: u32) -> u16 {
    let low = record_generation as u16;
    if low == 0 {
        1
    } else {
        low
    }
}

/// FNV-1a over the record with the checksum field treated as zero. Independent
/// of the other namespaces' checksums.
fn record_checksum(bytes: &[u8; VAULT_RECORD_BYTES]) -> u32 {
    let mut checksum = 0x811c_9dc5u32;
    for (index, byte) in bytes.iter().enumerate() {
        let value = if (OFF_CHECKSUM..OFF_CHECKSUM + 4).contains(&index) {
            0
        } else {
            *byte
        };
        checksum = (checksum ^ u32::from(value)).wrapping_mul(0x0100_0193);
    }
    checksum
}

/// Overwrites every byte of `buf` with zero using volatile writes so the
/// compiler cannot elide the clear. Reduces accidental retention only.
fn volatile_zeroize(buf: &mut [u8]) {
    for byte in buf.iter_mut() {
        unsafe {
            core::ptr::write_volatile(byte, 0);
        }
    }
}

fn get_u16(src: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([src[offset], src[offset + 1]])
}

fn get_u32(src: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        src[offset],
        src[offset + 1],
        src[offset + 2],
        src[offset + 3],
    ])
}

fn put_u16(dst: &mut [u8], offset: usize, value: u16) {
    dst[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(dst: &mut [u8], offset: usize, value: u32) {
    dst[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

// ------------------------------------------------------------------ tests

#[cfg(test)]
mod tests {
    use super::*;

    /// In-memory two-slot medium with fault injection.
    struct FakeMedium {
        slots: [Option<[u8; VAULT_RECORD_BYTES]>; 2],
        reject_write: [bool; 2],
        reject_erase: [bool; 2],
    }

    impl FakeMedium {
        fn new() -> Self {
            Self {
                slots: [None, None],
                reject_write: [false, false],
                reject_erase: [false, false],
            }
        }

        fn idx(slot: VaultSlot) -> usize {
            match slot {
                VaultSlot::A => 0,
                VaultSlot::B => 1,
            }
        }

        fn set_raw(&mut self, slot: VaultSlot, bytes: [u8; VAULT_RECORD_BYTES]) {
            self.slots[Self::idx(slot)] = Some(bytes);
        }

        fn raw(&self, slot: VaultSlot) -> Option<&[u8; VAULT_RECORD_BYTES]> {
            self.slots[Self::idx(slot)].as_ref()
        }

        fn flip_byte(&mut self, slot: VaultSlot, offset: usize) {
            if let Some(bytes) = self.slots[Self::idx(slot)].as_mut() {
                bytes[offset] ^= 0xff;
            }
        }
    }

    impl VaultMedium for FakeMedium {
        fn read_slot(&self, slot: VaultSlot, dst: &mut [u8; VAULT_RECORD_BYTES]) -> SlotRead {
            match self.slots[Self::idx(slot)] {
                Some(bytes) => {
                    *dst = bytes;
                    SlotRead::Present
                }
                None => SlotRead::Absent,
            }
        }

        fn write_slot(
            &mut self,
            slot: VaultSlot,
            src: &[u8; VAULT_RECORD_BYTES],
        ) -> Result<(), MediumFault> {
            if self.reject_write[Self::idx(slot)] {
                return Err(MediumFault);
            }
            self.slots[Self::idx(slot)] = Some(*src);
            Ok(())
        }

        fn erase_slot(&mut self, slot: VaultSlot) -> Result<(), MediumFault> {
            if self.reject_erase[Self::idx(slot)] {
                return Err(MediumFault);
            }
            self.slots[Self::idx(slot)] = None;
            Ok(())
        }
    }

    const APP: &[u8] = b"com.example.weather";
    const OTHER_APP: &[u8] = b"com.attacker.evil";
    const TOKEN: &[u8] = b"tok_live_abc123DEF456";

    fn ep(host: &str, port: u16) -> VaultEndpoint {
        VaultEndpoint::new(host, port).unwrap()
    }

    /// A store seeded with one bearer-token grant for APP -> api.example.com:443.
    fn store_with_grant() -> (VaultStore<FakeMedium>, CredentialHandle) {
        let (mut store, outcome) = VaultStore::load(FakeMedium::new());
        assert_eq!(outcome, LoadOutcome::Loaded);
        store
            .stage(
                APP,
                ServiceKind::Fetch,
                ep("api.example.com", 443),
                CredentialKind::BearerToken,
                &[],
                TOKEN,
            )
            .unwrap();
        let handle = store.commit().unwrap();
        (store, handle)
    }

    fn rebuild_medium(store: &VaultStore<FakeMedium>) -> FakeMedium {
        let mut medium = FakeMedium::new();
        if let Some(bytes) = store.medium.raw(VaultSlot::A) {
            medium.set_raw(VaultSlot::A, *bytes);
        }
        if let Some(bytes) = store.medium.raw(VaultSlot::B) {
            medium.set_raw(VaultSlot::B, *bytes);
        }
        medium
    }

    // -------------------------------------------------------------- format

    #[test]
    fn record_size_is_fixed() {
        assert_eq!(GRANT_STRIDE, 448);
        assert_eq!(VAULT_RECORD_BYTES, 32 + 4 * 448);
    }

    #[test]
    fn fresh_device_loads_empty_and_available() {
        let (store, outcome) = VaultStore::load(FakeMedium::new());
        assert_eq!(outcome, LoadOutcome::Loaded);
        assert!(store.available());
        assert!(store.is_empty());
        assert_eq!(store.generation(), 0);
    }

    #[test]
    fn round_trip_preserves_grant_scope() {
        let (store, _) = store_with_grant();
        let info = store.info(0).unwrap();
        assert_eq!(info.service, ServiceKind::Fetch);
        assert_eq!(info.kind, CredentialKind::BearerToken);
        assert_eq!(info.endpoint_port, 443);
        assert_eq!(usize::from(info.app_id_len), APP.len());
        assert_eq!(usize::from(info.secret_len), TOKEN.len());
    }

    // -------------------------------------------------------------- AC-7 matrix

    #[test]
    fn denial_default_no_grant() {
        // A fresh vault denies: no grant, no handle, injection unavailable.
        let (store, _) = VaultStore::load(FakeMedium::new());
        assert!(store
            .handle_for(APP, ServiceKind::Fetch, &ep("api.example.com", 443))
            .is_none());
        let bogus = CredentialHandle::from_raw(0x0001_0001);
        assert_eq!(
            store
                .injection_for(
                    bogus,
                    APP,
                    ServiceKind::Fetch,
                    &ep("api.example.com", 443),
                    true
                )
                .unwrap_err(),
            VaultError::NotFound
        );
    }

    #[test]
    fn grant_then_inject_bearer() {
        let (store, handle) = store_with_grant();
        let injection = store
            .injection_for(
                handle,
                APP,
                ServiceKind::Fetch,
                &ep("api.example.com", 443),
                true,
            )
            .unwrap();
        match injection {
            CredentialInjection::BearerToken { token } => assert_eq!(token, TOKEN),
            other => panic!("unexpected injection {other:?}"),
        }
    }

    #[test]
    fn wrong_app_is_refused() {
        let (store, handle) = store_with_grant();
        assert_eq!(
            store
                .injection_for(
                    handle,
                    OTHER_APP,
                    ServiceKind::Fetch,
                    &ep("api.example.com", 443),
                    true
                )
                .unwrap_err(),
            VaultError::ForeignApp
        );
    }

    #[test]
    fn wrong_origin_is_refused() {
        let (store, handle) = store_with_grant();
        // Different host.
        assert_eq!(
            store
                .injection_for(
                    handle,
                    APP,
                    ServiceKind::Fetch,
                    &ep("evil.example.com", 443),
                    true
                )
                .unwrap_err(),
            VaultError::EndpointMismatch
        );
        // Different port on the same host.
        assert_eq!(
            store
                .injection_for(
                    handle,
                    APP,
                    ServiceKind::Fetch,
                    &ep("api.example.com", 8443),
                    true
                )
                .unwrap_err(),
            VaultError::EndpointMismatch
        );
    }

    #[test]
    fn wrong_service_is_refused() {
        let (store, handle) = store_with_grant();
        assert_eq!(
            store
                .injection_for(
                    handle,
                    APP,
                    ServiceKind::Mqtt,
                    &ep("api.example.com", 443),
                    true
                )
                .unwrap_err(),
            VaultError::ServiceMismatch
        );
    }

    #[test]
    fn plain_http_request_is_refused() {
        // AC-5: a secret can never ride a non-TLS transport.
        let (store, handle) = store_with_grant();
        assert_eq!(
            store
                .injection_for(
                    handle,
                    APP,
                    ServiceKind::Fetch,
                    &ep("api.example.com", 443),
                    false
                )
                .unwrap_err(),
            VaultError::InsecureEndpoint
        );
    }

    #[test]
    fn revoke_invalidates_handle() {
        let (mut store, handle) = store_with_grant();
        let grant_id = store.info(0).unwrap().grant_id;
        store.revoke(grant_id).unwrap();
        assert!(store.is_empty());
        assert_eq!(
            store
                .injection_for(
                    handle,
                    APP,
                    ServiceKind::Fetch,
                    &ep("api.example.com", 443),
                    true
                )
                .unwrap_err(),
            VaultError::NotFound
        );
        // Idempotent revoke.
        assert_eq!(store.revoke(grant_id), Ok(()));
    }

    #[test]
    fn replace_bumps_generation_and_staleness() {
        let (mut store, old_handle) = store_with_grant();
        // Replace the same identity's secret.
        store
            .stage(
                APP,
                ServiceKind::Fetch,
                ep("api.example.com", 443),
                CredentialKind::BearerToken,
                &[],
                b"tok_live_rotated999",
            )
            .unwrap();
        let new_handle = store.commit().unwrap();
        assert_ne!(new_handle.raw(), old_handle.raw());
        // Old handle is now stale.
        assert_eq!(
            store
                .injection_for(
                    old_handle,
                    APP,
                    ServiceKind::Fetch,
                    &ep("api.example.com", 443),
                    true
                )
                .unwrap_err(),
            VaultError::StaleHandle
        );
        // New handle injects the rotated secret.
        let injection = store
            .injection_for(
                new_handle,
                APP,
                ServiceKind::Fetch,
                &ep("api.example.com", 443),
                true,
            )
            .unwrap();
        assert!(matches!(
            injection,
            CredentialInjection::BearerToken { token } if token == b"tok_live_rotated999"
        ));
    }

    #[test]
    fn stale_handle_after_reload() {
        // A handle minted for generation 1 must not resolve against a later
        // generation of the same grant id.
        let (mut store, handle) = store_with_grant();
        let grant_id = store.info(0).unwrap().grant_id;
        // Revoke then re-add: same identity, new grant generation.
        store.revoke(grant_id).unwrap();
        store
            .stage(
                APP,
                ServiceKind::Fetch,
                ep("api.example.com", 443),
                CredentialKind::BearerToken,
                &[],
                TOKEN,
            )
            .unwrap();
        store.commit().unwrap();
        assert_eq!(
            store
                .injection_for(
                    handle,
                    APP,
                    ServiceKind::Fetch,
                    &ep("api.example.com", 443),
                    true
                )
                .unwrap_err(),
            VaultError::StaleHandle
        );
    }

    #[test]
    fn corruption_fails_closed() {
        let (store, _) = store_with_grant();
        let mut medium = rebuild_medium(&store);
        // Corrupt whichever slot is populated.
        for slot in [VaultSlot::A, VaultSlot::B] {
            if medium.raw(slot).is_some() {
                medium.flip_byte(slot, HEADER_LEN + G_APP_ID + 1);
            }
        }
        let (reloaded, outcome) = VaultStore::load(medium);
        assert_eq!(outcome, LoadOutcome::Corrupt);
        assert!(!reloaded.available());
        assert!(reloaded.is_empty());
        // Injection on an unavailable vault fails closed.
        let bogus = CredentialHandle::from_raw(0x0001_0001);
        assert_eq!(
            reloaded
                .injection_for(
                    bogus,
                    APP,
                    ServiceKind::Fetch,
                    &ep("api.example.com", 443),
                    true
                )
                .unwrap_err(),
            VaultError::Unavailable
        );
    }

    #[test]
    fn torn_write_falls_back_to_prior_slot() {
        let (mut store, _) = store_with_grant();
        // Commit a second grant so both slots hold generations; then tear the
        // newest and confirm the prior good slot is selected.
        store
            .stage(
                APP,
                ServiceKind::Fetch,
                ep("api.other.com", 443),
                CredentialKind::BearerToken,
                &[],
                b"tok_second",
            )
            .unwrap();
        store.commit().unwrap();

        let mut medium = rebuild_medium(&store);
        // The newest slot is whichever the store last wrote.
        let newest = store.current_slot.unwrap();
        medium.flip_byte(newest, VAULT_RECORD_BYTES - 1);
        let (reloaded, outcome) = VaultStore::load(medium);
        assert_eq!(outcome, LoadOutcome::Loaded);
        // Falls back to the prior slot (one grant only).
        assert_eq!(reloaded.len(), 1);
    }

    #[test]
    fn reset_during_update_rolls_back() {
        // A commit whose write fails must leave the durable prior state intact.
        let (mut store, handle) = store_with_grant();
        store.medium.reject_write = [true, true];
        store
            .stage(
                APP,
                ServiceKind::Fetch,
                ep("api.example.com", 443),
                CredentialKind::BearerToken,
                &[],
                b"tok_would_be_rotated",
            )
            .unwrap();
        assert_eq!(store.commit(), Err(VaultError::MediumError));
        // Prior grant still resolves with its original handle and secret.
        let injection = store
            .injection_for(
                handle,
                APP,
                ServiceKind::Fetch,
                &ep("api.example.com", 443),
                true,
            )
            .unwrap();
        assert!(matches!(
            injection,
            CredentialInjection::BearerToken { token } if token == TOKEN
        ));
        assert!(store.staging_zeroized());
    }

    #[test]
    fn factory_reset_clears_and_stays_usable() {
        let (mut store, _) = store_with_grant();
        store.factory_reset().unwrap();
        assert!(store.available());
        assert!(store.is_empty());
        assert_eq!(store.generation(), 0);
        assert!(rebuild_medium(&store).raw(VaultSlot::A).is_none());
        assert!(rebuild_medium(&store).raw(VaultSlot::B).is_none());
    }

    // -------------------------------------------------------- redaction (AC-6)

    #[test]
    fn diagnostics_redact_secret_bytes() {
        let (store, handle) = store_with_grant();
        let info = store.info(0).unwrap();
        let rendered = format!("{info:?}");
        assert!(!rendered.contains("example.com"));
        assert!(!contains_bytes(rendered.as_bytes(), TOKEN));

        let endpoint = ep("api.example.com", 443);
        assert!(!format!("{endpoint:?}").contains("example.com"));

        let injection = store
            .injection_for(handle, APP, ServiceKind::Fetch, &endpoint, true)
            .unwrap();
        let rendered = format!("{injection:?}");
        assert!(!contains_bytes(rendered.as_bytes(), TOKEN));
        assert_eq!(rendered, "BearerToken(..)");

        assert!(!format!("{handle:?}").contains("tok_"));
    }

    fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|w| w == needle)
    }

    // -------------------------------------------------------- api key + mqtt

    #[test]
    fn api_key_header_injection() {
        let (mut store, _) = VaultStore::load(FakeMedium::new());
        store
            .stage(
                APP,
                ServiceKind::Fetch,
                ep("api.example.com", 443),
                CredentialKind::ApiKeyHeader,
                b"X-Api-Key",
                b"secretkey123",
            )
            .unwrap();
        let handle = store.commit().unwrap();
        let injection = store
            .injection_for(
                handle,
                APP,
                ServiceKind::Fetch,
                &ep("api.example.com", 443),
                true,
            )
            .unwrap();
        match injection {
            CredentialInjection::ApiKeyHeader { name, value } => {
                assert_eq!(name, b"X-Api-Key");
                assert_eq!(value, b"secretkey123");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn mqtt_login_injection() {
        let (mut store, _) = VaultStore::load(FakeMedium::new());
        store
            .stage(
                APP,
                ServiceKind::Mqtt,
                ep("broker.example.com", 8883),
                CredentialKind::MqttLogin,
                b"device01",
                b"p@ss w0rd\x01binary",
            )
            .unwrap();
        let handle = store.commit().unwrap();
        let injection = store
            .injection_for(
                handle,
                APP,
                ServiceKind::Mqtt,
                &ep("broker.example.com", 8883),
                true,
            )
            .unwrap();
        match injection {
            CredentialInjection::MqttLogin { username, password } => {
                assert_eq!(username, b"device01");
                assert_eq!(password, b"p@ss w0rd\x01binary");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    // -------------------------------------------------------- validation

    #[test]
    fn service_kind_mismatch_rejected() {
        let (mut store, _) = VaultStore::load(FakeMedium::new());
        assert_eq!(
            store.stage(
                APP,
                ServiceKind::Mqtt,
                ep("api.example.com", 443),
                CredentialKind::BearerToken,
                &[],
                TOKEN,
            ),
            Err(VaultError::InvalidKind)
        );
        assert!(store.staging_zeroized());
    }

    #[test]
    fn bearer_rejects_nonempty_aux() {
        let (mut store, _) = VaultStore::load(FakeMedium::new());
        assert_eq!(
            store.stage(
                APP,
                ServiceKind::Fetch,
                ep("api.example.com", 443),
                CredentialKind::BearerToken,
                b"unexpected",
                TOKEN,
            ),
            Err(VaultError::InvalidAux)
        );
    }

    #[test]
    fn header_secret_rejects_control_bytes() {
        let (mut store, _) = VaultStore::load(FakeMedium::new());
        assert_eq!(
            store.stage(
                APP,
                ServiceKind::Fetch,
                ep("api.example.com", 443),
                CredentialKind::BearerToken,
                &[],
                b"bad\r\ninject",
            ),
            Err(VaultError::InvalidSecret)
        );
    }

    #[test]
    fn invalid_app_id_rejected() {
        let (mut store, _) = VaultStore::load(FakeMedium::new());
        assert_eq!(
            store.stage(
                b"Not A Valid Id",
                ServiceKind::Fetch,
                ep("api.example.com", 443),
                CredentialKind::BearerToken,
                &[],
                TOKEN,
            ),
            Err(VaultError::InvalidAppId)
        );
    }

    #[test]
    fn noncanonical_endpoint_rejected() {
        assert_eq!(
            VaultEndpoint::new("API.Example.com", 443),
            Err(VaultError::InvalidEndpoint)
        );
        assert_eq!(
            VaultEndpoint::new("host", 0),
            Err(VaultError::InvalidEndpoint)
        );
        assert_eq!(
            VaultEndpoint::new("*.example.com", 443),
            Err(VaultError::InvalidEndpoint)
        );
    }

    #[test]
    fn store_full_rejects_fifth_grant() {
        let (mut store, _) = VaultStore::load(FakeMedium::new());
        for i in 0..MAX_GRANTS {
            let host = match i {
                0 => "a.example.com",
                1 => "b.example.com",
                2 => "c.example.com",
                _ => "d.example.com",
            };
            store
                .stage(
                    APP,
                    ServiceKind::Fetch,
                    ep(host, 443),
                    CredentialKind::BearerToken,
                    &[],
                    TOKEN,
                )
                .unwrap();
            store.commit().unwrap();
        }
        store
            .stage(
                APP,
                ServiceKind::Fetch,
                ep("e.example.com", 443),
                CredentialKind::BearerToken,
                &[],
                TOKEN,
            )
            .unwrap();
        assert_eq!(store.commit(), Err(VaultError::StoreFull));
    }

    #[test]
    fn newest_generation_selected_across_slots() {
        let (mut store, _) = store_with_grant();
        // Second commit writes the other slot with a newer generation.
        store
            .stage(
                APP,
                ServiceKind::Fetch,
                ep("api.example.com", 443),
                CredentialKind::BearerToken,
                &[],
                b"tok_newer",
            )
            .unwrap();
        store.commit().unwrap();
        let (reloaded, outcome) = VaultStore::load(rebuild_medium(&store));
        assert_eq!(outcome, LoadOutcome::Loaded);
        let handle = reloaded
            .handle_for(APP, ServiceKind::Fetch, &ep("api.example.com", 443))
            .unwrap();
        let injection = reloaded
            .injection_for(
                handle,
                APP,
                ServiceKind::Fetch,
                &ep("api.example.com", 443),
                true,
            )
            .unwrap();
        assert!(matches!(
            injection,
            CredentialInjection::BearerToken { token } if token == b"tok_newer"
        ));
    }

    #[test]
    fn zeroize_ram_disables_and_clears() {
        let (mut store, _) = store_with_grant();
        store.zeroize_ram();
        assert!(!store.available());
        assert!(store.is_empty());
        assert!(store.staging_zeroized());
    }
}
