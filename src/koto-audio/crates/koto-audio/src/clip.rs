use crate::{
    asset::{expected_payload_size, payload_size_matches_codec},
    AssetPlacement, AudioLimits, AudioPolicy, AudioResult, ClipAssetError, CodecId,
};

/// Number of channels accepted by the v0 mixer path.
pub const PCM16_MONO_CHANNELS: u8 = 1;

/// Loop repeat count for a clip loop.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoopCount {
    /// Repeat forever.
    Infinite,
    /// Repeat the loop a finite number of times.
    Finite(u32),
}

impl LoopCount {
    /// Returns whether this repeat count is valid.
    pub const fn is_valid(self) -> bool {
        match self {
            Self::Infinite => true,
            Self::Finite(count) => count > 0,
        }
    }
}

/// Runtime loop metadata for a clip.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClipLoop {
    /// Play the clip once.
    None,
    /// Loop the full clip range.
    Whole {
        /// Repeat count.
        count: LoopCount,
    },
    /// Loop a forward range with start inclusive and end exclusive.
    Forward {
        /// Inclusive loop start sample.
        start: u32,
        /// Exclusive loop end sample.
        end: u32,
        /// Repeat count.
        count: LoopCount,
    },
}

impl Default for ClipLoop {
    fn default() -> Self {
        Self::None
    }
}

/// Runtime-ready clip metadata and payload reference.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClipAsset<'a> {
    /// Clip codec identifier.
    pub codec: CodecId,
    /// Clip sample rate in hertz.
    pub sample_rate_hz: u32,
    /// Clip channel count.
    pub channels: u8,
    /// Mono frame/sample count.
    pub sample_count: u32,
    /// Encoded clip payload.
    pub payload: &'a [u8],
    /// Loop metadata.
    pub loop_metadata: ClipLoop,
    /// Lightweight placement or future expansion hint.
    pub placement: AssetPlacement,
}

impl<'a> ClipAsset<'a> {
    /// Creates a PCM16 mono clip with default no-loop metadata.
    pub const fn pcm16_mono(sample_rate_hz: u32, samples: u32, payload: &'a [u8]) -> Self {
        Self {
            codec: CodecId::Pcm16,
            sample_rate_hz,
            channels: PCM16_MONO_CHANNELS,
            sample_count: samples,
            payload,
            loop_metadata: ClipLoop::None,
            placement: AssetPlacement::Unspecified,
        }
    }

    /// Validates this clip against an audio policy.
    pub fn validate_for_policy(self, policy: AudioPolicy) -> AudioResult<Self> {
        self.validate(policy.limits)
    }

    /// Validates this clip against runtime limits.
    pub fn validate(self, limits: AudioLimits) -> AudioResult<Self> {
        self.validate_detailed(limits)
            .map_err(ClipAssetError::as_audio_error)?;
        Ok(self)
    }

    /// Validates this clip against runtime limits with a detailed asset reason.
    pub fn validate_detailed(self, limits: AudioLimits) -> Result<Self, ClipAssetError> {
        limits
            .validate()
            .map_err(|_| ClipAssetError::SampleRateMismatch)?;

        if !self.codec.is_supported_by_build() {
            return Err(ClipAssetError::UnsupportedCodec);
        }
        if self.channels != PCM16_MONO_CHANNELS {
            return Err(ClipAssetError::NonMono);
        }
        if self.sample_rate_hz != limits.sample_rate_hz {
            return Err(ClipAssetError::SampleRateMismatch);
        }

        let expected_payload_len = expected_payload_size(self.codec, self.sample_count)?;
        if !payload_size_matches_codec(self.codec, self.payload.len(), expected_payload_len) {
            return Err(ClipAssetError::PayloadSizeMismatch);
        }

        self.validate_loop()?;
        Ok(self)
    }

    fn validate_loop(self) -> Result<(), ClipAssetError> {
        match self.loop_metadata {
            ClipLoop::None => Ok(()),
            ClipLoop::Whole { count } => {
                if self.sample_count == 0 || !count.is_valid() {
                    Err(ClipAssetError::InvalidLoop)
                } else {
                    Ok(())
                }
            }
            ClipLoop::Forward { start, end, count } => {
                if start >= end || end > self.sample_count || !count.is_valid() {
                    Err(ClipAssetError::InvalidLoop)
                } else if self.codec == CodecId::Sldpcm4 && start != 0 {
                    Err(ClipAssetError::InvalidLoop)
                } else {
                    Ok(())
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AudioError;

    const LIMITS: AudioLimits = AudioLimits::v0_default();

    fn payload() -> &'static [u8] {
        &[1, 0, 255, 255]
    }

    #[test]
    fn valid_pcm16_mono_clip_validates() {
        let clip = ClipAsset::pcm16_mono(LIMITS.sample_rate_hz, 2, payload());

        assert_eq!(clip.validate(LIMITS), Ok(clip));
    }

    #[test]
    fn unsupported_codec_is_rejected() {
        let mut clip = ClipAsset::pcm16_mono(LIMITS.sample_rate_hz, 2, payload());
        clip.codec = CodecId::Unsupported(7);

        assert_eq!(clip.validate(LIMITS), Err(AudioError::UnsupportedCodec));
    }

    #[test]
    #[cfg(not(feature = "experimental-sldpcm4"))]
    fn sldpcm4_clip_is_rejected_without_feature() {
        let clip = ClipAsset {
            codec: CodecId::Sldpcm4,
            sample_count: 4,
            ..ClipAsset::pcm16_mono(LIMITS.sample_rate_hz, 2, payload())
        };

        assert_eq!(clip.validate(LIMITS), Err(AudioError::UnsupportedCodec));
    }

    #[test]
    #[cfg(feature = "experimental-sldpcm4")]
    fn sldpcm4_clip_validation_accepts_nibble_payload_with_feature() {
        let clip = ClipAsset {
            codec: CodecId::Sldpcm4,
            sample_count: 4,
            ..ClipAsset::pcm16_mono(LIMITS.sample_rate_hz, 2, &[0x01, 0x9f])
        };

        assert_eq!(clip.validate(LIMITS), Ok(clip));
    }

    #[test]
    #[cfg(feature = "experimental-sldpcm4")]
    fn sldpcm4_clip_validation_allows_extra_payload_with_feature() {
        let clip = ClipAsset {
            codec: CodecId::Sldpcm4,
            sample_count: 3,
            ..ClipAsset::pcm16_mono(LIMITS.sample_rate_hz, 2, &[0xf1, 0x90, 0xee])
        };

        assert_eq!(clip.validate(LIMITS), Ok(clip));
    }

    #[test]
    #[cfg(feature = "experimental-sldpcm4")]
    fn sldpcm4_clip_validation_rejects_short_payload_with_feature() {
        let clip = ClipAsset {
            codec: CodecId::Sldpcm4,
            sample_count: 3,
            ..ClipAsset::pcm16_mono(LIMITS.sample_rate_hz, 2, &[0xf1])
        };

        assert_eq!(clip.validate(LIMITS), Err(AudioError::MalformedAsset));
    }

    #[test]
    #[cfg(feature = "experimental-sldpcm4")]
    fn sldpcm4_non_zero_forward_loop_is_rejected_with_feature() {
        let mut clip = ClipAsset {
            codec: CodecId::Sldpcm4,
            sample_count: 4,
            ..ClipAsset::pcm16_mono(LIMITS.sample_rate_hz, 2, &[0xf1, 0x90])
        };
        clip.loop_metadata = ClipLoop::Forward {
            start: 1,
            end: 3,
            count: LoopCount::Finite(1),
        };

        assert_eq!(clip.validate(LIMITS), Err(AudioError::MalformedAsset));
    }

    #[test]
    fn sample_rate_mismatch_is_rejected() {
        let clip = ClipAsset::pcm16_mono(LIMITS.sample_rate_hz + 1, 2, payload());

        assert_eq!(clip.validate(LIMITS), Err(AudioError::MalformedAsset));
    }

    #[test]
    fn non_mono_is_rejected() {
        let mut clip = ClipAsset::pcm16_mono(LIMITS.sample_rate_hz, 2, payload());
        clip.channels = 2;

        assert_eq!(clip.validate(LIMITS), Err(AudioError::MalformedAsset));
    }

    #[test]
    fn invalid_payload_size_is_rejected() {
        let clip = ClipAsset::pcm16_mono(LIMITS.sample_rate_hz, 3, payload());

        assert_eq!(clip.validate(LIMITS), Err(AudioError::MalformedAsset));
    }

    #[test]
    fn invalid_loop_range_is_rejected() {
        let mut clip = ClipAsset::pcm16_mono(LIMITS.sample_rate_hz, 2, payload());
        clip.loop_metadata = ClipLoop::Forward {
            start: 2,
            end: 2,
            count: LoopCount::Infinite,
        };

        assert_eq!(clip.validate(LIMITS), Err(AudioError::MalformedAsset));
    }
}
