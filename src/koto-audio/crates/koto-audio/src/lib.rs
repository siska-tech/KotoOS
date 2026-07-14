#![no_std]
#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Bounded logical audio runtime skeleton for KotoOS.
//!
//! v0 starts with PCM16 mono SFX clips, logical source IDs, event/counter
//! reporting, experimental static sequence sources, and an abstract backend
//! boundary. Real audio output, production PicoCalc hardware backends, full
//! tracker-style BGM, and streams are intentionally left for later task slices.
//! Experimental compressed codecs may be compiled behind opt-in feature flags,
//! but PCM16 remains the required path. Runtime code does not parse WAV,
//! downmix, or resample; those responsibilities belong to host-side tooling.

mod asset;
mod backend;
mod builtin_drums;
#[allow(missing_docs)]
#[cfg(not(feature = "sldpcm4-drums"))]
mod builtin_drums_generated;
#[allow(missing_docs)]
#[cfg(feature = "sldpcm4-drums")]
mod builtin_drums_sldpcm4_generated;
mod clip;
mod codec;
mod counters;
mod decoder;
mod event;
mod hostcall;
mod mixer;
mod owned_clip;
mod policy;
mod runtime_cue;
mod sequence;
mod service;
mod source;
mod streaming_clip;

pub use asset::{
    parse_clip_asset, AssetPlacement, ClipAssetError, ClipAssetHeader, CodecId,
    CLIP_ASSET_HEADER_SIZE, CLIP_ASSET_MAGIC, CLIP_ASSET_VERSION, CODEC_ID_PCM16, CODEC_ID_SLDPCM4,
    LOOP_COUNT_INFINITE,
};
#[cfg(any(feature = "picocalc-backend", target_arch = "arm"))]
pub use backend::picocalc::{
    PicoCalcBackend, PicoCalcBackendConfig, PicoCalcBackendCounters, PicoCalcBackendSnapshot,
    PicoCalcBufferDepthCandidate, PicoCalcSampleRateCandidate, PicoCalcUnderrunReportPolicy,
};
pub use backend::{
    sim::{CaptureFullPolicy, InspectionBackend, SimulatorBackend, DEFAULT_SIM_CAPTURE_BLOCKS},
    AudioBackend, BackendError, BackendReport, BackendResult, BackendState,
};
pub use clip::{ClipAsset, ClipLoop, LoopCount, PCM16_MONO_CHANNELS};
pub use counters::AudioCounterSnapshot;
pub use decoder::{DecodeResult, Decoder};
pub use event::{AudioEvent, AudioEventKind};
#[cfg(any(test, debug_assertions, feature = "debug-hostcalls"))]
pub use hostcall::DebugHostcallAdapter;
pub use hostcall::{
    AudioFocus, BackendPolicy, DebugBackendStateDump, DebugMixerLoadDump, DebugUnderrunCounterDump,
    HostcallScope, NormalHostcallAdapter, SystemHostcallAdapter,
};
pub use mixer::{MixerBlock, MixerVolume, DEFAULT_MIXER_BLOCK_FRAMES};
pub use owned_clip::OwnedClipPlayer;
pub use policy::{AudioLimits, AudioPolicy, DropPolicy};
pub use runtime_cue::{
    runtime_cue_max_encoded_len, RuntimeCue, RuntimeCueError, RuntimeCueTrack, RUNTIME_CUE_MAGIC,
    RUNTIME_CUE_VERSION,
};
pub use sequence::{
    validate_compact_sequence, CompactEvent, CompactInstrument, CompactSequence,
    CompactSequenceError, CompactTempo, CompactTrack, PolyphonicSequence, PolyphonicSequenceVoice,
    RuntimeCuePlayer, Sequence, SequenceDrum, SequenceEvent, SequenceInstrument,
    SequenceInstrumentKind, SequencePitch, SequenceTempo, SequenceWaveform,
    BUILTIN_INSTRUMENT_BASS_DRUM, BUILTIN_INSTRUMENT_CLAP, BUILTIN_INSTRUMENT_CLOSED_HI_HAT,
    BUILTIN_INSTRUMENT_CRASH_CYMBAL, BUILTIN_INSTRUMENT_OPEN_HI_HAT, BUILTIN_INSTRUMENT_SAW,
    BUILTIN_INSTRUMENT_SAW_FAST, BUILTIN_INSTRUMENT_SNARE_DRUM_1, BUILTIN_INSTRUMENT_SNARE_DRUM_2,
    BUILTIN_INSTRUMENT_SQUARE, BUILTIN_INSTRUMENT_SQUARE_FAST, BUILTIN_INSTRUMENT_SYNTH_TOM_HIGH,
    BUILTIN_INSTRUMENT_SYNTH_TOM_LOW, BUILTIN_INSTRUMENT_SYNTH_TOM_MID,
    BUILTIN_INSTRUMENT_TRIANGLE, BUILTIN_INSTRUMENT_TRIANGLE_FAST, BUILTIN_SEQUENCE_INSTRUMENTS,
    MAX_SEQUENCE_VOICES, SEQUENCE_REPEAT_INFINITE,
};
pub use service::{
    AudioService, DefaultAudioService, DEFAULT_SERVICE_ACTIVE_SOURCES, DEFAULT_SERVICE_EVENT_QUEUE,
    DEFAULT_SERVICE_SOURCE_QUEUE, DEFAULT_SERVICE_SOURCE_RECORDS,
};
pub use source::{SourceGeneration, SourceId, SourceOwner};
pub use streaming_clip::StreamingClipDecoder;

/// Result type used by public KotoAudio operations.
pub type AudioResult<T> = core::result::Result<T, AudioError>;

/// Public logical audio errors.
///
/// Backend-specific hardware details are deliberately not exposed here.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AudioError {
    /// A source request could not be admitted under the current policy.
    AdmissionRejected,
    /// A bounded source or event queue is full.
    QueueFull,
    /// A runtime-ready asset failed validation.
    MalformedAsset,
    /// The requested codec or source type is not supported by this build.
    UnsupportedCodec,
    /// The requested hostcall operation is reserved for a later implementation.
    UnsupportedOperation,
    /// The abstract backend is unavailable or not in a usable state.
    BackendUnavailable,
    /// A logical source identifier no longer matches the current slot generation.
    StaleSourceId,
    /// A caller provided an out-of-range policy, limit, volume, or identifier.
    InvalidArgument,
}

impl From<BackendError> for AudioError {
    fn from(error: BackendError) -> Self {
        match error {
            BackendError::Unavailable | BackendError::NotRunning => Self::BackendUnavailable,
            BackendError::QueueFull => Self::QueueFull,
            BackendError::Underrun | BackendError::SubmitFailed => Self::BackendUnavailable,
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn optional_runtime_features_are_not_default_enabled() {
        let manifest = include_str!("../Cargo.toml");

        assert!(manifest.contains("experimental-sldpcm4 = []"));
        assert!(manifest.contains("picocalc-backend = []"));
        assert!(manifest.contains("sldpcm4-drums = []"));
        assert!(!manifest.contains("default = [\"experimental-sldpcm4\""));
        assert!(!manifest.contains("default = [\"picocalc-backend\""));
        assert!(!manifest.contains("default = [\"sldpcm4-drums\""));
    }

    #[test]
    fn crate_root_does_not_re_export_sldpcm4_decoder_state() {
        let lib = include_str!("lib.rs");

        for line in lib
            .lines()
            .filter(|line| line.trim_start().starts_with("pub use "))
        {
            assert!(!line.contains("Sldpcm4Decoder"));
            assert!(!line.contains("DecoderState"));
            assert!(!line.contains("Sldpcm4TableId"));
            assert!(!line.contains("Sldpcm4LoopState"));
        }
    }
}
