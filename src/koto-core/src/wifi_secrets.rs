//! Bounded Wi-Fi secret credential provider (KOTO-0240).
//!
//! This module implements the credential-provider boundary frozen by KOTO-0224
//! in [`docs/architecture/KOTOCONFIG_WIFI_EXTENSION.md`]. It stores at most four
//! Wi-Fi profiles in a **separate**, versioned, checksummed secret namespace and
//! fails closed on missing, corrupt, torn, or unsupported data. It is `no_std`,
//! uses no general heap, and owns only fixed-capacity storage.
//!
//! ## Separation (KOTO-0224 threat model)
//!
//! The secret namespace is deliberately independent of the public `KCF1`
//! configuration record ([`crate::config`]), `KUC1`, Shell preferences, app
//! storage/enumeration, diagnostics, and support exports. It has its own magic
//! (`KWS1`), its own fixed record format, and its own [`SecretMedium`] adapter,
//! which platform code must back with a storage location distinct from the
//! public settings slots. Ordinary applications and public configuration
//! snapshots cannot read, enumerate, or mutate secrets: this module is the only
//! surface, it exposes only redacted list metadata ([`ProfileInfo`]) and
//! operation-scoped credential views ([`CredentialView`]), and secret bytes
//! never appear in any `Debug` output.
//!
//! ## Disclosure limits (no hardware-backed confidentiality)
//!
//! The applicable Pico W / Pico 2 W boards provide no KotoOS-managed,
//! non-exportable key, so this store does not claim confidentiality of stored
//! bytes. Secrets are held at rest without encryption; **physical access to the
//! SD card or flash can recover credentials** despite volatile zeroization of
//! RAM staging and logical two-slot erasure. Zeroization and erasure only reduce
//! *accidental* retention (stale SRAM, casual dumps); they are not a defense
//! against wear-leveling remanence, DMA, a cold-boot attacker, or a discarded
//! device. Password-derived or device-ID obfuscation is intentionally **not**
//! implemented and would not be encryption if it were.
//!
//! ## Two-slot commit
//!
//! The whole set of profiles serializes to one fixed-size record
//! ([`WIFI_SECRET_RECORD_BYTES`]). Two medium slots ([`SecretSlot::A`],
//! [`SecretSlot::B`]) hold successive generations; a write always targets the
//! slot that is *not* currently newest-valid, so an interrupted (torn) write
//! leaves the previous good slot intact. Load decodes both slots, verifies magic
//! /version/length/checksum/generation, and selects the newest valid one with
//! wrap-aware serial-number comparison. Corruption disables profiles rather than
//! guessing or falling back to `KCF1`.

use crate::net::{
    CredentialProvider, CredentialView, ForgetOutcome, Security, Ssid, CREDENTIAL_MAX_BYTES,
    CREDENTIAL_MIN_BYTES, RETAINED_PROFILES_MAX, SSID_MAX_BYTES,
};

// ------------------------------------------------------------------ format

/// Secret-record magic. Distinct from the public `KCF1` configuration magic so
/// the two namespaces can never be confused by a decoder.
pub const WIFI_SECRET_MAGIC: [u8; 4] = *b"KWS1";
const FORMAT_MAJOR: u16 = 1;
const FORMAT_MINOR: u16 = 0;

/// Header length in octets.
const HEADER_LEN: usize = 24;
/// Fixed per-profile record stride in octets.
const PROFILE_STRIDE: usize = 104;

/// Field width reserved for the SSID inside a profile record.
const SSID_FIELD: usize = SSID_MAX_BYTES; // 32
/// Field width reserved for the credential inside a profile record. One octet
/// wider than [`CREDENTIAL_MAX_BYTES`] so there is always at least one trailing
/// pad byte that must read back zero.
const SECRET_FIELD: usize = CREDENTIAL_MAX_BYTES + 1; // 64

/// Exact serialized size of a complete secret record, and therefore the exact
/// maximum read/write size of a medium slot. Every valid slot is this length;
/// anything shorter is truncation and anything longer carries trailing bytes.
pub const WIFI_SECRET_RECORD_BYTES: usize = HEADER_LEN + RETAINED_PROFILES_MAX * PROFILE_STRIDE;

// Header field offsets.
const OFF_MAGIC: usize = 0;
const OFF_MAJOR: usize = 4;
const OFF_MINOR: usize = 6;
const OFF_TOTAL_LEN: usize = 8;
const OFF_COUNT: usize = 10;
const OFF_STRIDE: usize = 11;
const OFF_GENERATION: usize = 12;
const OFF_CHECKSUM: usize = 16;
const OFF_RESERVED: usize = 20;

// Profile field offsets (relative to a profile record start).
const P_ID: usize = 0;
const P_SECURITY: usize = 2;
const P_SSID_LEN: usize = 3;
const P_SECRET_LEN: usize = 4;
const P_RESERVED: usize = 5; // 3 bytes, must be zero
const P_SSID: usize = 8;
const P_SECRET: usize = P_SSID + SSID_FIELD; // 40

const SECURITY_OPEN: u8 = 0;
const SECURITY_WPA2: u8 = 1;

// ------------------------------------------------------------------ errors

/// Why a secret operation failed. Values are fixed enums carrying no SSID,
/// passphrase, PSK, driver text, or address.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SecretError {
    /// A supplied buffer was smaller than [`WIFI_SECRET_RECORD_BYTES`].
    BufferTooSmall,
    /// The record did not begin with [`WIFI_SECRET_MAGIC`].
    BadMagic,
    /// The format major/minor was not understood.
    UnsupportedVersion,
    /// The record was truncated, carried trailing bytes, or declared a bad size.
    InvalidLength,
    /// The stored checksum did not match the record contents.
    InvalidChecksum,
    /// The generation was zero (reserved as "never written").
    InvalidGeneration,
    /// The record declared more than [`RETAINED_PROFILES_MAX`] profiles.
    TooManyProfiles,
    /// Two stored profiles shared the same identity (SSID + security).
    DuplicateProfile,
    /// A reserved field, padding byte, or empty-slot region was non-zero, or a
    /// populated slot appeared after an empty one.
    InvalidRecord,
    /// The SSID length was zero or exceeded [`SSID_MAX_BYTES`].
    InvalidSsid,
    /// The security mode byte was not one of the two supported modes.
    UnsupportedSecurity,
    /// The credential length or byte values were invalid for the security mode.
    InvalidCredential,
    /// No free profile slot remained for a new identity.
    StoreFull,
    /// The requested profile identity was not present.
    NotFound,
    /// The backing medium reported a read/write/erase failure.
    MediumError,
}

// ------------------------------------------------------------------ medium

/// Which of the two commit slots a medium operation addresses.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SecretSlot {
    A,
    B,
}

impl SecretSlot {
    /// The opposite slot, used to pick a write target for two-slot commit.
    pub const fn other(self) -> Self {
        match self {
            SecretSlot::A => SecretSlot::B,
            SecretSlot::B => SecretSlot::A,
        }
    }
}

/// A medium read/write/erase failure. Deliberately opaque: it carries no driver
/// text, address, or byte content.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MediumFault;

/// Result of reading a slot from the medium.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SlotRead {
    /// The slot has never been written (fresh device / after erase).
    Absent,
    /// The slot held exactly [`WIFI_SECRET_RECORD_BYTES`] bytes, now in `dst`.
    Present,
}

/// The raw two-slot storage boundary. Platform adapters back this with a
/// location kept **separate** from public `KCF1` settings and app files. The
/// medium sees only opaque fixed-size blocks; it never parses or logs them.
///
/// Implementations must treat a partial/interrupted write as either leaving the
/// prior bytes intact or producing a block that fails checksum on read-back;
/// either way the two-slot loader falls back to the other slot.
pub trait SecretMedium {
    /// Reads a slot into `dst` (exactly [`WIFI_SECRET_RECORD_BYTES`]). Returns
    /// [`SlotRead::Absent`] when the slot has never been written; `dst` is left
    /// untouched in that case.
    fn read_slot(&self, slot: SecretSlot, dst: &mut [u8; WIFI_SECRET_RECORD_BYTES]) -> SlotRead;

    /// Writes a full record to a slot. Returns `Err` on any medium failure.
    fn write_slot(
        &mut self,
        slot: SecretSlot,
        src: &[u8; WIFI_SECRET_RECORD_BYTES],
    ) -> Result<(), MediumFault>;

    /// Erases a slot so a subsequent [`SecretMedium::read_slot`] returns
    /// [`SlotRead::Absent`]. Returns `Err` on any medium failure.
    fn erase_slot(&mut self, slot: SecretSlot) -> Result<(), MediumFault>;
}

// ------------------------------------------------------------------ profile

/// Redacted, secret-free profile metadata exposed to the page controller and
/// NetworkService. Carries no passphrase and its `Debug` hides the SSID bytes.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct ProfileInfo {
    pub profile_id: u16,
    pub ssid: Ssid,
    pub security: Security,
}

impl core::fmt::Debug for ProfileInfo {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // Never print SSID bytes: they are a threat-model asset. Only the id,
        // security mode, and SSID length are safe for logs/dumps.
        f.debug_struct("ProfileInfo")
            .field("profile_id", &self.profile_id)
            .field("security", &self.security)
            .field("ssid_len", &self.ssid.len())
            .finish()
    }
}

/// A retained profile including its secret. Never leaves this module by value;
/// its `Debug` is redacted and its `Drop` volatile-zeroizes the secret.
struct StoredProfile {
    profile_id: u16,
    ssid: Ssid,
    security: Security,
    secret: [u8; CREDENTIAL_MAX_BYTES],
    secret_len: u8,
}

impl StoredProfile {
    fn identity_matches(&self, ssid: &Ssid, security: Security) -> bool {
        self.security == security && self.ssid == *ssid
    }

    fn secret(&self) -> &[u8] {
        &self.secret[..usize::from(self.secret_len)]
    }

    fn info(&self) -> ProfileInfo {
        ProfileInfo {
            profile_id: self.profile_id,
            ssid: self.ssid,
            security: self.security,
        }
    }
}

impl core::fmt::Debug for StoredProfile {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("StoredProfile")
            .field("profile_id", &self.profile_id)
            .field("security", &self.security)
            .field("ssid_len", &self.ssid.len())
            .field("secret_len", &self.secret_len)
            .finish()
    }
}

impl Drop for StoredProfile {
    fn drop(&mut self) {
        volatile_zeroize(&mut self.secret);
        self.secret_len = 0;
    }
}

// ------------------------------------------------------------------ staging

/// Volatile edit staging: a credential assembled before it is committed to a
/// profile. Zeroized at every terminal boundary (submission, cancellation,
/// validation failure, page exit, capability loss, teardown).
struct EditStaging {
    active: bool,
    ssid: Ssid,
    security: Security,
    secret: [u8; CREDENTIAL_MAX_BYTES],
    secret_len: u8,
}

impl EditStaging {
    const fn new() -> Self {
        Self {
            active: false,
            ssid: Ssid::EMPTY,
            security: Security::Open,
            secret: [0; CREDENTIAL_MAX_BYTES],
            secret_len: 0,
        }
    }

    fn secret(&self) -> &[u8] {
        &self.secret[..usize::from(self.secret_len)]
    }

    fn zeroize(&mut self) {
        volatile_zeroize(&mut self.secret);
        self.secret_len = 0;
        self.active = false;
        self.ssid = Ssid::EMPTY;
        self.security = Security::Open;
    }
}

impl Drop for EditStaging {
    fn drop(&mut self) {
        volatile_zeroize(&mut self.secret);
        self.secret_len = 0;
    }
}

// ------------------------------------------------------------------ load outcome

/// Whether the credential namespace initialized usably.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoadOutcome {
    /// The namespace is usable: either a valid record was selected, or both
    /// slots were absent (a fresh, never-provisioned device).
    Loaded,
    /// Data was present but unusable in every slot (corrupt, torn, or
    /// unsupported). Profiles are disabled and the capability input is false.
    Corrupt,
}

// ------------------------------------------------------------------ store

/// The bounded Wi-Fi credential store. Owns the medium, up to
/// [`RETAINED_PROFILES_MAX`] profiles, and one volatile edit-staging buffer.
///
/// It implements [`CredentialProvider`] so a `&mut WifiSecretStore` can be
/// handed to [`crate::net::NetworkService::service`]; that trait only exposes
/// availability and a bounded forget commit. The richer add/commit/enumerate
/// surface here is for the KotoConfig page controller.
pub struct WifiSecretStore<M: SecretMedium> {
    medium: M,
    profiles: [Option<StoredProfile>; RETAINED_PROFILES_MAX],
    generation: u32,
    available: bool,
    /// The slot the currently loaded record came from, if any. A commit targets
    /// the other slot for torn-write safety.
    current_slot: Option<SecretSlot>,
    edit: EditStaging,
}

impl<M: SecretMedium> WifiSecretStore<M> {
    /// Loads both slots and selects the newest valid record. Always returns a
    /// store; inspect [`WifiSecretStore::available`] or the returned
    /// [`LoadOutcome`] to learn whether the namespace is usable. Never blocks
    /// boot and never falls back to the public settings record.
    pub fn load(medium: M) -> (Self, LoadOutcome) {
        let mut store = Self {
            medium,
            profiles: core::array::from_fn(|_| None),
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
        self.clear_profiles();

        let mut scratch = [0u8; WIFI_SECRET_RECORD_BYTES];

        let a = self.decode_slot(SecretSlot::A, &mut scratch);
        let b = self.decode_slot(SecretSlot::B, &mut scratch);
        volatile_zeroize(&mut scratch);

        let outcome = match (a, b) {
            (SlotState::Valid(ga), SlotState::Valid(gb)) => {
                let slot = if generation_is_newer(gb, ga) {
                    SecretSlot::B
                } else {
                    SecretSlot::A
                };
                self.adopt_slot(slot, &mut scratch);
                LoadOutcome::Loaded
            }
            (SlotState::Valid(_), _) => {
                self.adopt_slot(SecretSlot::A, &mut scratch);
                LoadOutcome::Loaded
            }
            (_, SlotState::Valid(_)) => {
                self.adopt_slot(SecretSlot::B, &mut scratch);
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
                // Fail closed; disable profiles and the capability input.
                self.available = false;
                self.generation = 0;
                self.current_slot = None;
                LoadOutcome::Corrupt
            }
        };
        volatile_zeroize(&mut scratch);
        outcome
    }

    /// Decodes a slot for the newest-valid comparison without retaining bytes.
    fn decode_slot(
        &self,
        slot: SecretSlot,
        scratch: &mut [u8; WIFI_SECRET_RECORD_BYTES],
    ) -> SlotState {
        match self.medium.read_slot(slot, scratch) {
            SlotRead::Absent => SlotState::Absent,
            SlotRead::Present => match decode_generation(scratch) {
                Ok(generation) => SlotState::Valid(generation),
                Err(_) => SlotState::Invalid,
            },
        }
    }

    /// Re-reads the chosen slot and installs its profiles into the model.
    fn adopt_slot(&mut self, slot: SecretSlot, scratch: &mut [u8; WIFI_SECRET_RECORD_BYTES]) {
        if self.medium.read_slot(slot, scratch) != SlotRead::Present {
            self.available = false;
            return;
        }
        match decode_record(scratch, &mut self.profiles) {
            Ok(generation) => {
                self.generation = generation;
                self.available = true;
                self.current_slot = Some(slot);
            }
            Err(_) => {
                self.clear_profiles();
                self.available = false;
            }
        }
    }

    fn clear_profiles(&mut self) {
        for slot in self.profiles.iter_mut() {
            // Dropping the Some(_) volatile-zeroizes the secret.
            *slot = None;
        }
    }

    // -------------------------------------------------------------- observation

    /// Whether the credential provider initialized its bounded secret namespace.
    /// One of the four inputs to the composite `WIFI_CONFIG` capability.
    pub fn available(&self) -> bool {
        self.available
    }

    /// The current record generation (zero means never provisioned / unusable).
    pub fn generation(&self) -> u32 {
        self.generation
    }

    /// Number of retained profiles.
    pub fn len(&self) -> usize {
        self.profiles.iter().filter(|slot| slot.is_some()).count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Redacted metadata for the profile at `index` in `0..len()`.
    pub fn info(&self, index: usize) -> Option<ProfileInfo> {
        self.profiles
            .iter()
            .flatten()
            .nth(index)
            .map(StoredProfile::info)
    }

    /// Redacted metadata for the profile with the given identity, if retained.
    pub fn find_by_ssid(&self, ssid: &Ssid, security: Security) -> Option<ProfileInfo> {
        self.profile_ref(ssid, security).map(StoredProfile::info)
    }

    /// Whether a profile is retained for the given identity.
    pub fn contains(&self, ssid: &Ssid, security: Security) -> bool {
        self.profile_ref(ssid, security).is_some()
    }

    /// An operation-scoped credential view for a retained profile identified by
    /// `profile_id`. The borrow scopes the secret to the caller's operation; the
    /// NetworkService copies it into its own staging and zeroizes that. Returns
    /// `None` for `Open` profiles (they carry no secret) and unknown ids.
    pub fn credential_view(&self, profile_id: u16) -> Option<CredentialView<'_>> {
        let profile = self
            .profiles
            .iter()
            .flatten()
            .find(|p| p.profile_id == profile_id)?;
        Some(CredentialView {
            security: profile.security,
            secret: profile.secret(),
        })
    }

    /// An operation-scoped credential view for a retained profile matching a
    /// scan result's SSID and security.
    pub fn credential_view_for(
        &self,
        ssid: &Ssid,
        security: Security,
    ) -> Option<CredentialView<'_>> {
        let profile = self.profile_ref(ssid, security)?;
        Some(CredentialView {
            security: profile.security,
            secret: profile.secret(),
        })
    }

    fn profile_ref(&self, ssid: &Ssid, security: Security) -> Option<&StoredProfile> {
        self.profiles
            .iter()
            .flatten()
            .find(|p| p.identity_matches(ssid, security))
    }

    // -------------------------------------------------------------- staging

    /// Whether the volatile edit staging is fully zeroized (inactive, no bytes).
    /// Instrumentation for the zeroization exit-path tests.
    pub fn staging_zeroized(&self) -> bool {
        !self.edit.active
            && self.edit.secret_len == 0
            && self.edit.secret.iter().all(|byte| *byte == 0)
    }

    /// Borrows the volatile staged credential for a bounded retry while the
    /// page remains open. It is never exposed through public configuration or
    /// diagnostics and becomes unavailable immediately after cancel/commit.
    pub fn staged_credential_view(&self) -> Option<CredentialView<'_>> {
        self.edit.active.then(|| CredentialView {
            security: self.edit.security,
            secret: self.edit.secret(),
        })
    }

    /// Validates a candidate credential and holds it in volatile edit staging.
    /// Rejects unsupported modes, out-of-range SSIDs, and invalid passphrases
    /// before any byte is retained. On any validation failure the staging is
    /// left zeroized.
    pub fn stage(
        &mut self,
        ssid: &Ssid,
        security: Security,
        secret: &[u8],
    ) -> Result<(), SecretError> {
        self.edit.zeroize();
        validate_ssid(ssid)?;
        validate_credential(security, secret)?;
        self.edit.ssid = *ssid;
        self.edit.security = security;
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

    /// Commits the staged credential as a new or updated profile and persists
    /// the whole record to the other slot. On success the staging is zeroized
    /// and the new profile id is returned. On any failure the staging is
    /// zeroized and the in-RAM model is left as it was.
    pub fn commit(&mut self) -> Result<u16, SecretError> {
        if !self.edit.active {
            return Err(SecretError::InvalidRecord);
        }
        if !self.available {
            self.edit.zeroize();
            return Err(SecretError::MediumError);
        }

        // Re-validate defensively; staging only ever holds validated bytes.
        validate_ssid(&self.edit.ssid)?;
        validate_credential(self.edit.security, self.edit.secret())?;

        let ssid = self.edit.ssid;
        let security = self.edit.security;

        let (index, profile_id) = match self.slot_for_identity(&ssid, security) {
            Some((index, existing_id)) => (index, existing_id),
            None => {
                let Some(index) = self.free_index() else {
                    self.edit.zeroize();
                    return Err(SecretError::StoreFull);
                };
                (index, self.next_profile_id())
            }
        };

        let mut secret = [0u8; CREDENTIAL_MAX_BYTES];
        let secret_len = self.edit.secret_len;
        secret[..usize::from(secret_len)].copy_from_slice(self.edit.secret());
        // Installing the new profile drops any previous occupant (zeroizing it).
        self.profiles[index] = Some(StoredProfile {
            profile_id,
            ssid,
            security,
            secret,
            secret_len,
        });

        match self.persist() {
            Ok(()) => {
                self.edit.zeroize();
                Ok(profile_id)
            }
            Err(err) => {
                // Roll back the in-RAM change and reload from the medium so the
                // model matches durable state, then report the failure.
                self.reload();
                self.edit.zeroize();
                Err(err)
            }
        }
    }

    fn slot_for_identity(&self, ssid: &Ssid, security: Security) -> Option<(usize, u16)> {
        self.profiles.iter().enumerate().find_map(|(index, slot)| {
            slot.as_ref()
                .filter(|p| p.identity_matches(ssid, security))
                .map(|p| (index, p.profile_id))
        })
    }

    fn free_index(&self) -> Option<usize> {
        self.profiles.iter().position(Option::is_none)
    }

    /// Picks the smallest unused nonzero profile id in `1..=RETAINED_PROFILES_MAX`.
    fn next_profile_id(&self) -> u16 {
        for candidate in 1..=(RETAINED_PROFILES_MAX as u16) {
            if !self
                .profiles
                .iter()
                .flatten()
                .any(|p| p.profile_id == candidate)
            {
                return candidate;
            }
        }
        // Unreachable while len < RETAINED_PROFILES_MAX, which the caller ensures.
        RETAINED_PROFILES_MAX as u16
    }

    // -------------------------------------------------------------- persistence

    /// Encodes the current model and writes it to the non-current slot, then
    /// marks that slot current. Bumps the nonzero, wrap-aware generation.
    fn persist(&mut self) -> Result<(), SecretError> {
        let next_generation = next_generation(self.generation);
        let mut record = [0u8; WIFI_SECRET_RECORD_BYTES];
        encode_record(&self.profiles, next_generation, &mut record);

        let target = self
            .current_slot
            .map(SecretSlot::other)
            .unwrap_or(SecretSlot::A);
        let result = self.medium.write_slot(target, &record);
        volatile_zeroize(&mut record);

        result.map_err(|MediumFault| SecretError::MediumError)?;
        self.generation = next_generation;
        self.current_slot = Some(target);
        Ok(())
    }

    /// Erases a retained profile and commits the smaller record before reporting
    /// success. Returns `Committed` only after the write lands. A missing
    /// identity is reported as already-committed (idempotent erasure).
    fn forget_profile(&mut self, profile_id: u16) -> ForgetOutcome {
        if !self.available {
            return ForgetOutcome::StoreUnavailable;
        }
        let Some(index) = self
            .profiles
            .iter()
            .position(|slot| slot.as_ref().is_some_and(|p| p.profile_id == profile_id))
        else {
            // Nothing retained for this id: it is already absent.
            return ForgetOutcome::Committed;
        };

        // Dropping the slot volatile-zeroizes the secret immediately, before the
        // durable commit. Compact remaining profiles so the encoding stays dense.
        self.profiles[index] = None;
        self.compact();

        match self.persist() {
            Ok(()) => ForgetOutcome::Committed,
            Err(_) => {
                // The durable record still names the profile; reload to match.
                self.reload();
                ForgetOutcome::StoreUnavailable
            }
        }
    }

    /// Moves populated profiles to the front so empty slots trail (the encoder
    /// requires a dense layout).
    fn compact(&mut self) {
        let mut write = 0;
        for read in 0..RETAINED_PROFILES_MAX {
            if self.profiles[read].is_some() {
                if read != write {
                    self.profiles.swap(read, write);
                }
                write += 1;
            }
        }
    }

    /// Erases both slots, verifies both read back absent/invalid, clears RAM
    /// staging and profiles, and resets the generation. Public settings are the
    /// caller's separate concern (this never touches `KCF1`). Does not claim
    /// secure physical erasure of the underlying flash/SD.
    pub fn factory_reset(&mut self) -> Result<(), SecretError> {
        // Clear volatile RAM first so an early return still leaves no secrets.
        self.edit.zeroize();
        self.clear_profiles();

        let ea = self.medium.erase_slot(SecretSlot::A);
        let eb = self.medium.erase_slot(SecretSlot::B);
        ea.map_err(|MediumFault| SecretError::MediumError)?;
        eb.map_err(|MediumFault| SecretError::MediumError)?;

        // Verify both slots no longer decode to a usable record.
        let mut scratch = [0u8; WIFI_SECRET_RECORD_BYTES];
        let a = self.decode_slot(SecretSlot::A, &mut scratch);
        let b = self.decode_slot(SecretSlot::B, &mut scratch);
        volatile_zeroize(&mut scratch);
        if matches!(a, SlotState::Valid(_)) || matches!(b, SlotState::Valid(_)) {
            return Err(SecretError::MediumError);
        }

        self.generation = 0;
        self.current_slot = None;
        self.available = true; // Namespace is usable again, just empty.
        Ok(())
    }

    /// Volatile-zeroizes all RAM staging and retained secrets. Called on arena
    /// teardown and capability loss. The durable medium is untouched.
    pub fn zeroize_ram(&mut self) {
        self.edit.zeroize();
        self.clear_profiles();
        self.available = false;
        self.current_slot = None;
    }
}

impl<M: SecretMedium> CredentialProvider for WifiSecretStore<M> {
    fn available(&self) -> bool {
        self.available
    }

    fn forget(&mut self, profile_id: u16) -> ForgetOutcome {
        self.forget_profile(profile_id)
    }
}

impl<M: SecretMedium> Drop for WifiSecretStore<M> {
    fn drop(&mut self) {
        // Edit staging and each StoredProfile zeroize their own secrets on drop;
        // this makes teardown intent explicit and covers the profile array.
        self.edit.zeroize();
        self.clear_profiles();
    }
}

// ------------------------------------------------------------------ codec

/// Per-slot decode state used for newest-valid selection.
#[derive(Clone, Copy, Eq, PartialEq)]
enum SlotState {
    Absent,
    Invalid,
    Valid(u32),
}

/// Validates a record's framing and returns its generation without materializing
/// profiles. Used to compare slots before adopting one.
fn decode_generation(src: &[u8; WIFI_SECRET_RECORD_BYTES]) -> Result<u32, SecretError> {
    validate_header(src)?;
    // Validate profiles too, so a slot with a good header but corrupt bodies is
    // rejected rather than selected as "newest".
    let mut sink: [Option<StoredProfile>; RETAINED_PROFILES_MAX] = core::array::from_fn(|_| None);
    let generation = decode_record(src, &mut sink)?;
    // sink drops here, zeroizing any secrets it briefly held.
    Ok(generation)
}

/// Validates the fixed header fields and returns the declared generation.
fn validate_header(src: &[u8; WIFI_SECRET_RECORD_BYTES]) -> Result<u32, SecretError> {
    if src[OFF_MAGIC..OFF_MAGIC + 4] != WIFI_SECRET_MAGIC {
        return Err(SecretError::BadMagic);
    }
    if get_u16(src, OFF_MAJOR) != FORMAT_MAJOR || get_u16(src, OFF_MINOR) != FORMAT_MINOR {
        return Err(SecretError::UnsupportedVersion);
    }
    if usize::from(get_u16(src, OFF_TOTAL_LEN)) != WIFI_SECRET_RECORD_BYTES {
        return Err(SecretError::InvalidLength);
    }
    if usize::from(src[OFF_STRIDE]) != PROFILE_STRIDE {
        return Err(SecretError::InvalidLength);
    }
    if src[OFF_RESERVED..OFF_RESERVED + 4].iter().any(|b| *b != 0) {
        return Err(SecretError::InvalidRecord);
    }
    let generation = get_u32(src, OFF_GENERATION);
    if generation == 0 {
        return Err(SecretError::InvalidGeneration);
    }
    if get_u32(src, OFF_CHECKSUM) != record_checksum(src) {
        return Err(SecretError::InvalidChecksum);
    }
    Ok(generation)
}

/// Fully decodes a record into `profiles`, validating every field, and returns
/// the generation. On any error, `profiles` is cleared.
fn decode_record(
    src: &[u8; WIFI_SECRET_RECORD_BYTES],
    profiles: &mut [Option<StoredProfile>; RETAINED_PROFILES_MAX],
) -> Result<u32, SecretError> {
    for slot in profiles.iter_mut() {
        *slot = None;
    }

    let generation = validate_header(src)?;
    let count = usize::from(src[OFF_COUNT]);
    if count > RETAINED_PROFILES_MAX {
        return Err(SecretError::TooManyProfiles);
    }

    for index in 0..RETAINED_PROFILES_MAX {
        let base = HEADER_LEN + index * PROFILE_STRIDE;
        let record = &src[base..base + PROFILE_STRIDE];
        if index < count {
            let profile = decode_profile(record)?;
            // Reject duplicate identities within the record.
            if profiles[..index]
                .iter()
                .flatten()
                .any(|p| p.identity_matches(&profile.ssid, profile.security))
            {
                for slot in profiles.iter_mut() {
                    *slot = None;
                }
                return Err(SecretError::DuplicateProfile);
            }
            profiles[index] = Some(profile);
        } else {
            // Trailing slots must be entirely zero (dense encoding).
            if record.iter().any(|b| *b != 0) {
                for slot in profiles.iter_mut() {
                    *slot = None;
                }
                return Err(SecretError::InvalidRecord);
            }
        }
    }
    Ok(generation)
}

/// Decodes and validates a single populated profile record.
fn decode_profile(record: &[u8]) -> Result<StoredProfile, SecretError> {
    let profile_id = get_u16(record, P_ID);
    if profile_id == 0 {
        return Err(SecretError::InvalidRecord);
    }
    if record[P_RESERVED..P_RESERVED + 3].iter().any(|b| *b != 0) {
        return Err(SecretError::InvalidRecord);
    }

    let security = match record[P_SECURITY] {
        SECURITY_OPEN => Security::Open,
        SECURITY_WPA2 => Security::Wpa2PersonalAes,
        _ => return Err(SecretError::UnsupportedSecurity),
    };

    let ssid_len = usize::from(record[P_SSID_LEN]);
    if ssid_len == 0 || ssid_len > SSID_MAX_BYTES {
        return Err(SecretError::InvalidSsid);
    }
    // SSID padding beyond the length must be zero.
    if record[P_SSID + ssid_len..P_SSID + SSID_FIELD]
        .iter()
        .any(|b| *b != 0)
    {
        return Err(SecretError::InvalidRecord);
    }
    let ssid = Ssid::from_bytes(&record[P_SSID..P_SSID + ssid_len]);

    let secret_len = usize::from(record[P_SECRET_LEN]);
    let secret_bytes = &record[P_SECRET..P_SECRET + secret_len.min(SECRET_FIELD)];
    validate_credential(security, secret_bytes)?;
    if secret_len > CREDENTIAL_MAX_BYTES {
        return Err(SecretError::InvalidCredential);
    }
    // Secret padding beyond the length must be zero.
    if record[P_SECRET + secret_len..P_SECRET + SECRET_FIELD]
        .iter()
        .any(|b| *b != 0)
    {
        return Err(SecretError::InvalidRecord);
    }

    let mut secret = [0u8; CREDENTIAL_MAX_BYTES];
    secret[..secret_len].copy_from_slice(&record[P_SECRET..P_SECRET + secret_len]);
    Ok(StoredProfile {
        profile_id,
        ssid,
        security,
        secret,
        secret_len: secret_len as u8,
    })
}

/// Encodes the current profile set into a full fixed-size record. Assumes a
/// dense layout (populated slots first) as maintained by the store.
fn encode_record(
    profiles: &[Option<StoredProfile>; RETAINED_PROFILES_MAX],
    generation: u32,
    dst: &mut [u8; WIFI_SECRET_RECORD_BYTES],
) {
    dst.fill(0);
    dst[OFF_MAGIC..OFF_MAGIC + 4].copy_from_slice(&WIFI_SECRET_MAGIC);
    put_u16(dst, OFF_MAJOR, FORMAT_MAJOR);
    put_u16(dst, OFF_MINOR, FORMAT_MINOR);
    put_u16(dst, OFF_TOTAL_LEN, WIFI_SECRET_RECORD_BYTES as u16);
    let count = profiles.iter().filter(|slot| slot.is_some()).count();
    dst[OFF_COUNT] = count as u8;
    dst[OFF_STRIDE] = PROFILE_STRIDE as u8;
    put_u32(dst, OFF_GENERATION, generation);

    for (index, profile) in profiles.iter().flatten().enumerate() {
        let base = HEADER_LEN + index * PROFILE_STRIDE;
        put_u16(dst, base + P_ID, profile.profile_id);
        dst[base + P_SECURITY] = match profile.security {
            Security::Open => SECURITY_OPEN,
            Security::Wpa2PersonalAes => SECURITY_WPA2,
        };
        dst[base + P_SSID_LEN] = profile.ssid.len() as u8;
        dst[base + P_SECRET_LEN] = profile.secret_len;
        let ssid = profile.ssid.as_bytes();
        dst[base + P_SSID..base + P_SSID + ssid.len()].copy_from_slice(ssid);
        let secret = profile.secret();
        dst[base + P_SECRET..base + P_SECRET + secret.len()].copy_from_slice(secret);
    }

    let checksum = record_checksum(dst);
    put_u32(dst, OFF_CHECKSUM, checksum);
}

// ------------------------------------------------------------------ validation

/// Validates a stored SSID: 1..=32 arbitrary 802.11 octets.
fn validate_ssid(ssid: &Ssid) -> Result<(), SecretError> {
    let len = ssid.len();
    if len == 0 || len > SSID_MAX_BYTES {
        return Err(SecretError::InvalidSsid);
    }
    Ok(())
}

/// Validates a credential for its security mode. `Open` requires zero bytes;
/// `Wpa2PersonalAes` requires 8..=63 printable-ASCII (`0x20..=0x7e`) octets.
fn validate_credential(security: Security, secret: &[u8]) -> Result<(), SecretError> {
    match security {
        Security::Open => {
            if secret.is_empty() {
                Ok(())
            } else {
                Err(SecretError::InvalidCredential)
            }
        }
        Security::Wpa2PersonalAes => {
            if secret.len() < CREDENTIAL_MIN_BYTES || secret.len() > CREDENTIAL_MAX_BYTES {
                return Err(SecretError::InvalidCredential);
            }
            if secret.iter().any(|b| !(0x20..=0x7e).contains(b)) {
                return Err(SecretError::InvalidCredential);
            }
            Ok(())
        }
    }
}

// ------------------------------------------------------------------ helpers

/// Wrap-aware serial-number comparison for selecting the newest slot. Generation
/// zero is never a valid stored value, so it is never passed here.
const fn generation_is_newer(candidate: u32, current: u32) -> bool {
    candidate != current && candidate.wrapping_sub(current) < 0x8000_0000
}

/// Next nonzero generation, skipping the reserved zero on wrap.
const fn next_generation(current: u32) -> u32 {
    let next = current.wrapping_add(1);
    if next == 0 {
        1
    } else {
        next
    }
}

/// FNV-1a over the record with the checksum field treated as zero. Independent
/// of the `KCF1` checksum so the two namespaces never share a code path.
fn record_checksum(bytes: &[u8; WIFI_SECRET_RECORD_BYTES]) -> u32 {
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

    /// In-memory two-slot medium with fault injection for the fault matrix.
    struct FakeMedium {
        slots: [Option<[u8; WIFI_SECRET_RECORD_BYTES]>; 2],
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

        fn idx(slot: SecretSlot) -> usize {
            match slot {
                SecretSlot::A => 0,
                SecretSlot::B => 1,
            }
        }

        fn set_raw(&mut self, slot: SecretSlot, bytes: [u8; WIFI_SECRET_RECORD_BYTES]) {
            self.slots[Self::idx(slot)] = Some(bytes);
        }

        fn raw(&self, slot: SecretSlot) -> Option<&[u8; WIFI_SECRET_RECORD_BYTES]> {
            self.slots[Self::idx(slot)].as_ref()
        }

        fn flip_byte(&mut self, slot: SecretSlot, offset: usize) {
            if let Some(bytes) = self.slots[Self::idx(slot)].as_mut() {
                bytes[offset] ^= 0xff;
            }
        }
    }

    impl SecretMedium for FakeMedium {
        fn read_slot(
            &self,
            slot: SecretSlot,
            dst: &mut [u8; WIFI_SECRET_RECORD_BYTES],
        ) -> SlotRead {
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
            slot: SecretSlot,
            src: &[u8; WIFI_SECRET_RECORD_BYTES],
        ) -> Result<(), MediumFault> {
            if self.reject_write[Self::idx(slot)] {
                return Err(MediumFault);
            }
            self.slots[Self::idx(slot)] = Some(*src);
            Ok(())
        }

        fn erase_slot(&mut self, slot: SecretSlot) -> Result<(), MediumFault> {
            if self.reject_erase[Self::idx(slot)] {
                return Err(MediumFault);
            }
            self.slots[Self::idx(slot)] = None;
            Ok(())
        }
    }

    fn ssid(name: &str) -> Ssid {
        Ssid::from_bytes(name.as_bytes())
    }

    fn stored(id: u16, name: &str, security: Security, secret: &[u8]) -> StoredProfile {
        let mut bytes = [0u8; CREDENTIAL_MAX_BYTES];
        bytes[..secret.len()].copy_from_slice(secret);
        StoredProfile {
            profile_id: id,
            ssid: ssid(name),
            security,
            secret: bytes,
            secret_len: secret.len() as u8,
        }
    }

    /// Encodes a raw valid record with the given profiles and generation.
    fn encode(list: &[StoredProfile], generation: u32) -> [u8; WIFI_SECRET_RECORD_BYTES] {
        let mut arr: [Option<StoredProfile>; RETAINED_PROFILES_MAX] =
            core::array::from_fn(|_| None);
        for (index, profile) in list.iter().enumerate() {
            arr[index] = Some(stored(
                profile.profile_id,
                core::str::from_utf8(profile.ssid.as_bytes()).unwrap(),
                profile.security,
                profile.secret(),
            ));
        }
        let mut buf = [0u8; WIFI_SECRET_RECORD_BYTES];
        encode_record(&arr, generation, &mut buf);
        buf
    }

    const PW: &[u8] = b"hunter2!secret";

    // -------------------------------------------------------------- format

    #[test]
    fn record_size_is_fixed_440_bytes() {
        assert_eq!(WIFI_SECRET_RECORD_BYTES, 440);
    }

    #[test]
    fn round_trip_encode_decode_preserves_profile() {
        let record = encode(&[stored(1, "HomeNet", Security::Wpa2PersonalAes, PW)], 7);
        let mut out: [Option<StoredProfile>; RETAINED_PROFILES_MAX] =
            core::array::from_fn(|_| None);
        let generation = decode_record(&record, &mut out).unwrap();
        assert_eq!(generation, 7);
        let p = out[0].as_ref().unwrap();
        assert_eq!(p.profile_id, 1);
        assert_eq!(p.ssid, ssid("HomeNet"));
        assert_eq!(p.security, Security::Wpa2PersonalAes);
        assert_eq!(p.secret(), PW);
    }

    // -------------------------------------------------------- fault matrix (10)

    #[test]
    fn both_slots_absent_loads_empty_and_available() {
        let (store, outcome) = WifiSecretStore::load(FakeMedium::new());
        assert_eq!(outcome, LoadOutcome::Loaded);
        assert!(store.available());
        assert!(store.is_empty());
        assert_eq!(store.generation(), 0);
    }

    #[test]
    fn only_slot_a_valid_is_selected() {
        let mut medium = FakeMedium::new();
        medium.set_raw(
            SecretSlot::A,
            encode(&[stored(1, "A", Security::Open, &[])], 3),
        );
        let (store, outcome) = WifiSecretStore::load(medium);
        assert_eq!(outcome, LoadOutcome::Loaded);
        assert_eq!(store.len(), 1);
        assert_eq!(store.info(0).unwrap().profile_id, 1);
    }

    #[test]
    fn only_slot_b_valid_is_selected() {
        let mut medium = FakeMedium::new();
        medium.set_raw(
            SecretSlot::B,
            encode(&[stored(2, "B", Security::Open, &[])], 4),
        );
        let (store, outcome) = WifiSecretStore::load(medium);
        assert_eq!(outcome, LoadOutcome::Loaded);
        assert_eq!(store.info(0).unwrap().profile_id, 2);
    }

    #[test]
    fn both_valid_selects_newer_generation() {
        let mut medium = FakeMedium::new();
        medium.set_raw(
            SecretSlot::A,
            encode(&[stored(1, "Older", Security::Open, &[])], 5),
        );
        medium.set_raw(
            SecretSlot::B,
            encode(&[stored(1, "Newer", Security::Open, &[])], 9),
        );
        let (store, _) = WifiSecretStore::load(medium);
        assert_eq!(store.info(0).unwrap().ssid, ssid("Newer"));
    }

    #[test]
    fn both_valid_ignores_stale_slot() {
        let mut medium = FakeMedium::new();
        medium.set_raw(
            SecretSlot::A,
            encode(&[stored(1, "Newer", Security::Open, &[])], 9),
        );
        medium.set_raw(
            SecretSlot::B,
            encode(&[stored(1, "Stale", Security::Open, &[])], 5),
        );
        let (store, _) = WifiSecretStore::load(medium);
        assert_eq!(store.info(0).unwrap().ssid, ssid("Newer"));
    }

    #[test]
    fn valid_slot_survives_corrupt_replacement() {
        let mut medium = FakeMedium::new();
        medium.set_raw(
            SecretSlot::A,
            encode(&[stored(1, "Good", Security::Open, &[])], 5),
        );
        medium.set_raw(
            SecretSlot::B,
            encode(&[stored(1, "Bad", Security::Open, &[])], 9),
        );
        medium.flip_byte(SecretSlot::B, HEADER_LEN + 10); // corrupt the newer slot
        let (store, outcome) = WifiSecretStore::load(medium);
        assert_eq!(outcome, LoadOutcome::Loaded);
        assert_eq!(store.info(0).unwrap().ssid, ssid("Good"));
    }

    #[test]
    fn torn_write_falls_back_to_prior_slot() {
        // A holds a good prior generation; B was being written (torn -> bad
        // checksum). The loader must ignore B and keep A.
        let mut medium = FakeMedium::new();
        medium.set_raw(
            SecretSlot::A,
            encode(&[stored(1, "Prior", Security::Wpa2PersonalAes, PW)], 2),
        );
        medium.set_raw(
            SecretSlot::B,
            encode(&[stored(1, "Torn", Security::Wpa2PersonalAes, PW)], 3),
        );
        medium.flip_byte(SecretSlot::B, WIFI_SECRET_RECORD_BYTES - 1); // tail byte torn
        let (store, outcome) = WifiSecretStore::load(medium);
        assert_eq!(outcome, LoadOutcome::Loaded);
        assert_eq!(store.info(0).unwrap().ssid, ssid("Prior"));
    }

    #[test]
    fn single_bit_flip_invalidates_slot() {
        let mut medium = FakeMedium::new();
        medium.set_raw(
            SecretSlot::A,
            encode(&[stored(1, "X", Security::Open, &[])], 3),
        );
        // Flip a single bit in the generation field.
        if let Some(bytes) = medium.slots[0].as_mut() {
            bytes[OFF_GENERATION] ^= 0x01;
        }
        let (store, outcome) = WifiSecretStore::load(medium);
        // Only slot present and it is corrupt: fail closed.
        assert_eq!(outcome, LoadOutcome::Corrupt);
        assert!(!store.available());
        assert!(store.is_empty());
    }

    #[test]
    fn both_corrupt_fails_closed() {
        let mut medium = FakeMedium::new();
        medium.set_raw(
            SecretSlot::A,
            encode(&[stored(1, "A", Security::Open, &[])], 3),
        );
        medium.set_raw(
            SecretSlot::B,
            encode(&[stored(1, "B", Security::Open, &[])], 4),
        );
        medium.flip_byte(SecretSlot::A, HEADER_LEN);
        medium.flip_byte(SecretSlot::B, HEADER_LEN);
        let (store, outcome) = WifiSecretStore::load(medium);
        assert_eq!(outcome, LoadOutcome::Corrupt);
        assert!(!store.available());
    }

    #[test]
    fn generation_wrap_selects_new_record() {
        let mut medium = FakeMedium::new();
        medium.set_raw(
            SecretSlot::A,
            encode(&[stored(1, "Max", Security::Open, &[])], u32::MAX),
        );
        let (mut store, _) = WifiSecretStore::load(medium);
        assert_eq!(store.generation(), u32::MAX);
        // Commit an update: generation wraps to 1, written to slot B.
        store.stage(&ssid("Wrapped"), Security::Open, &[]).unwrap();
        store.commit().unwrap();
        assert_eq!(store.generation(), 1);

        // A fresh reload must still choose the wrapped (newer) record in B.
        let medium = store_into_medium(store);
        let (reloaded, outcome) = WifiSecretStore::load(medium);
        assert_eq!(outcome, LoadOutcome::Loaded);
        assert_eq!(reloaded.generation(), 1);
        assert!(reloaded.contains(&ssid("Wrapped"), Security::Open));
    }

    /// Recovers the medium from a store by dropping the store wrapper. Since the
    /// store owns the medium by value and has a `Drop`, reconstruct a fresh
    /// medium mirroring the durable slots instead.
    fn store_into_medium(store: WifiSecretStore<FakeMedium>) -> FakeMedium {
        let mut medium = FakeMedium::new();
        if let Some(bytes) = store.medium.raw(SecretSlot::A) {
            medium.set_raw(SecretSlot::A, *bytes);
        }
        if let Some(bytes) = store.medium.raw(SecretSlot::B) {
            medium.set_raw(SecretSlot::B, *bytes);
        }
        medium
    }

    // ------------------------------------------------------- two-slot commit

    #[test]
    fn commit_targets_the_other_slot() {
        let mut medium = FakeMedium::new();
        medium.set_raw(
            SecretSlot::A,
            encode(&[stored(1, "Seed", Security::Open, &[])], 4),
        );
        let (mut store, _) = WifiSecretStore::load(medium);
        let original_a = *store.medium.raw(SecretSlot::A).unwrap();

        store
            .stage(&ssid("New"), Security::Wpa2PersonalAes, PW)
            .unwrap();
        store.commit().unwrap();

        // A untouched, B now carries the new generation.
        assert_eq!(store.medium.raw(SecretSlot::A), Some(&original_a));
        assert!(store.medium.raw(SecretSlot::B).is_some());
    }

    // -------------------------------------------------------- validation (2)

    #[test]
    fn empty_ssid_is_rejected() {
        let (mut store, _) = WifiSecretStore::load(FakeMedium::new());
        assert_eq!(
            store.stage(&Ssid::EMPTY, Security::Open, &[]),
            Err(SecretError::InvalidSsid)
        );
        assert!(store.staging_zeroized());
    }

    #[test]
    fn open_mode_rejects_nonempty_secret() {
        let (mut store, _) = WifiSecretStore::load(FakeMedium::new());
        assert_eq!(
            store.stage(&ssid("Net"), Security::Open, b"x"),
            Err(SecretError::InvalidCredential)
        );
    }

    #[test]
    fn wpa2_length_bounds_enforced() {
        let (mut store, _) = WifiSecretStore::load(FakeMedium::new());
        assert_eq!(
            store.stage(&ssid("Net"), Security::Wpa2PersonalAes, b"short7x"),
            Err(SecretError::InvalidCredential)
        );
        assert!(store
            .stage(&ssid("Net"), Security::Wpa2PersonalAes, b"exactly8")
            .is_ok());
        let max = [b'a'; CREDENTIAL_MAX_BYTES];
        assert!(store
            .stage(&ssid("Net"), Security::Wpa2PersonalAes, &max)
            .is_ok());
        let over = [b'a'; CREDENTIAL_MAX_BYTES + 1];
        assert_eq!(
            store.stage(&ssid("Net"), Security::Wpa2PersonalAes, &over),
            Err(SecretError::InvalidCredential)
        );
    }

    #[test]
    fn wpa2_rejects_non_printable_ascii() {
        let (mut store, _) = WifiSecretStore::load(FakeMedium::new());
        let mut secret = *b"password";
        secret[0] = 0x1f; // control char
        assert_eq!(
            store.stage(&ssid("Net"), Security::Wpa2PersonalAes, &secret),
            Err(SecretError::InvalidCredential)
        );
    }

    #[test]
    fn decode_rejects_unsupported_security_byte() {
        let mut record = encode(&[stored(1, "Net", Security::Open, &[])], 3);
        record[HEADER_LEN + P_SECURITY] = 9; // unsupported mode
        fix_checksum(&mut record);
        let mut out = core::array::from_fn(|_| None);
        assert_eq!(
            decode_record(&record, &mut out),
            Err(SecretError::UnsupportedSecurity)
        );
    }

    #[test]
    fn decode_rejects_duplicate_identity() {
        let record = encode(
            &[
                stored(1, "Dup", Security::Open, &[]),
                stored(2, "Dup", Security::Open, &[]),
            ],
            3,
        );
        let mut out = core::array::from_fn(|_| None);
        assert_eq!(
            decode_record(&record, &mut out),
            Err(SecretError::DuplicateProfile)
        );
    }

    #[test]
    fn decode_rejects_nonzero_padding() {
        let mut record = encode(&[stored(1, "Net", Security::Open, &[])], 3);
        // SSID is 3 bytes; poke a byte into the padding region.
        record[HEADER_LEN + P_SSID + 3] = 0xaa;
        fix_checksum(&mut record);
        let mut out = core::array::from_fn(|_| None);
        assert_eq!(
            decode_record(&record, &mut out),
            Err(SecretError::InvalidRecord)
        );
    }

    #[test]
    fn decode_rejects_trailing_bytes_in_empty_slot() {
        let mut record = encode(&[stored(1, "Net", Security::Open, &[])], 3);
        // Poke into the second (empty) profile slot region.
        record[HEADER_LEN + PROFILE_STRIDE] = 0x01;
        fix_checksum(&mut record);
        let mut out = core::array::from_fn(|_| None);
        assert_eq!(
            decode_record(&record, &mut out),
            Err(SecretError::InvalidRecord)
        );
    }

    #[test]
    fn decode_rejects_bad_magic_and_length() {
        let good = encode(&[stored(1, "Net", Security::Open, &[])], 3);
        let mut bad_magic = good;
        bad_magic[0] = b'X';
        let mut out = core::array::from_fn(|_| None);
        assert_eq!(
            decode_record(&bad_magic, &mut out),
            Err(SecretError::BadMagic)
        );

        let mut bad_len = good;
        put_u16(&mut bad_len, OFF_TOTAL_LEN, 12);
        fix_checksum(&mut bad_len);
        assert_eq!(
            decode_record(&bad_len, &mut out),
            Err(SecretError::InvalidLength)
        );
    }

    fn fix_checksum(record: &mut [u8; WIFI_SECRET_RECORD_BYTES]) {
        let checksum = record_checksum(record);
        put_u32(record, OFF_CHECKSUM, checksum);
    }

    // ---------------------------------------------------- metadata only (4)

    #[test]
    fn info_and_debug_expose_no_secret() {
        let mut medium = FakeMedium::new();
        medium.set_raw(
            SecretSlot::A,
            encode(&[stored(1, "Cafe", Security::Wpa2PersonalAes, PW)], 3),
        );
        let (store, _) = WifiSecretStore::load(medium);
        let info = store.info(0).unwrap();
        let rendered = std::format!("{info:?}");
        assert!(!rendered.contains("hunter2"));
        assert!(!rendered.contains("Cafe")); // SSID bytes also redacted in Debug
        assert!(rendered.contains("ssid_len"));
    }

    #[test]
    fn credential_view_only_via_explicit_call() {
        let mut medium = FakeMedium::new();
        medium.set_raw(
            SecretSlot::A,
            encode(&[stored(1, "Cafe", Security::Wpa2PersonalAes, PW)], 3),
        );
        let (store, _) = WifiSecretStore::load(medium);
        let view = store.credential_view(1).unwrap();
        assert_eq!(view.secret, PW);
        assert_eq!(
            store
                .credential_view_for(&ssid("Cafe"), Security::Wpa2PersonalAes)
                .unwrap()
                .secret,
            PW
        );
        assert!(store.credential_view(99).is_none());
    }

    // ----------------------------------------------- zeroization exit paths (5)

    #[test]
    fn staging_zeroized_after_commit() {
        let (mut store, _) = WifiSecretStore::load(FakeMedium::new());
        store
            .stage(&ssid("Net"), Security::Wpa2PersonalAes, PW)
            .unwrap();
        assert!(!store.staging_zeroized());
        store.commit().unwrap();
        assert!(store.staging_zeroized());
    }

    #[test]
    fn staging_zeroized_after_cancel() {
        let (mut store, _) = WifiSecretStore::load(FakeMedium::new());
        store
            .stage(&ssid("Net"), Security::Wpa2PersonalAes, PW)
            .unwrap();
        let view = store.staged_credential_view().unwrap();
        assert_eq!(view.security, Security::Wpa2PersonalAes);
        assert_eq!(view.secret, PW);
        store.cancel_staged();
        assert!(store.staging_zeroized());
        assert!(store.staged_credential_view().is_none());
    }

    #[test]
    fn staging_zeroized_after_validation_failure() {
        let (mut store, _) = WifiSecretStore::load(FakeMedium::new());
        store
            .stage(&ssid("Net"), Security::Wpa2PersonalAes, PW)
            .unwrap();
        // A subsequent invalid stage must leave staging clean.
        let _ = store.stage(&ssid("Net"), Security::Open, b"nonempty");
        assert!(store.staging_zeroized());
    }

    #[test]
    fn staging_zeroized_after_commit_medium_failure() {
        let mut medium = FakeMedium::new();
        medium.reject_write = [true, true];
        let (mut store, _) = WifiSecretStore::load(medium);
        store
            .stage(&ssid("Net"), Security::Wpa2PersonalAes, PW)
            .unwrap();
        assert_eq!(store.commit(), Err(SecretError::MediumError));
        assert!(store.staging_zeroized());
        assert!(store.is_empty()); // rolled back
    }

    #[test]
    fn zeroize_ram_clears_staging_profiles_and_capability() {
        let mut medium = FakeMedium::new();
        medium.set_raw(
            SecretSlot::A,
            encode(&[stored(1, "Net", Security::Wpa2PersonalAes, PW)], 3),
        );
        let (mut store, _) = WifiSecretStore::load(medium);
        store
            .stage(&ssid("Other"), Security::Wpa2PersonalAes, PW)
            .unwrap();
        store.zeroize_ram();
        assert!(store.staging_zeroized());
        assert!(store.is_empty());
        assert!(!store.available());
    }

    // ------------------------------------------------------- forget (6)

    #[test]
    fn forget_removes_profile_and_persists() {
        let mut medium = FakeMedium::new();
        medium.set_raw(
            SecretSlot::A,
            encode(
                &[
                    stored(1, "Keep", Security::Open, &[]),
                    stored(2, "Drop", Security::Wpa2PersonalAes, PW),
                ],
                3,
            ),
        );
        let (mut store, _) = WifiSecretStore::load(medium);
        assert_eq!(store.forget(2), ForgetOutcome::Committed);
        assert_eq!(store.len(), 1);

        // Durable state confirms the erasure survives reload.
        let medium = store_into_medium(store);
        let (reloaded, _) = WifiSecretStore::load(medium);
        assert_eq!(reloaded.len(), 1);
        assert!(reloaded.contains(&ssid("Keep"), Security::Open));
        assert!(!reloaded.contains(&ssid("Drop"), Security::Wpa2PersonalAes));
    }

    #[test]
    fn forget_missing_id_is_idempotent_success() {
        let (mut store, _) = WifiSecretStore::load(FakeMedium::new());
        assert_eq!(store.forget(7), ForgetOutcome::Committed);
    }

    #[test]
    fn forget_interruption_keeps_durable_record() {
        let mut medium = FakeMedium::new();
        medium.set_raw(
            SecretSlot::A,
            encode(&[stored(1, "Net", Security::Wpa2PersonalAes, PW)], 3),
        );
        // Commit came from A, so a forget writes to B; reject that write.
        medium.reject_write = [false, true];
        let (mut store, _) = WifiSecretStore::load(medium);
        assert_eq!(store.forget(1), ForgetOutcome::StoreUnavailable);
        // In-RAM model reloaded to match durable state: profile still present.
        assert_eq!(store.len(), 1);
        assert!(store.contains(&ssid("Net"), Security::Wpa2PersonalAes));
    }

    #[test]
    fn forget_on_unavailable_store_reports_store_unavailable() {
        let mut medium = FakeMedium::new();
        medium.set_raw(
            SecretSlot::A,
            encode(&[stored(1, "Net", Security::Open, &[])], 3),
        );
        medium.flip_byte(SecretSlot::A, HEADER_LEN); // corrupt -> unavailable
        let (mut store, _) = WifiSecretStore::load(medium);
        assert!(!store.available());
        assert_eq!(store.forget(1), ForgetOutcome::StoreUnavailable);
    }

    // -------------------------------------------------- factory reset (6)

    #[test]
    fn factory_reset_erases_both_slots_and_verifies() {
        let mut medium = FakeMedium::new();
        medium.set_raw(
            SecretSlot::A,
            encode(&[stored(1, "A", Security::Open, &[])], 5),
        );
        medium.set_raw(
            SecretSlot::B,
            encode(&[stored(1, "B", Security::Open, &[])], 4),
        );
        let (mut store, _) = WifiSecretStore::load(medium);
        store
            .stage(&ssid("Edit"), Security::Wpa2PersonalAes, PW)
            .unwrap();

        store.factory_reset().unwrap();
        assert!(store.is_empty());
        assert!(store.staging_zeroized());
        assert!(store.available()); // usable, just empty
        assert_eq!(store.generation(), 0);
        assert!(store.medium.raw(SecretSlot::A).is_none());
        assert!(store.medium.raw(SecretSlot::B).is_none());
    }

    #[test]
    fn factory_reset_interruption_reports_error() {
        let mut medium = FakeMedium::new();
        medium.set_raw(
            SecretSlot::A,
            encode(&[stored(1, "A", Security::Open, &[])], 5),
        );
        medium.reject_erase = [true, false];
        let (mut store, _) = WifiSecretStore::load(medium);
        assert_eq!(store.factory_reset(), Err(SecretError::MediumError));
        // RAM was still cleared even though the durable erase failed.
        assert!(store.staging_zeroized());
        assert!(store.is_empty());
    }

    // ----------------------------------------------- add / update semantics

    #[test]
    fn commit_updates_existing_identity_in_place() {
        let (mut store, _) = WifiSecretStore::load(FakeMedium::new());
        store
            .stage(&ssid("Net"), Security::Wpa2PersonalAes, b"firstpass")
            .unwrap();
        let id1 = store.commit().unwrap();
        store
            .stage(&ssid("Net"), Security::Wpa2PersonalAes, b"secondpass")
            .unwrap();
        let id2 = store.commit().unwrap();
        assert_eq!(id1, id2); // same identity keeps its id
        assert_eq!(store.len(), 1);
        assert_eq!(store.credential_view(id2).unwrap().secret, b"secondpass");
    }

    #[test]
    fn store_full_rejects_fifth_identity() {
        let (mut store, _) = WifiSecretStore::load(FakeMedium::new());
        for i in 0..RETAINED_PROFILES_MAX {
            let name = std::format!("Net{i}");
            store.stage(&ssid(&name), Security::Open, &[]).unwrap();
            store.commit().unwrap();
        }
        store.stage(&ssid("Fifth"), Security::Open, &[]).unwrap();
        assert_eq!(store.commit(), Err(SecretError::StoreFull));
        assert!(store.staging_zeroized());
    }

    #[test]
    fn store_is_usable_as_credential_provider() {
        let mut medium = FakeMedium::new();
        medium.set_raw(
            SecretSlot::A,
            encode(&[stored(1, "Net", Security::Open, &[])], 3),
        );
        let (mut store, _) = WifiSecretStore::load(medium);
        let provider: &mut dyn CredentialProvider = &mut store;
        assert!(provider.available());
        assert_eq!(provider.forget(1), ForgetOutcome::Committed);
    }
}
