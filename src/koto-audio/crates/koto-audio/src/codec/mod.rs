//! Codec-specific decoder implementations.

pub mod pcm16;
#[cfg(feature = "experimental-sldpcm4")]
pub mod sldpcm4;

/// Fixed SLDPCM4 v0 delta table (`KotoAudioExperimentalStandardLikeV0`),
/// shared by the experimental clip decoder and the optional SLDPCM4 built-in
/// drum tables so both decode identically.
#[cfg(any(feature = "experimental-sldpcm4", feature = "sldpcm4-drums"))]
pub(crate) const SLDPCM4_DELTAS_V0: [i16; 16] = [
    -32768, -16384, -8192, -4096, -2048, -1024, -512, -256, 0, 256, 512, 1024, 2048, 4096, 8192,
    16384,
];
