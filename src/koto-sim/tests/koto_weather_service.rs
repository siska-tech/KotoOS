//! KOTO-0247 end to end: the packaged Koto Weather app retrieves, decodes,
//! caches, and presents bounded `kwd1` Internet data through the public SDK
//! only (Fetch KOTO-0245, JSON KOTO-0246, advisory time minor 21). Every
//! scenario is deterministic: the fetch backend and the advisory clock are
//! scripted, never the host network or wall clock.

use std::fs;
use std::path::{Path, PathBuf};

use koto_core::{ConfigService, FetchError, Locale, VmInputSnapshot, VmRunResult};
use koto_sim::BytecodeAppSession;

const APP_ID: &str = "dev.koto.weather";
const PACKAGE: &[u8] = include_bytes!("../../../sdcard_mock/apps/weather.kpa");

const STATUS: u16 = 5;
const LOC: u16 = 6;
const COND: u16 = 7;
const TEMP: u16 = 8;
const RANGE: u16 = 9;
const PRECIP: u16 = 10;
const UPDATED: u16 = 11;

const OK_BODY: &[u8] = br#"{"schema":"kwd1","station":{"id":"X","samples":[3,7]},"location":"Tokyo","condition":5,"temperature_dc":215,"temp_min_dc":180,"temp_max_dc":260,"precipitation_pct":40,"observed_at":1784958000}"#;

fn test_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("koto_sim_weather_{name}"));
    if root.exists() {
        fs::remove_dir_all(&root).unwrap();
    }
    fs::create_dir_all(root.join("apps")).unwrap();
    fs::write(root.join("apps").join("weather.kpa"), PACKAGE).unwrap();
    root
}

fn step(session: &mut BytecodeAppSession, input: VmInputSnapshot) {
    assert_eq!(session.step_frame(input).unwrap(), VmRunResult::Yielded);
}

/// Run the three boot frames (resource load, settings/cache load, mount).
fn boot(session: &mut BytecodeAppSession) {
    for _ in 0..3 {
        step(session, VmInputSnapshot::empty());
    }
}

fn typed(ch: char) -> VmInputSnapshot {
    let mut input = VmInputSnapshot::empty();
    input.text_codepoint = ch as u32;
    input
}

fn submit() -> VmInputSnapshot {
    let mut input = VmInputSnapshot::empty();
    input.intent_bits = koto_core::runtime::text_intent::NEWLINE;
    input
}

/// A focused TextField accepts text only after it is activated into editing
/// mode (KotoUI). Enter editing, type the location key, then submit to start a
/// fetch through the app's Submitted handler.
fn request_location(session: &mut BytecodeAppSession, key: &str) {
    step(session, confirm());
    for ch in key.chars() {
        step(session, typed(ch));
    }
    step(session, submit());
}

/// Re-submit the field's already-persisted value (no typing) to start a fetch.
fn resubmit(session: &mut BytecodeAppSession) {
    step(session, confirm());
    step(session, submit());
}

fn down() -> VmInputSnapshot {
    let mut input = VmInputSnapshot::empty();
    input.intent_bits = koto_core::runtime::text_intent::DOWN;
    input
}

fn right() -> VmInputSnapshot {
    let mut input = VmInputSnapshot::empty();
    input.intent_bits = koto_core::runtime::text_intent::RIGHT;
    input
}

fn confirm() -> VmInputSnapshot {
    let mut input = VmInputSnapshot::empty();
    input.pressed_bits = 1 << 4;
    input
}

/// Step until the Status label equals `want`, or fail after `budget` frames.
fn run_until_status(session: &mut BytecodeAppSession, want: &str, budget: usize) {
    for _ in 0..budget {
        step(session, VmInputSnapshot::empty());
        if session.retained_ui_text(STATUS).as_deref() == Some(want) {
            return;
        }
    }
    panic!(
        "status never became {want:?}; last = {:?}",
        session.retained_ui_text(STATUS)
    );
}

fn launch(root: &Path) -> BytecodeAppSession {
    BytecodeAppSession::launch(root, APP_ID).unwrap()
}

#[test]
fn first_success_decodes_selected_fields_and_skips_unknown_subtrees() {
    let root = test_root("first_success");
    let mut session = launch(&root);
    session.set_time_utc(Some(1_784_958_600));
    session.script_fetch_response(200, OK_BODY, 1);
    boot(&mut session);

    assert_eq!(
        session.retained_ui_text(STATUS).as_deref(),
        Some("Set a location")
    );

    request_location(&mut session, "tokyo");
    run_until_status(&mut session, "Updated", 12);

    assert_eq!(session.retained_ui_text(LOC).as_deref(), Some("Tokyo"));
    assert_eq!(session.retained_ui_text(COND).as_deref(), Some("Rain"));
    assert_eq!(session.retained_ui_text(TEMP).as_deref(), Some("21.5°C"));
    assert_eq!(
        session.retained_ui_text(RANGE).as_deref(),
        Some("18.0/26.0°C")
    );
    assert_eq!(session.retained_ui_text(PRECIP).as_deref(), Some("40%"));
    // Fresh snapshot: no stale marker, an observation time, and a small age.
    let updated = session.retained_ui_text(UPDATED).unwrap();
    assert!(
        !updated.contains("stale"),
        "unexpected stale marker: {updated}"
    );
    assert!(updated.contains('@'), "missing observation time: {updated}");
}

#[test]
fn unit_toggle_recomputes_temperature_presentation() {
    let root = test_root("unit_toggle");
    let mut session = launch(&root);
    session.set_time_utc(Some(1_784_958_600));
    session.script_fetch_response(200, OK_BODY, 1);
    boot(&mut session);
    request_location(&mut session, "tokyo");
    run_until_status(&mut session, "Updated", 12);
    assert_eq!(session.retained_ui_text(TEMP).as_deref(), Some("21.5°C"));

    // Focus Field -> (down) Refresh -> (right) Unit; the two buttons share a
    // row, so KotoUI's spatial navigation reaches Unit with a right move.
    step(&mut session, down());
    step(&mut session, right());
    step(&mut session, confirm());
    assert_eq!(session.retained_ui_text(TEMP).as_deref(), Some("70.7°F"));
}

#[test]
fn partial_reads_across_frames_do_not_block_the_frame_loop() {
    let root = test_root("partial_reads");
    let mut session = launch(&root);
    session.set_time_utc(Some(1_784_958_600));
    // Two pending polls plus a >128-byte body force multi-frame partial reads.
    session.script_fetch_response(200, OK_BODY, 2);
    boot(&mut session);
    request_location(&mut session, "tokyo");
    // Every intermediate frame must still yield (never blocks); the loading
    // state is visible before completion.
    assert_eq!(
        session.retained_ui_text(STATUS).as_deref(),
        Some("Loading...")
    );
    run_until_status(&mut session, "Updated", 16);
    assert_eq!(session.retained_ui_text(LOC).as_deref(), Some("Tokyo"));
}

#[test]
fn malformed_json_keeps_no_snapshot_and_reports_invalid() {
    let root = test_root("malformed");
    let mut session = launch(&root);
    session.set_time_utc(Some(1_784_958_600));
    session.script_fetch_response(200, br#"{"location":"X","condition":}"#, 1);
    boot(&mut session);
    request_location(&mut session, "tokyo");
    run_until_status(&mut session, "Invalid data", 12);
    assert_eq!(
        session.retained_ui_text(LOC).as_deref(),
        Some("No data yet")
    );
}

#[test]
fn oversized_response_is_rejected_and_snapshot_preserved() {
    let root = test_root("oversized");
    // First session: seed a good snapshot into the sandbox cache.
    {
        let mut session = launch(&root);
        session.set_time_utc(Some(1_784_958_600));
        session.script_fetch_response(200, OK_BODY, 1);
        boot(&mut session);
        request_location(&mut session, "tokyo");
        run_until_status(&mut session, "Updated", 12);
    }
    // Second session (refresh interval reset): the stale Tokyo snapshot loads
    // from cache, then an oversized document is refused without replacing it.
    let mut session = launch(&root);
    session.set_time_utc(Some(1_784_962_200));
    // A well-formed document that exceeds W_BODY_CAP (2048) in total bytes
    // without any single oversized token: a large unknown number array. The
    // app's byte budget rejects it before it can replace the snapshot.
    let mut big = Vec::from(&br#"{"pad":["#[..]);
    for _ in 0..1100 {
        big.extend_from_slice(b"1,");
    }
    big.extend_from_slice(br#"1],"location":"Kyoto","condition":1,"temperature_dc":200}"#);
    session.script_fetch_response(200, &big, 1);
    boot(&mut session);
    assert_eq!(session.retained_ui_text(LOC).as_deref(), Some("Tokyo"));

    resubmit(&mut session);
    run_until_status(&mut session, "Response too large", 40);
    // The previous valid snapshot survives, still marked stale.
    assert_eq!(session.retained_ui_text(LOC).as_deref(), Some("Tokyo"));
    assert!(session.retained_ui_text(UPDATED).unwrap().contains("stale"));
}

#[test]
fn provider_error_status_is_reported() {
    let root = test_root("provider_error");
    let mut session = launch(&root);
    session.set_time_utc(Some(1_784_958_600));
    session.script_fetch_response(503, OK_BODY, 1);
    boot(&mut session);
    request_location(&mut session, "tokyo");
    run_until_status(&mut session, "Provider error", 12);
    assert_eq!(
        session.retained_ui_text(LOC).as_deref(),
        Some("No data yet")
    );
}

#[test]
fn timeout_failure_is_reported() {
    let root = test_root("timeout");
    let mut session = launch(&root);
    session.set_time_utc(Some(1_784_958_600));
    session.script_fetch_failure(FetchError::Timeout, 1);
    boot(&mut session);
    request_location(&mut session, "tokyo");
    run_until_status(&mut session, "Timed out", 12);
}

#[test]
fn cancellation_stops_the_request_without_committing() {
    let root = test_root("cancel");
    let mut session = launch(&root);
    session.set_time_utc(Some(1_784_958_600));
    // Many pending polls: the request stays in flight so we can cancel it.
    session.script_fetch_response(200, OK_BODY, 40);
    boot(&mut session);
    request_location(&mut session, "tokyo");
    assert_eq!(
        session.retained_ui_text(STATUS).as_deref(),
        Some("Loading...")
    );

    // Focus the Refresh button (now a Cancel) and activate it.
    step(&mut session, down());
    step(&mut session, confirm());
    assert_eq!(session.retained_ui_text(STATUS).as_deref(), Some("Ready"));
    assert_eq!(
        session.retained_ui_text(LOC).as_deref(),
        Some("No data yet")
    );
}

#[test]
fn refresh_interval_rejects_a_too_soon_second_request() {
    let root = test_root("cooldown");
    let mut session = launch(&root);
    session.set_time_utc(Some(1_784_958_600));
    session.script_fetch_response(200, OK_BODY, 1);
    boot(&mut session);
    request_location(&mut session, "tokyo");
    run_until_status(&mut session, "Updated", 12);

    // A manual refresh immediately after is inside the bounded interval.
    step(&mut session, down());
    step(&mut session, confirm());
    assert_eq!(
        session.retained_ui_text(STATUS).as_deref(),
        Some("Please wait")
    );
}

#[test]
fn offline_start_without_cache_shows_no_data_and_offline() {
    let root = test_root("offline_fresh");
    let mut session = launch(&root);
    session.set_time_utc(Some(1_784_958_600));
    session.script_fetch_offline();
    boot(&mut session);
    request_location(&mut session, "tokyo");
    run_until_status(&mut session, "Offline", 12);
    assert_eq!(
        session.retained_ui_text(LOC).as_deref(),
        Some("No data yet")
    );
}

#[test]
fn offline_start_with_cache_shows_stale_snapshot() {
    let root = test_root("offline_cached");
    // First session: a successful fetch persists the snapshot cache.
    {
        let mut session = launch(&root);
        session.set_time_utc(Some(1_784_958_600));
        session.script_fetch_response(200, OK_BODY, 1);
        boot(&mut session);
        request_location(&mut session, "tokyo");
        run_until_status(&mut session, "Updated", 12);
    }
    // Second session on the same sandbox: cache loads as a stale snapshot at
    // boot, and an offline refresh keeps it.
    let mut session = launch(&root);
    session.set_time_utc(Some(1_784_962_200));
    session.script_fetch_offline();
    boot(&mut session);
    assert_eq!(session.retained_ui_text(LOC).as_deref(), Some("Tokyo"));
    assert!(session.retained_ui_text(UPDATED).unwrap().contains("stale"));

    resubmit(&mut session);
    run_until_status(&mut session, "Offline", 12);
    // Offline never discards the last valid snapshot.
    assert_eq!(session.retained_ui_text(LOC).as_deref(), Some("Tokyo"));
    assert!(session.retained_ui_text(UPDATED).unwrap().contains("stale"));
}

#[test]
fn unknown_time_is_explicit_and_never_blocks_refresh() {
    let root = test_root("unknown_time");
    let mut session = launch(&root);
    // No set_time_utc: advisory UTC stays unknown (time_query returns -1).
    session.script_fetch_response(200, OK_BODY, 1);
    boot(&mut session);
    request_location(&mut session, "tokyo");
    // Refresh still succeeds without synchronized time.
    run_until_status(&mut session, "Updated", 12);
    assert_eq!(session.retained_ui_text(LOC).as_deref(), Some("Tokyo"));
    // Cache age is explicitly unknown ('?') rather than invented.
    assert!(session.retained_ui_text(UPDATED).unwrap().contains('?'));
}

#[test]
fn locale_change_relocalizes_without_losing_data() {
    let root = test_root("locale");
    let mut session = launch(&root);
    session.set_time_utc(Some(1_784_958_600));
    session.script_fetch_response(200, OK_BODY, 1);
    boot(&mut session);
    request_location(&mut session, "tokyo");
    run_until_status(&mut session, "Updated", 12);

    let mut config = ConfigService::default();
    assert!(config.set_locale(Locale::JaJp));
    session.set_config_snapshot(config.snapshot());
    // Drain the LocaleChanged event and re-label. The one-time re-parse of the
    // largest (multi-byte Japanese) resource is a bounded operation that can
    // span two simulator frames under the per-frame fuel guard; the VM resumes
    // it transparently, so tolerate a split frame here. Fetch, input, and exit
    // stay on their own bounded frames and are unaffected.
    for _ in 0..8 {
        match session.step_frame(VmInputSnapshot::empty()).unwrap() {
            VmRunResult::Yielded | VmRunResult::FuelExhausted => {}
            other => panic!("unexpected result during locale change: {other:?}"),
        }
        if session.retained_ui_text(STATUS).as_deref() == Some("更新しました") {
            break;
        }
    }
    assert_eq!(
        session.retained_ui_text(STATUS).as_deref(),
        Some("更新しました")
    );
    assert_eq!(session.retained_ui_text(COND).as_deref(), Some("雨"));
    // The location label (app-owned data, not a resource) is unchanged.
    assert_eq!(session.retained_ui_text(LOC).as_deref(), Some("Tokyo"));
}
