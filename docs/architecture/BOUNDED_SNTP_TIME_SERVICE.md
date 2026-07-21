# Bounded SNTP time service

- Status: implemented; hardware validation pending
- Issue: KOTO-0244

The portable `koto_core::time::TimeService` owns policy and packet validation.
It is `no_std`, allocation-free, monotonic-driven, and permits one 48-byte SNTP
request at a time. The Pico W network-generation arena owns DNS, UDP, packet
metadata, and the Embassy socket. KotoConfig, Shell, filesystem code, and apps
never receive these handles.

Synchronization begins only while DHCP reports config-up. A request times out
after 5 seconds, retry starts at 15 seconds and caps at 1 hour, and a successful
clock refreshes no more often than every 6 hours. Network-generation loss drops
the in-flight future and zeroes request identity. It never blocks boot, UI
painting, audio, storage, or offline launch.

Responses must be exactly 48 bytes and pass server mode, NTP v3/v4, leap alarm,
stratum, originate identity, nonzero transmit timestamp, NTP-era unfolding, and
the supported 2000-2099 calendar range. Resynchronization jumps over 24 hours
are rejected. The snapshot contains only validity, fixed source/failure enums,
generation, age, and UTC seconds; no packet or server diagnostic text is kept.

KotoConfig persists a fixed offset from `UTC-12:00` through `UTC+14:00` in
15-minute steps. Locale never chooses it. English and Japanese UI copy states
that daylight-saving changes are not automatic. Shell derives local civil time
from synchronized UTC plus this offset and damages only `SHELL_CLOCK_RECT` when
the displayed minute changes.

SNTP is unauthenticated advisory time. It must not authorize updates, validate
certificates, expire credentials, order security records, or serve as an audit
timestamp. Before first synchronization, Shell retains its unknown placeholder
and FAT uses the explicit `1980-01-01 00:00:00` safe fallback. SD access never
waits for time.

The dedicated UDP payload cost is 96 bytes (48-byte RX plus 48-byte TX), two
single-entry packet-metadata arrays, and the fixed portable `TimeService`. It
uses one OS-internal socket slot. Offline builds do not enable `embassy-net` and
link no SNTP task or socket state.

KotoConfig also persists one curated endpoint selection: `pool.ntp.org`,
`ntp.nict.jp`, `time.cloudflare.com`, or `time.google.com`. Changing it advances
the private time-client generation and triggers an immediate bounded resync.
Arbitrary application-supplied hostnames and raw DNS/UDP handles remain outside
the public contract.
