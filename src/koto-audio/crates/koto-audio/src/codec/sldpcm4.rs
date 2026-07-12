use crate::{ClipAsset, ClipLoop, DecodeResult, Decoder, LoopCount};

/// Experimental SLDPCM4 decode table selection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)]
pub(crate) enum Sldpcm4TableId {
    /// KotoAudio experimental fixed standard-like delta table.
    KotoAudioExperimentalStandardLikeV0,
    /// Reserved room for comparing a future KotoAudio current table.
    KotoAudioCurrentReserved,
}

/// Experimental SLDPCM4 decoder state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Sldpcm4Decoder<'a> {
    clip: ClipAsset<'a>,
    sample_cursor: u32,
    payload_cursor: usize,
    high_nibble_next: bool,
    previous_sample: i16,
    completed_loops: u32,
    ended: bool,
    table_id: Sldpcm4TableId,
    loop_state: Sldpcm4LoopState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Sldpcm4LoopState {
    payload_cursor: usize,
    high_nibble_next: bool,
    previous_sample: i16,
}

impl<'a> Sldpcm4Decoder<'a> {
    /// Creates decoder state for a validated experimental SLDPCM4 mono clip.
    pub const fn new(clip: ClipAsset<'a>) -> Self {
        Self {
            clip,
            sample_cursor: 0,
            payload_cursor: 0,
            high_nibble_next: true,
            previous_sample: 0,
            completed_loops: 0,
            ended: false,
            table_id: Sldpcm4TableId::KotoAudioExperimentalStandardLikeV0,
            loop_state: Sldpcm4LoopState {
                payload_cursor: 0,
                high_nibble_next: true,
                previous_sample: 0,
            },
        }
    }

    fn next_nibble(&mut self) -> Option<u8> {
        let byte = *self.clip.payload.get(self.payload_cursor)?;
        let nibble = if self.high_nibble_next {
            byte >> 4
        } else {
            byte & 0x0f
        };

        if self.high_nibble_next {
            self.high_nibble_next = false;
        } else {
            self.high_nibble_next = true;
            self.payload_cursor = self.payload_cursor.saturating_add(1);
        }

        Some(nibble)
    }

    fn decode_nibble(&self, nibble: u8) -> i16 {
        match self.table_id {
            Sldpcm4TableId::KotoAudioExperimentalStandardLikeV0 => {
                crate::codec::SLDPCM4_DELTAS_V0[usize::from(nibble & 0x0f)]
            }
            Sldpcm4TableId::KotoAudioCurrentReserved => 0,
        }
    }

    fn advance_after_sample(&mut self) {
        self.sample_cursor = self.sample_cursor.saturating_add(1);
        self.apply_loop_or_end();
    }

    fn apply_loop_or_end(&mut self) {
        match self.clip.loop_metadata {
            ClipLoop::None => {
                if self.sample_cursor >= self.clip.sample_count {
                    self.ended = true;
                }
            }
            ClipLoop::Whole { count } => self.loop_if_needed(0, self.clip.sample_count, count),
            ClipLoop::Forward { start, end, count } => self.loop_if_needed(start, end, count),
        }
    }

    fn loop_if_needed(&mut self, start: u32, end: u32, count: LoopCount) {
        if self.sample_cursor < end {
            return;
        }

        match count {
            LoopCount::Infinite => self.restart_loop(start),
            LoopCount::Finite(max_loops) if self.completed_loops < max_loops => {
                self.restart_loop(start);
            }
            LoopCount::Finite(_) => {
                if self.sample_cursor >= self.clip.sample_count {
                    self.ended = true;
                }
            }
        }
    }

    fn restart_loop(&mut self, start: u32) {
        self.completed_loops = self.completed_loops.saturating_add(1);
        self.sample_cursor = start;
        self.payload_cursor = self.loop_state.payload_cursor;
        self.high_nibble_next = self.loop_state.high_nibble_next;
        self.previous_sample = self.loop_state.previous_sample;
    }
}

impl Decoder for Sldpcm4Decoder<'_> {
    fn next_sample(&mut self) -> DecodeResult {
        if self.ended {
            return DecodeResult::End;
        }
        if self.sample_cursor >= self.clip.sample_count {
            self.ended = true;
            return DecodeResult::End;
        }

        let Some(nibble) = self.next_nibble() else {
            self.ended = true;
            return DecodeResult::End;
        };
        let sample = self
            .previous_sample
            .saturating_add(self.decode_nibble(nibble));
        self.previous_sample = sample;
        self.advance_after_sample();
        DecodeResult::Sample(sample)
    }

    fn is_ended(&self) -> bool {
        self.ended
    }

    fn completed_loops(&self) -> u32 {
        self.completed_loops
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AssetPlacement, AudioLimits, CodecId, PCM16_MONO_CHANNELS};

    const LIMITS: AudioLimits = AudioLimits::v0_default();

    fn clip(payload: &'static [u8], sample_count: u32) -> ClipAsset<'static> {
        ClipAsset {
            codec: CodecId::Sldpcm4,
            sample_rate_hz: LIMITS.sample_rate_hz,
            channels: PCM16_MONO_CHANNELS,
            sample_count,
            payload,
            loop_metadata: ClipLoop::None,
            placement: AssetPlacement::Unspecified,
        }
    }

    #[test]
    fn decodes_single_byte_high_then_low_nibbles() {
        let mut decoder = Sldpcm4Decoder::new(clip(&[0xf1], 2));

        assert_eq!(decoder.next_sample(), DecodeResult::Sample(16384));
        assert_eq!(decoder.next_sample(), DecodeResult::Sample(0));
        assert_eq!(decoder.next_sample(), DecodeResult::End);
    }

    #[test]
    fn decodes_exact_standard_like_table_output() {
        let mut decoder =
            Sldpcm4Decoder::new(clip(&[0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54, 0x32, 0x10], 16));
        let mut out = [0; 16];

        assert_eq!(decoder.read_samples(&mut out), 16);
        assert_eq!(
            out,
            [
                16384, 24576, 28672, 30720, 31744, 32256, 32512, 32512, 32256, 31744, 30720, 28672,
                24576, 16384, 0, -32768
            ]
        );
        assert_eq!(decoder.next_sample(), DecodeResult::End);
    }

    #[test]
    fn previous_sample_accumulates_deltas() {
        let mut decoder = Sldpcm4Decoder::new(clip(&[0xff, 0x99], 4));

        assert_eq!(decoder.next_sample(), DecodeResult::Sample(16384));
        assert_eq!(decoder.next_sample(), DecodeResult::Sample(32767));
        assert_eq!(decoder.next_sample(), DecodeResult::Sample(32767));
        assert_eq!(decoder.next_sample(), DecodeResult::Sample(32767));
    }

    #[test]
    fn reconstruction_saturates_positive() {
        let mut decoder = Sldpcm4Decoder::new(clip(&[0xff], 2));
        decoder.previous_sample = i16::MAX - 10;

        assert_eq!(decoder.next_sample(), DecodeResult::Sample(i16::MAX));
        assert_eq!(decoder.next_sample(), DecodeResult::Sample(i16::MAX));
    }

    #[test]
    fn reconstruction_saturates_negative() {
        let mut decoder = Sldpcm4Decoder::new(clip(&[0x00], 2));
        decoder.previous_sample = i16::MIN + 10;

        assert_eq!(decoder.next_sample(), DecodeResult::Sample(i16::MIN));
        assert_eq!(decoder.next_sample(), DecodeResult::Sample(i16::MIN));
    }

    #[test]
    fn odd_sample_count_ignores_padding_nibble() {
        let mut decoder = Sldpcm4Decoder::new(clip(&[0xf0], 1));

        assert_eq!(decoder.next_sample(), DecodeResult::Sample(16384));
        assert_eq!(decoder.next_sample(), DecodeResult::End);
    }

    #[test]
    fn end_of_clip_is_reported() {
        let mut decoder = Sldpcm4Decoder::new(clip(&[0x88], 2));

        assert_eq!(decoder.read_samples(&mut [0; 2]), 2);
        assert_eq!(decoder.next_sample(), DecodeResult::End);
        assert_eq!(decoder.next_sample(), DecodeResult::End);
        assert!(decoder.is_ended());
    }

    #[test]
    fn whole_clip_loop_restores_predictor_and_nibble_position() {
        let mut clip = clip(&[0xf1], 2);
        clip.loop_metadata = ClipLoop::Whole {
            count: LoopCount::Finite(1),
        };
        let mut decoder = Sldpcm4Decoder::new(clip);
        let mut out = [0; 4];

        assert_eq!(decoder.read_samples(&mut out), 4);
        assert_eq!(out, [16384, 0, 16384, 0]);
        assert_eq!(decoder.next_sample(), DecodeResult::End);
        assert_eq!(decoder.completed_loops(), 1);
    }

    #[test]
    fn zero_start_forward_loop_restores_initial_state() {
        let mut clip = clip(&[0xf1, 0x90], 3);
        clip.loop_metadata = ClipLoop::Forward {
            start: 0,
            end: 2,
            count: LoopCount::Finite(1),
        };
        let mut decoder = Sldpcm4Decoder::new(clip);
        let mut out = [0; 5];

        assert_eq!(decoder.read_samples(&mut out), 5);
        assert_eq!(out, [16384, 0, 16384, 0, 256]);
        assert_eq!(decoder.next_sample(), DecodeResult::End);
        assert_eq!(decoder.completed_loops(), 1);
    }

    #[test]
    fn table_id_keeps_reserved_future_slot_internal() {
        let mut decoder = Sldpcm4Decoder::new(clip(&[0x77], 2));
        decoder.table_id = Sldpcm4TableId::KotoAudioCurrentReserved;

        assert_eq!(decoder.next_sample(), DecodeResult::Sample(0));
        assert_eq!(decoder.next_sample(), DecodeResult::Sample(0));
    }
}
