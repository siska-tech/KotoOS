# KOTO-0248: Application network credential vault and grants

- Status: in-progress
- Type: feature
- Priority: P1
- Requirements: FR-SDK-5, FR-RT-4, FR-FS-2, FR-PKG-1, NFR-MEM-2, NFR-PORT-1, NFR-PORT-3, NFR-REL-1, NFR-REL-5, NFR-DEV-3, NFR-DEV-4
- Related: KOTO-0036, KOTO-0239, KOTO-0240, KOTO-0245, KOTO-0249

## Goal

Provide an OS-owned vault and explicit user grants for application network
credentials so authenticated services can be used without placing passwords,
tokens, or client keys in `.kpa` files, VM memory, app sandbox files, settings,
logs, or the general Fetch/MQTT ABI.

## Acceptance Criteria

- [ ] Write a threat model covering a malicious package, compromised network,
  stolen SD card, diagnostics, crash/reset, app upgrade, and physical device
  access. Document which threats are mitigated by storage protection and which
  cannot be solved without secure hardware or signed packages.
- [ ] Define fixed-capacity opaque credential handles scoped to an exact app ID,
  service kind, and canonical origin/broker. Apps can request use of a granted
  handle but can never read, enumerate, copy, export, or log secret bytes.
- [ ] Add an OS-owned bilingual consent/management flow for creating, replacing,
  revoking, and inspecting grants. Default is denied; package updates and origin
  changes do not silently broaden an existing grant.
- [ ] Keep application credentials in a separate versioned store from public
  ConfigService settings, app sandboxes, and KOTO-0240 Wi-Fi credentials. Use
  authenticated integrity protection where supported and fail closed on
  corruption, unknown versions, or unavailable device protection.
- [ ] Inject credentials only inside the OS-owned authenticated transport after
  origin and app-context validation. Secret-bearing requests cannot use plain
  HTTP, unverified TLS, redirects, or destinations outside the grant.
- [ ] Zeroize transient secret buffers on completion, cancellation, capability
  loss, app exit, disconnect, and error. Diagnostics expose fixed credential
  state/slot generations only and redact all names, values, headers, and keys.
- [ ] KotoSim uses deterministic synthetic handles and fake secrets that never
  read host credential stores by default. Add tests for denial, grant, wrong
  app/origin, revoke, stale handle, corruption, reset during update, failed TLS,
  and log/diagnostic redaction.
- [ ] Record persistent and transient storage costs and validate lifecycle on
  supported hardware. Clearly document any board profile on which secure
  credential persistence is unavailable or only obfuscation.

## Non-goals

- Exposing secret values to applications or placing them in package manifests
- A general password manager, browser cookie jar, or Wi-Fi credential API
- Claiming hardware-backed secrecy on boards that cannot provide it

## Progress (2026-07-21, core-first slice, uncommitted)

Landed the portable bounded core, mirroring how KOTO-0240 landed the Wi-Fi
secret store ahead of its page/device slices:

- **Threat model + contract**: `docs/architecture/APP_CREDENTIAL_VAULT.md`
  documents the eight-vector threat model (malicious package, compromised
  network, stolen media, diagnostics, crash/reset, app upgrade, physical access),
  the handle/scope model, storage separation, injection, zeroization, disclosure
  limits, and v1 capacities. States plainly which threats are unmitigated
  without secure hardware or signed packages.
- **`koto-core::vault`** (`no_std`, no heap): `KAV1` versioned namespace,
  distinct from `KWS1`/`KCF1`; two-slot generation commit with FNV-1a checksum,
  fail-closed on corrupt/torn/unsupported. Fixed capacities: 4 grants, app id 64
  B, endpoint host 128 B, secret 192 B, header name 32 B, MQTT username 48 B
  (record = 1,824 B/slot). Opaque `CredentialHandle` (grant id + monotonic
  record-generation stamp so revoke+re-add never resurrects a stale handle).
  Grants bind to exact (app id, service, endpoint); default-denied.
  `injection_for` re-checks handle generation, owning app id, service, exact
  endpoint, and TLS-only before yielding an operation-scoped
  `CredentialInjection` (BearerToken / ApiKeyHeader / MqttLogin). Volatile
  zeroize on drop/revoke/reset/teardown/cancel; every `Debug` redacts secret,
  host, header, and username bytes.
- **Tests**: 27 host tests green covering the AC-7 matrix — denial, grant,
  wrong app/origin/service, revoke, stale handle (incl. after reload),
  corruption fail-closed, torn write, reset-during-update rollback, failed-TLS
  refusal, factory reset, store-full, and diagnostic/log redaction — plus API
  key + MQTT injection and validation. `cargo fmt`/`clippy -D warnings`/koto-core
  tests all clean.

### AC status

- [x] AC-1 threat model (architecture doc).
- [~] AC-2 opaque scoped handles — core done; the host-call ABI surface is
  defined (`vault_handle` 0x57, `fetch_start_authenticated` 0x58, Host ABI minor
  22); the SDK language wrappers now compile (`vault_handle`,
  `fetch_start_authenticated` intrinsics + `VAULT_SERVICE_FETCH/MQTT` constants,
  KOTO_SDK.md); a sample and the sim host impl are pending.
- [~] AC-3 consent/management flow — core create/replace/revoke/inspect +
  default-denied done; the bilingual KotoConfig page is a follow-up slice.
- [x] AC-4 separate versioned store, integrity, fail-closed (core).
- [~] AC-5 injection after origin/app validation, TLS-only — core contract done;
  the Fetch request encoder now injects the credential at the byte level
  (`encode_fetch_get_request_with_injection`: `Authorization: Bearer <token>` /
  `<name>: <value>` written into the same transport buffer the caller zeroizes,
  MQTT-login shape refused, wire-safety re-checked). Threading the handle through
  `AppFetchController` start/mailbox and the MQTT CONNECT encoder are follow-up
  device slices.
- [x] AC-6 zeroization + redacted diagnostics (core).
- [~] AC-7 KotoSim synthetic-handle fakes + tests — koto-core host fault matrix
  done; the KotoSim deterministic fake vault (`SimVault`: real `VaultStore` over
  an in-memory two-slot medium, synthetic handles, fake token, never reads a
  host store) is wired into the sim host's `vault_handle` /
  `fetch_start_authenticated` with 7 sim tests (deterministic handle, denial,
  grant-over-TLS, wrong origin, plain-HTTP refusal, stale/unknown handle, fake
  secret never exposed). A packaged end-to-end sample app
  (`dev.koto.samples.vault-fetch`) resolves a handle, completes an authenticated
  GET, and denies an ungranted origin, verified by an e2e sim test.
- [ ] AC-8 record storage/transient costs + device lifecycle validation.

### Landed host-call ABI (2026-07-21)

`koto-vm` now defines the two vault host calls and bumps Host ABI minor 21→22:

- `vault_handle` (`0x57`, stack effect `(3,2)`): `(service, url_ptr, url_len)` →
  opaque handle (`0` = no grant). Never exposes a secret.
- `fetch_start_authenticated` (`0x58`, `(3,2)`): `(url_ptr, url_len, handle)` →
  request id; the OS injects the granted credential in-transport.

Both are wired through the five in-sync sites (constant, `name()`, `VmHost`
trait default = `UNSUPPORTED`, `exec_host_call` dispatch, `known_host_call`,
`host_call_stack_effect`). Two koto-vm tests green (arg marshalling/dispatch +
known/name/arity/minor). `RUNTIME_BYTECODE_ABI.md` gains the two rows and a
minor-22 section. Existing minor ≤ 21 KBC still verify (no mass rebuild yet;
that lands with the sample). koto-vm fmt/clippy clean, koto-sim builds, koto-core
412 tests still green.

### Landed SDK wrappers (2026-07-21)

`koto-compiler` gains two intrinsics — `vault_handle` (3 args →
`vault_handle`) and `fetch_start_authenticated` (3 args →
`fetch_start_authenticated`), both `ResultKind::Value` — and two SDK constants
`VAULT_SERVICE_FETCH`/`VAULT_SERVICE_MQTT` sourced from
`koto_core::vault::app_vault` so they cannot drift from the store's `ServiceKind`
tags. `KOTO_SDK.md` documents the calls and constants. Two koto-compiler tests
green (host-call emission + wrong-arity rejection). koto-core 412 / koto-compiler
142 tests green, fmt/clippy clean. No app uses them yet, so no KBC/golden regen.

### Landed KotoSim fake vault (2026-07-21)

`src/koto-sim/src/runtime/sim_vault.rs`: `SimVault` seeds the real `VaultStore`
over an in-memory two-slot `SimVaultMedium` with one deterministic fake bearer
grant for the running app to `https://api.example.com`; synthetic handles, a
fake token, never touches a host store. The sim host gained a `vault` field and
`vault_handle` / `fetch_start_authenticated` methods that delegate to it (the
authenticated start validates the grant, then delegates to the existing fetch
path; a failed validation is a fixed `PERMISSION_DENIED`). Seven sim tests
green. koto-sim lib clippy-clean (the `-D warnings` failures under
`--all-targets` are pre-existing KOTO-0247 `koto_weather_service.rs` WIP, not
this slice).

### Landed sample app (2026-07-21)

`apps/samples/vault_fetch/` (`dev.koto.samples.vault-fetch`, package
`sample_vault_fetch`): a Koto app that resolves an opaque handle with
`vault_handle`, starts an authenticated GET with `fetch_start_authenticated`
(draining the body to completion), and shows an ungranted origin resolving to no
handle. `kbc-asm` gained the two host-call name mappings so the app assembles.
The shell golden trace was regenerated (23→24 packages). New e2e sim test
`koto_vault_service.rs` drives the packaged app against the scripted fetch +
deterministic `SimVault`, asserting the authenticated fetch completes, the
ungranted origin is denied, and no secret bytes reach the app surface.

### Remaining slices

`AppFetchController` start/mailbox handle threading + MQTT CONNECT encoder
injection, KotoConfig bilingual consent page, `VaultMedium` firmware adapter on a
storage location separate from settings/Wi-Fi/app files, and device
cost/lifecycle measurement. None are testable off-device here.

### Landed encoder injection (2026-07-21)

`koto-core::fetch::encode_fetch_get_request_with_injection` is the OS-private
transport injection point: it takes a `vault::CredentialInjection` and writes the
`Authorization: Bearer <token>` or `<name>: <value>` header between the Host line
and the fixed Accept/Connection headers, sizing the whole request before
touching the destination. The secret is written straight into the transport
buffer (no second staging copy), MQTT-login injections are refused, and header
name/value bytes are re-validated for wire safety (defense in depth over the
vault's own validation). 6 encoder tests green.
