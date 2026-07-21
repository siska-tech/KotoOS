//! KOTO-0249 end to end: the packaged MQTT telemetry sample connects to a
//! manifest-declared broker, subscribes to its one granted exact topic, and
//! drains complete messages into its own bounded buffers. The broker is a
//! deterministic sim fake (retained value first, then live samples); no host
//! network or wall clock is touched. The app only ever holds a broker/topic
//! index and message bytes — never a socket, TLS state, or credential.

use std::path::PathBuf;

use koto_core::{VmInputSnapshot, VmRunResult};
use koto_sim::BytecodeAppSession;

fn staged_sdcard() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("sdcard_mock")
}

#[test]
fn packaged_mqtt_sample_receives_retained_then_live_telemetry() {
    let mut session =
        BytecodeAppSession::launch(staged_sdcard(), "dev.koto.samples.mqtt-telemetry").unwrap();

    let mut saw_online = false;
    let mut saw_retained_value = false;
    let mut saw_live_value = false;

    for _ in 0..10 {
        assert_eq!(
            session.step_frame(VmInputSnapshot::empty()).unwrap(),
            VmRunResult::Yielded
        );
        let text = session.text();
        let has = |needle: &str| text.iter().any(|(_, _, t)| t == needle);

        if has("mqtt: online") {
            saw_online = true;
        }
        // The broker's retained value is delivered first, marked retained.
        if has("source: retained") && has("21.4") {
            saw_retained_value = true;
        }
        // Then live samples arrive, marked live.
        if has("source: live") && (has("21.7") || has("22.1")) {
            saw_live_value = true;
        }
        // The session must never fail in this scenario.
        assert!(!has("mqtt: offline"), "session must not go offline");
    }

    assert!(saw_online, "subscription never reached the online state");
    assert!(
        saw_retained_value,
        "retained telemetry value was never displayed"
    );
    assert!(saw_live_value, "live telemetry value was never displayed");

    let exit = VmInputSnapshot {
        intent_bits: koto_core::runtime::text_intent::EXIT,
        ..VmInputSnapshot::empty()
    };
    assert_eq!(session.step_frame(exit).unwrap(), VmRunResult::Exited(0));
}

#[test]
fn packaged_mqtt_sample_reports_offline_when_the_broker_is_unavailable() {
    let mut session =
        BytecodeAppSession::launch(staged_sdcard(), "dev.koto.samples.mqtt-telemetry").unwrap();
    // A network-disabled / unsupported build refuses the connect outright.
    session.script_mqtt_offline();

    let mut saw_offline = false;
    for _ in 0..6 {
        assert_eq!(
            session.step_frame(VmInputSnapshot::empty()).unwrap(),
            VmRunResult::Yielded
        );
        let text = session.text();
        if text.iter().any(|(_, _, t)| t == "mqtt: offline") {
            saw_offline = true;
        }
        // No telemetry value is ever displayed on an unavailable broker.
        assert!(
            !text
                .iter()
                .any(|(_, _, t)| t == "source: retained" || t == "source: live"),
            "an offline broker must deliver no telemetry"
        );
    }
    assert!(saw_offline, "offline broker was not reported");
}
