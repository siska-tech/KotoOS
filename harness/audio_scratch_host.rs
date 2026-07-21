//! Host-executable regression tests for the dependency-free KOTO-0226 scratch
//! module. `check_audio_scratch.py` compiles this file directly so the RP-only
//! `koto-pico` crate does not need to build for the host.

mod firmware {
    pub mod audio {}
}

#[path = "../src/koto-pico/src/firmware/audio_scratch.rs"]
mod audio_scratch;

#[test]
fn layout_exclusion_and_guard_reporting() {
    audio_scratch::reset_diagnostics();
    assert_eq!(audio_scratch::stats(), audio_scratch::AudioScratchStats::default());

    audio_scratch::try_with_stream(|encoded, _| {
        assert!(audio_scratch::try_with_stream(|_, _| ()).is_err());
        encoded[0] = 0x5a;
    })
    .unwrap();

    audio_scratch::try_with_stream(|encoded, decoded| {
        assert_eq!(encoded.len(), audio_scratch::STREAM_PCM16_BYTES);
        assert_eq!(audio_scratch::STREAM_SLD4_BYTES, 1_024);
        assert_eq!(decoded.len(), audio_scratch::STREAM_DECODE_FRAMES);
        assert_eq!(decoded.as_ptr() as usize % core::mem::align_of::<i16>(), 0);
        encoded[0] = 0xa5;
        decoded[0] = 1234;
    })
    .unwrap();

    audio_scratch::corrupt_trailing_guard_for_test();
    audio_scratch::try_with_stream(|_, _| ()).unwrap();
    audio_scratch::record_load_acquisition();
    audio_scratch::record_pcm16_stream_start();
    audio_scratch::record_sld4_stream_start();
    audio_scratch::record_cold_cue_load();
    audio_scratch::record_cue_cache_hit();

    assert_eq!(
        audio_scratch::stats(),
        audio_scratch::AudioScratchStats {
            load_acquisitions: 1,
            stream_acquisitions: 3,
            rejected_acquisitions: 1,
            corruption_failures: 1,
        }
    );
    assert_eq!(
        audio_scratch::regression_stages(),
        audio_scratch::AudioRegressionStageStats {
            pcm16_stream_starts: 1,
            sld4_stream_starts: 1,
            cold_cue_loads: 1,
            cue_cache_hits: 1,
        }
    );
}
