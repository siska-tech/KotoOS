//! Deterministic KOTO-0244 scenarios. No host network or wall clock is used.

use koto_core::{
    ShellState, TimeFailure, TimeService, TimeServiceAction, UtcOffset, SHELL_CLOCK_RECT,
};

const NTP_UNIX_DELTA: u64 = 2_208_988_800;

fn request(service: &mut TimeService, now_ms: u64) -> [u8; 48] {
    service.set_network(true, 11, now_ms);
    match service.poll(now_ms) {
        TimeServiceAction::Send(packet) => packet,
        TimeServiceAction::None => panic!("expected bounded SNTP request"),
    }
}

fn response(request: &[u8; 48], unix_seconds: u64) -> [u8; 48] {
    let mut packet = [0u8; 48];
    packet[0] = (4 << 3) | 4;
    packet[1] = 2;
    packet[24..32].copy_from_slice(&request[40..48]);
    packet[40..48].copy_from_slice(&((unix_seconds + NTP_UNIX_DELTA) << 32).to_be_bytes());
    packet
}

#[test]
fn first_sync_drives_shell_by_minute_with_fixed_offset() {
    let mut time = TimeService::new();
    let request = request(&mut time, 100);
    time.accept_response(&response(&request, 1_752_892_800), 200)
        .unwrap();

    let offset = UtcOffset::from_minutes(9 * 60).unwrap();
    let mut shell = ShellState::empty();
    let first = time.shell_clock(200, offset).unwrap();
    assert!(shell.set_clock_if_minute_changed(first));
    assert!(!shell.set_clock_if_minute_changed(first));
    assert!(!shell.set_clock_if_minute_changed(time.shell_clock(30_200, offset).unwrap()));
    assert!(shell.set_clock_if_minute_changed(time.shell_clock(60_200, offset).unwrap()));
    assert_eq!(SHELL_CLOCK_RECT.y, 0);
    assert_eq!(SHELL_CLOCK_RECT.h, 20);
}

#[test]
fn replay_and_network_loss_are_harmless_to_published_time() {
    let mut time = TimeService::new();
    let request = request(&mut time, 0);
    let packet = response(&request, 1_752_892_800);
    time.accept_response(&packet, 10).unwrap();
    assert_eq!(
        time.accept_response(&packet, 11),
        Err(TimeFailure::StaleResponse)
    );
    time.set_network(false, 0, 12);
    assert!(time.snapshot(1_012).valid);
    assert_eq!(time.snapshot(1_012).utc_seconds, 1_752_892_801);
    assert_eq!(time.poll(99_999), TimeServiceAction::None);
}
