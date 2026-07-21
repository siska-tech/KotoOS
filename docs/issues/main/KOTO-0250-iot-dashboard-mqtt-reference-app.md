# KOTO-0250: IoT Dashboard MQTT reference app

- Status: todo
- Type: feature
- Priority: P2
- Requirements: FR-SDK-5, FR-SDK-9, FR-RT-4, FR-FS-2, FR-PKG-1, FR-PKG-3, NFR-MEM-2, NFR-PORT-3, NFR-REL-1, NFR-I18N-1, NFR-I18N-2, NFR-DEV-3, NFR-DEV-4
- Related: KOTO-0047, KOTO-0052, KOTO-0230, KOTO-0246, KOTO-0248, KOTO-0249

## Goal

Ship a bilingual IoT Dashboard application that demonstrates bounded live MQTT
telemetry using only public KotoSDK APIs. The dashboard visualizes recent sensor
state and connection freshness while remaining responsive and truthful during
message bursts, disconnects, reconnects, stale data, and credential denial.

## Acceptance Criteria

- [ ] Package the dashboard through the normal `.kpa` toolchain with explicit
  broker/topic permissions and an optional opaque credential grant. It uses no
  native socket, MQTT, filesystem, or secret escape hatch.
- [ ] Define a fixed-capacity dashboard configuration for a bounded number of
  named tiles, exact MQTT topics, value type/unit, scale/range, and stale
  timeout. Reject malformed, duplicate, or over-capacity configuration without
  partially applying it.
- [ ] Decode bounded UTF-8 scalar or JSON telemetry through KOTO-0246, validate
  finite numeric ranges, and retain only the latest value plus bounded display
  history per configured tile. Unknown fields and malformed messages cannot
  modify the last valid value.
- [ ] Display connecting, live, stale, reconnecting, denied, offline, queue
  overflow, and unavailable states distinctly. Broker arrival is described as
  live/best-effort data, not a hard real-time guarantee.
- [ ] Coalesce burst updates into bounded UI refresh work so key input, exit,
  redraw, and audio remain responsive. Leaving the app disconnects its MQTT
  session and no background subscription remains.
- [ ] Provide `ja-JP` and `en-US` strings with deterministic English fallback,
  unit formatting, clipping, and pseudolocale coverage. Logs and UI never reveal
  credentials or full secret-bearing broker details.
- [ ] Add deterministic KotoSim scenarios for scalar/JSON updates, retained
  messages, bursts and overflow, malformed/out-of-range values, stale timeout,
  disconnect/reconnect, denied/revoked credentials, locale change, and app exit
  without host MQTT or wall-clock dependencies.
- [ ] Validate the packaged dashboard on hardware against a controlled broker
  and record package size, peak memory, sustained accepted message rate, redraw
  cost, reconnect behavior, and audio/UI responsiveness.

## Non-goals

- Publishing commands or controlling actuators
- Arbitrary dashboard scripting, unbounded charts, broker administration, or
  discovery of undeclared topics/devices
- Background alerts or notifications while the app is inactive
