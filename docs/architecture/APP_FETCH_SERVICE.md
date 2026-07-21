# App Fetch Service boundary

Status: KOTO-0245 completed, 2026-07-22. RP2040 device HTTPS is enabled with
the measured TLS/audio-exclusive transport and controlled-endpoint validation
described below.

## Ownership and lifecycle

`AppFetchController` is the backend-independent OS policy and lifecycle state
above `NetworkService`; `AppFetchService` is its compatibility wrapper for
owned backends. The device host keeps the controller in the app-session overlay
and accesses `WifiRuntime` through a narrow external `FetchBackend` adapter.
The active package supplies an implicit `AppContext` and a copied
`FetchAllowlist`; bytecode can only hold a generation-tagged `FetchRequestId`.
DNS, IP addresses, sockets, TLS objects, decoder state, URL staging, and
network-stack handles stay behind the backend boundary.

The global limit is two requests and the per-application limit is one. A start
validates the complete URL and exact canonical origin before invoking the
backend. Polling and reading are nonblocking. A request expires after 15,000 ms;
each read copies at most 512 bytes and a response exposes at most 65,536 bytes.
Cancellation, app exit, capability loss, Wi-Fi disconnect, NetworkService
generation change, and service teardown cancel backend work, erase transient
buffers, and advance the request generation. Old or other-app IDs fail with
fixed `StaleRequest` or `ForeignRequest` results.

Frozen v1 capacities are:

| Resource | Maximum |
| :-- | --: |
| Origins per package | 4 |
| Hostname | 253 bytes |
| URL | 384 bytes |
| Response header block | 1,024 bytes |
| Application read | 512 bytes |
| Exposed response body | 65,536 bytes |
| Request duration | 15,000 ms |
| Active requests globally / per app | 2 / 1 |

## Request and security policy

V1 is `GET` only. Redirects, retries, bodies, cookies, authorization, ambient
credentials, caller headers, and uploads are absent. Release builds accept only
HTTPS. Plain HTTP may be compiled into an explicitly insecure development
profile; it accepts no secrets, displays an insecure indicator, is not
persistent, and cannot launch as a release package.

Origin permission is checked before DNS. Every DNS answer is checked with
`release_ipv4_allowed` / `release_ipv6_allowed` and checked again immediately
before connect. Release rejects unspecified, loopback, multicast, link-local,
private/unique-local, carrier-grade NAT, and benchmark ranges. A changed DNS
answer receives the same checks; there is no redirect path that could cross an
origin.

Policy and transport share the allocation-free `parse_fetch_url` grammar. Its
borrowed result contains the canonical origin plus the request-target suffix;
an empty suffix means `/`, while a suffix beginning with `?` is encoded as
`/?...`. Fragments, whitespace/control bytes, non-ASCII wire text, missing
authorities, and values over 384 bytes are rejected before backend submission.

The device network future implements dispatch with a fixed
384-byte URL copy. It races DNS A lookup against cancellation and the request
deadline, then requires every returned IPv4 address to pass the release
predicate. The selected address is checked again immediately before a TCP
connect using one existing 1,536-byte RX/TX socket window. Connect is also
cancellable and deadline-bounded; the socket is aborted, bounded-flushed, and
dropped before cancellation is acknowledged. On RP2040, successful TCP
preflight enters the TLS/audio-exclusive boundary, runs the pinned TLS 1.3
exchange, and streams the bounded HTTP response to the application.

`encode_fetch_get_request` writes the v1 request head directly into
caller-owned transport storage after computing its complete size. It adds no
resident request buffer and leaves an undersized destination untouched. The
wire form fixes `GET`, `Host`, `Accept: application/json`, and
`Connection: close`; callers cannot add headers or a body.

Device HTTPS uses manifest-declared SHA-256 SPKI pins, not SNTP time, as its
authentication root. A package may carry two pins for one origin (current and
next); rotation is a signed package update, never a downloaded instruction.
Mismatch is terminal `Tls`, logs no certificate/server text, and does not retry.
Pins are canonical 64-character lower-case hex digests. The fixed table is
indexed by the corresponding allowlist entry; pinning HTTP origins, empty or
duplicate pin sets, more than two pins, and partially pinned device HTTPS
allowlists fail closed. KotoSim retains pins for manifest parity but its fake
does not claim to authenticate a peer.

The portable certificate boundary accepts at most 8,192 bytes of caller-owned
DER. `extract_certificate_spki_der` walks the fixed X.509 TBSCertificate field
order, accepts only canonical definite DER lengths, and returns the complete
SPKI TLV without allocating or copying. The TLS backend hashes exactly that TLV
with SHA-256 and compares all configured pin slots without an early exit.
Malformed/oversized certificates, extraction failure, pin mismatch, and
CertificateVerify signature failure all map to terminal `Tls`.

## TLS backend evaluation history and selection

The evaluation narrative and intermediate measurements in this section record
how the final backend was selected. The accepted product layout is
`embedded-tls` 0.19 with TLS-scoped audio exclusion: a 2,776-byte session in a
3,072-byte Wi-Fi arena slot and a measured 8,468-byte crypto peak in a
10,184-byte scratch-plus-PCM stack. The controlled endpoint completed HTTP 200,
and post-Fetch Wi-Fi Disconnect restored game audio on hardware.

RP2350A uses the same pinned session and 3,072-byte arena slot but does not
exclude audio. Its larger SRAM provides a separate 36,408-byte, one-owner TLS
workspace with 16,640-byte RX, 2,048-byte TX, and a 16 KiB switched crypto
stack. The workspace is volatile-zeroized on every exit; a 4 KiB measured
stack-headroom gate fails closed. This keeps PCM and rich audio storage live
during HTTPS. Pico 2 W hardware completed the controlled HTTP 200 exchange in
389 ms with an 8,352/16,384-byte crypto peak, while PCM/SLDPCM and game audio
continued normally.

The current probe target is `embedded-tls` 0.19 with default features disabled:
it is Rust-native, no-std, TLS 1.3-only, exposes async `embedded-io` transport,
and provides a verifier hook containing both the certificate and handshake
signature. It is not yet a product dependency. Its own documentation describes
the implementation as work in progress and notes that reduced TLS record
buffers are not guaranteed to accept every peer. In addition, KotoOS's current
`embassy-net` 0.7.1 socket implements `embedded-io-async` 0.6 while
`embedded-tls` 0.19 uses 0.7. The evaluation feature now includes an owned,
allocation-free adapter that forwards only read/write/flush and maps every 0.6
error kind into 0.7. Live cancellation/deadline behavior still requires the
controlled endpoint probe.

The device probe must therefore prove all of the following before selection:

- a custom verifier performs SPKI pin matching and CertificateVerify signature
  validation; `NoVerify` is forbidden;
- 4 KiB, 8 KiB, and 16 KiB record-buffer profiles are measured against the
  controlled endpoint, with oversized handshakes failing closed;
- the 0.6-to-0.7 socket adapter preserves cancellation and deadlines without
  allocation or background tasks;
- the TLS crate's existing SHA-256 implementation is reused rather than adding
  an independent cryptographic implementation;
- release flash, static SRAM, handshake peak, and audio residency margins pass
  on RP2040 and RP2350A.

The current RP2040 measurements make this a constrained experiment rather than
a presumed fit. The 36 KiB switchable arena holds 13,296 bytes of CYW43 driver
storage and a measured 15,048-byte network future (including the 9,264-byte IP
stack), leaving 8,520 bytes before Fetch/TLS state. The 2,450-byte portable
Fetch control plane leaves only 950 bytes after 4 KiB receive and 1 KiB transmit
record buffers. A 16 KiB-duplex profile cannot fit at all. The preflight model
admits a target probe only if RP2040 reclaims one 3,072-byte socket window and
the 1,076-byte HTTP decoder is lifecycle-overlaid with handshake storage; that
leaves 5,098 bytes for the unmeasured TLS connection/future and safety margin.
Both optimizations require implementation proof. They do not authorize product
HTTPS, and a peer that cannot negotiate/use the bounded record profile must
fail closed. RP2350A remains the lower-risk product target for HTTPS.

The isolated RP2040 release probe rejected concurrent TLS plus stream audio.
`TlsConnection<ProbeSocket, Aes128GcmSha256>` is 1,264 bytes and the complete
4 KiB/1 KiB handshake task pool is 7,528 bytes. After the one-socket reclaim
and decoder overlay, only 2,690 bytes remain, below the 4,096-byte safety floor.
The handshake state machine adds at least 71,084 bytes of `.text`; this remains
a lower bound because the fail-closed layout verifier deliberately rejects all
peers and does not link production CertificateVerify cryptography or the
Embassy socket adapter. The later adapter variant adds 120 bytes of `.text`, 8
bytes of `.rodata`, and 16 bytes of `.bss`; its task pool is 7,544 bytes and
its transparent socket wrapper is 8 bytes. RP2350A cross-builds both variants,
but still requires its own full firmware and hardware peak gate.

RP2040 therefore uses TLS-scoped audio exclusion. Ordinary Wi-Fi, DHCP, DNS,
SNTP, scans, and non-TLS development traffic retain `WifiStreamAudio`. Before
an HTTPS connection is created, the residency owner enters
`QuiescingStreamForTls`; new audio calls return `TemporaryUnavailable`, queued
PCM is discarded, PWM/DMA and worker access must be acknowledged stopped, and
only then may the owner publish `TlsExclusive`. The exclusion lasts for the
entire TLS connection, not merely its handshake, because record buffers and
connection keys remain live while encrypted response bytes are read.
Completion, cancellation, timeout, disconnect, and every error path must erase
and drop TLS state before `RestoringStreamAfterTls` rebuilds stream audio. A
failed or timed-out quiesce starts no TLS operation and lands in a safe offline
state.

The RP2040 audio backend enforces the worker side of this boundary. It aborts
the audio DMA channel, waits for BUSY to clear, fixes both PWM outputs at the
silent midpoint, resets the PCM ring, and only then acknowledges
`TlsExclusive`. Restoration fills the duty ring with silence and re-arms DMA
pacing before `WifiStreamAudio` is published. CPU0's package-stream refill path
does not acquire its scratch region in transitional or TLS-owned states.

The implemented workspace retains the 8,192-byte CPU1 stack, refill/decode
scratch, and DMA ring and loans only the PCM sample array: 8,192 bytes. A
generation-owned handle can install one type-erased future in those bytes.
Returning the handle performs a volatile full overwrite before it permits the
audio restore transition. Dropping or leaking the handle fails safe by leaving
audio unavailable. The larger 14,392-byte PCM/scratch/DMA reclaim remains only
an optional candidate and is not counted by the admission gate.

The production-shaped evaluation verifier is now linked as a fourth probe
variant. It hashes the exact leaf SPKI DER, compares both package pins without
an early exit, accepts only `id-ecPublicKey`/`prime256v1` with an uncompressed
point, and then requires TLS 1.3 `EcdsaSecp256r1Sha256` CertificateVerify over
the captured transcript. It performs no SNTP or certificate-time check because
the signed package pin is the trust root. Other key types and signature schemes
fail closed. The verifier is 256 bytes, its provider is 264 bytes, and the task
pool is 7,744 bytes, leaving 448 bytes inside the PCM workspace and 6,518 bytes
across the RP2040 exclusion envelope, above the 4,096-byte floor.
Relative to the socket-adapter probe it adds 24,716 bytes of `.text`, 312 bytes
of `.rodata`, and 200 bytes of `.bss`.
An 8 KiB/2 KiB record-buffer preflight also fits, but requires its own exact ELF
and controlled-peer measurement. Device capacity is one TLS transaction because
the exclusion workspace has one owner. RP2350A does not require audio exclusion.

Candidate integration also must account for `embedded-tls` 0.19 treating an
error returned while acquiring its verifier as permission to skip certificate
and CertificateVerify checks. A KotoOS provider would have to make verifier
availability structurally infallible and return terminal verification errors
from the verifier methods themselves. This API behavior means `embedded-tls`
remains a probe candidate rather than an approved product backend even though
TLS-scoped audio exclusion now clears the RP2040 SRAM admission floor.

Evaluation references: [embedded-tls 0.19 crate documentation](https://docs.rs/embedded-tls/0.19.0/embedded_tls/),
[its custom `TlsVerifier` contract](https://docs.rs/embedded-tls/0.19.0/embedded_tls/trait.TlsVerifier.html),
and [embedded-tls 0.17 dependency metadata](https://docs.rs/crate/embedded-tls/0.17.0)
for the older `embedded-io-async` 0.6-compatible baseline. The older release is
not selected merely for interface compatibility; dependency/security review and
target measurements are mandatory.
The RP2040 and RP2350A product backends expose HTTPS while their Wi-Fi runtime
is active. Networking-disabled and unsupported device profiles return
`Unavailable`; KotoSim's deterministic fake does not claim to validate TLS.

## Diagnostics and offline behavior

Diagnostics contain only state, allowlist origin index, request generation,
copied byte count, and bounded elapsed milliseconds. Paths, queries, bodies,
header values, DNS names returned by a server, TLS material, and Wi-Fi secrets
are never logged. Networking-disabled and unsupported builds use an unavailable
backend and allocate or schedule no socket, TLS, host-network, wall-clock, or
retry work.

## Streaming HTTP decoder

`HttpResponseDecoder` incrementally consumes caller-retained input into
caller-owned output. Its 1,024-byte header staging and 16-byte chunk-size staging
never allocate from response-controlled lengths. V1 accepts HTTP/1.1 with one
bounded `Content-Length` or exact `Transfer-Encoding: chunked`; conflicting
lengths, length-plus-transfer-encoding, folded/invalid headers, redirects,
informational responses, chunk extensions, trailers, close-delimited bodies,
oversized metadata, and bodies above 65,536 bytes fail closed with fixed
`Protocol` or `ResponseTooLarge` errors. `reset` zeroizes all partial parsing
state before reuse.

## Device loader gate

The portable `parse_manifest_fetch_permission` walker validates the complete
metadata JSON with no allocation and a maximum nesting depth of eight. It reads
only the root `permissions.network` member, rejects duplicate or ambiguous
permission shapes, and feeds every origin through the same `FetchOrigin`
canonicalization used by the service. KotoSim retains the resulting allowlist.
The device catalog validates it but keeps only compact `PackageInfo`; when the
app launches, the selected KPA metadata is reread and its allowlist/pins become
a generation-owned device Fetch session. Its four VM hostcalls and
authenticated HTTPS transport are live on the validated RP2040 Pico W profile.
The 2,304-byte catalog manifest scratch is overlaid by this
control plane for the app lifetime, rather than adding a second permanent SRAM
allocation or multiplying permissions across the catalog. Offline builds still
link no Fetch socket, TLS, DNS, retry, or background state.

The Wi-Fi runtime remains OS-owned while an app executes. App frames issue eight
bounded cooperative polls through an opaque background-service capability, so
the CYW43 runner and Embassy stack keep progressing while the shell loop is
suspended. This capability exposes no DNS, `Stack`, socket, or packet access and
is not part of `VmHost`; it only services the arena-owned future.

## Measured portable SRAM floor

The RP2040 release-layout probe `probe_app_fetch_service` records the state that
exists before a device transport is enabled:

| Component | Bytes |
| :-- | --: |
| Two-slot service control (`UnavailableFetchBackend`) | 72 |
| Four-origin allowlist | 1,034 |
| Streaming HTTP decoder | 1,076 |
| Four-origin SPKI pin table (current + next) | 268 |
| Total | 2,450 |

The same probe separately measures a 596-byte, one-request transport mailbox:
384 URL bytes, one 128-byte response chunk, the selected two-pin set, request
generation/state, and explicit cancel acknowledgement. It has a distinct
640-byte ceiling. `WifiRuntime` wraps it in a four-byte synchronized view and
initializes the resulting 604-byte slot after the 12,688-byte CYW43 `State`
inside the existing 13,296-byte driver reservation, leaving 4 bytes. The final
four bytes hold a generation-tagged TLS/audio exclusion coordinator;
it consumes no product-static SRAM and does not reduce the 23,568-byte
runner/network-future region. The RP2350A target reports the same component
sizes. `check_app_fetch_budget.py` gates the RP2040 portable total at 3,072
bytes. DNS query storage,
TCP socket buffers remain separately owned. The TLS task and its PCM-backed
workspace have separate measured ceilings but are not yet wired into the
device Fetch backend.

The RP2040 CPU0 audio facade consumes coordinator quiesce requests during app
audio service. It publishes `ExclusiveReady` only after the CPU1/DMA fence has
made `TlsExclusive` observable. Cancellation before workspace transfer either
leaves active stream audio untouched or claims and immediately releases the
stopped PCM workspace, invoking its full zeroization and normal restore path.
Completion is not published until `WifiStreamAudio` is observable. The network
side now takes `WorkspaceOwned` after TCP connect and revalidates the global
residency generation and one-owner CAS. Until product TLS is linked it
immediately returns and zeroizes the loan, then a bounded app-teardown drain
keeps the network and audio fences live through restoration. Device Fetch stays
unavailable during this proof path. RP2350A does not service this exclusion
state because audio and TLS remain concurrent.
`check_app_fetch_tls_feasibility.py` combines the existing measurements into a
preflight admission envelope; it does not replace the required TLS target ELF
and handshake peak measurement.
`check_app_fetch_tls_probe.py` records the subsequent isolated candidate ELF.
Under the TLS/audio exclusion envelope it reports
`continue_controlled_endpoint_probe`; this permits further measurement, not
product HTTPS.
