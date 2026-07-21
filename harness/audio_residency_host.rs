//! Host-executable KOTO-0227 residency state-machine regression tests.

#[path = "../src/koto-pico/src/firmware/audio_residency.rs"]
mod audio_residency;

use audio_residency::{AudioResidencyOwner, ResidencyState, TransitionError};

#[test]
fn one_hundred_round_trips_invalidate_stale_tokens() {
    let mut owner = AudioResidencyOwner::new();
    let initial = owner.token();

    for _ in 0..100 {
        let wifi_transition = owner.begin_wifi().unwrap();
        assert_eq!(owner.state(), ResidencyState::QuiescingAudio);
        assert!(!owner.rich_audio_available(initial));
        assert!(!owner.stream_audio_available(wifi_transition));
        assert_eq!(
            owner.mark_audio_offline(initial),
            Err(TransitionError::StaleToken)
        );
        owner.mark_audio_offline(wifi_transition).unwrap();
        assert_eq!(owner.state(), ResidencyState::Offline);
        owner.activate_wifi(wifi_transition).unwrap();
        assert_eq!(owner.state(), ResidencyState::WifiStreamAudio);
        assert!(owner.stream_audio_available(wifi_transition));
        assert!(!owner.rich_audio_available(wifi_transition));

        let audio_transition = owner.begin_full_audio().unwrap();
        assert_eq!(owner.state(), ResidencyState::QuiescingWifi);
        assert!(!owner.stream_audio_available(audio_transition));
        owner.mark_wifi_offline(audio_transition).unwrap();
        assert_eq!(owner.state(), ResidencyState::Offline);
        owner.activate_full_audio(audio_transition).unwrap();
        assert_eq!(owner.state(), ResidencyState::FullAudio);
        assert!(owner.rich_audio_available(audio_transition));
    }

    assert_eq!(owner.transition_failures(), 0);
    assert_eq!(owner.token().generation(), 201);
}

#[test]
fn transition_fault_lands_offline_and_invalidates_handle() {
    let mut owner = AudioResidencyOwner::new();
    let stale = owner.token();
    owner.begin_wifi().unwrap();
    let offline = owner.fail_to_offline();

    assert_eq!(owner.state(), ResidencyState::Offline);
    assert_eq!(owner.transition_failures(), 1);
    assert!(!owner.rich_audio_available(stale));
    assert!(!owner.stream_audio_available(offline));
    assert_eq!(
        owner.activate_wifi(stale),
        Err(TransitionError::StaleToken)
    );
}

#[test]
fn tls_exclusion_is_bounded_to_https_and_invalidates_stream_tokens() {
    let mut owner = AudioResidencyOwner::new();
    let wifi = owner.begin_wifi().unwrap();
    owner.mark_audio_offline(wifi).unwrap();
    owner.activate_wifi(wifi).unwrap();

    let tls = owner.begin_tls_exclusive().unwrap();
    assert_eq!(owner.state(), ResidencyState::QuiescingStreamForTls);
    assert!(!owner.stream_audio_available(wifi));
    assert_eq!(
        owner.activate_tls_exclusive(wifi),
        Err(TransitionError::StaleToken)
    );
    owner.activate_tls_exclusive(tls).unwrap();
    assert_eq!(owner.state(), ResidencyState::TlsExclusive);
    assert!(!owner.stream_audio_available(tls));
    assert_eq!(
        owner.begin_full_audio(),
        Err(TransitionError::InvalidState)
    );

    let restored = owner.begin_stream_restore_after_tls().unwrap();
    assert_eq!(owner.state(), ResidencyState::RestoringStreamAfterTls);
    assert_eq!(
        owner.activate_stream_after_tls(tls),
        Err(TransitionError::StaleToken)
    );
    owner.activate_stream_after_tls(restored).unwrap();
    assert_eq!(owner.state(), ResidencyState::WifiStreamAudio);
    assert!(owner.stream_audio_available(restored));

    // The Wi-Fi arena is still the same linear loan even though TLS advanced
    // the audio transition generation. Returning that arena must be allowed to
    // start the reverse transition back to rich audio.
    let full_audio = owner.begin_full_audio().unwrap();
    assert_eq!(owner.state(), ResidencyState::QuiescingWifi);
    owner.mark_wifi_offline(full_audio).unwrap();
    owner.activate_full_audio(full_audio).unwrap();
    assert_eq!(owner.state(), ResidencyState::FullAudio);
    assert!(owner.rich_audio_available(full_audio));
    assert_eq!(owner.transition_failures(), 0);
}
