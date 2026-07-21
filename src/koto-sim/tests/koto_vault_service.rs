//! KOTO-0248 end to end: the packaged credential-vault sample resolves an
//! opaque handle for a granted TLS origin and completes an authenticated GET
//! whose credential the OS injects — the app never sees a secret. It also shows
//! that an ungranted origin resolves to no handle (default-denied). The vault
//! and the fetch backend are deterministic sim fakes; no host credential store
//! or network is touched.

use std::path::PathBuf;

use koto_core::{VmInputSnapshot, VmRunResult};
use koto_sim::BytecodeAppSession;

fn staged_sdcard() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("sdcard_mock")
}

#[test]
fn packaged_vault_sample_uses_granted_credential_and_denies_ungranted() {
    let mut session =
        BytecodeAppSession::launch(staged_sdcard(), "dev.koto.samples.vault-fetch").unwrap();
    // Deterministic authenticated response, pending for two polls then complete.
    session.script_fetch_response(200, br#"{"ok":true}"#, 2);

    let mut saw_ok = false;
    for _ in 0..8 {
        assert_eq!(
            session.step_frame(VmInputSnapshot::empty()).unwrap(),
            VmRunResult::Yielded
        );
        let text = session.text();
        // The ungranted origin never yields a handle, on every frame.
        assert!(
            text.iter().any(|(_, _, t)| t == "other: no grant"),
            "ungranted origin must be denied"
        );
        assert!(
            !text.iter().any(|(_, _, t)| t == "other: LEAK"),
            "ungranted origin must never resolve a handle"
        );
        if text.iter().any(|(_, _, t)| t == "auth: ok") {
            saw_ok = true;
            break;
        }
        // The authenticated request must never fail or be denied in this scenario.
        for bad in ["auth: failed", "auth: no grant"] {
            assert!(
                !text.iter().any(|(_, _, t)| t == bad),
                "unexpected authenticated-fetch status {bad:?}"
            );
        }
    }
    assert!(saw_ok, "authenticated fetch never completed");

    // The fake token never appears in any drawn text.
    assert!(
        !session
            .text()
            .iter()
            .any(|(_, _, t)| t.contains("sim-fake-token")),
        "secret bytes must never reach the app surface"
    );

    let exit = VmInputSnapshot {
        intent_bits: koto_core::runtime::text_intent::EXIT,
        ..VmInputSnapshot::empty()
    };
    assert_eq!(session.step_frame(exit).unwrap(), VmRunResult::Exited(0));
}
