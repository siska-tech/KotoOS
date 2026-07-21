# KOTO-0245: Bounded app Fetch service and SDK

- Status: done
- Type: feature
- Priority: P1
- Requirements: FR-SDK-5, FR-RT-4, FR-PKG-1, FR-PKG-3, NFR-MEM-2, NFR-PORT-1, NFR-PORT-3, NFR-PORT-6, NFR-REL-1, NFR-REL-3, NFR-REL-5, NFR-DEV-3, NFR-DEV-4, NFR-DEV-5
- Related: KOTO-0019, KOTO-0036, KOTO-0047, KOTO-0239, KOTO-0240, KOTO-0242, KOTO-0244

## Goal

Allow sandboxed Koto applications to retrieve bounded public data through an
OS-owned, capability-gated Fetch service and a common KotoSDK API on KotoSim
and supported devices. Preserve NetworkService ownership, VM isolation,
deterministic resource use, audio responsiveness, and useful offline behavior;
do not expose raw TCP, UDP, DNS, TLS, or embassy-net handles to applications.

The first version is a read-only `GET` facility. Request bodies, credentials,
cookies, arbitrary request headers, uploads, and general-purpose sockets remain
outside this issue.

## Acceptance Criteria

- [x] Document and freeze an `AppFetchService` boundary and request lifecycle
  above the firmware-owned NetworkService. It has fixed request/socket/buffer
  capacity, at most one active request per application, a measured global
  limit, generation-tagged request IDs, bounded deadlines, and cancellation;
  application code never receives a network-stack handle or retained raw
  pointer.
- [x] Version the `.kpa` permission schema so network access is default-denied
  and an application can declare a fixed-capacity allowlist of canonical
  `(scheme, hostname, port)` origins. Reject malformed, duplicate, oversized,
  wildcard, user-info, and unsupported declarations consistently in the
  packer, KotoSim, and device package loaders.
- [x] Add stable nonblocking Runtime ABI operations equivalent to
  `fetch_start`, `fetch_poll`, `fetch_read`, and `fetch_cancel`. They use the
  implicit unforgeable `AppContext`, copy through caller-owned bounded VM
  buffers, return fixed result/error enums, reject stale or foreign request
  IDs, and update the bytecode ABI/version and verifier documentation.
- [x] Provide allocation-free KotoSDK wrappers and a small packaged sample
  that performs a `GET`, incrementally reads a bounded response, handles
  `Pending`/timeout/offline/denied states, and remains responsive while a
  request is active. The same application source runs on KotoSim and device.
- [x] Limit v1 requests to `GET`; disable redirects, cookies, ambient
  credentials, authorization headers, request bodies, and automatic retries
  of application requests. Define fixed maxima for URL, origin, headers,
  status metadata, response bytes per read, total response bytes, and request
  duration. Oversized or malformed input fails closed without partial parsing
  state leaking into the next request.
- [x] Authenticate device traffic without treating KOTO-0244's
  unauthenticated SNTP clock as trusted certificate-validation time. Record
  the selected bounded policy (for example, manifest-declared SPKI pinning),
  its key-rotation behavior, TLS/SRAM/flash cost, and failure semantics before
  enabling device HTTPS. Plain HTTP, if retained for controlled development,
  is visibly insecure, accepts no secrets, and is disabled in release
  profiles.
- [x] Enforce the declared origin before DNS and again after resolution.
  Release profiles reject loopback, unspecified, multicast, link-local, and
  private destinations, DNS answers that change into a forbidden range, and
  cross-origin redirects. Any privileged local-network development override
  is explicit, non-persistent, and unavailable to packaged release apps.
- [x] Treat all response bytes as untrusted. The HTTP decoder is streaming and
  fixed-capacity, validates framing (including conflicting lengths and bounded
  chunk metadata), never allocates from response-controlled sizes, and
  returns deterministic protocol/size errors while preserving VM and OS
  memory isolation.
- [x] Cancelling an application, losing its network permission, changing
  NetworkService generation, disconnecting Wi-Fi, or tearing down the service
  cancels its requests and zeroizes transient URL/header/TLS state. Offline or
  unsupported builds return a stable `Unavailable` result and do not link
  sockets, TLS state, or background retry work.
- [x] Diagnostics expose only fixed status, origin index, request generation,
  byte counts, and bounded timing information. They never log URL paths,
  queries, response bodies, header values, TLS key material, Wi-Fi secrets, or
  server-provided diagnostic text.
- [x] Add deterministic KotoSim fakes and unit/integration tests for success,
  partial reads, cancellation, timeout, offline/unavailable service, denied
  origin, DNS failure, forbidden/rebound address, malformed framing,
  oversized response, disconnect, capability loss, and stale request IDs.
  Default tests use neither the host network nor host wall clock.
- [x] Cross-build supported Wi-Fi-enabled and disabled RP2040/RP2350 profiles,
  record SRAM/flash/socket/TLS budgets, and validate the packaged sample on
  hardware against a controlled authenticated endpoint. Existing audio,
  Shell, filesystem, and network-service residency canaries remain within
  their accepted floors during transfer and failure recovery.

## Dependencies

KOTO-0239 owns the bounded network stack and reserved socket policy.
KOTO-0242 provides the deterministic simulator network model. Device HTTPS is
not enabled until this issue records and verifies an authentication policy
that is independent of KOTO-0244's advisory SNTP time. KOTO-0240's Wi-Fi
credential store is not an application credential store and must not be
reused through the Fetch ABI.

## Non-goals

- Raw TCP/UDP sockets, DNS handles, listeners, or application-owned TLS state
- `POST`/`PUT`, uploads, WebSocket, MQTT, streaming media, or server push
- Cookies, bearer tokens, passwords, client certificates, or a general secret
  store for applications
- Background execution, scheduled polling, or network access while an app is
  not the active sandbox
- Trusting downloaded content as executable code, an update, or a security
  decision without a separate signed-content design

## Implementation Progress

### 2026-07-19: portable boundary started

Implementation has started with the portable, allocation-free Fetch contract in
`koto-core`. This slice freezes the bounded origin, request-ID, lifecycle,
deadline, cancellation, diagnostic, destination-filter, and backend seams used
by both KotoSim and firmware. Manifest v2 exact-origin validation now agrees in
the core model, packer, and KotoSim; host ABI minor 19 and compiler-prelude
wrappers expose start/poll/read/cancel; and KotoSim has a deterministic scripted
backend with no host network or wall clock. The architecture contract records
the device SPKI-pin gate. Streaming HTTP decoding, packaged sample, live runtime
wiring, device transport/pin store, budget accounting, cross-builds, and
hardware validation remain open and keep the issue in progress.

### 2026-07-19: streaming decoder and packaged simulator path

The portable boundary now includes a fixed-capacity streaming HTTP/1.1 decoder
for bounded Content-Length and chunked responses, with fail-closed handling for
ambiguous framing, redirects, informational responses, extensions, trailers,
and oversized metadata or bodies. KotoSim's live VM host now carries the
manifest-v2 origin allowlist into `AppFetchService`, advances a deterministic
host-network-free backend by simulated frames, and serves the stable ABI with
frame-stable poll results. The packaged Fetch Weather sample exercises pending,
incremental read, success metadata, failure/offline states, cancellation, and
responsiveness; an end-to-end test launches its committed `.kpa` fixture.
Device transport and pin-store implementation, device-loader parity, measured
budgets, cross-builds, and hardware validation remain open.

### 2026-07-19: device manifest gate and cross-build parity

KotoSim and the PicoCalc catalog loader now share a fixed-depth, allocation-free
parser for the root manifest Fetch permission. The device rejects malformed,
duplicate, wildcard, escaped, oversized, or version-mismatched origin lists
before catalog admission while retaining no network transport state. Offline
RP2040 and RP2350A library profiles and `network_service`-enabled Pico W and
Pico 2 W profiles cross-check successfully. The device VM host deliberately
continues to return the stable `Unavailable` result; authenticated transport,
SPKI pin storage, measured linked flash/SRAM budgets, and hardware endpoint
validation remain open.

### 2026-07-19: portable SRAM floor measured

An RP2040 release-layout ELF probe now freezes the pre-transport Fetch control
plane at 2,182 bytes: 72 bytes of two-slot service state, 1,034 bytes for four
origins, and 1,076 bytes for the streaming HTTP decoder. A machine-readable
gate enforces a 3,072-byte ceiling and its parser self-test runs in the standard
harness. The offline backend is zero-sized and retains stable `Unavailable`
semantics; lifecycle tests now prove that teardown cancels both global slots
and invalidates their request generation. DNS/TCP/TLS/SPKI storage and linked
flash remain unbudgeted until the authenticated device transport is selected.

### 2026-07-19: SPKI pin model and rotation frozen

Manifest v2 now supports an HTTPS origin object with one current and one
optional successor SHA-256 SPKI digest. The portable parser, KPA packer,
KotoSim, and device catalog gate agree on exact lower-case hex encoding,
HTTPS-only use, duplicate rejection, and the two-pin ceiling. The fixed
origin-indexed table costs 268 bytes on RP2040, bringing the measured portable
control plane to 2,450 bytes under its 3,072-byte gate. Rotation requires
overlapping signed package updates; response data cannot alter trust state.
The table and rotation policy are ready; hashing the extracted SPKI and
CertificateVerify validation remain responsibilities of the unselected TLS
backend.

### 2026-07-19: certificate/SPKI boundary and TLS candidate gate

The portable boundary now extracts the exact SPKI DER TLV from a caller-owned
X.509 certificate with canonical definite-length parsing, an 8,192-byte
certificate ceiling, no allocation, and deterministic malformed/size errors.
Pin comparison visits every configured slot without an early exit. Research
selects `embedded-tls` 0.19 only as the next probe target: it offers no-std async
TLS 1.3 and a custom verifier contract, but remains explicitly gated because it
is documented as work in progress, may require large record buffers, and uses
`embedded-io-async` 0.7 while the current Embassy socket uses 0.6. Product
selection requires an adapter, CertificateVerify validation, target memory and
flash reports, and controlled-endpoint tests; `NoVerify` is prohibited.

### 2026-07-19: RP2040 TLS admission envelope measured

The existing Wi-Fi measurements leave 8,520 bytes in the 36 KiB switchable
arena before Fetch/TLS state. Conventional 16 KiB-duplex buffers cannot fit;
even 4 KiB receive plus 1 KiB transmit leaves only 950 bytes after the measured
2,450-byte Fetch control plane. A dependency-free feasibility report now admits
only a target probe that both reclaims one 3,072-byte application socket window
and lifecycle-overlays the 1,076-byte HTTP decoder with handshake storage. That
constrained case leaves 5,098 bytes for unmeasured TLS state and margin, so it
is a probe hypothesis rather than product approval. RP2040 device HTTPS stays
`Unavailable` unless exact ELF/handshake measurements and controlled-endpoint
tests pass; RP2350A is the lower-risk HTTPS target.

### 2026-07-19: embedded-tls concurrent-audio RP2040 layout rejected

An evaluation-only Cargo feature island now cross-builds `embedded-tls` 0.19
without making it reachable from a board or product feature. The RP2040 release
ELF measures a 1,264-byte connection and a 7,528-byte 4 KiB/1 KiB handshake
task. Even after reclaiming one socket and overlaying the HTTP decoder, the
remaining arena margin is 2,690 bytes, below the 4,096-byte admission floor.
The linked handshake state machine also adds at least 71,084 bytes of `.text`;
production signature verification and the Embassy adapter would increase it.
The original report recorded `reject_for_rp2040`. RP2350A cross-builds with the
same task size and remains eligible for a later full-firmware/hardware probe.
Source inspection also found that verifier acquisition errors cause the crate
to skip certificate and CertificateVerify checks, so any future provider must
make verifier availability structurally infallible and fail inside verifier
methods. The diagnostic provider always rejects every peer.

### 2026-07-19: RP2040 TLS-scoped audio exclusion selected

RP2040 now preserves stream audio during ordinary Wi-Fi operation and makes it
temporarily unavailable only for the complete lifetime of an HTTPS/TLS
connection. A generation-owned state contract adds
`QuiescingStreamForTls -> TlsExclusive -> RestoringStreamAfterTls`; TLS cannot
touch audio bytes before stop acknowledgement, and audio cannot restart before
TLS state is erased and dropped. The conservative budget keeps the 8,192-byte
CPU1 stack and reclaims the 8,208-byte PCM ring, 5,164-byte decode/refill
scratch, and 1,024-byte DMA ring, totaling 14,396 bytes. The measured 4 KiB/1
KiB TLS task then leaves 12,938 bytes, so the candidate report advances to
`continue_controlled_endpoint_probe`. This does not enable device HTTPS: full
production verification, controlled-peer tests, and hardware
stack/latency measurements remain open. The RP2040 audio backend now implements
the worker side of the fence: it aborts DMA, fixes PWM at the silent midpoint,
clears the PCM ring, and publishes the TLS-exclusive acknowledgement only after
those operations complete. Restore pre-fills the DMA ring with silence and
re-arms pacing before audio becomes available. The actual TLS workspace loan
and borrower zeroization are still gated on the selected transport. RP2350A
keeps concurrent audio/TLS behavior.

### 2026-07-19: embedded-io socket adapter measured

The evaluation island now owns the required `embedded-io-async` 0.6-to-0.7
adapter. It forwards only read, write, and flush, preserves every known error
kind, allocates no storage, and type-checks directly with embassy-net 0.7.1's
`TcpSocket` on RP2040 and RP2350A. The adapter-backed RP2040 release probe adds
120 bytes of `.text`, 8 bytes of `.rodata`, and 16 bytes of `.bss`; the wrapper
is 8 bytes and the complete task pool grows from 7,528 to 7,544 bytes. The
TLS/audio-exclusive margin is therefore 12,922 bytes, still above the 4,096-byte
floor. Live socket cancellation/deadline tests, production verification, TLS
workspace loan/zeroization, and controlled-peer negotiation remain open.

### 2026-07-20: pinned P-256 CertificateVerify linked

The evaluation transport now includes the production-shaped authentication
path. It hashes the exact leaf SPKI TLV, compares both manifest pins, admits
only an uncompressed P-256 key with exact algorithm/curve OIDs, and verifies the
TLS 1.3 server CertificateVerify transcript signature. Trust remains independent
of unauthenticated SNTP time; unsupported curves and signature schemes fail
closed. A host known-answer test covers success, transcript alteration, and
malformed signatures. The verifier is 256 bytes, the provider is 264 bytes,
and the complete task is 7,744 bytes. Versus the adapter variant this adds
24,716 bytes of `.text`, 312 bytes of `.rodata`, and 200 bytes of `.bss`.

### 2026-07-20: PCM-backed TLS workspace ownership implemented

The RP2040 audio backend now lends only the 8,192-byte PCM sample array through
a generation-owned `TlsAudioWorkspace`; scratch, DMA storage, and the CPU1 stack
remain untouched. The handle installs a bounded `ArenaFuture`, so Rust prevents
return while the TLS future still borrows the bytes. Release performs a volatile
full overwrite before starting audio reconstruction; a leaked handle fails safe
with audio unavailable. The 7,744-byte verifier task leaves 448 bytes inside
the workspace and 6,518 bytes across the implemented envelope, 2,422 bytes over
the 4,096-byte floor. Host regressions cover future cancellation/drop ordering
and workspace zeroization. Wiring the live Fetch transport into this handle,
peer negotiation, and cancellation/deadline hardware tests remain open.

### 2026-07-20: device app-session boundary wired without new SRAM residency

The PicoCalc VM host now implements all four Fetch ABI calls through a real
generation-owned `AppFetchController` session instead of inheriting generic
`UNSUPPORTED` methods. At launch the device rereads the selected KPA metadata
and rebuilds its exact origin and two-pin tables; the shell catalog still keeps
only `PackageInfo`, avoiding a per-package SRAM multiplier. The 2,304-byte
catalog manifest scratch is overlaid by the Fetch control plane while an app
runs. This removed a rejected intermediate layout's 2,440-byte static/future
increase: the initial RP2040 release image measured 205,612 bytes of
`.data + .bss` and the same 205,904-byte static span as the preceding workspace
build. The backend remains fail-closed `Unavailable`; the
next step is an OS-private live executor owning DNS/TCP/TLS without exporting
an Embassy stack or socket to the app.

### 2026-07-20: app frames keep the network executor live

The product Wi-Fi runtime was previously polled only by the shell loop, so
launching an app suspended the CYW43 runner, DHCP, stack monitor, and SNTP
future. `run_device_app` now accepts an OS-only background-service capability
and performs the same bounded eight cooperative polls at the start of every app
frame. `WifiRuntime` forwards the caller's waker into its arena-owned lifecycle
future; offline builds use a no-op value. No `Stack`, socket, or driver handle
reaches `DeviceHost` or the VM. RP2040, Pico W, and Pico 2 W cross-checks pass,
and the RP2040 static span remains 205,904 bytes. A command consumer can now be
added inside the live
network future without first adding an app-specific executor or SRAM regression.

### 2026-07-20: single-request transport mailbox contract measured

A portable caller-owned mailbox now freezes the only data allowed to cross
between `DeviceHost` and the network future: one 384-byte URL, one 128-byte
response chunk, the selected two-pin set, request generation/state, HTTP status,
and an explicit cancellation acknowledgement after the executor has dropped
its socket/TLS future. It rejects a second request, stale IDs, oversized copies,
and producer reuse before the VM drains the current chunk; terminal release and
cancel acknowledgement zero the payload arrays. Nineteen Fetch host tests pass.
The RP2040 ELF probe measures 596 bytes under a separate 640-byte gate.
`WifiRuntime` wraps it in a critical-section view and initializes the current
604-byte synchronized slot after the 12,688-byte CYW43 `State` inside the
existing 13,296-byte driver reservation, leaving 4 bytes. The
23,568-byte runner/network region and product static SRAM are unchanged. The
next step is the live network dispatcher; no socket/TLS object may leave the
arena.

The synchronized view is now passed into the arena-owned network future. Both
sides receive only short closure-based access, so neither can retain a mailbox
reference across an async wait. Cancellation acknowledgement is issued only
from inside that future after any active transport future has been dropped.

The device session now separates its generation/slot-owning
`AppFetchController` from the external backend. `DeviceHost` forwards only the
pin set selected by the validated origin into `WifiRuntime`'s synchronized
mailbox; a regression test prevents the pins of any other allowed origin from
crossing that boundary. Availability remains false until the live dispatcher
can own and cancel DNS/TCP/TLS state. This split raises `.data + .bss` to
205,684 bytes (`+248 B` from KOTO-0226), but the static span remains exactly
205,904 bytes (`+152 B`), so the SRAM end address and 2,422-byte admission-floor
margin are unchanged.

### 2026-07-20: transport URL grammar frozen before DNS dispatch

The portable core now exposes one allocation-free `parse_fetch_url` result for
both allowlist enforcement and the future network executor. It yields the
already-validated scheme, hostname, port, and borrowed HTTP request-target
suffix, including explicit root and query-only cases. Wire-unsafe whitespace,
non-ASCII text, fragments, missing authorities, noncanonical origins, and URLs
over 384 bytes fail before backend submission. This prevents the DNS/TCP/TLS
dispatcher from growing an independent URL parser or disagreeing with the
permission check. Twenty-one Fetch unit tests, the KotoSim packaged integration,
all three firmware cross-builds, and the SRAM gates pass. The RP2040 report is
unchanged at 205,684 bytes `.data + .bss` and a 205,904-byte static span.

### 2026-07-20: cancellable device DNS preflight installed

The arena-owned network future now consumes a queued URL into fixed 384-byte
executor scratch, reuses `parse_fetch_url`, rejects non-HTTPS requests and
missing pins, and performs an A-record query only while DHCP configuration is
up. DNS is raced against the synchronized cancellation watcher and the
15-second deadline; cancellation drops the DNS future before acknowledging the
mailbox. Empty/error answers map to `Dns`, timeout to `Timeout`, and every
returned IPv4 address must pass the release destination predicate or the whole
request fails `ForbiddenAddress`. After a valid public answer the stage still
returns `Unavailable`: no TCP connection begins until the TLS/audio-workspace
owner is connected, and `fetch_available()` remains false. RP2040, Pico W, and
Pico 2 W release builds pass with unchanged static SRAM: 205,684 bytes
`.data + .bss`, 205,904-byte span.

### 2026-07-20: bounded GET encoder and TCP preflight installed

The portable core now encodes the complete v1 request head directly into
caller-owned storage: `GET`, root/path/query target, canonical `Host` including
an explicit non-default port, JSON accept type, and `Connection: close`. It
sizes and validates before writing, so a short destination is unchanged and no
second request buffer is needed alongside the TLS 1 KiB transmit record area.
Twenty-three Fetch tests cover root, query-only, explicit-port, and short-buffer
cases.

The device dispatcher now borrows the first existing 1,536-byte RX/TX socket
window after DNS. It checks the selected address again immediately before
connect, races connect against cancellation and the 15-second deadline, and on
every exit performs `abort`, a bounded flush, and socket drop before updating
the mailbox. A successful TCP connection still ends `Unavailable` because TLS
has not claimed the audio workspace; applications remain unable to submit the
request. All three release profiles and the RP2040 SRAM gate pass with the same
205,684-byte `.data + .bss` and 205,904-byte static span.

### 2026-07-20: generation-tagged TLS/audio coordinator fitted in arena padding

The network/audio ownership handshake now has an explicit one-word coordinator
with `Idle`, quiesce request/ACK, exclusive-ready, workspace-owned, restore,
cancel, complete, and failed states. A nonzero wrapping 24-bit generation and
the state share one atomic word; every transition compares both, so stale or
out-of-order ACKs cannot advance a newer TLS request. It carries no pointer or
workspace reference. The four bytes extend the synchronized Wi-Fi slot from
600 to 604 bytes and consume existing driver-reservation padding, leaving four
bytes. The 36 KiB arena, 23,568-byte future/network reserve, `.data + .bss`, and
205,904-byte static span remain unchanged. The next slice must make the CPU0
audio facade service these states and return the zeroized workspace before a
terminal ACK.

### 2026-07-20: CPU0 audio exclusion consumer wired into app frames

`AppBackgroundService` now has an OS-only TLS/audio service hook. Each device
app audio-service pass first advances the existing CPU1 residency fence and
then lets `WifiRuntime` consume the one-word coordinator. On RP2040 a quiesce
request invokes `begin_tls_audio_quiesce`; only the observed `TlsExclusive`
state advances to `ExclusiveReady`. A cancellation before workspace transfer
either completes immediately while stream audio is still active or, after the
worker stopped, claims and releases the empty workspace so all 8 KiB are
zeroized before restoration. `Restoring` becomes `Complete` only after
`WifiStreamAudio` is observable again. RP2350A's hook is a no-op because it
retains concurrent audio.

At this checkpoint the network future did not yet advance `ExclusiveReady` to
`WorkspaceOwned`, so no product TLS request could borrow the bytes and
app-visible Fetch remained disabled.

### 2026-07-20: network workspace handoff and teardown drain connected

The network future now requests RP2040 audio exclusion only after TCP connect,
races the two-second quiesce against Fetch cancellation, and claims the shared
`TlsAudioWorkspace` only after `ExclusiveReady`. The claim rechecks the global
audio residency generation and one-owner CAS without retaining a
`PicoAudioBackend` pointer. Because product TLS is still unlinked, the current
stage immediately returns the loan: all 8 KiB are zeroized, a new restore
generation is issued, and the coordinator advances through `Restoring` to
`Complete` and back to `Idle`. This exercises the real ownership transfer while
still returning `Unavailable` and never sending an HTTP request.

Every app exit path now reaches an outer bounded drain after Fetch teardown.
For at most two seconds it polls the network future, CPU1 fence, workspace
return, and stream restoration; failure stays fail-safe with audio unavailable
rather than reusing live TLS bytes. Audio residency and arena-future host
harnesses pass 3 and 7 tests respectively, including stale-generation and
zeroization coverage. The extra async drain state moves the RP2040 report to
`.data=66,344 B`, `.bss=139,384 B`, `.data + .bss=205,728 B`, and a
205,944-byte static span (`+292 B` and `+192 B` from KOTO-0226). The TLS
admission-floor excess is now 2,382 bytes. Pico W/Pico 2 W release builds,
23 Fetch tests, KotoSim integration, and the SRAM gate pass. App-visible Fetch
remains disabled until the measured TLS future occupies the ownership interval.

### 2026-07-21: product TLS session occupies the ownership interval

A new additive `app_fetch_https` product feature links the pinned TLS 1.3
session into the network dispatcher; the evaluation probe island is unchanged.
After TCP connect and audio quiesce, the 8 KiB PCM workspace is split into the
measured 4 KiB/1 KiB record buffers plus a session-future arena tail, and one
type-erased `ArenaFuture` performs handshake, `PinnedP256Verifier`
pin-and-CertificateVerify authentication (ROSC hardware RNG; SNTP never
consulted), the encoded GET head, and the streaming decoder pump. The decoder,
128-byte staging pair, and 768-byte encoded head live in the network-future
frame so the by-value arena install stays small (KOTO-0172/0252 stack lesson);
the session races the 15-second deadline and mailbox cancellation, and every
exit zeroizes staging, resets the decoder, and releases the workspace through
the volatile-overwrite path before audio restoration.

The transport mailbox gained a producer-visible Headers observation: the
device `fetch_poll` now uses `poll_mut`, and the producer holds the first body
chunk until the app has seen `Headers { status }` once, restoring KotoSim's
success-metadata guarantee on device. `fetch_available()` is now true for
active `app_fetch_https` RP2040 runtimes; every other combination keeps
`Unavailable`. Baseline comparison shows the feature is static-SRAM-neutral
(`.data=67,052 B`, `.bss=142,524 B` with and without it — the drift from the
2026-07-20 row landed with KOTO-0251/0252) and costs 115,316 bytes of `.text`
(release image 1,516,488 B of 2 MiB). koto-core passes 377 host tests
including the new observation test; all four embedded profiles (a new
`app_fetch_https` cross-check row included), KotoSim's packaged sample, and
the Wi-Fi-layout/Fetch/NetworkService/audio-residency SRAM gates pass.

That layout was superseded by the 2026-07-22 real-device diagnosis below.

### 2026-07-22: crypto stack moved to the full PCM ring

Real-device packet capture isolated the post-ClientHello deadlock to the
device transmit path: the server repeatedly sent its 767-byte handshake flight
but received no ACK, while the device waited for Finished and could not even
send an RST at timeout. Certificate and pin verification were correct. The
crypto-stack high-water was 5,116 of 5,120 bytes; an interrupt at that peak
pushed its frame below the painted region, outside the high-water scan, into
the adjacent CYW43/embassy-net residency and wedged network transmission.

The first repaired RP2040 TLS-exclusive layout used the full 8,192-byte PCM
sample ring as the crypto stack, a 2,048-byte receive-record prefix plus a
3,072-byte session-future arena in the guarded CPU0 stream scratch, and the
1,024-byte transmit record in the quiesced DMA ring. ELF placement confirmed
the fetch-local stream scratch is
immediately below the PCM stack, rather than the live network arena. A
2,048-byte minimum post-exchange stack-headroom check fails closed as `Tls`;
an incoming TLS record larger than the fixed receive buffer also fails closed.
The max-fragment-length extension remains disabled because the controlled
server did not complete the handshake when it was requested.

The product HTTPS release build passes with `.data=67,052 B`,
`.bss=142,588 B`, `.data + .bss=209,640 B`, and a 210,172-byte static span.
Hardware revalidation then completed the TLS and HTTP exchange (`tls_phase=23`,
HTTP 200, 12 body bytes), proving that the ACK/TX deadlock was repaired. It
also corrected the truncated old high-water assumption: the painted 8 KiB
stack measured 7,836 bytes including the real interrupt pattern, leaving only
356 bytes, and the session future occupied 3,056 of the actual 3,072-byte raw
arena. The safety gate therefore intentionally returned `Tls` after the
otherwise successful exchange.

The follow-up layout grows the PCM ring/crypto stack to 10,240 bytes (2,404
bytes above that measured peak) and adds 256 bytes of TLS-only tail padding to
the audio scratch, making the session arena 3,328 bytes (272 bytes above the
measured future). Audio remains unavailable only during the TLS ownership
interval. Its release image measures `.data=67,052 B`, `.bss=144,636 B`, and a
212,220-byte static span; the extra static span is 2,048 bytes because existing
alignment padding absorbs the scratch-tail increase. All SRAM gates pass. A
second hardware run must confirm a crypto peak no greater than 8,192 bytes
(`crypto=.../10240`), `tls_future=3056/3328`, and application-visible
completion.

That static-growth experiment was rejected on hardware: although its TLS-local
regions fit, the additional 2,048-byte static span consumed nearly all of the
already measured CPU0 application-stack margin. Audio became corrupt and the
firmware panicked in `embassy-rp`'s time-driver queue. The image must not be
used. The layout is restored to the SRAM-neutral 8 KiB PCM loan. P-256 now
prehashes the TLS CertificateVerify message and calls a non-inlined prehash
verifier frame; the TLS trait callback is also non-inlined, preventing its
transcript frame from remaining live across the deepest scalar-arithmetic
call. Hardware must remeasure the resulting 8 KiB crypto high-water before the
headroom gate can be accepted or tuned. The immediate gate is 512 bytes: 128
times the failed four-byte margin and large enough for the observed interrupt
frame, while the preferred long-term target remains 2 KiB. Disassembly confirms
the live `verify_signature` frame at the P-256 call fell from 212 to 20 bytes;
the stored verifier transcript also shrank from a SHA-256 state to its 32-byte
digest.

Hardware rejected that stack-only optimization too: the completed HTTP 200
exchange measured `crypto=8188/8192`, again only four bytes free, while the
smaller verifier state reduced the session future to 2,776 bytes. The final
SRAM-neutral partition therefore reserves 3,072 bytes at the end of the
existing Wi-Fi lifecycle-future region for that session. Moving the 1,076-byte
HTTP decoder and 256-byte plaintext staging into TLS-exclusive audio scratch
shrinks the lifecycle future from 21,536 to 20,248 bytes; it fits its remaining
20,496-byte region. The same audio scratch holds a 1,792-byte RX record, then
its unused tail continues directly into the 8,192-byte PCM samples. Scratch
guards/counters were reordered ahead of the raw bytes so they remain valid
while the TLS future is parked. This yields a 10,184-byte crypto-stack region
without changing `.bss` or the CPU0 application-stack boundary. TLS teardown
volatile-zeroes the session, audio scratch, TX DMA ring, and PCM loan, then
reconstructs scratch guards and audio ownership before restoration. The
1,536-byte headroom gate now admits a crypto peak up to 8,648 bytes. Hardware
must confirm application-visible completion, `tls_future=2776/3072`, and
`crypto=.../10184` without audio faults.

Hardware validation accepted that final partition. The packaged app reported
`fetch complete`, `{"ok":true}`, and HTTP 200 with 12 body bytes. Diagnostics
measured `tls_future=2776/3072` and `crypto=8468/10184`: the session has 296
bytes of arena margin and the crypto stack has 1,716 bytes of measured
headroom, so the 1,536-byte safety gate passes. PCM and SLDPCM playback also
remain clean after the exchange.

The same session exposed a reverse-residency bookkeeping defect on Wi-Fi
Disconnect. `WifiResidencyArena` retained the generation at which rich audio
lent its bytes to the Wi-Fi lifecycle, while TLS exclusion and stream restore
legitimately advanced the audio transition generation twice. Teardown compared
those different epochs and therefore rejected the unique returned arena as a
stale token after every successful TLS exchange, leaving games without rich
audio. The reverse boundary now consumes the arena as the linear proof that the
Wi-Fi runtime returned the bytes, requires the global owner to have reached
`WifiStreamAudio`, and rebases the arena onto the newly issued
`QuiescingWifi` token before the existing completion-time stale-token fence.
This adds no static SRAM. Host residency tests (including TLS followed by full
audio restoration), all embedded cross-build profiles, and the RP2040 SRAM gate
pass. Hardware validation then confirmed that a game can start and play audio
after the successful Fetch and Wi-Fi Disconnect sequence, proving that the
reverse boundary restores FullAudio in the product flow. This closes the final
open KOTO-0245 acceptance item.

### 2026-07-22: RP2350A concurrent HTTPS product path

Pico 2 W now enables the same pinned `app_fetch_https` transport without the
RP2040 audio-exclusion compromise. RP2350A's 520 KiB internal SRAM carries one
36,408-byte static TLS workspace: a 16,640-byte receive record, 2,048-byte
transmit record, decoder/plaintext staging, and a dedicated 16,384-byte crypto
stack with a 4 KiB minimum headroom gate. The complete region has a one-owner
atomic claim and is volatile-zeroized before release. The 2,776-byte TLS
session continues to use a 3,072-byte tail in the Wi-Fi arena.

Release type-size measurement records the Pico 2 W lifecycle future at
20,168/20,496 bytes. The HTTPS image uses `.data=83,040 B` and
`.bss=215,260 B` (`298,300 B` total), ending at `0x200490dc` and leaving
233,252 bytes before the RP2350A RAM limit. The non-HTTPS network image uses
262,360 bytes, so the feature's measured static-SRAM delta is 35,940 bytes.
All six embedded profile checks pass. Controlled-endpoint and concurrent-audio
behavior remain to be confirmed on Pico 2 W hardware. The validation image is
built with `tools\build-rp2350a.ps1 -AppFetchHttps`, which uses `picotool` with
the RP2350 Arm Secure family and absolute image-definition block; generic
`elf2uf2-rs` output is not valid for this target.

The generated validation artifact is
`koto_firmware-picocalc-pico2w-rp2350a-app-fetch-https.uf2`. Initial hardware
diagnostics returned `Unavailable` at zero milliseconds before DNS because the
AP association result became visible before embassy-net completed DHCP. Fetch
now waits cooperatively and cancellably for `Stack::is_config_up()` within the
bounded deadline instead of failing that normal transition window. The rebuilt
artifact is 3,015,168 bytes with SHA-256
`FF7AB7E3A229DDF85D471A83D7EC7464F30A52D27EC0CF0F93B5FC2B5BDBD155`.
`picotool info -a` confirms family `rp2350-arm-s`, target RP2350, ARM Secure
image type, and the image-definition metadata block at `0x10000114`.

The rebuilt image passed Pico 2 W hardware validation: the controlled endpoint
returned HTTP 200 and 12 body bytes in 389 ms (`tls_phase=23`,
`tls_io=1370/346`, `reads=18/119`). The session occupied 2,776/3,072 bytes and
the dedicated crypto stack peaked at 8,352/16,384 bytes, leaving 8,032 bytes.
PCM/SLDPCM and game audio remained normal, confirming concurrent HTTPS and
audio without the RP2040 exclusion boundary.
