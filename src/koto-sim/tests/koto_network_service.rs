//! Fixture-driven fake NetworkService replay (KOTO-0242).
//!
//! Drives the real `koto_core::net::NetworkService` through deterministic driver
//! doubles and asserts its snapshots match the checked-in KOTO-0224 fixture. No
//! host network, wall clock, or RNG is on the path.

use koto_core::{
    KotoConfigWifiUi, Locale, NetworkError, OperationState, WifiIntent, WifiKey,
    WifiPageController, WifiPageState,
};
use koto_sim::fake_network::{parse_fixture, replay_all, replay_trace, FakeNetworkUiSession};
use serde_json::Value;
use std::path::PathBuf;

const V1_FIXTURE: &str =
    include_str!("../../../harness/fixtures/network_service/network_service_v1.json");

#[test]
fn v1_fixture_parses_and_validates() {
    parse_fixture(V1_FIXTURE).expect("fixture must parse and validate");
}

#[test]
fn v1_fixture_replays_every_scenario() {
    let names = replay_all(V1_FIXTURE).expect("all scenarios must replay");
    assert_eq!(
        names,
        vec![
            "capability-absent",
            "scan-connect-disconnect",
            "authentication-failure",
            "cancel-scan",
            "radio-loss-discards-late-completion",
            "forget-commit",
        ]
    );
}

#[test]
fn rejects_unknown_schema() {
    let json = V1_FIXTURE.replace(
        "koto.fake-network-service.v1",
        "koto.fake-network-service.v2",
    );
    let err = parse_fixture(&json).expect_err("unknown schema must be rejected");
    assert!(err.reason.contains("schema"), "{err}");
}

#[test]
fn rejects_unknown_field() {
    // Inject an unknown key into the top-level object.
    let json = V1_FIXTURE.replacen("\"tick_unit_ms\"", "\"bogus\": 1, \"tick_unit_ms\"", 1);
    assert!(
        parse_fixture(&json).is_err(),
        "unknown field must be rejected"
    );
}

#[test]
fn rejects_wrong_limit() {
    let json = V1_FIXTURE.replace("\"scan_results\": 16", "\"scan_results\": 8");
    let err = parse_fixture(&json).expect_err("wrong limit must be rejected");
    assert!(err.reason.contains("scan_results"), "{err}");
}

#[test]
fn rejects_unknown_operation() {
    let json = V1_FIXTURE.replacen("\"op\": \"scan\"", "\"op\": \"teleport\"", 1);
    assert!(parse_fixture(&json).is_err(), "unknown op must be rejected");
}

#[test]
fn rejects_unknown_enums_and_non_integer_ticks() {
    let json = V1_FIXTURE.replacen("\"state\": \"RadioEnabling\"", "\"state\": \"Warping\"", 1);
    assert!(
        parse_fixture(&json).is_err(),
        "unknown state must be rejected"
    );

    let json = V1_FIXTURE.replacen("\"security\": \"Open\"", "\"security\": \"Wep\"", 1);
    assert!(
        parse_fixture(&json).is_err(),
        "unknown security must be rejected"
    );

    let json = V1_FIXTURE.replacen("\"tick\": 0", "\"tick\": 0.5", 1);
    assert!(
        parse_fixture(&json).is_err(),
        "fractional ticks must be rejected"
    );
}

#[test]
fn rejects_invalid_network_identity_and_duplicate_ids() {
    let json = V1_FIXTURE.replacen("ff4b6f746f", "gg4b6f746f", 1);
    assert!(
        parse_fixture(&json).is_err(),
        "invalid SSID hex must be rejected"
    );

    let json = V1_FIXTURE.replacen("02:00:00:00:00:02", "02:00:00:00:00:01", 1);
    assert!(
        parse_fixture(&json).is_err(),
        "duplicate BSSID must be rejected"
    );

    let mut value: Value = serde_json::from_str(V1_FIXTURE).unwrap();
    value["networks"][1]["result_id"] = Value::from(1);
    assert!(
        parse_fixture(&serde_json::to_string(&value).unwrap()).is_err(),
        "duplicate result id must be rejected"
    );
}

#[test]
fn rejects_decreasing_ticks_duplicate_requests_and_stale_snapshots() {
    let mut value: Value = serde_json::from_str(V1_FIXTURE).unwrap();
    value["scenarios"][1]["actions"][2]["tick"] = Value::from(1);
    assert!(
        parse_fixture(&serde_json::to_string(&value).unwrap()).is_err(),
        "decreasing action ticks must be rejected"
    );

    let mut value: Value = serde_json::from_str(V1_FIXTURE).unwrap();
    value["scenarios"][1]["actions"][1]["request_id"] = Value::from(1);
    assert!(
        parse_fixture(&serde_json::to_string(&value).unwrap()).is_err(),
        "duplicate submitted request ids must be rejected"
    );

    let mut value: Value = serde_json::from_str(V1_FIXTURE).unwrap();
    value["scenarios"][1]["snapshots"][0]["request_id"] = Value::from(4);
    assert!(
        parse_fixture(&serde_json::to_string(&value).unwrap()).is_err(),
        "a snapshot cannot reference a future request"
    );
}

#[test]
fn every_checked_in_fake_network_fixture_strictly_replays() {
    let fixture_dir =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../harness/fixtures/network_service");
    let mut paths: Vec<_> = std::fs::read_dir(&fixture_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .collect();
    paths.sort();
    assert!(
        !paths.is_empty(),
        "at least one fake-network fixture is required"
    );
    for path in paths {
        let json = std::fs::read_to_string(&path).unwrap();
        replay_all(&json).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    }
}

#[test]
fn fake_snapshots_drive_kotoconfig_wifi_semantics() {
    let trace = replay_trace(V1_FIXTURE).expect("fixture trace must replay");
    for required in [
        OperationState::RadioUnavailable,
        OperationState::Scanning,
        OperationState::Results,
        OperationState::Connecting,
        OperationState::Connected,
        OperationState::Disconnecting,
        OperationState::Failed,
    ] {
        assert!(
            trace.iter().any(|item| item.snapshot.state == required),
            "trace must cover {required:?}"
        );
    }

    let results = trace
        .iter()
        .find(|item| {
            item.scenario == "scan-connect-disconnect"
                && item.snapshot.state == OperationState::Results
                && item.results.len() == 2
        })
        .expect("scan results observation");
    let mut page = WifiPageController::new();
    assert_eq!(
        page.update(&results.snapshot, results.results.iter().copied(), None),
        WifiIntent::None
    );
    assert_eq!(page.state(), WifiPageState::Results);
    assert_eq!(page.row_count(), 2);
    assert!(std::str::from_utf8(page.rows().nth(1).unwrap().ssid.as_bytes()).is_err());

    assert_eq!(
        page.update(
            &results.snapshot,
            results.results.iter().copied(),
            Some(WifiKey::Enter),
        ),
        WifiIntent::None
    );
    assert_eq!(page.state(), WifiPageState::CredentialEntry);
    for &byte in b"password1" {
        page.update(
            &results.snapshot,
            results.results.iter().copied(),
            Some(WifiKey::Char(byte)),
        );
    }
    assert_eq!(
        page.update(
            &results.snapshot,
            results.results.iter().copied(),
            Some(WifiKey::Enter),
        ),
        WifiIntent::Connect {
            result_id: 1,
            security: koto_core::Security::Wpa2PersonalAes,
        }
    );
    // The launch-path driver borrows the bytes for `NetworkService::connect`,
    // then must clear the page staging immediately after submission.
    page.clear_credential();
    assert!(page.credential_zeroized());

    let unavailable = trace
        .iter()
        .find(|item| item.scenario == "capability-absent")
        .unwrap();
    let mut page = WifiPageController::new();
    assert_eq!(
        page.update(
            &unavailable.snapshot,
            unavailable.results.iter().copied(),
            Some(WifiKey::Esc),
        ),
        WifiIntent::Exit
    );
    assert_eq!(page.state(), WifiPageState::RadioUnavailable);
}

#[test]
fn command_driven_fake_runs_native_wifi_page_to_connected_and_loss() {
    let mut backend = FakeNetworkUiSession::new(V1_FIXTURE).unwrap();
    let mut ui = KotoConfigWifiUi::new(Locale::EnUs, backend.snapshot());

    let intent = ui.update(backend.snapshot(), backend.results(), Some(WifiKey::Enter));
    assert_eq!(intent, WifiIntent::EnableRadio);
    assert!(backend.submit(intent, ui.credential()));
    backend.service_frame();
    ui.update(backend.snapshot(), backend.results(), None);
    backend.service_frame();
    let intent = ui.update(backend.snapshot(), backend.results(), None);
    assert_eq!(intent, WifiIntent::Scan);
    assert!(backend.submit(intent, &[]));

    backend.service_frame();
    ui.update(backend.snapshot(), backend.results(), None);
    backend.service_frame();
    ui.update(backend.snapshot(), backend.results(), None);
    assert_eq!(ui.state(), WifiPageState::Results);
    assert_eq!(ui.row_count(), 2);

    ui.update(backend.snapshot(), backend.results(), Some(WifiKey::Enter));
    for &byte in b"password1" {
        ui.update(
            backend.snapshot(),
            backend.results(),
            Some(WifiKey::Char(byte)),
        );
    }
    let intent = ui.update(backend.snapshot(), backend.results(), Some(WifiKey::Enter));
    assert!(backend.submit(intent, ui.credential()));
    ui.submission_complete(intent);
    assert!(ui.credential_zeroized());
    backend.service_frame();
    ui.update(backend.snapshot(), backend.results(), None);
    backend.service_frame();
    ui.update(backend.snapshot(), backend.results(), None);
    assert_eq!(ui.state(), WifiPageState::Connected);

    backend.lose_capability(NetworkError::RadioUnavailable);
    ui.update(backend.snapshot(), backend.results(), None);
    assert_eq!(ui.state(), WifiPageState::RadioUnavailable);
    assert!(ui.credential_zeroized());
    assert_eq!(
        ui.update(backend.snapshot(), backend.results(), Some(WifiKey::Esc)),
        WifiIntent::Exit
    );
}
