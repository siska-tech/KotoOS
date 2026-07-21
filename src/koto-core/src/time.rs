//! Bounded, transport-independent SNTP time service (KOTO-0244).
//!
//! The OS-owned network adapter supplies monotonic milliseconds and moves the
//! fixed 48-byte packets. This module never owns a socket, DNS handle, wall
//! clock, allocator, or server-provided text.

use crate::shell::ShellClock;

pub const SNTP_PACKET_BYTES: usize = 48;
pub const SNTP_TIMEOUT_MS: u64 = 5_000;
pub const SNTP_REFRESH_MS: u64 = 6 * 60 * 60 * 1_000;
pub const SNTP_RETRY_MAX_MS: u64 = 60 * 60 * 1_000;
pub const SNTP_MAX_JUMP_SECONDS: i64 = 24 * 60 * 60;
/// Selector codes for the app-facing `time_query` host call (`0x56`, Host ABI
/// minor 21, KOTO-0247). The exposed clock stays the KOTO-0244 advisory SNTP
/// snapshot: unauthenticated display/cache-age data, never an authorization
/// or trust signal. UTC is `-1` until the first valid synchronization, and
/// after 2038-01-19 (beyond `i32`), so apps must treat unknown time as a
/// normal presentation state.
pub mod app_time_query {
    /// Synchronized UTC seconds since the Unix epoch, or `-1` while unknown.
    pub const UTC_SECONDS: i32 = 0;
    /// The user-configured KotoConfig UTC offset in minutes (KOTO-0244).
    pub const OFFSET_MINUTES: i32 = 1;
    /// Monotonic milliseconds masked to [`MONOTONIC_MASK`]: always
    /// non-negative, wrapping about every 12.4 days. Compare instants with
    /// `(now - then) & MONOTONIC_MASK`, never with raw subtraction.
    pub const MONOTONIC_MS: i32 = 2;
    /// The 30-bit wrap mask applied to [`MONOTONIC_MS`] values.
    pub const MONOTONIC_MASK: i32 = 0x3FFF_FFFF;
}

const NTP_UNIX_EPOCH_SECONDS: u64 = 2_208_988_800;
const MIN_UNIX_SECONDS: i64 = 946_684_800; // 2000-01-01
const MAX_UNIX_SECONDS: i64 = 4_102_444_799; // 2099-12-31 23:59:59

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimeSourceKind {
    None,
    SntpUnauthenticated,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimeFailure {
    None,
    CapabilityLost,
    Dns,
    Timeout,
    StaleResponse,
    ResponseLength,
    ServerMode,
    Version,
    LeapAlarm,
    Stratum,
    RequestMismatch,
    MissingTransmitTime,
    CalendarRange,
    ImplausibleJump,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimeServiceAction {
    None,
    Send([u8; SNTP_PACKET_BYTES]),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimeSnapshot {
    pub valid: bool,
    pub source: TimeSourceKind,
    pub generation: u32,
    pub age_ms: u64,
    pub utc_seconds: i64,
    pub failure: TimeFailure,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CalendarTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Phase {
    Offline,
    Ready,
    InFlight,
    Waiting,
}

/// A validated fixed UTC offset. DST transitions are deliberately not inferred.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct UtcOffset {
    minutes: i16,
}

impl UtcOffset {
    pub const MINUTES_MIN: i16 = -12 * 60;
    pub const MINUTES_MAX: i16 = 14 * 60;
    /// Current civil UTC offsets are representable in 15-minute increments.
    pub const STEP_MINUTES: i16 = 15;

    pub const fn from_minutes(minutes: i16) -> Option<Self> {
        if minutes >= Self::MINUTES_MIN
            && minutes <= Self::MINUTES_MAX
            && minutes % Self::STEP_MINUTES == 0
        {
            Some(Self { minutes })
        } else {
            None
        }
    }

    pub const fn minutes(self) -> i16 {
        self.minutes
    }
}

/// One-request-at-a-time SNTP policy and monotonic-backed UTC publication.
#[derive(Clone, Copy, Debug)]
pub struct TimeService {
    phase: Phase,
    network_generation: u32,
    request_identity: u64,
    deadline_ms: u64,
    next_attempt_ms: u64,
    retry_count: u8,
    sync_utc_seconds: i64,
    sync_mono_ms: u64,
    generation: u32,
    failure: TimeFailure,
}

impl Default for TimeService {
    fn default() -> Self {
        Self::new()
    }
}

impl TimeService {
    pub const fn new() -> Self {
        Self {
            phase: Phase::Offline,
            network_generation: 0,
            request_identity: 0,
            deadline_ms: 0,
            next_attempt_ms: 0,
            retry_count: 0,
            sync_utc_seconds: 0,
            sync_mono_ms: 0,
            generation: 0,
            failure: TimeFailure::None,
        }
    }

    /// Publishes DHCP/config capability. Generation loss cancels and zeroizes
    /// volatile request identity; previously synchronized advisory time remains
    /// available and monotonic-backed until explicitly invalidated.
    pub fn set_network(&mut self, config_up: bool, network_generation: u32, now_ms: u64) {
        if !config_up || network_generation == 0 {
            if self.phase != Phase::Offline {
                self.failure = TimeFailure::CapabilityLost;
            }
            self.phase = Phase::Offline;
            self.network_generation = 0;
            self.request_identity = 0;
            self.deadline_ms = 0;
            return;
        }
        if self.network_generation != network_generation {
            self.request_identity = 0;
            self.retry_count = 0;
            self.next_attempt_ms = now_ms;
        }
        self.network_generation = network_generation;
        if self.phase == Phase::Offline {
            self.phase = Phase::Ready;
        }
    }

    /// Advances timeout/backoff and returns at most one fixed request packet.
    pub fn poll(&mut self, now_ms: u64) -> TimeServiceAction {
        if self.phase == Phase::InFlight && deadline_reached(now_ms, self.deadline_ms) {
            self.fail(TimeFailure::Timeout, now_ms);
        }
        if matches!(self.phase, Phase::Ready | Phase::Waiting)
            && deadline_reached(now_ms, self.next_attempt_ms)
        {
            let identity = 0x4b4f_544f_0000_0000u64
                ^ (u64::from(self.network_generation) << 24)
                ^ now_ms.rotate_left(13)
                ^ u64::from(self.generation);
            self.request_identity = if identity == 0 { 1 } else { identity };
            self.deadline_ms = now_ms.saturating_add(SNTP_TIMEOUT_MS);
            self.phase = Phase::InFlight;
            let mut packet = [0u8; SNTP_PACKET_BYTES];
            packet[0] = (4 << 3) | 3; // LI=0, VN=4, client mode
            packet[40..48].copy_from_slice(&self.request_identity.to_be_bytes());
            return TimeServiceAction::Send(packet);
        }
        TimeServiceAction::None
    }

    pub fn report_dns_failure(&mut self, now_ms: u64) {
        if self.phase == Phase::InFlight {
            self.fail(TimeFailure::Dns, now_ms);
        }
    }

    /// Reports a fixed transport failure without exposing driver text or
    /// transport ownership to this model.
    pub fn report_transport_failure(&mut self, failure: TimeFailure, now_ms: u64) {
        if self.phase == Phase::InFlight {
            self.fail(failure, now_ms);
        }
    }

    /// Validates an SNTP response and publishes a fresh advisory generation.
    pub fn accept_response(&mut self, response: &[u8], now_ms: u64) -> Result<(), TimeFailure> {
        let result = self.validate_response(response, now_ms);
        match result {
            Ok(utc) => {
                self.sync_utc_seconds = utc;
                self.sync_mono_ms = now_ms;
                self.generation = next_generation(self.generation);
                self.failure = TimeFailure::None;
                self.retry_count = 0;
                self.request_identity = 0;
                self.next_attempt_ms = now_ms.saturating_add(SNTP_REFRESH_MS);
                self.phase = Phase::Waiting;
                Ok(())
            }
            Err(error) => {
                self.fail(error, now_ms);
                Err(error)
            }
        }
    }

    fn validate_response(&self, response: &[u8], now_ms: u64) -> Result<i64, TimeFailure> {
        if self.phase != Phase::InFlight {
            return Err(TimeFailure::StaleResponse);
        }
        if response.len() != SNTP_PACKET_BYTES {
            return Err(TimeFailure::ResponseLength);
        }
        let header = response[0];
        if header & 0x07 != 4 {
            return Err(TimeFailure::ServerMode);
        }
        if !matches!((header >> 3) & 0x07, 3 | 4) {
            return Err(TimeFailure::Version);
        }
        if header >> 6 == 3 {
            return Err(TimeFailure::LeapAlarm);
        }
        if !(1..=15).contains(&response[1]) {
            return Err(TimeFailure::Stratum);
        }
        if response[24..32] != self.request_identity.to_be_bytes() {
            return Err(TimeFailure::RequestMismatch);
        }
        let transmit = u64::from_be_bytes(response[40..48].try_into().unwrap_or([0; 8]));
        if transmit == 0 {
            return Err(TimeFailure::MissingTransmitTime);
        }
        let raw_ntp_seconds = transmit >> 32;
        // RFC 5905 era unfolding for the deliberately bounded 2000..2099
        // calendar: low values after the 2036 rollover belong to era 1.
        let ntp_seconds = raw_ntp_seconds
            + if raw_ntp_seconds < NTP_UNIX_EPOCH_SECONDS {
                1u64 << 32
            } else {
                0
            };
        let utc = ntp_seconds
            .checked_sub(NTP_UNIX_EPOCH_SECONDS)
            .and_then(|seconds| i64::try_from(seconds).ok())
            .ok_or(TimeFailure::CalendarRange)?;
        if !(MIN_UNIX_SECONDS..=MAX_UNIX_SECONDS).contains(&utc) {
            return Err(TimeFailure::CalendarRange);
        }
        if self.generation != 0 {
            let expected = self.utc_seconds(now_ms).ok_or(TimeFailure::CalendarRange)?;
            if utc.abs_diff(expected) > SNTP_MAX_JUMP_SECONDS as u64 {
                return Err(TimeFailure::ImplausibleJump);
            }
        }
        Ok(utc)
    }

    fn fail(&mut self, failure: TimeFailure, now_ms: u64) {
        self.failure = failure;
        self.request_identity = 0;
        self.retry_count = self.retry_count.saturating_add(1);
        let shift = self.retry_count.saturating_sub(1).min(6);
        let delay = (15_000u64 << shift).min(SNTP_RETRY_MAX_MS);
        self.next_attempt_ms = now_ms.saturating_add(delay);
        self.phase = if self.network_generation == 0 {
            Phase::Offline
        } else {
            Phase::Waiting
        };
    }

    pub fn utc_seconds(&self, now_ms: u64) -> Option<i64> {
        (self.generation != 0).then(|| {
            self.sync_utc_seconds
                .saturating_add(now_ms.saturating_sub(self.sync_mono_ms) as i64 / 1_000)
        })
    }

    pub fn snapshot(&self, now_ms: u64) -> TimeSnapshot {
        TimeSnapshot {
            valid: self.generation != 0,
            source: if self.generation == 0 {
                TimeSourceKind::None
            } else {
                TimeSourceKind::SntpUnauthenticated
            },
            generation: self.generation,
            age_ms: if self.generation == 0 {
                0
            } else {
                now_ms.saturating_sub(self.sync_mono_ms)
            },
            utc_seconds: self.utc_seconds(now_ms).unwrap_or(0),
            failure: self.failure,
        }
    }

    pub fn shell_clock(&self, now_ms: u64, offset: UtcOffset) -> Option<ShellClock> {
        unix_to_shell_clock(
            self.utc_seconds(now_ms)?
                .checked_add(i64::from(offset.minutes()) * 60)?,
        )
    }
}

pub fn unix_to_shell_clock(seconds: i64) -> Option<ShellClock> {
    let calendar = unix_to_calendar(seconds)?;
    Some(ShellClock {
        year: calendar.year,
        month: calendar.month,
        day: calendar.day,
        hour: calendar.hour,
        minute: calendar.minute,
    })
}

pub fn unix_to_calendar(seconds: i64) -> Option<CalendarTime> {
    if !(MIN_UNIX_SECONDS - 12 * 3600..=MAX_UNIX_SECONDS + 14 * 3600).contains(&seconds) {
        return None;
    }
    let days = seconds.div_euclid(86_400);
    let day_seconds = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days)?;
    Some(CalendarTime {
        year,
        month,
        day,
        hour: (day_seconds / 3_600) as u8,
        minute: ((day_seconds % 3_600) / 60) as u8,
        second: (day_seconds % 60) as u8,
    })
}

fn civil_from_days(days_since_unix: i64) -> Option<(u16, u8, u8)> {
    let z = days_since_unix + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    Some((u16::try_from(year).ok()?, month as u8, day as u8))
}

const fn next_generation(current: u32) -> u32 {
    let next = current.wrapping_add(1);
    if next == 0 {
        1
    } else {
        next
    }
}

const fn deadline_reached(now: u64, deadline: u64) -> bool {
    now >= deadline
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(service: &mut TimeService, now: u64) -> [u8; 48] {
        service.set_network(true, 7, now);
        match service.poll(now) {
            TimeServiceAction::Send(packet) => packet,
            _ => panic!("request"),
        }
    }

    fn response(request: &[u8; 48], unix: u64) -> [u8; 48] {
        let mut packet = [0u8; 48];
        packet[0] = (4 << 3) | 4;
        packet[1] = 2;
        packet[24..32].copy_from_slice(&request[40..48]);
        packet[40..48].copy_from_slice(&((unix + NTP_UNIX_EPOCH_SECONDS) << 32).to_be_bytes());
        packet
    }

    #[test]
    fn first_sync_advances_monotonically_and_rolls_minute() {
        let mut service = TimeService::new();
        let req = request(&mut service, 1_000);
        service
            .accept_response(&response(&req, 1_735_689_659), 1_100)
            .unwrap();
        assert_eq!(
            service
                .shell_clock(1_100, UtcOffset::default())
                .unwrap()
                .minute,
            0
        );
        assert_eq!(
            service
                .shell_clock(2_100, UtcOffset::default())
                .unwrap()
                .minute,
            1
        );
        assert_eq!(service.snapshot(2_100).age_ms, 1_000);
    }

    #[test]
    fn leap_year_and_offset_boundaries_convert() {
        assert_eq!(
            unix_to_shell_clock(951_827_696),
            Some(ShellClock {
                year: 2000,
                month: 2,
                day: 29,
                hour: 12,
                minute: 34
            })
        );
        assert!(UtcOffset::from_minutes(-720).is_some());
        assert!(UtcOffset::from_minutes(840).is_some());
        assert!(UtcOffset::from_minutes(841).is_none());
        assert!(UtcOffset::from_minutes(1).is_none());
    }

    #[test]
    fn timeout_backs_off_and_capability_loss_cancels() {
        let mut service = TimeService::new();
        request(&mut service, 0);
        assert_eq!(service.poll(SNTP_TIMEOUT_MS), TimeServiceAction::None);
        assert_eq!(
            service.snapshot(SNTP_TIMEOUT_MS).failure,
            TimeFailure::Timeout
        );
        assert_eq!(
            service.poll(SNTP_TIMEOUT_MS + 14_999),
            TimeServiceAction::None
        );
        assert!(matches!(
            service.poll(SNTP_TIMEOUT_MS + 15_000),
            TimeServiceAction::Send(_)
        ));
        service.set_network(false, 0, 20_001);
        assert_eq!(service.poll(99_999), TimeServiceAction::None);
    }

    #[test]
    fn network_or_server_generation_change_forces_immediate_refresh() {
        let mut service = TimeService::new();
        let req = request(&mut service, 0);
        service
            .accept_response(&response(&req, 1_735_689_600), 1)
            .unwrap();
        assert_eq!(service.poll(2), TimeServiceAction::None);
        service.set_network(true, 8, 2);
        assert!(matches!(service.poll(2), TimeServiceAction::Send(_)));
    }

    #[test]
    fn rejects_malformed_mismatched_and_replayed_replies() {
        let mut service = TimeService::new();
        let req = request(&mut service, 0);
        let good = response(&req, 1_735_689_600);
        let mut malformed = good;
        malformed[0] = (4 << 3) | 3;
        assert_eq!(
            service.accept_response(&malformed, 1),
            Err(TimeFailure::ServerMode)
        );

        let req = match service.poll(15_001) {
            TimeServiceAction::Send(p) => p,
            _ => panic!(),
        };
        let mut mismatch = response(&req, 1_735_689_600);
        mismatch[24] ^= 1;
        assert_eq!(
            service.accept_response(&mismatch, 15_002),
            Err(TimeFailure::RequestMismatch)
        );

        let req = match service.poll(45_002) {
            TimeServiceAction::Send(p) => p,
            _ => panic!(),
        };
        let good = response(&req, 1_735_689_600);
        service.accept_response(&good, 45_003).unwrap();
        assert_eq!(
            service.accept_response(&good, 45_004),
            Err(TimeFailure::StaleResponse)
        );
    }

    #[test]
    fn rejects_alarm_stratum_range_and_large_resync_jump() {
        let mut service = TimeService::new();
        let req = request(&mut service, 0);
        let mut alarm = response(&req, 1_735_689_600);
        alarm[0] |= 0xc0;
        assert_eq!(
            service.accept_response(&alarm, 1),
            Err(TimeFailure::LeapAlarm)
        );

        let req = match service.poll(15_001) {
            TimeServiceAction::Send(packet) => packet,
            _ => panic!(),
        };
        service
            .accept_response(&response(&req, 1_735_689_600), 15_002)
            .unwrap();
        let refresh = 15_002 + SNTP_REFRESH_MS;
        let req = match service.poll(refresh) {
            TimeServiceAction::Send(packet) => packet,
            _ => panic!(),
        };
        assert_eq!(
            service.accept_response(&response(&req, 1_735_689_600 + 3 * 86_400), refresh + 1),
            Err(TimeFailure::ImplausibleJump)
        );
    }

    #[test]
    fn unfolds_ntp_era_after_2036_within_supported_calendar() {
        let mut service = TimeService::new();
        let req = request(&mut service, 0);
        service
            .accept_response(&response(&req, 4_102_444_799), 1)
            .unwrap();
        let clock = service.shell_clock(1, UtcOffset::default()).unwrap();
        assert_eq!((clock.year, clock.month, clock.day), (2099, 12, 31));
    }
}
