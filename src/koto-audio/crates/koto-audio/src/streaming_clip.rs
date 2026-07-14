use crate::{
    codec::SLDPCM4_DELTAS_V0, AudioLimits, ClipAssetError, ClipAssetHeader, ClipLoop, CodecId,
};

/// Incremental decoder for a one-shot KACL payload.
///
/// The caller owns storage and supplies arbitrary payload chunks. Decoder state
/// retains split PCM bytes and the second SLD4 nibble across chunk boundaries.
#[derive(Clone, Copy, Debug)]
pub struct StreamingClipDecoder {
    codec: CodecId,
    sample_count: u32,
    decoded_samples: u32,
    previous_sample: i16,
    pending_pcm_byte: Option<u8>,
    pending_sld4_nibble: Option<u8>,
}

impl StreamingClipDecoder {
    /// Validates a KACL header for bounded one-shot streaming.
    pub fn from_header(
        header: ClipAssetHeader,
        limits: AudioLimits,
    ) -> Result<Self, ClipAssetError> {
        if !header.codec.is_supported_by_build() {
            return Err(ClipAssetError::UnsupportedCodec);
        }
        if header.channels != 1 {
            return Err(ClipAssetError::NonMono);
        }
        if header.sample_rate_hz != limits.sample_rate_hz {
            return Err(ClipAssetError::SampleRateMismatch);
        }
        if !matches!(header.loop_metadata()?, ClipLoop::None) {
            return Err(ClipAssetError::InvalidLoop);
        }
        let expected = match header.codec {
            CodecId::Pcm16 => header.sample_count.checked_mul(2),
            CodecId::Sldpcm4 => header.sample_count.checked_add(1).map(|n| n / 2),
            CodecId::Unsupported(_) => None,
        }
        .ok_or(ClipAssetError::InvalidSampleCount)?;
        if header.payload_size != expected {
            return Err(ClipAssetError::InvalidSampleCount);
        }
        Ok(Self {
            codec: header.codec,
            sample_count: header.sample_count,
            decoded_samples: 0,
            previous_sample: 0,
            pending_pcm_byte: None,
            pending_sld4_nibble: None,
        })
    }

    /// Decodes as much of `input` as fits in `output`.
    /// Returns `(input_bytes_consumed, samples_written)`.
    pub fn decode_chunk(&mut self, input: &[u8], output: &mut [i16]) -> (usize, usize) {
        match self.codec {
            CodecId::Pcm16 => self.decode_pcm16(input, output),
            CodecId::Sldpcm4 => self.decode_sld4(input, output),
            CodecId::Unsupported(_) => (0, 0),
        }
    }

    /// Returns whether all declared samples were decoded.
    pub const fn is_finished(&self) -> bool {
        self.decoded_samples >= self.sample_count
    }

    /// Number of decoded samples so far.
    pub const fn decoded_samples(&self) -> u32 {
        self.decoded_samples
    }

    fn decode_pcm16(&mut self, input: &[u8], output: &mut [i16]) -> (usize, usize) {
        let mut consumed = 0;
        let mut written = 0;
        while written < output.len() && !self.is_finished() {
            let low = match self.pending_pcm_byte.take() {
                Some(low) => low,
                None => {
                    let Some(&low) = input.get(consumed) else {
                        break;
                    };
                    consumed += 1;
                    low
                }
            };
            let Some(&high) = input.get(consumed) else {
                self.pending_pcm_byte = Some(low);
                break;
            };
            consumed += 1;
            output[written] = i16::from_le_bytes([low, high]);
            written += 1;
            self.decoded_samples += 1;
        }
        (consumed, written)
    }

    fn decode_sld4(&mut self, input: &[u8], output: &mut [i16]) -> (usize, usize) {
        let mut consumed = 0;
        let mut written = 0;
        while written < output.len() && !self.is_finished() {
            let nibble = match self.pending_sld4_nibble.take() {
                Some(nibble) => nibble,
                None => {
                    let Some(&byte) = input.get(consumed) else {
                        break;
                    };
                    consumed += 1;
                    if self.decoded_samples + 1 < self.sample_count {
                        self.pending_sld4_nibble = Some(byte & 0x0f);
                    }
                    byte >> 4
                }
            };
            self.previous_sample = self
                .previous_sample
                .saturating_add(SLDPCM4_DELTAS_V0[usize::from(nibble)]);
            output[written] = self.previous_sample;
            written += 1;
            self.decoded_samples += 1;
        }
        (consumed, written)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AssetPlacement, ClipAsset, ClipAssetHeader, PCM16_MONO_CHANNELS};

    fn header(codec: CodecId, sample_count: u32, payload: &[u8]) -> ClipAssetHeader {
        ClipAssetHeader::from_clip(
            ClipAsset {
                codec,
                sample_rate_hz: 16_000,
                channels: PCM16_MONO_CHANNELS,
                sample_count,
                payload,
                loop_metadata: ClipLoop::None,
                placement: AssetPlacement::Unspecified,
            },
            0,
        )
        .unwrap()
    }

    #[test]
    fn pcm16_survives_split_bytes() {
        let mut decoder = StreamingClipDecoder::from_header(
            header(CodecId::Pcm16, 2, &[1, 0, 0xff, 0xff]),
            AudioLimits::v0_default(),
        )
        .unwrap();
        let mut out = [0i16; 2];
        assert_eq!(decoder.decode_chunk(&[1], &mut out), (1, 0));
        assert_eq!(decoder.decode_chunk(&[0, 0xff, 0xff], &mut out), (3, 2));
        assert_eq!(out, [1, -1]);
    }

    #[test]
    #[cfg(any(feature = "experimental-sldpcm4", feature = "sldpcm4-drums"))]
    fn sld4_retains_low_nibble_between_outputs() {
        let mut decoder = StreamingClipDecoder::from_header(
            header(CodecId::Sldpcm4, 2, &[0xf1]),
            AudioLimits::v0_default(),
        )
        .unwrap();
        let mut first = [0i16; 1];
        let mut second = [0i16; 1];
        assert_eq!(decoder.decode_chunk(&[0xf1], &mut first), (1, 1));
        assert_eq!(decoder.decode_chunk(&[], &mut second), (0, 1));
        assert_eq!(first, [16_384]);
        assert_eq!(second, [0]);
    }
}
