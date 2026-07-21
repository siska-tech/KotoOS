# Application network credential vault

- Status: KOTO-0248 implementation contract, 2026-07-21. Core store and grant
  model land first (`koto-core::vault`); the KotoConfig consent page, the
  Fetch/MQTT injection wiring, the SDK handle surface, KotoSim fakes, and device
  validation are follow-up slices, mirroring how KOTO-0240 landed the Wi-Fi
  secret store ahead of its page (KOTO-0241) and device work (KOTO-0243).
- Related: KOTO-0036, KOTO-0239, KOTO-0240, KOTO-0245, KOTO-0249.

## Decision

Authenticated network services need per-application secrets — a bearer token, an
API key header, or an MQTT username/password — but no application may ever hold,
read, enumerate, copy, export, or log a secret byte. KotoOS owns a bounded
**credential vault** in a separate versioned secret namespace (`KAV1`) and
issues **opaque handles** that an application can only *ask the OS to use* on its
behalf. The OS injects the secret inside its own authenticated transport, after
validating the requesting app's identity and the exact destination origin, and
zeroizes every transient copy.

This is the same separation KOTO-0240 froze for Wi-Fi credentials, applied to
application service credentials. The vault is a distinct namespace from the
`KWS1` Wi-Fi store, the `KCF1` public settings, application sandboxes, and
package manifests. Nothing here claims hardware-backed secrecy on boards that
cannot provide it (see [Disclosure limits](#disclosure-limits)).

## Threat model

Assets: application service secrets (tokens, API keys, MQTT passwords). The
adversary's goal is to read a secret, use it outside its grant, or broaden a
grant without consent.

| Threat | Vector | Mitigation in this design | Residual / out of scope |
| :-- | :-- | :-- | :-- |
| Malicious package | A `.kpa` ships code that tries to read, enumerate, or exfiltrate a secret, or forges another app's identity to borrow its grant. | Apps hold only opaque `CredentialHandle`s, never bytes. The vault has no read/export/enumerate call. A grant binds to the exact canonical package `app_id`; the runtime supplies the app identity, not the bytecode. Injection is refused when the requesting app's id does not own the grant. | A package can still *use* a secret it was granted (that is the point). Consent gates the grant, not each use. |
| Secret in a package or manifest | Author embeds a token in the `.kpa`, manifest, or an asset to avoid the vault. | The vault is the only authenticated-credential path; Fetch/MQTT accept no app-supplied `Authorization`/login. Reviews and the manifest schema keep secret fields out of packages. | The OS cannot stop an author from hard-coding a secret in their own app logic and misusing it against a service; that is the author's own credential to leak. |
| Compromised network / on-path attacker | Downgrade to plain HTTP or MQTT, strip TLS, or redirect to an attacker origin to capture the injected secret. | Secret-bearing requests require the authenticated scheme (`https` / `mqtts`) with verified TLS and pinning as configured. Grant creation and injection both reject non-TLS endpoints. No redirect path can cross an origin (KOTO-0245). Injection re-validates the exact origin before the secret is written to the transport buffer. | Confidentiality depends on the peer's TLS trust chain and pin set; a mis-issued certificate accepted by the pin policy is out of scope. |
| Stolen SD card / discarded device | Attacker reads the raw flash or SD to recover stored secrets. | Storage separation and a distinct namespace reduce accidental exposure, but **secrets are stored at rest without confidentiality on the applicable boards**. This threat is *not* mitigated without secure hardware. | Physical media recovery is explicitly unmitigated; documented per board profile. See [Disclosure limits](#disclosure-limits). |
| Diagnostics / support export | A crash dump, log line, memory view, or support bundle leaks a secret, header, or grant secret-field. | Every vault error/diagnostic is a fixed enum carrying only slot generation, grant count, and per-grant redacted scope (app id length, service kind, endpoint host length, secret length). No `Debug` prints secret, username, header value, or host bytes. The Fetch/MQTT transport redacts injected headers. | A platform that dumps raw RAM outside the vault API can still capture resident secret bytes; the vault only bounds and zeroizes its own buffers. |
| Crash / reset mid-write | Power loss during a grant create/replace/revoke tears the record. | Two-slot generation commit (`SecretSlot::A`/`B`): a write targets the non-current slot, so a torn write leaves the prior good slot intact; load selects the newest valid slot by wrap-aware generation and fails closed if none validates. | A torn write loses only the in-flight change; the prior grant set survives. |
| App upgrade / origin change | A package update, or a manifest that changes an app id or a grant's origin, silently inherits an existing grant. | Grants bind to the exact `app_id` string and exact canonical endpoint. A changed app id or endpoint does not match; the grant is not offered and injection is refused. Default is denied; broadening requires a fresh consent. | Reinstalling the *same* app id keeps its grants by design (stable identity across reinstalls); a malicious actor able to publish under the victim's exact app id is a packaging/signing problem (KOTO-0036), not solved here. |
| Physical device access | An unlocked device operator inspects or copies grants. | The management flow shows only redacted grant scope, never secret bytes. Reset/forget zeroizes RAM immediately and logically erases both slots. | Physical erasure of the underlying flash/SD is not guaranteed (wear-leveling remanence); documented, not claimed. |

Threats mitigated by storage protection and process boundaries: malicious
package read/enumerate/export, forged app identity, diagnostics leakage, torn
write, silent grant broadening. Threats that **cannot** be solved without secure
hardware or signed packages: stolen-media secret recovery, cold-boot RAM
capture, and impersonation by a package published under the victim's exact app
id.

## Handles and scope

An application never sees a secret. It sees a `CredentialHandle`: a fixed,
opaque, generation-tagged 32-bit token minted by the OS for one grant. The
handle is meaningless to bytecode — it cannot be reversed to bytes, enumerated,
or used by another app. A start call on Fetch/MQTT may carry a handle; the OS
resolves it against the live vault and the requesting `AppContext` before use.

A **grant** scopes a secret to an exact triple:

| Field | Rule |
| :-- | :-- |
| `app_id` | The canonical package app id (reverse-DNS, ≤ 64 bytes, validated by `package::validate_app_id`). Durable identity across reinstalls; the ephemeral runtime `AppContext.app_id` is bound to it at grant time. |
| `service` | `ServiceKind::Fetch` or `ServiceKind::Mqtt`. A Fetch grant can never be used on MQTT or vice versa. |
| `endpoint` | A canonical `(scheme, host, port)` requiring the authenticated scheme (`https` / `mqtts`). Built from the same grammar as `FetchOrigin` / `MqttOrigin`. Non-TLS endpoints are rejected at creation. |

The `kind` of secret determines how the OS injects it, never what the app sees:

| `CredentialKind` | Service | Injection |
| :-- | :-- | :-- |
| `BearerToken` | Fetch | `Authorization: Bearer <secret>` header inside the TLS request. |
| `ApiKeyHeader { name }` | Fetch | `<name>: <secret>` header; `name` is a stored, validated header token. |
| `MqttLogin { username }` | MQTT | Username (clear) + password (secret) in the CONNECT packet over TLS. |

## Grant lifecycle and consent

Default is **denied**: a fresh vault offers no grant, and an app requesting a
handle it was never granted is refused. Only an OS-owned bilingual (日本語 /
English) consent flow creates, replaces, revokes, and inspects grants; ordinary
applications have no path to it. The flow:

- **Create** — the operator confirms the exact app, service, and endpoint before
  a secret is staged and committed. Nothing is stored on cancel.
- **Replace** — updating an existing `(app_id, service, endpoint)` grant's secret
  advances its generation and invalidates the prior handle.
- **Revoke** — removes the grant, zeroizes its RAM secret immediately, and
  commits the smaller record. Any outstanding handle for it fails closed.
- **Inspect** — lists redacted grant scope only (app id, service, endpoint host
  length, secret length, generation). Never secret bytes.

A package update or an origin change does not silently broaden a grant: the
match is exact, so a changed field simply does not match and the operator must
grant again.

## Storage separation and integrity

The vault is a separate, versioned, checksummed namespace:

- Magic `KAV1`, distinct from `KWS1` (Wi-Fi), `KCF1` (settings), `KUC1`.
- Its own fixed record format and its own `VaultMedium` adapter, which platform
  code must back with a location distinct from public settings, the Wi-Fi
  secrets, and app sandboxes.
- Fail closed on missing/unknown version, bad length, bad checksum, or torn
  data: the vault reports unavailable rather than guessing or falling back.
- Two-slot generation commit for torn-write safety (as above).
- FNV-1a integrity checksum over the record. This detects accidental corruption
  and torn writes; it is **not** an authentication tag against a deliberate
  offline forger with write access to the media, which the applicable boards
  cannot prevent. On a board that offers authenticated storage, the medium
  adapter is where that protection is added.

## Injection

Secrets are injected only inside the OS-private authenticated transport, after
the origin and app-context checks pass:

1. The app's start call carries a `CredentialHandle` and a URL / broker.
2. The controller resolves the handle against the live vault generation and the
   requesting `AppContext.app_id`. A stale, foreign, or unknown handle fails with
   a fixed error before any secret is touched.
3. The parsed canonical endpoint must exactly equal the grant's endpoint and use
   the authenticated scheme. A plain-HTTP, unverified-TLS, or off-grant
   destination is refused.
4. The OS copies the secret into a transient transport buffer, writes the header
   or CONNECT field, and zeroizes the transient buffer on completion,
   cancellation, capability loss, app exit, disconnect, and error.

The secret never enters VM memory, the app read buffer, the response path, or any
diagnostic.

## Zeroization

Every transient secret buffer is volatile-zeroized at each terminal boundary:
submission, cancellation, validation failure, capability loss, app exit,
disconnect, teardown, and error. The store's resident grant secrets are
zeroized on revoke, factory reset, RAM teardown, and `Drop`. This reduces
*accidental* retention (stale SRAM, casual dumps); it is not a defense against
media remanence, DMA, or a cold-boot attacker.

## Disclosure limits

The applicable Pico W / Pico 2 W boards provide no KotoOS-managed, non-exportable
key, so the vault does **not** claim confidentiality of stored bytes. Secrets are
held at rest without encryption; physical access to the SD card or flash can
recover them despite RAM zeroization and logical two-slot erasure.
Password-derived or device-ID obfuscation is intentionally not implemented and
would not be encryption if it were. Each board profile documents whether secure
credential persistence is available or only obfuscation; on the current profiles
it is neither — it is separation and integrity without confidentiality.

## Bounded capacities (v1)

All limits are octets, not displayed characters. No vault state uses the general
heap; the whole grant set serializes to one fixed-size record.

| Item | Limit |
| :-- | --: |
| Grants | 4 |
| App id | 64 B |
| Endpoint host | 128 B |
| Secret | 192 B |
| API-key header name | 32 B |
| MQTT username | 48 B |

Persistent cost is one fixed record per slot (two slots). The exact `.text` /
`.rodata` / `.bss` deltas, the resident RAM model size, and the transient
transport-buffer peak are recorded by the implementation issue on the supported
targets before any board profile claims lifecycle validation.
