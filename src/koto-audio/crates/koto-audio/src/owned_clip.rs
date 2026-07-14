#[cfg(any(feature = "experimental-sldpcm4", feature = "sldpcm4-drums"))]
use crate::codec::SLDPCM4_DELTAS_V0;
use crate::{
    parse_clip_asset, AudioLimits, ClipAssetError, ClipLoop, CodecId, DecodeResult, LoopCount,
};

/// Fixed-capacity player for a complete runtime-ready KACL image.
///
/// Package readers may reuse their scratch buffer immediately after
/// [`play_image`](Self::play_image): the encoded clip is copied into this player.
#[derive(Clone, Copy, Debug)]
pub struct OwnedClipPlayer<const N: usize> {
    image: [u8; N],
    payload_start: usize,
    payload_len: usize,
    codec: CodecId,
    sample_count: u32,
    loop_metadata: ClipLoop,
    sample_cursor: u32,
    payload_cursor: usize,
    high_nibble_next: bool,
    previous_sample: i16,
    loop_payload_cursor: usize,
    loop_high_nibble_next: bool,
    loop_previous_sample: i16,
    completed_loops: u32,
    playing: bool,
}

impl<const N: usize> OwnedClipPlayer<N> {
    /// Creates an empty player.
    pub const fn new() -> Self {
        Self {
            image: [0; N],
            payload_start: 0,
            payload_len: 0,
            codec: CodecId::Pcm16,
            sample_count: 0,
            loop_metadata: ClipLoop::None,
            sample_cursor: 0,
            payload_cursor: 0,
            high_nibble_next: true,
            previous_sample: 0,
            loop_payload_cursor: 0,
            loop_high_nibble_next: true,
            loop_previous_sample: 0,
            completed_loops: 0,
            playing: false,
        }
    }

    /// Validates and copies one KACL image, replacing the current clip.
    pub fn play_image(&mut self, image: &[u8], limits: AudioLimits) -> Result<(), ClipAssetError> {
        if image.len() > N {
            return Err(ClipAssetError::PayloadSizeMismatch);
        }
        let clip = parse_clip_asset(image, limits)?;
        let payload_start = clip.payload.as_ptr() as usize - image.as_ptr() as usize;
        self.image[..image.len()].copy_from_slice(image);
        self.payload_start = payload_start;
        self.payload_len = clip.payload.len();
        self.codec = clip.codec;
        self.sample_count = clip.sample_count;
        self.loop_metadata = clip.loop_metadata;
        self.sample_cursor = 0;
        self.payload_cursor = 0;
        self.high_nibble_next = true;
        self.previous_sample = 0;
        self.loop_payload_cursor = 0;
        self.loop_high_nibble_next = true;
        self.loop_previous_sample = 0;
        self.completed_loops = 0;
        self.playing = self.sample_count != 0;
        Ok(())
    }

    /// Stops playback without discarding the owned image.
    pub fn stop(&mut self) {
        self.playing = false;
    }

    /// Returns whether the player can still produce samples.
    pub const fn is_playing(&self) -> bool {
        self.playing
    }

    /// Decodes the next mono sample.
    pub fn next_sample(&mut self) -> DecodeResult {
        if !self.playing || self.sample_cursor >= self.sample_count {
            self.playing = false;
            return DecodeResult::End;
        }

        self.capture_loop_state();
        let sample = match self.codec {
            CodecId::Pcm16 => {
                let at = self.payload_start + self.payload_cursor;
                let Some(bytes) = self.image.get(at..at + 2) else {
                    self.playing = false;
                    return DecodeResult::End;
                };
                self.payload_cursor += 2;
                i16::from_le_bytes([bytes[0], bytes[1]])
            }
            CodecId::Sldpcm4 => {
                #[cfg(not(any(feature = "experimental-sldpcm4", feature = "sldpcm4-drums")))]
                {
                    self.playing = false;
                    return DecodeResult::End;
                }
                #[cfg(any(feature = "experimental-sldpcm4", feature = "sldpcm4-drums"))]
                {
                    let at = self.payload_start + self.payload_cursor;
                    let Some(&byte) = self.image.get(at) else {
                        self.playing = false;
                        return DecodeResult::End;
                    };
                    let nibble = if self.high_nibble_next {
                        byte >> 4
                    } else {
                        byte & 0x0f
                    };
                    if self.high_nibble_next {
                        self.high_nibble_next = false;
                    } else {
                        self.high_nibble_next = true;
                        self.payload_cursor += 1;
                    }
                    self.previous_sample = self
                        .previous_sample
                        .saturating_add(SLDPCM4_DELTAS_V0[usize::from(nibble)]);
                    self.previous_sample
                }
            }
            CodecId::Unsupported(_) => {
                self.playing = false;
                return DecodeResult::End;
            }
        };
        self.sample_cursor += 1;
        self.apply_loop_or_end();
        DecodeResult::Sample(sample)
    }

    fn capture_loop_state(&mut self) {
        let start = match self.loop_metadata {
            ClipLoop::Forward { start, .. } => start,
            ClipLoop::Whole { .. } | ClipLoop::None => 0,
        };
        if self.sample_cursor == start {
            self.loop_payload_cursor = self.payload_cursor;
            self.loop_high_nibble_next = self.high_nibble_next;
            self.loop_previous_sample = self.previous_sample;
        }
    }

    fn apply_loop_or_end(&mut self) {
        let (start, end, count) = match self.loop_metadata {
            ClipLoop::None => {
                if self.sample_cursor >= self.sample_count {
                    self.playing = false;
                }
                return;
            }
            ClipLoop::Whole { count } => (0, self.sample_count, count),
            ClipLoop::Forward { start, end, count } => (start, end, count),
        };
        if self.sample_cursor < end {
            return;
        }
        let repeat = match count {
            LoopCount::Infinite => true,
            LoopCount::Finite(max) => self.completed_loops < max,
        };
        if repeat {
            self.completed_loops = self.completed_loops.saturating_add(1);
            self.sample_cursor = start;
            self.payload_cursor = self.loop_payload_cursor;
            self.high_nibble_next = self.loop_high_nibble_next;
            self.previous_sample = self.loop_previous_sample;
        } else if self.sample_cursor >= self.sample_count {
            self.playing = false;
        }
    }
}

impl<const N: usize> Default for OwnedClipPlayer<N> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use crate::{AssetPlacement, ClipAsset, ClipAssetHeader, PCM16_MONO_CHANNELS};
    use std::vec::Vec;

    fn image(codec: CodecId, samples: u32, payload: &[u8]) -> Vec<u8> {
        let clip = ClipAsset {
            codec,
            sample_rate_hz: 16_000,
            channels: PCM16_MONO_CHANNELS,
            sample_count: samples,
            payload,
            loop_metadata: ClipLoop::None,
            placement: AssetPlacement::Unspecified,
        };
        let mut image = ClipAssetHeader::from_clip(clip, 0)
            .unwrap()
            .encode()
            .to_vec();
        image.extend_from_slice(payload);
        image
    }

    #[test]
    fn owns_and_decodes_pcm16_image() {
        let bytes = image(CodecId::Pcm16, 2, &[1, 0, 0xff, 0xff]);
        let mut player = OwnedClipPlayer::<64>::new();
        player
            .play_image(&bytes, AudioLimits::v0_default())
            .unwrap();
        assert_eq!(player.next_sample(), DecodeResult::Sample(1));
        assert_eq!(player.next_sample(), DecodeResult::Sample(-1));
        assert_eq!(player.next_sample(), DecodeResult::End);
    }

    #[cfg(any(feature = "experimental-sldpcm4", feature = "sldpcm4-drums"))]
    #[test]
    fn owns_and_decodes_sldpcm4_image() {
        let bytes = image(CodecId::Sldpcm4, 2, &[0xf1]);
        let mut player = OwnedClipPlayer::<64>::new();
        player
            .play_image(&bytes, AudioLimits::v0_default())
            .unwrap();
        assert_eq!(player.next_sample(), DecodeResult::Sample(16_384));
        assert_eq!(player.next_sample(), DecodeResult::Sample(0));
    }
}
