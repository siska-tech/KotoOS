use crate::{ClipAsset, ClipLoop, DecodeResult, Decoder, LoopCount};

/// PCM16 little-endian mono clip decoder state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Pcm16Decoder<'a> {
    clip: ClipAsset<'a>,
    cursor: u32,
    completed_loops: u32,
    ended: bool,
}

impl<'a> Pcm16Decoder<'a> {
    /// Creates decoder state for a validated PCM16 mono clip.
    pub const fn new(clip: ClipAsset<'a>) -> Self {
        Self {
            clip,
            cursor: 0,
            completed_loops: 0,
            ended: false,
        }
    }

    fn sample_at(&self, index: u32) -> Option<i16> {
        if index >= self.clip.sample_count {
            return None;
        }

        let byte_index = usize::try_from(index).ok()?.checked_mul(2)?;
        let bytes = self.clip.payload.get(byte_index..byte_index + 2)?;
        Some(i16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn advance_after_sample(&mut self) {
        self.cursor = self.cursor.saturating_add(1);
        self.apply_loop_or_end();
    }

    fn apply_loop_or_end(&mut self) {
        match self.clip.loop_metadata {
            ClipLoop::None => {
                if self.cursor >= self.clip.sample_count {
                    self.ended = true;
                }
            }
            ClipLoop::Whole { count } => {
                self.loop_if_needed(0, self.clip.sample_count, count);
            }
            ClipLoop::Forward { start, end, count } => {
                self.loop_if_needed(start, end, count);
            }
        }
    }

    fn loop_if_needed(&mut self, start: u32, end: u32, count: LoopCount) {
        if self.cursor < end {
            return;
        }

        match count {
            LoopCount::Infinite => {
                self.completed_loops = self.completed_loops.saturating_add(1);
                self.cursor = start;
            }
            LoopCount::Finite(max_loops) if self.completed_loops < max_loops => {
                self.completed_loops = self.completed_loops.saturating_add(1);
                self.cursor = start;
            }
            LoopCount::Finite(_) => {
                if self.cursor >= self.clip.sample_count {
                    self.ended = true;
                }
            }
        }
    }
}

impl Decoder for Pcm16Decoder<'_> {
    fn next_sample(&mut self) -> DecodeResult {
        if self.ended {
            return DecodeResult::End;
        }

        let Some(sample) = self.sample_at(self.cursor) else {
            self.ended = true;
            return DecodeResult::End;
        };

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
    use crate::{AudioLimits, ClipAsset};

    const LIMITS: AudioLimits = AudioLimits::v0_default();
    const PAYLOAD: &[u8] = &[1, 0, 254, 255, 0, 1];

    fn clip() -> ClipAsset<'static> {
        ClipAsset::pcm16_mono(LIMITS.sample_rate_hz, 3, PAYLOAD)
    }

    #[test]
    fn exact_samples_are_decoded() {
        let mut decoder = Pcm16Decoder::new(clip());

        assert_eq!(decoder.next_sample(), DecodeResult::Sample(1));
        assert_eq!(decoder.next_sample(), DecodeResult::Sample(-2));
        assert_eq!(decoder.next_sample(), DecodeResult::Sample(256));
    }

    #[test]
    fn end_of_clip_is_reported() {
        let mut decoder = Pcm16Decoder::new(clip());

        assert_eq!(decoder.read_samples(&mut [0; 3]), 3);
        assert_eq!(decoder.next_sample(), DecodeResult::End);
        assert_eq!(decoder.next_sample(), DecodeResult::End);
        assert!(decoder.is_ended());
    }

    #[test]
    fn whole_clip_loop_decodes() {
        let mut clip = clip();
        clip.loop_metadata = ClipLoop::Whole {
            count: LoopCount::Finite(1),
        };
        let mut decoder = Pcm16Decoder::new(clip);
        let mut out = [0; 6];

        assert_eq!(decoder.read_samples(&mut out), 6);
        assert_eq!(out, [1, -2, 256, 1, -2, 256]);
        assert_eq!(decoder.next_sample(), DecodeResult::End);
        assert_eq!(decoder.completed_loops(), 1);
    }

    #[test]
    fn forward_loop_decodes() {
        let mut clip = clip();
        clip.loop_metadata = ClipLoop::Forward {
            start: 1,
            end: 3,
            count: LoopCount::Finite(1),
        };
        let mut decoder = Pcm16Decoder::new(clip);
        let mut out = [0; 5];

        assert_eq!(decoder.read_samples(&mut out), 5);
        assert_eq!(out, [1, -2, 256, -2, 256]);
        assert_eq!(decoder.next_sample(), DecodeResult::End);
        assert_eq!(decoder.completed_loops(), 1);
    }
}
