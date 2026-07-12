//! Fixed built-in sequence drum sample data.
//!
//! Runtime drums decode to signed PCM16 mono at the active mixer sample rate.
//! The current policy default is 16000 Hz, and the tables are authored for
//! that rate. Drum playback ignores note pitch, starts at sample index zero
//! for each note, and yields silence after the table ends.
//!
//! Two storage representations exist behind a feature switch:
//!
//! * default: raw PCM16 static slices ([`builtin_drums_generated`]);
//! * `sldpcm4-drums`: SLDPCM4 nibble payloads (~4x smaller, lossy) decoded
//!   incrementally by the drum voice ([`builtin_drums_sldpcm4_generated`]).
//!   Regenerate with `koto-audio-drum-sldpcm4-table` from the PCM16 module.
//!
//! The generated arrays are not MusLib, P/ECE, or other third-party sample
//! data; replace them only with data whose license has been verified.
//!
//! [`builtin_drums_generated`]: crate::builtin_drums_generated
//! [`builtin_drums_sldpcm4_generated`]: crate::builtin_drums_sldpcm4_generated

#[cfg(not(feature = "sldpcm4-drums"))]
use crate::builtin_drums_generated;
#[cfg(feature = "sldpcm4-drums")]
use crate::builtin_drums_sldpcm4_generated as sldpcm4_generated;
use crate::sequence::SequenceDrum;

/// Built-in drum sample rate used by the current placeholder tables.
pub(crate) const BUILTIN_DRUM_SAMPLE_RATE_HZ: u32 = 16_000;

/// Runtime representation for fixed built-in drum samples.
#[cfg(not(feature = "sldpcm4-drums"))]
pub(crate) type BuiltinDrumSample = &'static [i16];

#[cfg(not(feature = "sldpcm4-drums"))]
pub(crate) const fn samples(drum: SequenceDrum) -> BuiltinDrumSample {
    match drum {
        SequenceDrum::BassDrum => builtin_drums_generated::BUILTIN_DRUM_BASS_DRUM,
        SequenceDrum::SnareDrum2 => builtin_drums_generated::BUILTIN_DRUM_SNARE_DRUM_1,
        SequenceDrum::SnareDrum1 => builtin_drums_generated::BUILTIN_DRUM_SNARE_DRUM_1,
        SequenceDrum::OpenHiHat => builtin_drums_generated::BUILTIN_DRUM_OPEN_HI_HAT,
        SequenceDrum::ClosedHiHat => builtin_drums_generated::BUILTIN_DRUM_CLOSED_HI_HAT,
        SequenceDrum::CrashCymbal => builtin_drums_generated::BUILTIN_DRUM_CRASH_CYMBAL,
        SequenceDrum::SynthTomHigh => builtin_drums_generated::BUILTIN_DRUM_SYNTH_TOM_HIGH,
        SequenceDrum::SynthTomMid => builtin_drums_generated::BUILTIN_DRUM_SYNTH_TOM_MID,
        SequenceDrum::SynthTomLow => builtin_drums_generated::BUILTIN_DRUM_SYNTH_TOM_LOW,
        SequenceDrum::Clap => builtin_drums_generated::BUILTIN_DRUM_CLAP,
    }
}

/// One SLDPCM4-encoded built-in drum: nibble payload (high nibble first) plus
/// the decoded sample count.
#[cfg(feature = "sldpcm4-drums")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BuiltinDrumSldpcm4 {
    /// SLDPCM4 nibble payload, two samples per byte.
    pub payload: &'static [u8],
    /// Decoded PCM16 sample count (an odd count ignores the padding nibble).
    pub sample_count: u32,
}

#[cfg(feature = "sldpcm4-drums")]
pub(crate) const fn sldpcm4_samples(drum: SequenceDrum) -> BuiltinDrumSldpcm4 {
    match drum {
        SequenceDrum::BassDrum => BuiltinDrumSldpcm4 {
            payload: sldpcm4_generated::BUILTIN_DRUM_BASS_DRUM_SLDPCM4,
            sample_count: sldpcm4_generated::BUILTIN_DRUM_BASS_DRUM_SAMPLE_COUNT,
        },
        SequenceDrum::SnareDrum2 | SequenceDrum::SnareDrum1 => BuiltinDrumSldpcm4 {
            payload: sldpcm4_generated::BUILTIN_DRUM_SNARE_DRUM_1_SLDPCM4,
            sample_count: sldpcm4_generated::BUILTIN_DRUM_SNARE_DRUM_1_SAMPLE_COUNT,
        },
        SequenceDrum::OpenHiHat => BuiltinDrumSldpcm4 {
            payload: sldpcm4_generated::BUILTIN_DRUM_OPEN_HI_HAT_SLDPCM4,
            sample_count: sldpcm4_generated::BUILTIN_DRUM_OPEN_HI_HAT_SAMPLE_COUNT,
        },
        SequenceDrum::ClosedHiHat => BuiltinDrumSldpcm4 {
            payload: sldpcm4_generated::BUILTIN_DRUM_CLOSED_HI_HAT_SLDPCM4,
            sample_count: sldpcm4_generated::BUILTIN_DRUM_CLOSED_HI_HAT_SAMPLE_COUNT,
        },
        SequenceDrum::CrashCymbal => BuiltinDrumSldpcm4 {
            payload: sldpcm4_generated::BUILTIN_DRUM_CRASH_CYMBAL_SLDPCM4,
            sample_count: sldpcm4_generated::BUILTIN_DRUM_CRASH_CYMBAL_SAMPLE_COUNT,
        },
        SequenceDrum::SynthTomHigh => BuiltinDrumSldpcm4 {
            payload: sldpcm4_generated::BUILTIN_DRUM_SYNTH_TOM_HIGH_SLDPCM4,
            sample_count: sldpcm4_generated::BUILTIN_DRUM_SYNTH_TOM_HIGH_SAMPLE_COUNT,
        },
        SequenceDrum::SynthTomMid => BuiltinDrumSldpcm4 {
            payload: sldpcm4_generated::BUILTIN_DRUM_SYNTH_TOM_MID_SLDPCM4,
            sample_count: sldpcm4_generated::BUILTIN_DRUM_SYNTH_TOM_MID_SAMPLE_COUNT,
        },
        SequenceDrum::SynthTomLow => BuiltinDrumSldpcm4 {
            payload: sldpcm4_generated::BUILTIN_DRUM_SYNTH_TOM_LOW_SLDPCM4,
            sample_count: sldpcm4_generated::BUILTIN_DRUM_SYNTH_TOM_LOW_SAMPLE_COUNT,
        },
        SequenceDrum::Clap => BuiltinDrumSldpcm4 {
            payload: sldpcm4_generated::BUILTIN_DRUM_CLAP_SLDPCM4,
            sample_count: sldpcm4_generated::BUILTIN_DRUM_CLAP_SAMPLE_COUNT,
        },
    }
}
