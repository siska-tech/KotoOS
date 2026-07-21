use std::path::PathBuf;

use koto_core::{VmInputSnapshot, VmRunResult};
use koto_sim::BytecodeAppSession;

fn staged_sdcard() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("sdcard_mock")
}

#[test]
fn packaged_fetch_sample_yields_reads_and_completes_without_host_network() {
    let mut session =
        BytecodeAppSession::launch(staged_sdcard(), "dev.koto.samples.fetch-weather").unwrap();

    let mut saw_pending = false;
    let mut saw_complete = false;
    for _ in 0..8 {
        assert_eq!(
            session.step_frame(VmInputSnapshot::empty()).unwrap(),
            VmRunResult::Yielded
        );
        saw_pending |= session
            .text()
            .iter()
            .any(|(_, _, text)| text == "fetch pending");
        saw_complete |= session
            .text()
            .iter()
            .any(|(_, _, text)| text == "fetch complete");
        if saw_complete {
            assert!(session
                .text()
                .iter()
                .any(|(_, _, text)| text.contains("temperature_c")));
            assert!(session.text().iter().any(|(_, _, text)| text == "HTTP 200"));
            break;
        }
    }
    assert!(saw_pending);
    assert!(saw_complete);

    let exit = VmInputSnapshot {
        intent_bits: koto_core::runtime::text_intent::EXIT,
        ..VmInputSnapshot::empty()
    };
    assert_eq!(session.step_frame(exit).unwrap(), VmRunResult::Exited(0));
}
