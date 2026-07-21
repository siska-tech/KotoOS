//! Deterministic KotoSim application credential vault (KOTO-0248, AC-7).
//!
//! KotoSim never reads a host credential store. This fake seeds the real
//! [`VaultStore`] with **synthetic** grants and **fake** secrets over an
//! in-memory two-slot medium, so it exercises the exact production vault logic
//! (scope binding, handle generation, TLS-only injection, redaction) with
//! reproducible bytes and nothing sensitive on the machine.
//!
//! The running application is granted one deterministic fake bearer token for
//! `https://api.example.com`. `vault_handle` resolves the opaque handle for a
//! destination the app is granted (`0` otherwise), and `resolve_fetch`
//! re-validates a handle before an authenticated GET exactly as the device host
//! would, returning only a fixed error — never a secret byte.

use koto_core::vault::{
    app_vault, CredentialHandle, CredentialKind, ServiceKind, SlotRead, VaultEndpoint, VaultError,
    VaultMedium, VaultSlot, VaultStore, VAULT_RECORD_BYTES,
};
use koto_core::{parse_fetch_url, FetchScheme};

/// The synthetic secret the sim grants. It is obviously fake and is never a
/// real credential; it exists only to prove the store round-trips a secret it
/// never discloses.
const SIM_FAKE_TOKEN: &[u8] = b"sim-fake-token-0248";

/// The single deterministic endpoint the running app is granted in the sim.
const SIM_GRANT_HOST: &str = "api.example.com";
const SIM_GRANT_PORT: u16 = 443;

/// In-memory two-slot medium. Purely volatile; nothing is persisted to disk.
#[derive(Default)]
struct SimVaultMedium {
    slots: [Option<[u8; VAULT_RECORD_BYTES]>; 2],
}

impl SimVaultMedium {
    fn idx(slot: VaultSlot) -> usize {
        match slot {
            VaultSlot::A => 0,
            VaultSlot::B => 1,
        }
    }
}

impl VaultMedium for SimVaultMedium {
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
    ) -> Result<(), koto_core::vault::MediumFault> {
        self.slots[Self::idx(slot)] = Some(*src);
        Ok(())
    }

    fn erase_slot(&mut self, slot: VaultSlot) -> Result<(), koto_core::vault::MediumFault> {
        self.slots[Self::idx(slot)] = None;
        Ok(())
    }
}

/// The sim-owned deterministic credential vault for one running application.
pub(super) struct SimVault {
    store: VaultStore<SimVaultMedium>,
    app_id: String,
}

impl core::fmt::Debug for SimVault {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // Never print the app id or any grant detail; expose only the count so
        // an inspector dump can never leak scope or secret bytes.
        f.debug_struct("SimVault")
            .field("grants", &self.store.len())
            .finish()
    }
}

impl SimVault {
    /// Builds a vault seeded with the deterministic scenario grant for `app_id`.
    /// A malformed or non-canonical app id simply yields no grant (denial), just
    /// as an ungranted app would on device.
    pub(super) fn seeded(app_id: &str) -> Self {
        let (mut store, _) = VaultStore::load(SimVaultMedium::default());
        if let Ok(endpoint) = VaultEndpoint::new(SIM_GRANT_HOST, SIM_GRANT_PORT) {
            if store
                .stage(
                    app_id.as_bytes(),
                    ServiceKind::Fetch,
                    endpoint,
                    CredentialKind::BearerToken,
                    &[],
                    SIM_FAKE_TOKEN,
                )
                .is_ok()
            {
                let _ = store.commit();
            }
        }
        Self {
            store,
            app_id: app_id.to_string(),
        }
    }

    /// Resolves the opaque handle for `url` under `service`, or `0` when the
    /// running app holds no matching grant. Never exposes a secret.
    pub(super) fn handle(&self, service: i32, url: &str) -> i32 {
        // Only the Fetch service is wired end to end in the sim; an MQTT
        // selector has no seeded grant and resolves to "no grant".
        if service != app_vault::SERVICE_FETCH {
            return 0;
        }
        let Some((endpoint, _tls)) = fetch_endpoint(url) else {
            return 0;
        };
        self.store
            .handle_for(self.app_id.as_bytes(), ServiceKind::Fetch, &endpoint)
            .map_or(0, |handle| handle.raw() as i32)
    }

    /// Re-validates `handle` for an authenticated GET to `url`: the grant
    /// generation, the running app id, the Fetch service, the exact endpoint,
    /// and TLS must all match. Returns the fixed error the host maps to a host
    /// error code; a successful validation drops the borrowed injection so no
    /// secret byte escapes the vault.
    pub(super) fn resolve_fetch(&self, url: &str, handle: i32) -> Result<(), VaultError> {
        let (endpoint, is_tls) = fetch_endpoint(url).ok_or(VaultError::InvalidEndpoint)?;
        let handle = CredentialHandle::from_raw(handle as u32);
        self.store
            .injection_for(
                handle,
                self.app_id.as_bytes(),
                ServiceKind::Fetch,
                &endpoint,
                is_tls,
            )
            .map(|_injection| ())
    }
}

/// Parses a Fetch URL into a canonical vault endpoint and whether it is TLS.
fn fetch_endpoint(url: &str) -> Option<(VaultEndpoint, bool)> {
    let target = parse_fetch_url(url).ok()?;
    let endpoint = VaultEndpoint::new(target.hostname(), target.port()).ok()?;
    Some((endpoint, target.scheme() == FetchScheme::Https))
}

#[cfg(test)]
mod tests {
    use super::*;

    const APP: &str = "dev.koto.vaultdemo";
    const GRANTED_URL: &str = "https://api.example.com/v1/data";

    #[test]
    fn synthetic_handle_is_deterministic_and_nonzero() {
        let a = SimVault::seeded(APP);
        let b = SimVault::seeded(APP);
        let ha = a.handle(app_vault::SERVICE_FETCH, GRANTED_URL);
        let hb = b.handle(app_vault::SERVICE_FETCH, GRANTED_URL);
        assert_ne!(ha, 0);
        assert_eq!(
            ha, hb,
            "same app + endpoint yields a stable synthetic handle"
        );
    }

    #[test]
    fn denial_when_no_grant() {
        let vault = SimVault::seeded(APP);
        // Ungranted origin.
        assert_eq!(
            vault.handle(app_vault::SERVICE_FETCH, "https://evil.example.com/x"),
            0
        );
        // MQTT selector has no seeded grant.
        assert_eq!(vault.handle(app_vault::SERVICE_MQTT, GRANTED_URL), 0);
        // A malformed app id was never granted.
        let ungranted = SimVault::seeded("Not A Valid Id");
        assert_eq!(ungranted.handle(app_vault::SERVICE_FETCH, GRANTED_URL), 0);
    }

    #[test]
    fn grant_resolves_over_tls() {
        let vault = SimVault::seeded(APP);
        let handle = vault.handle(app_vault::SERVICE_FETCH, GRANTED_URL);
        assert_ne!(handle, 0);
        assert_eq!(vault.resolve_fetch(GRANTED_URL, handle), Ok(()));
    }

    #[test]
    fn wrong_origin_is_refused() {
        let vault = SimVault::seeded(APP);
        let handle = vault.handle(app_vault::SERVICE_FETCH, GRANTED_URL);
        assert_eq!(
            vault.resolve_fetch("https://other.example.com/x", handle),
            Err(VaultError::EndpointMismatch)
        );
    }

    #[test]
    fn plain_http_is_refused() {
        let vault = SimVault::seeded(APP);
        let handle = vault.handle(app_vault::SERVICE_FETCH, GRANTED_URL);
        assert_eq!(
            vault.resolve_fetch("http://api.example.com/v1/data", handle),
            Err(VaultError::InsecureEndpoint)
        );
    }

    #[test]
    fn stale_or_unknown_handle_is_refused() {
        let vault = SimVault::seeded(APP);
        // A never-minted handle id.
        assert_eq!(
            vault.resolve_fetch(GRANTED_URL, 0),
            Err(VaultError::NotFound)
        );
        // The right grant id with a wrong generation stamp is stale. The seeded
        // grant is id 1, generation 1 (handle raw 0x0001_0001); bump the high
        // half to force a generation mismatch.
        let real = vault.handle(app_vault::SERVICE_FETCH, GRANTED_URL);
        let stale = (real as u32).wrapping_add(0x0001_0000) as i32;
        assert_eq!(
            vault.resolve_fetch(GRANTED_URL, stale),
            Err(VaultError::StaleHandle)
        );
    }

    #[test]
    fn fake_secret_never_leaves_the_vault() {
        // The only observable outputs are an opaque handle and a unit/error from
        // resolve; neither is or contains the fake token bytes.
        let vault = SimVault::seeded(APP);
        let handle = vault.handle(app_vault::SERVICE_FETCH, GRANTED_URL);
        let handle_bytes = (handle as u32).to_le_bytes();
        assert!(
            !SIM_FAKE_TOKEN
                .windows(handle_bytes.len())
                .any(|w| w == handle_bytes),
            "handle must not embed secret bytes"
        );
        assert_eq!(vault.resolve_fetch(GRANTED_URL, handle), Ok(()));
    }
}
