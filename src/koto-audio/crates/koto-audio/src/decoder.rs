use crate::{
    codec::pcm16::Pcm16Decoder,
    sequence::{PolyphonicSequenceDecoder, SequenceDecoder},
    AudioError, AudioLimits, AudioResult, ClipAsset, CodecId, PolyphonicSequence, Sequence,
};

#[cfg(feature = "experimental-sldpcm4")]
use crate::codec::sldpcm4::Sldpcm4Decoder;

/// Single-sample decoder result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecodeResult {
    /// A signed mono PCM16 sample was decoded.
    Sample(i16),
    /// The clip has reached its terminal end.
    End,
}

/// Codec-specific state hidden behind the runtime decoder boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DecoderState<'a> {
    /// PCM16 decoder state.
    Pcm16(Pcm16Decoder<'a>),
    /// Minimal monophonic sequence synth state.
    Sequence(SequenceDecoder<'a>),
    /// Experimental fixed-capacity polyphonic sequence synth state.
    PolyphonicSequence(PolyphonicSequenceDecoder<'a>),
    /// Experimental SLDPCM4 decoder state.
    #[cfg(feature = "experimental-sldpcm4")]
    Sldpcm4(Sldpcm4Decoder<'a>),
}

/// Mono signed PCM16 decoder interface.
pub trait Decoder {
    /// Decodes the next sample or reports terminal end.
    fn next_sample(&mut self) -> DecodeResult;

    /// Decodes up to `out.len()` samples and returns the number written.
    fn read_samples(&mut self, out: &mut [i16]) -> usize {
        let mut written = 0;
        for sample in out {
            match self.next_sample() {
                DecodeResult::Sample(decoded) => {
                    *sample = decoded;
                    written += 1;
                }
                DecodeResult::End => break,
            }
        }
        written
    }

    /// Returns whether the decoder has reached terminal end.
    fn is_ended(&self) -> bool;

    /// Returns how many finite loop iterations completed.
    fn completed_loops(&self) -> u32;
}

impl<'a> DecoderState<'a> {
    /// Creates a decoder for a validated runtime-ready clip.
    pub fn new(clip: ClipAsset<'a>, limits: AudioLimits) -> AudioResult<Self> {
        clip.validate(limits)?;
        match clip.codec {
            CodecId::Pcm16 => Ok(Self::Pcm16(Pcm16Decoder::new(clip))),
            #[cfg(feature = "experimental-sldpcm4")]
            CodecId::Sldpcm4 => Ok(Self::Sldpcm4(Sldpcm4Decoder::new(clip))),
            #[cfg(not(feature = "experimental-sldpcm4"))]
            CodecId::Sldpcm4 => Err(AudioError::UnsupportedCodec),
            CodecId::Unsupported(_) => Err(AudioError::UnsupportedCodec),
        }
    }

    /// Creates a decoder for a validated static sequence.
    pub fn new_sequence(sequence: Sequence<'a>, limits: AudioLimits) -> AudioResult<Self> {
        Ok(Self::Sequence(SequenceDecoder::new(sequence, limits)?))
    }

    /// Creates a decoder for a validated fixed-capacity polyphonic sequence.
    pub fn new_polyphonic_sequence(
        sequence: PolyphonicSequence<'a>,
        limits: AudioLimits,
    ) -> AudioResult<Self> {
        Ok(Self::PolyphonicSequence(PolyphonicSequenceDecoder::new(
            sequence, limits,
        )?))
    }
}

impl Decoder for DecoderState<'_> {
    #[cfg_attr(feature = "ram-hot-mix", link_section = ".data.koto_audio_mix")]
    fn next_sample(&mut self) -> DecodeResult {
        match self {
            Self::Pcm16(decoder) => decoder.next_sample(),
            Self::Sequence(decoder) => decoder.next_sample(),
            Self::PolyphonicSequence(decoder) => decoder.next_sample(),
            #[cfg(feature = "experimental-sldpcm4")]
            Self::Sldpcm4(decoder) => decoder.next_sample(),
        }
    }

    fn is_ended(&self) -> bool {
        match self {
            Self::Pcm16(decoder) => decoder.is_ended(),
            Self::Sequence(decoder) => decoder.is_ended(),
            Self::PolyphonicSequence(decoder) => decoder.is_ended(),
            #[cfg(feature = "experimental-sldpcm4")]
            Self::Sldpcm4(decoder) => decoder.is_ended(),
        }
    }

    fn completed_loops(&self) -> u32 {
        match self {
            Self::Pcm16(decoder) => decoder.completed_loops(),
            Self::Sequence(decoder) => decoder.completed_loops(),
            Self::PolyphonicSequence(decoder) => decoder.completed_loops(),
            #[cfg(feature = "experimental-sldpcm4")]
            Self::Sldpcm4(decoder) => decoder.completed_loops(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PCM16_MONO_CHANNELS;

    const LIMITS: AudioLimits = AudioLimits::v0_default();

    #[test]
    #[cfg(not(feature = "experimental-sldpcm4"))]
    fn sldpcm4_decoder_dispatch_rejects_without_feature() {
        let clip = ClipAsset {
            codec: CodecId::Sldpcm4,
            sample_rate_hz: LIMITS.sample_rate_hz,
            channels: PCM16_MONO_CHANNELS,
            sample_count: 2,
            payload: &[0x11],
            loop_metadata: crate::ClipLoop::None,
            placement: crate::AssetPlacement::Unspecified,
        };

        assert_eq!(
            DecoderState::new(clip, LIMITS),
            Err(AudioError::UnsupportedCodec)
        );
    }

    #[test]
    #[cfg(feature = "experimental-sldpcm4")]
    fn sldpcm4_decoder_dispatch_constructs_with_feature() {
        let clip = ClipAsset {
            codec: CodecId::Sldpcm4,
            sample_rate_hz: LIMITS.sample_rate_hz,
            channels: PCM16_MONO_CHANNELS,
            sample_count: 2,
            payload: &[0x11],
            loop_metadata: crate::ClipLoop::None,
            placement: crate::AssetPlacement::Unspecified,
        };

        let mut decoder = DecoderState::new(clip, LIMITS).unwrap();

        assert_eq!(decoder.next_sample(), DecodeResult::Sample(-16384));
        assert_eq!(decoder.next_sample(), DecodeResult::Sample(-32768));
    }
}
