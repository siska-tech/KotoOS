use crate::hal::{AudioBuffer, AudioSource, HalError};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AudioError {
    StreamListFull,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PcmSliceStream<'a> {
    samples: &'a [i16],
    position: usize,
}

impl<'a> PcmSliceStream<'a> {
    pub const fn new(samples: &'a [i16]) -> Self {
        Self {
            samples,
            position: 0,
        }
    }

    pub fn remaining(&self) -> usize {
        self.samples.len().saturating_sub(self.position)
    }

    pub fn is_finished(&self) -> bool {
        self.position >= self.samples.len()
    }

    fn next_sample(&mut self) -> Option<i16> {
        let sample = self.samples.get(self.position).copied()?;
        self.position += 1;
        Some(sample)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PcmMixer<'a, const N: usize> {
    streams: [Option<PcmSliceStream<'a>>; N],
}

impl<'a, const N: usize> PcmMixer<'a, N> {
    pub const fn new() -> Self {
        Self { streams: [None; N] }
    }

    pub fn add_stream(&mut self, stream: PcmSliceStream<'a>) -> Result<(), AudioError> {
        for slot in &mut self.streams {
            if slot.is_none() {
                *slot = Some(stream);
                return Ok(());
            }
        }

        Err(AudioError::StreamListFull)
    }

    pub fn active_stream_count(&self) -> usize {
        self.streams.iter().filter(|slot| slot.is_some()).count()
    }

    pub fn clear(&mut self) {
        self.streams.fill(None);
    }

    pub fn mix_into(&mut self, output: &mut [i16]) {
        for out in output {
            let mut mixed = 0i32;

            for slot in &mut self.streams {
                let Some(stream) = slot else {
                    continue;
                };

                match stream.next_sample() {
                    Some(sample) => {
                        mixed = mixed.saturating_add(i32::from(sample));
                    }
                    None => {
                        *slot = None;
                    }
                }
            }

            *out = clamp_i16(mixed);
        }
    }

    pub fn fill_buffer(&mut self, buffer: &mut AudioBuffer<'_>) {
        let sample_count = buffer
            .frames
            .saturating_mul(usize::from(buffer.channels))
            .min(buffer.samples.len());
        self.mix_into(&mut buffer.samples[..sample_count]);
        buffer.samples[sample_count..].fill(0);
    }
}

impl<'a, const N: usize> Default for PcmMixer<'a, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a, const N: usize> AudioSource for PcmMixer<'a, N> {
    fn fill(&mut self, buffer: &mut AudioBuffer<'_>) -> Result<(), HalError> {
        self.fill_buffer(buffer);
        Ok(())
    }
}

fn clamp_i16(sample: i32) -> i16 {
    sample.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outputs_silence_without_streams() {
        let mut mixer = PcmMixer::<2>::new();
        let mut output = [7i16; 4];

        mixer.mix_into(&mut output);

        assert_eq!(output, [0, 0, 0, 0]);
    }

    #[test]
    fn copies_single_stream_and_pads_with_silence() {
        let mut mixer = PcmMixer::<2>::new();
        mixer.add_stream(PcmSliceStream::new(&[1, -2, 3])).unwrap();
        let mut output = [0i16; 5];

        mixer.mix_into(&mut output);

        assert_eq!(output, [1, -2, 3, 0, 0]);
        assert_eq!(mixer.active_stream_count(), 0);
    }

    #[test]
    fn sums_two_streams() {
        let mut mixer = PcmMixer::<2>::new();
        mixer
            .add_stream(PcmSliceStream::new(&[100, 200, 300]))
            .unwrap();
        mixer.add_stream(PcmSliceStream::new(&[-10, 20])).unwrap();
        let mut output = [0i16; 4];

        mixer.mix_into(&mut output);

        assert_eq!(output, [90, 220, 300, 0]);
    }

    #[test]
    fn clamps_to_i16_range() {
        let mut mixer = PcmMixer::<2>::new();
        mixer
            .add_stream(PcmSliceStream::new(&[30_000, -30_000]))
            .unwrap();
        mixer
            .add_stream(PcmSliceStream::new(&[30_000, -30_000]))
            .unwrap();
        let mut output = [0i16; 2];

        mixer.mix_into(&mut output);

        assert_eq!(output, [i16::MAX, i16::MIN]);
    }

    #[test]
    fn fills_only_declared_buffer_frames() {
        let mut mixer = PcmMixer::<1>::new();
        mixer
            .add_stream(PcmSliceStream::new(&[1, 2, 3, 4, 5]))
            .unwrap();
        let mut samples = [9i16; 6];
        let mut buffer = AudioBuffer {
            sample_rate: 22_050,
            channels: 2,
            frames: 2,
            samples: &mut samples,
        };

        mixer.fill_buffer(&mut buffer);

        assert_eq!(samples, [1, 2, 3, 4, 0, 0]);
    }

    #[test]
    fn rejects_streams_when_full() {
        let mut mixer = PcmMixer::<1>::new();
        mixer.add_stream(PcmSliceStream::new(&[1])).unwrap();

        assert_eq!(
            mixer.add_stream(PcmSliceStream::new(&[2])),
            Err(AudioError::StreamListFull)
        );
    }
}
