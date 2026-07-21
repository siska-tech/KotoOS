# KOTO-0249: Bounded application MQTT subscribe service and SDK

- Status: in progress
- Type: feature
- Priority: P1
- Requirements: FR-SDK-5, FR-RT-4, FR-PKG-1, FR-PKG-3, NFR-MEM-2, NFR-PORT-1, NFR-PORT-3, NFR-PORT-6, NFR-REL-1, NFR-REL-3, NFR-REL-5, NFR-DEV-3, NFR-DEV-4, NFR-DEV-5
- Related: KOTO-0019, KOTO-0047, KOTO-0239, KOTO-0242, KOTO-0245, KOTO-0246, KOTO-0248, KOTO-0250

## Goal

Allow an active sandboxed application to receive bounded live telemetry from
an MQTT broker through an OS-owned, capability-gated service. Implement a small
documented MQTT 3.1.1 subscribe-only profile with deterministic memory,
reconnection, and delivery behavior rather than exposing a general socket or
background networking API.

## Acceptance Criteria

- [ ] Freeze a fixed-capacity MQTT 3.1.1 client profile and state machine above
  NetworkService, including broker/session lifecycle, keepalive, bounded
  reconnect backoff, packet-size limit, topic count/length, message queue depth,
  and one active broker session per foreground application.
- [ ] Version the `.kpa` permission schema with default-denied canonical broker
  origins and exact topic filters. Reject unsupported wildcard breadth,
  malformed UTF-8/topics, duplicate filters, oversized declarations, and
  subscriptions outside the manifest before connecting.
- [x] Add nonblocking Runtime ABI operations equivalent to connect, poll,
  subscribe, read-message, and disconnect. All handles are app-context-bound
  and generation-tagged; message bytes are copied into caller-owned bounded VM
  buffers and truncation is never reported as a complete message.
- [ ] Support subscribe-only QoS 0 initially and document retained-message,
  duplicate, ordering, session-cleanup, keepalive, broker disconnect, and queue
  overflow semantics. Unsupported MQTT packet types and QoS levels fail or are
  acknowledged exactly as required without unbounded buffering.
- [ ] Authenticate release traffic independently of advisory SNTP time, using
  the bounded TLS policy established for KOTO-0245. Authenticated brokers use
  opaque KOTO-0248 grants; secret material never crosses the app ABI. Plain
  MQTT is development-only, accepts no credentials, and is disabled in release
  profiles.
- [ ] Cancel and zeroize the session on app exit, capability loss, network
  generation change, permission revocation, or service teardown. An inactive
  app receives no background messages and unsupported/offline builds return a
  stable `Unavailable` result.
- [ ] Bound incoming packet parsing before allocation/copy, validate remaining
  length and UTF-8/topic framing, and define deterministic overflow policy. A
  malicious broker cannot starve UI/audio work, escape the VM sandbox, or grow
  memory with message rate or payload size.
- [x] Provide allocation-free KotoSDK wrappers plus deterministic KotoSim broker
  scenarios for connect, subscribe, partial packets, retained message, burst
  overflow, malformed/oversized packets, keepalive, disconnect/reconnect,
  credential denial/revocation, cancellation, and stale handles. Default tests
  use no host network or wall clock. (Partial/malformed/oversized packet framing
  is covered by the `MqttPacketDecoder` host tests in `koto_core::mqtt`; the sim
  broker delivers already-validated messages and covers the app-visible
  lifecycle. Keepalive and hardware timing stay device-scope.)
- [ ] Cross-build supported network-enabled and disabled RP2040/RP2350 profiles,
  record SRAM/flash/socket/TLS budgets, and validate against a controlled broker
  on hardware while audio and UI responsiveness canaries remain within their
  accepted floors.

## Non-goals

- Publishing or controlling devices, QoS 1/2, MQTT 5, WebSocket transport, or
  multiple simultaneous brokers
- Background subscriptions while an application is inactive
- Exposing raw MQTT packets, sockets, TLS state, or credential bytes to apps
- Hard real-time delivery guarantees; this service carries best-effort live
  telemetry

## Implementation Progress

### 2026-07-19: portable subscribe core

Implementation started with the portable, allocation-free, `no_std` MQTT
subscribe profile in `koto-core::mqtt`, mirroring the KOTO-0245 Fetch and
KOTO-0246 JSON "core-first" landings. This slice freezes the fixed-capacity
MQTT 3.1.1 QoS-0 profile and its seams for both KotoSim and firmware:

- **Profile constants** — one active broker session (`MAX_GLOBAL_MQTT_SESSIONS`),
  two declared broker origins, eight exact topic filters, 128 B topic bytes,
  192 B payload, 256 B total packet ceiling, eight-deep inbound queue, 60 s
  keepalive, 15 s connect deadline, and a 1–30 s reconnect-backoff window.
- **Permission model** — `MqttOrigin` (canonical `mqtt`/`mqtts` authority parse,
  default-port/wildcard/user-info/upper-case rejection; `mqtt://` is
  development-only and `release_allowed()` is `mqtts`-only), exact
  `TopicFilter` validation (rejects `+`/`#` wildcards, control chars, bad
  UTF-8, oversize), and duplicate-free `BrokerAllowlist` / `TopicFilterSet`.
- **Manifest gate** — `parse_manifest_mqtt_permission` reads the root
  `permissions.mqtt = { brokers, topics }` member of a schema-v2 manifest with
  a fixed-depth, allocation-free cursor that structurally skips unknown members
  (a nested `mqtt` key cannot be mistaken for the root declaration) and is
  default-denied when absent.
- **Bounded decoder** — `MqttPacketDecoder` is a streaming fixed-capacity parser
  for inbound CONNACK/SUBACK/PUBLISH/PINGRESP. It bounds the remaining-length
  varint before buffering any body, rejects reserved packet types, QoS 1/2,
  oversized packets/payloads, and malformed/wildcarded topics, and pushes only
  validated PUBLISH messages into `MqttMessageQueue` (drop-oldest ring with a
  saturating overflow counter — memory never grows with message rate).
- **Service boundary** — `AppMqttService<B: MqttBackend>` issues
  generation-tagged `MqttSessionId`s, enforces the broker/topic allowlist, one
  session per app plus the global cap, the connect deadline, stale/foreign
  session rejection, truncation-never-complete copy into caller buffers, and
  teardown that disconnects and zeroizes every session. `UnavailableMqttBackend`
  is the zero-sized offline backend returning a stable `Unavailable`.

Seventeen host tests (byte-by-byte decoder sweep across every chunk size, the
adversarial packet matrix, manifest accept/reject cases, queue overflow and
oversize-without-partial, and the full connect→subscribe→receive/ownership/
deadline/teardown service flow) pass with neither a host network nor a wall
clock. `koto-core` builds and its 367 host tests are green; `mqtt.rs` is
`rustfmt`- and `clippy`-clean (pre-existing `--all-targets` drift in
`fetch.rs`/`json.rs`/`shell.rs` is unrelated).

Remaining: `.koto` SDK subscribe wrappers plus a packaged sample; Runtime ABI
host calls (connect/poll/subscribe/read-message/disconnect) and bytecode ABI
version bump; KotoSim deterministic broker scenarios and live VM wiring on the
KOTO-0242 fake network; device transport (TLS via the KOTO-0245 bounded policy,
KOTO-0248 opaque grants for authenticated brokers), catalog-loader manifest
parity, release-ELF budget accounting, cross-builds, and hardware validation
against a controlled broker with the audio/UI canaries held.

### 2026-07-21: Runtime ABI, SDK, KotoSim, and packaged sample

The environment-testable follow-up slices landed, mirroring the KOTO-0246 JSON
and KOTO-0248 vault landings.

- **Runtime ABI (Host ABI minor 22 → 23)** — seven host calls `0x59`–`0x5F`
  (`mqtt_connect`/`mqtt_subscribe`/`mqtt_poll`/`mqtt_peek`/`mqtt_read`/
  `mqtt_disconnect`/`mqtt_dropped`), wired across the six `koto-vm` sync sites
  (const, `name`, `VmHost` default, dispatch, `known_host_call`, arity) plus the
  `kbc-asm` name→op table. `mqtt_read` consumes the oldest message and returns a
  single value; because it is not idempotent, its companion `(topic_len,
  payload_len)` are read through the separate idempotent `mqtt_peek`. Two disjoint
  caller buffers are marshalled with a new `heap_two_slices_mut` that rejects
  overlap so the two `&mut` never alias. Poll-state and read-result codes are the
  frozen `koto_core::mqtt::app_mqtt` constants; a new `AppMqttService::peek`
  exposes `front_lengths`.
- **SDK** — `koto-compiler` intrinsics `mqtt_connect`, `mqtt_subscribe`,
  `mqtt_poll`, `mqtt_peek_topic_len`/`mqtt_peek_payload_len` (Value2 aliases),
  `mqtt_read`, `mqtt_disconnect`, `mqtt_dropped`, plus `MQTT_*` state,
  `MQTT_READ_*`, and capacity SDK constants sourced from koto-core.
- **KotoSim** — `SimMqttBackend` deterministic fake broker (connect delay,
  retained-then-live delivery, burst drop-oldest overflow, clean disconnect,
  refused/offline connect) with no host network or wall clock; `AppMqttService`
  wired into `SimRuntimeHost` on the shared frame clock; manifest `permissions.mqtt`
  parsed by the catalog loader (v1 → default-denied) and applied at both launch
  paths. `build_apps.py` now emits schema v2 when `mqtt` is declared.
- **Packaged sample** — `dev.koto.samples.mqtt-telemetry` connects, subscribes to
  its one granted exact topic, and renders the retained value then live samples;
  two e2e sim tests (happy path + offline broker) drive the full VM→host→service
  path.
- **Docs / regen** — `RUNTIME_BYTECODE_ABI.md` and `KOTO_SDK.md` document the
  minor-23 calls and constants; all app KBC rebuilt at ABI minor 23, packages
  repacked (24 → 25), and the golden frame trace regenerated. Tests: vm 45,
  compiler 146, koto-core 412, sim fake-broker 7 + e2e 2, all green; fmt/clippy
  clean on the touched crates (pre-existing `koto_weather_service.rs` deref lints
  and the unrelated SKK gallery-dictionary failure predate this work).

Remaining (device / hardware only): firmware transport (TLS via KOTO-0245,
KOTO-0248 grants for authenticated brokers, a firmware `MqttBackend`), keepalive
timing, release-ELF/socket/TLS budgets, cross-builds, and validation against a
controlled broker with the audio/UI canaries held.
