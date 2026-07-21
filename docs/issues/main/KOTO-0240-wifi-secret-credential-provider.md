# KOTO-0240: Wi-Fi secret credential provider and persistence

- Status: in progress
- Type: security feature
- Priority: P1
- Requirements: FR-CONFIG-3, NFR-MEM-2, NFR-PORT-3, NFR-PORT-4, NFR-REL-1, NFR-REL-5
- Related: KOTO-0223, KOTO-0224, KOTO-0239, KOTO-0241, KOTO-0242, KOTO-0243
- Roadmap: [KotoConfig Roadmap](../../planning/KOTOCONFIG_ROADMAP.md)

## Goal

Implement the bounded credential-provider boundary frozen by KOTO-0224. Store
at most four Wi-Fi profiles outside public `KCF1` settings, fail closed after
corruption or incomplete writes, and minimize accidental credential exposure
without claiming hardware-backed confidentiality.

## Acceptance Criteria

- [x] Define a versioned, fixed-size, checksummed secret format with two-slot
  commit, exact maximum read/write size, nonzero generation, and four profiles.
- [x] Validate SSID length/bytes, `Open`, and 8..63 printable-ASCII
  `Wpa2PersonalAes` passphrases; reject unsupported modes, duplicate profile
  identities, invalid padding, truncation, trailing bytes, and oversized input.
- [x] Keep the namespace and platform adapter separate from `KCF1`, KUC1,
  Shell preferences, app storage/enumeration, diagnostics, and support exports.
  (Core boundary: own `KWS1` magic, own codec/checksum, `SecretMedium` seam,
  zero coupling to `config`. Firmware `SdSecretMedium` backs it with slot files
  `KWSA/KWSB.BIN`, distinct from the public settings slots `KCFGA/KCFGB.BIN`.
  Runtime wiring into the live service + hardware validation land with KOTO-0243.)
- [x] Expose only fixed-capacity list metadata and operation-scoped credential
  views to NetworkService; ordinary apps and public configuration snapshots
  cannot read, enumerate, or mutate secrets.
- [x] Volatile-zeroize edit/operation staging after submission, cancellation,
  failure, page exit, forget, capability loss, and arena teardown, with focused
  instrumentation tests for every exit path.
- [ ] Make forget report success only after committed erasure; factory reset
  erases and verifies both slots, clears RAM staging, and resets public settings
  independently without claiming secure physical flash/SD erasure. (Forget-
  commit and factory-reset erase/verify/clear done and tested. Remaining:
  public-settings reset is coordinated with `ConfigService` at the app/firmware
  level — the store deliberately never touches `KCF1`.)
- [x] Missing, corrupt, torn, or unsupported secret data disables saved Wi-Fi
  profiles and `WIFI_CONFIG` while preserving boot, language settings, Shell,
  and offline app launch. It never guesses data or falls back to `KCF1`.
- [ ] Ensure logs, errors, panic/crash records, screenshots, fixtures, and dumps
  contain no passphrase, PSK, complete secret record, or unredacted driver text.
  (Code guarantee done: no `Debug`/`Display` exposes secret or SSID bytes,
  errors are fixed redacted enums, test asserts redaction. Remaining: verify at
  the screenshot/fixture/panic-dump integration level — KOTO-0242/0243.)
- [x] Document on-device disclosure limits: applicable Pico W/Pico 2 W boards
  have no KotoOS-managed non-exportable key, and physical storage access may
  recover credentials despite zeroization and logical erasure.
- [x] Host fault-matrix tests cover both slots absent, either valid, stale/newer
  generations, corrupt replacement, torn write, bit flips, forget interruption,
  factory reset interruption, and generation wrap.

## Progress

**Portable core landed (koto-core, host-tested):**
`src/koto-core/src/wifi_secrets.rs` implements the KOTO-0224 credential
boundary. `KWS1` is a fixed 440-byte record (24-byte header + 4×104-byte
profile slots) with FNV-1a checksum, nonzero wrap-aware generation, and dense
encoding. `SecretMedium` is the two-slot (A/B) platform adapter seam; a write
always targets the non-current slot so an interrupted write leaves the prior
good slot intact. `WifiSecretStore<M>` owns the medium and up to four profiles,
selects the newest valid slot on load, fails closed to
`LoadOutcome::Corrupt`/`available=false` when data is present-but-unusable, and
treats both-absent as a usable empty (fresh) namespace. It implements the
existing `net::CredentialProvider` trait (`available` + committed `forget`) so a
`&mut WifiSecretStore` drops straight into `NetworkService::service`. The page
gets only redacted `ProfileInfo` and borrow-scoped `CredentialView`s. Volatile
zeroization covers edit staging (`Drop` + every exit path), retained secrets
(`StoredProfile: Drop`), `zeroize_ram` (teardown/capability loss), and load
scratch; `staging_zeroized()` instruments the exit-path tests. 38 host tests
(fault matrix + zeroization + validation + forget/factory-reset) green;
`koto-core` fmt/clippy(-D warnings)/296 lib tests green.

**Firmware platform adapter landed (koto-pico, cross-compiled):**
`src/koto-pico/src/firmware/secret_store.rs` (gated behind `network_service`)
implements `SecretMedium` over two root-volume files `KWSA/KWSB.BIN`, whose
names are distinct from the public settings slots `KCFGA/KCFGB.BIN`, mirroring
the KotoConfig persistence adapter. Missing or zero-length slot → `Absent`
(fresh/erased); wrong-length → invalid record (fail closed). `erase_slot`
truncates to zero; `load_wifi_secret_store` builds the store. pico2w
`network_service` build + clippy green; offline RP2040 build links none of it.

**Remaining (integration, tracked with KOTO-0241/0242/0243):**
the KOTO-0243 Pico 2 W product path now threads a persistent
`WifiSecretStore` through the live NetworkService and KotoConfig LCD page,
including stage-on-connect, commit after association, cancel/failure/exit
zeroization, saved-profile reconnect without copying the secret into page
storage, bounded retry borrowing, and durable-id forget. Remaining work is
factory-reset/public-settings coordination, Pico W switched-residency wiring,
hardware validation, and screenshot/fixture/panic-dump redaction verification.

## Non-goals

- Password-derived or device-ID obfuscation presented as encryption
- Cloud synchronization, account credentials, certificates, or enterprise EAP
- Enabling the radio or implementing KotoConfig page rendering
