use std::path::PathBuf;

use koto_core::{VmInputSnapshot, VmRunResult};
use koto_sim::BytecodeAppSession;

fn staged_sdcard() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("sdcard_mock")
}

/// KOTO-0246 end-to-end: the packaged sample streams the deterministic fetch
/// response through the bounded `json_*` host calls, skips the unknown nested
/// `station` object and its array, and extracts the two named fields — all
/// without host network access or an in-memory document tree.
#[test]
fn packaged_json_sample_selects_named_fields_and_skips_unknown_subtrees() {
    let mut session =
        BytecodeAppSession::launch(staged_sdcard(), "dev.koto.samples.json-weather").unwrap();

    let mut saw_parsed = false;
    for _ in 0..8 {
        assert_eq!(
            session.step_frame(VmInputSnapshot::empty()).unwrap(),
            VmRunResult::Yielded
        );
        saw_parsed = session.text().iter().any(|(_, _, text)| text == "parsed");
        if saw_parsed {
            break;
        }
    }
    assert!(saw_parsed, "decoder never reached EndDocument");

    // Selected fields, skipped past `station`'s subtree and `wind`/`ok`.
    let text = session.text();
    assert!(text.iter().any(|(_, _, t)| t == "Tokyo"));
    assert!(text.iter().any(|(_, _, t)| t == "21"));
    // No error/missing/duplicate/wrong-type state is displayed.
    for bad in ["json error", "missing", "duplicate", "wrong type"] {
        assert!(
            !text.iter().any(|(_, _, t)| t == bad),
            "unexpected status {bad:?}"
        );
    }

    let exit = VmInputSnapshot {
        intent_bits: koto_core::runtime::text_intent::EXIT,
        ..VmInputSnapshot::empty()
    };
    assert_eq!(session.step_frame(exit).unwrap(), VmRunResult::Exited(0));
}
