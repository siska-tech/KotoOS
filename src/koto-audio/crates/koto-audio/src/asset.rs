use crate::{AudioError, AudioLimits, ClipAsset, ClipLoop, LoopCount};

/// Runtime-ready clip asset magic bytes.
pub const CLIP_ASSET_MAGIC: [u8; 4] = *b"KACL";

/// Runtime-ready clip asset format version accepted by this runtime.
pub const CLIP_ASSET_VERSION: u16 = 1;

/// Size in bytes of the v1 runtime-ready clip asset header.
pub const CLIP_ASSET_HEADER_SIZE: usize = 48;

/// Numeric codec id for PCM16 little-endian mono payloads.
pub const CODEC_ID_PCM16: u16 = 1;

/// Numeric codec id reserved for experimental SLDPCM4 payloads.
pub const CODEC_ID_SLDPCM4: u16 = 16;

/// Numeric placement hint for unspecified placement.
pub const PLACEMENT_ID_UNSPECIFIED: u16 = 0;

/// Numeric placement hint for resident addressable memory.
pub const PLACEMENT_ID_RESIDENT: u16 = 1;

/// Loop count sentinel for infinite looping.
pub const LOOP_COUNT_INFINITE: u32 = u32::MAX;

/// Runtime clip codec identifier.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodecId {
    /// Signed little-endian PCM16 samples.
    Pcm16,
    /// Experimental 4-bit static logarithmic delta PCM.
    Sldpcm4,
    /// Reserved id for codecs not supported by this v0 runtime.
    Unsupported(u16),
}

impl CodecId {
    /// Returns the little-endian on-disk numeric codec id.
    pub const fn to_asset_id(self) -> u16 {
        match self {
            Self::Pcm16 => CODEC_ID_PCM16,
            Self::Sldpcm4 => CODEC_ID_SLDPCM4,
            Self::Unsupported(id) => id,
        }
    }

    /// Creates a codec id from the little-endian on-disk numeric id.
    pub const fn from_asset_id(id: u16) -> Self {
        match id {
            CODEC_ID_PCM16 => Self::Pcm16,
            CODEC_ID_SLDPCM4 => Self::Sldpcm4,
            other => Self::Unsupported(other),
        }
    }

    /// Returns whether this codec is accepted by this runtime build.
    pub const fn is_supported_by_build(self) -> bool {
        match self {
            Self::Pcm16 => true,
            Self::Sldpcm4 => cfg!(any(
                feature = "experimental-sldpcm4",
                feature = "sldpcm4-drums"
            )),
            Self::Unsupported(_) => false,
        }
    }
}

/// Placement hint for runtime-ready clip payloads.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AssetPlacement {
    /// No placement preference.
    #[default]
    Unspecified,
    /// Payload is expected to remain in normal addressable memory.
    Resident,
}

impl AssetPlacement {
    /// Returns the little-endian on-disk numeric placement id.
    pub const fn to_asset_id(self) -> u16 {
        match self {
            Self::Unspecified => PLACEMENT_ID_UNSPECIFIED,
            Self::Resident => PLACEMENT_ID_RESIDENT,
        }
    }

    /// Creates a placement hint from the little-endian on-disk numeric id.
    pub const fn from_asset_id(id: u16) -> Self {
        match id {
            PLACEMENT_ID_RESIDENT => Self::Resident,
            _ => Self::Unspecified,
        }
    }
}

/// Validation failure reason shared by the runtime parser and host converter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClipAssetError {
    /// The byte slice is too small or structurally incomplete.
    Truncated,
    /// The magic bytes do not identify a KotoAudio clip asset.
    InvalidMagic,
    /// The format version is not supported by this runtime.
    UnsupportedVersion,
    /// The header size is smaller than the v1 base header or beyond the slice.
    InvalidHeaderSize,
    /// The codec id is not supported by this runtime build.
    UnsupportedCodec,
    /// The asset channel count is not v0 mono.
    NonMono,
    /// The asset sample rate does not match the runtime mixer rate.
    SampleRateMismatch,
    /// The declared payload size does not match the remaining byte slice.
    PayloadSizeMismatch,
    /// The declared payload size cannot hold the declared sample count.
    InvalidSampleCount,
    /// The loop metadata is not valid for the sample count.
    InvalidLoop,
}

impl ClipAssetError {
    /// Maps detailed validation reasons onto the public logical audio error.
    pub const fn as_audio_error(self) -> AudioError {
        match self {
            Self::UnsupportedCodec => AudioError::UnsupportedCodec,
            Self::Truncated
            | Self::InvalidMagic
            | Self::UnsupportedVersion
            | Self::InvalidHeaderSize
            | Self::NonMono
            | Self::SampleRateMismatch
            | Self::PayloadSizeMismatch
            | Self::InvalidSampleCount
            | Self::InvalidLoop => AudioError::MalformedAsset,
        }
    }
}

/// Runtime-ready clip asset header.
///
/// All fields are encoded little-endian. Payload bytes immediately follow
/// `header_size`; v1 writes zeroes to reserved bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClipAssetHeader {
    /// Format version.
    pub version: u16,
    /// Total encoded header size in bytes.
    pub header_size: u16,
    /// Clip codec identifier.
    pub codec: CodecId,
    /// Clip channel count.
    pub channels: u16,
    /// Clip sample rate in hertz.
    pub sample_rate_hz: u32,
    /// Mono frame/sample count.
    pub sample_count: u32,
    /// Inclusive loop start sample.
    pub loop_start: u32,
    /// Exclusive loop end sample.
    pub loop_end: u32,
    /// Loop repeat count, zero for no loop and [`LOOP_COUNT_INFINITE`] for infinite.
    pub loop_count: u32,
    /// Encoded payload byte count.
    pub payload_size: u32,
    /// Lightweight placement hint.
    pub placement: AssetPlacement,
    /// Optional memory budget hint in bytes. Zero means unspecified.
    pub budget_hint_bytes: u32,
}

impl ClipAssetHeader {
    /// Creates a v1 header from a clip.
    pub fn from_clip(clip: ClipAsset<'_>, budget_hint_bytes: u32) -> Result<Self, ClipAssetError> {
        let payload_size =
            u32::try_from(clip.payload.len()).map_err(|_| ClipAssetError::PayloadSizeMismatch)?;
        let (loop_start, loop_end, loop_count) = encode_loop(clip.loop_metadata);

        Ok(Self {
            version: CLIP_ASSET_VERSION,
            header_size: CLIP_ASSET_HEADER_SIZE as u16,
            codec: clip.codec,
            channels: u16::from(clip.channels),
            sample_rate_hz: clip.sample_rate_hz,
            sample_count: clip.sample_count,
            loop_start,
            loop_end,
            loop_count,
            payload_size,
            placement: clip.placement,
            budget_hint_bytes,
        })
    }

    /// Encodes this header as fixed-size v1 little-endian bytes.
    pub fn encode(self) -> [u8; CLIP_ASSET_HEADER_SIZE] {
        let mut out = [0u8; CLIP_ASSET_HEADER_SIZE];
        out[0..4].copy_from_slice(&CLIP_ASSET_MAGIC);
        write_u16(&mut out, 4, self.version);
        write_u16(&mut out, 6, self.header_size);
        write_u16(&mut out, 8, self.codec.to_asset_id());
        write_u16(&mut out, 10, self.channels);
        write_u32(&mut out, 12, self.sample_rate_hz);
        write_u32(&mut out, 16, self.sample_count);
        write_u32(&mut out, 20, self.loop_start);
        write_u32(&mut out, 24, self.loop_end);
        write_u32(&mut out, 28, self.loop_count);
        write_u32(&mut out, 32, self.payload_size);
        write_u16(&mut out, 36, self.placement.to_asset_id());
        write_u32(&mut out, 38, self.budget_hint_bytes);
        out
    }

    /// Decodes and validates the v1 header fields available before payload checks.
    pub fn decode(bytes: &[u8]) -> Result<Self, ClipAssetError> {
        if bytes.len() < CLIP_ASSET_HEADER_SIZE {
            return Err(ClipAssetError::Truncated);
        }
        if bytes[0..4] != CLIP_ASSET_MAGIC {
            return Err(ClipAssetError::InvalidMagic);
        }

        let version = read_u16(bytes, 4);
        if version != CLIP_ASSET_VERSION {
            return Err(ClipAssetError::UnsupportedVersion);
        }

        let header_size = read_u16(bytes, 6);
        if usize::from(header_size) < CLIP_ASSET_HEADER_SIZE
            || usize::from(header_size) > bytes.len()
        {
            return Err(ClipAssetError::InvalidHeaderSize);
        }

        Ok(Self {
            version,
            header_size,
            codec: CodecId::from_asset_id(read_u16(bytes, 8)),
            channels: read_u16(bytes, 10),
            sample_rate_hz: read_u32(bytes, 12),
            sample_count: read_u32(bytes, 16),
            loop_start: read_u32(bytes, 20),
            loop_end: read_u32(bytes, 24),
            loop_count: read_u32(bytes, 28),
            payload_size: read_u32(bytes, 32),
            placement: AssetPlacement::from_asset_id(read_u16(bytes, 36)),
            budget_hint_bytes: read_u32(bytes, 38),
        })
    }

    /// Decodes the loop metadata represented by this header.
    pub fn loop_metadata(self) -> Result<ClipLoop, ClipAssetError> {
        decode_loop(
            self.loop_start,
            self.loop_end,
            self.loop_count,
            self.sample_count,
        )
    }
}

/// Parses a runtime-ready clip asset from a byte slice and validates it for limits.
pub fn parse_clip_asset<'a>(
    bytes: &'a [u8],
    limits: AudioLimits,
) -> Result<ClipAsset<'a>, ClipAssetError> {
    limits
        .validate()
        .map_err(|_| ClipAssetError::SampleRateMismatch)?;
    let header = ClipAssetHeader::decode(bytes)?;

    if !header.codec.is_supported_by_build() {
        return Err(ClipAssetError::UnsupportedCodec);
    }
    if header.channels != u16::from(crate::PCM16_MONO_CHANNELS) {
        return Err(ClipAssetError::NonMono);
    }
    if header.sample_rate_hz != limits.sample_rate_hz {
        return Err(ClipAssetError::SampleRateMismatch);
    }

    let payload_start = usize::from(header.header_size);
    let payload_size =
        usize::try_from(header.payload_size).map_err(|_| ClipAssetError::PayloadSizeMismatch)?;
    let payload_end = payload_start
        .checked_add(payload_size)
        .ok_or(ClipAssetError::PayloadSizeMismatch)?;
    if payload_end != bytes.len() {
        return Err(ClipAssetError::PayloadSizeMismatch);
    }

    let expected_payload_size = expected_payload_size(header.codec, header.sample_count)?;
    if !payload_size_matches_codec(header.codec, payload_size, expected_payload_size) {
        return Err(ClipAssetError::InvalidSampleCount);
    }

    let clip = ClipAsset {
        codec: header.codec,
        sample_rate_hz: header.sample_rate_hz,
        channels: crate::PCM16_MONO_CHANNELS,
        sample_count: header.sample_count,
        payload: &bytes[payload_start..payload_end],
        loop_metadata: header.loop_metadata()?,
        placement: header.placement,
    };

    clip.validate_detailed(limits)?;
    Ok(clip)
}

pub(crate) fn expected_payload_size(
    codec: CodecId,
    sample_count: u32,
) -> Result<usize, ClipAssetError> {
    let sample_count =
        usize::try_from(sample_count).map_err(|_| ClipAssetError::InvalidSampleCount)?;
    match codec {
        CodecId::Pcm16 => sample_count
            .checked_mul(2)
            .ok_or(ClipAssetError::InvalidSampleCount),
        CodecId::Sldpcm4 => sample_count
            .checked_add(1)
            .map(|count| count / 2)
            .ok_or(ClipAssetError::InvalidSampleCount),
        CodecId::Unsupported(_) => Err(ClipAssetError::UnsupportedCodec),
    }
}

pub(crate) const fn payload_size_matches_codec(
    codec: CodecId,
    actual: usize,
    expected: usize,
) -> bool {
    match codec {
        CodecId::Pcm16 => actual == expected,
        CodecId::Sldpcm4 => actual >= expected,
        CodecId::Unsupported(_) => false,
    }
}

fn encode_loop(loop_metadata: ClipLoop) -> (u32, u32, u32) {
    match loop_metadata {
        ClipLoop::None => (0, 0, 0),
        ClipLoop::Whole { count } => (0, 0, encode_loop_count(count)),
        ClipLoop::Forward { start, end, count } => (start, end, encode_loop_count(count)),
    }
}

fn decode_loop(
    loop_start: u32,
    loop_end: u32,
    loop_count: u32,
    sample_count: u32,
) -> Result<ClipLoop, ClipAssetError> {
    if loop_count == 0 {
        return if loop_start == 0 && loop_end == 0 {
            Ok(ClipLoop::None)
        } else {
            Err(ClipAssetError::InvalidLoop)
        };
    }

    let count = if loop_count == LOOP_COUNT_INFINITE {
        LoopCount::Infinite
    } else {
        LoopCount::Finite(loop_count)
    };

    if loop_start == 0 && loop_end == 0 {
        return Ok(ClipLoop::Whole { count });
    }
    if loop_start < loop_end && loop_end <= sample_count {
        Ok(ClipLoop::Forward {
            start: loop_start,
            end: loop_end,
            count,
        })
    } else {
        Err(ClipAssetError::InvalidLoop)
    }
}

fn encode_loop_count(count: LoopCount) -> u32 {
    match count {
        LoopCount::Infinite => LOOP_COUNT_INFINITE,
        LoopCount::Finite(count) => count,
    }
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn write_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AudioLimits, PCM16_MONO_CHANNELS};

    const LIMITS: AudioLimits = AudioLimits::v0_default();
    const PAYLOAD: &[u8] = &[1, 0, 255, 255];

    fn asset_bytes() -> [u8; CLIP_ASSET_HEADER_SIZE + 4] {
        let clip = ClipAsset::pcm16_mono(LIMITS.sample_rate_hz, 2, PAYLOAD);
        let header = ClipAssetHeader::from_clip(clip, 128).unwrap().encode();
        let mut bytes = [0u8; CLIP_ASSET_HEADER_SIZE + 4];
        bytes[..CLIP_ASSET_HEADER_SIZE].copy_from_slice(&header);
        bytes[CLIP_ASSET_HEADER_SIZE..].copy_from_slice(PAYLOAD);
        bytes
    }

    #[test]
    fn header_encode_decode_round_trips() {
        let clip = ClipAsset {
            loop_metadata: ClipLoop::Forward {
                start: 1,
                end: 2,
                count: LoopCount::Infinite,
            },
            placement: AssetPlacement::Resident,
            ..ClipAsset::pcm16_mono(LIMITS.sample_rate_hz, 2, PAYLOAD)
        };
        let header = ClipAssetHeader::from_clip(clip, 256).unwrap();

        let decoded = ClipAssetHeader::decode(&header.encode()).unwrap();

        assert_eq!(decoded, header);
        assert_eq!(decoded.loop_metadata(), Ok(clip.loop_metadata));
    }

    #[test]
    fn valid_pcm16_asset_parses() {
        let bytes = asset_bytes();

        let clip = parse_clip_asset(&bytes, LIMITS).unwrap();

        assert_eq!(clip.codec, CodecId::Pcm16);
        assert_eq!(clip.channels, PCM16_MONO_CHANNELS);
        assert_eq!(clip.payload, PAYLOAD);
    }

    #[test]
    fn extended_header_bytes_are_skipped_for_forward_compatibility() {
        let mut base = asset_bytes();
        write_u16(&mut base, 6, (CLIP_ASSET_HEADER_SIZE + 4) as u16);
        let mut bytes = [0u8; CLIP_ASSET_HEADER_SIZE + 4 + 4];
        bytes[..CLIP_ASSET_HEADER_SIZE].copy_from_slice(&base[..CLIP_ASSET_HEADER_SIZE]);
        bytes[CLIP_ASSET_HEADER_SIZE..CLIP_ASSET_HEADER_SIZE + 4].copy_from_slice(&[9, 8, 7, 6]);
        bytes[CLIP_ASSET_HEADER_SIZE + 4..].copy_from_slice(PAYLOAD);

        let clip = parse_clip_asset(&bytes, LIMITS).unwrap();

        assert_eq!(clip.payload, PAYLOAD);
    }

    #[test]
    fn invalid_magic_is_rejected() {
        let mut bytes = asset_bytes();
        bytes[0] = b'X';

        assert_eq!(
            parse_clip_asset(&bytes, LIMITS),
            Err(ClipAssetError::InvalidMagic)
        );
    }

    #[test]
    fn unsupported_version_is_rejected() {
        let mut bytes = asset_bytes();
        write_u16(&mut bytes, 4, CLIP_ASSET_VERSION + 1);

        assert_eq!(
            parse_clip_asset(&bytes, LIMITS),
            Err(ClipAssetError::UnsupportedVersion)
        );
    }

    #[test]
    fn unsupported_codec_is_rejected() {
        let mut bytes = asset_bytes();
        write_u16(&mut bytes, 8, 99);

        assert_eq!(
            parse_clip_asset(&bytes, LIMITS),
            Err(ClipAssetError::UnsupportedCodec)
        );
    }

    #[test]
    #[cfg(not(feature = "experimental-sldpcm4"))]
    fn sldpcm4_codec_id_decodes_but_is_rejected_without_feature() {
        let mut bytes = asset_bytes();
        write_u16(&mut bytes, 8, CODEC_ID_SLDPCM4);
        write_u32(&mut bytes, 16, 8);
        write_u32(&mut bytes, 32, 4);

        let header = ClipAssetHeader::decode(&bytes).unwrap();

        assert_eq!(header.codec, CodecId::Sldpcm4);
        assert_eq!(
            parse_clip_asset(&bytes, LIMITS),
            Err(ClipAssetError::UnsupportedCodec)
        );
    }

    #[test]
    #[cfg(feature = "experimental-sldpcm4")]
    fn sldpcm4_codec_id_parses_with_feature_enabled() {
        let mut bytes = asset_bytes();
        write_u16(&mut bytes, 8, CODEC_ID_SLDPCM4);
        write_u32(&mut bytes, 16, 8);
        write_u32(&mut bytes, 32, 4);

        let clip = parse_clip_asset(&bytes, LIMITS).unwrap();

        assert_eq!(clip.codec, CodecId::Sldpcm4);
        assert_eq!(clip.sample_count, 8);
        assert_eq!(clip.payload, PAYLOAD);
    }

    #[test]
    #[cfg(feature = "experimental-sldpcm4")]
    fn sldpcm4_asset_parses_with_extra_payload_with_feature_enabled() {
        let mut bytes = asset_bytes();
        write_u16(&mut bytes, 8, CODEC_ID_SLDPCM4);
        write_u32(&mut bytes, 16, 3);
        write_u32(&mut bytes, 32, 4);

        let clip = parse_clip_asset(&bytes, LIMITS).unwrap();

        assert_eq!(clip.codec, CodecId::Sldpcm4);
        assert_eq!(clip.sample_count, 3);
        assert_eq!(clip.payload, PAYLOAD);
    }

    #[test]
    #[cfg(feature = "experimental-sldpcm4")]
    fn sldpcm4_asset_rejects_short_payload_with_feature_enabled() {
        let mut bytes = asset_bytes();
        write_u16(&mut bytes, 8, CODEC_ID_SLDPCM4);
        write_u32(&mut bytes, 16, 5);
        write_u32(&mut bytes, 32, 2);
        let bytes = &bytes[..CLIP_ASSET_HEADER_SIZE + 2];

        assert_eq!(
            parse_clip_asset(bytes, LIMITS),
            Err(ClipAssetError::InvalidSampleCount)
        );
    }

    #[test]
    fn sample_rate_mismatch_is_rejected() {
        let mut bytes = asset_bytes();
        write_u32(&mut bytes, 12, LIMITS.sample_rate_hz + 1);

        assert_eq!(
            parse_clip_asset(&bytes, LIMITS),
            Err(ClipAssetError::SampleRateMismatch)
        );
    }

    #[test]
    fn non_mono_is_rejected() {
        let mut bytes = asset_bytes();
        write_u16(&mut bytes, 10, 2);

        assert_eq!(
            parse_clip_asset(&bytes, LIMITS),
            Err(ClipAssetError::NonMono)
        );
    }

    #[test]
    fn payload_size_mismatch_is_rejected() {
        let mut bytes = asset_bytes();
        write_u32(&mut bytes, 32, 2);

        assert_eq!(
            parse_clip_asset(&bytes, LIMITS),
            Err(ClipAssetError::PayloadSizeMismatch)
        );
    }

    #[test]
    fn invalid_loop_is_rejected() {
        let mut bytes = asset_bytes();
        write_u32(&mut bytes, 20, 2);
        write_u32(&mut bytes, 24, 1);
        write_u32(&mut bytes, 28, 1);

        assert_eq!(
            parse_clip_asset(&bytes, LIMITS),
            Err(ClipAssetError::InvalidLoop)
        );
    }
}
