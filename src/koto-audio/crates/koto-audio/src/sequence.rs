use crate::{
    builtin_drums, AudioError, AudioLimits, AudioResult, DecodeResult, Decoder, MixerVolume,
    RuntimeCue,
};

/// Maximum voice count for the experimental polyphonic sequence foundation.
pub const MAX_SEQUENCE_VOICES: usize = 4;

/// Repeat count value for an infinite [`SequenceEvent::LoopEnd`] loop.
pub const SEQUENCE_REPEAT_INFINITE: u8 = 0;

/// P/ECE/MusLib-style built-in instrument id for a fast square tone.
pub const BUILTIN_INSTRUMENT_SQUARE_FAST: u8 = 0;
/// P/ECE/MusLib-style built-in instrument id for a fast saw tone.
pub const BUILTIN_INSTRUMENT_SAW_FAST: u8 = 1;
/// P/ECE/MusLib-style built-in instrument id for a fast triangle tone.
pub const BUILTIN_INSTRUMENT_TRIANGLE_FAST: u8 = 2;
/// P/ECE/MusLib-style built-in instrument id for a square tone.
pub const BUILTIN_INSTRUMENT_SQUARE: u8 = 3;
/// P/ECE/MusLib-style built-in instrument id for a saw tone.
pub const BUILTIN_INSTRUMENT_SAW: u8 = 4;
/// P/ECE/MusLib-style built-in instrument id for a triangle tone.
pub const BUILTIN_INSTRUMENT_TRIANGLE: u8 = 5;
/// P/ECE/MusLib-style built-in instrument id for bass drum.
pub const BUILTIN_INSTRUMENT_BASS_DRUM: u8 = 6;
/// P/ECE/MusLib-style built-in instrument id for snare drum 2.
pub const BUILTIN_INSTRUMENT_SNARE_DRUM_2: u8 = 7;
/// P/ECE/MusLib-style built-in instrument id for snare drum 1.
pub const BUILTIN_INSTRUMENT_SNARE_DRUM_1: u8 = 8;
/// P/ECE/MusLib-style built-in instrument id for open hi-hat.
pub const BUILTIN_INSTRUMENT_OPEN_HI_HAT: u8 = 9;
/// P/ECE/MusLib-style built-in instrument id for closed hi-hat.
pub const BUILTIN_INSTRUMENT_CLOSED_HI_HAT: u8 = 10;
/// P/ECE/MusLib-style built-in instrument id for crash cymbal.
pub const BUILTIN_INSTRUMENT_CRASH_CYMBAL: u8 = 11;
/// P/ECE/MusLib-style built-in instrument id for high synth tom.
pub const BUILTIN_INSTRUMENT_SYNTH_TOM_HIGH: u8 = 13;
/// P/ECE/MusLib-style built-in instrument id for middle synth tom.
pub const BUILTIN_INSTRUMENT_SYNTH_TOM_MID: u8 = 14;
/// P/ECE/MusLib-style built-in instrument id for low synth tom.
pub const BUILTIN_INSTRUMENT_SYNTH_TOM_LOW: u8 = 15;
/// P/ECE/MusLib-style built-in instrument id for clap.
pub const BUILTIN_INSTRUMENT_CLAP: u8 = 16;

/// Pitch helper for the minimal monophonic sequence synth.
///
/// The stored value is hertz, matching the existing [`SequenceEvent::Note`]
/// layout. MIDI conversion uses fixed-point integer math so the runtime stays
/// `no_std` and does not require floating point.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SequencePitch {
    /// Rounded pitch frequency in hertz.
    pub frequency_hz: u16,
}

impl SequencePitch {
    /// Middle C.
    pub const C4: Self = Self::from_midi_note(60);
    /// D above middle C.
    pub const D4: Self = Self::from_midi_note(62);
    /// E above middle C.
    pub const E4: Self = Self::from_midi_note(64);
    /// F above middle C.
    pub const F4: Self = Self::from_midi_note(65);
    /// G above middle C.
    pub const G4: Self = Self::from_midi_note(67);
    /// Concert A.
    pub const A4: Self = Self::from_midi_note(69);
    /// B above middle C.
    pub const B4: Self = Self::from_midi_note(71);
    /// C one octave above middle C.
    pub const C5: Self = Self::from_midi_note(72);

    /// Creates a pitch from a frequency in hertz.
    pub const fn from_hz(frequency_hz: u16) -> Self {
        Self { frequency_hz }
    }

    /// Creates a pitch from a MIDI note number, rounded to the nearest hertz.
    pub const fn from_midi_note(note: u8) -> Self {
        const A4_MIDI_NOTE: u8 = 69;
        const A4_Q16: u32 = 440 << 16;
        const SEMITONE_UP_Q16: u32 = 69_434;
        const SEMITONE_DOWN_Q16: u32 = 61_858;

        let mut frequency_q16 = A4_Q16;
        if note >= A4_MIDI_NOTE {
            let mut remaining = note - A4_MIDI_NOTE;
            while remaining > 0 {
                frequency_q16 =
                    ((frequency_q16 as u64 * SEMITONE_UP_Q16 as u64 + 32_768) >> 16) as u32;
                remaining -= 1;
            }
        } else {
            let mut remaining = A4_MIDI_NOTE - note;
            while remaining > 0 {
                frequency_q16 =
                    ((frequency_q16 as u64 * SEMITONE_DOWN_Q16 as u64 + 32_768) >> 16) as u32;
                remaining -= 1;
            }
        }

        let rounded_hz = (frequency_q16 + 32_768) >> 16;
        Self {
            frequency_hz: if rounded_hz > u16::MAX as u32 {
                u16::MAX
            } else {
                rounded_hz as u16
            },
        }
    }

    /// Returns the fixed phase step used by the sequence oscillator.
    pub const fn sample_step(self, sample_rate_hz: u32) -> u32 {
        phase_step(self.frequency_hz, sample_rate_hz)
    }
}

/// Minimal tempo helper for static sequence construction.
///
/// This stores only a BPM and ticks-per-beat pair, then derives the sequence
/// tick rate used by [`Sequence::new`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SequenceTempo {
    /// Beats per minute.
    pub bpm: u16,
    /// Sequence ticks in one beat.
    pub ticks_per_beat: u16,
}

impl SequenceTempo {
    /// Creates a tempo helper from BPM and ticks per beat.
    pub const fn from_bpm(bpm: u16, ticks_per_beat: u16) -> Self {
        Self {
            bpm,
            ticks_per_beat,
        }
    }

    /// Returns the tick rate in hertz, rounded to the nearest integer.
    pub const fn tick_rate_hz(self) -> u16 {
        if self.bpm == 0 || self.ticks_per_beat == 0 {
            return 0;
        }

        let ticks_per_minute = self.bpm as u32 * self.ticks_per_beat as u32;
        let rounded = (ticks_per_minute + 30) / 60;
        if rounded > u16::MAX as u32 {
            u16::MAX
        } else {
            rounded as u16
        }
    }

    /// Returns one beat in sequence ticks.
    pub const fn beat(self) -> u16 {
        self.ticks_per_beat
    }

    /// Returns a whole-note duration in sequence ticks.
    pub const fn whole(self) -> u16 {
        self.ticks_per_beat.saturating_mul(4)
    }

    /// Returns a half-note duration in sequence ticks.
    pub const fn half(self) -> u16 {
        self.ticks_per_beat.saturating_mul(2)
    }

    /// Returns a quarter-note duration in sequence ticks.
    pub const fn quarter(self) -> u16 {
        self.ticks_per_beat
    }

    /// Returns an eighth-note duration in sequence ticks.
    pub const fn eighth(self) -> u16 {
        self.ticks_per_beat / 2
    }
}

/// Static monophonic sequence asset for the minimal BGM/jingle engine.
///
/// This is an experimental M11 shape, not a stable numeric or raw-memory ABI.
/// Events and instruments are borrowed slices so callers can keep them in
/// static storage without heap allocation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Sequence<'a> {
    /// Event stream interpreted by the monophonic sequencer.
    pub events: &'a [SequenceEvent],
    /// Built-in synth instruments referenced by note events.
    pub instruments: &'a [SequenceInstrument],
    /// Fixed sequence tick rate used for event durations.
    pub tick_rate_hz: u16,
}

/// One voice in an experimental fixed-capacity polyphonic sequence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PolyphonicSequenceVoice<'a> {
    /// Static monophonic sequence interpreted by this voice.
    pub sequence: Sequence<'a>,
    /// Voice-local gain applied before the polyphonic sum.
    pub gain: MixerVolume,
}

impl<'a> PolyphonicSequenceVoice<'a> {
    /// Creates a voice with an explicit voice-local gain.
    pub const fn new(sequence: Sequence<'a>, gain: MixerVolume) -> Self {
        Self { sequence, gain }
    }

    /// Creates a voice at unity gain.
    pub const fn unity(sequence: Sequence<'a>) -> Self {
        Self::new(sequence, MixerVolume::UNITY)
    }
}

/// Experimental no-heap polyphonic sequence asset.
///
/// This is a small M12 foundation for BGM-like playback. It borrows a static
/// voice slice, caps runtime interpretation at [`MAX_SEQUENCE_VOICES`], and is
/// not a stable memory or numeric hostcall ABI.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PolyphonicSequence<'a> {
    /// Fixed caller-owned voice slice.
    pub voices: &'a [PolyphonicSequenceVoice<'a>],
}

/// Runtime-ready compact sequence table for future parser/tool output.
///
/// This M12-006 skeleton is a borrowed-table boundary, not a KotoMML parser,
/// `.kmml` loader, binary asset format, or stable numeric ABI. Instruments,
/// tracks, and event streams are caller-owned so generated Rust tables can be
/// validated without heap allocation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompactSequence<'a> {
    /// Compact instrument descriptors referenced by track events.
    pub instruments: &'a [CompactInstrument],
    /// Caller-owned monophonic tracks. Each track maps to one sequence voice.
    pub tracks: &'a [CompactTrack<'a>],
    /// Runtime-ready tempo/tick metadata.
    pub tempo: CompactTempo,
}

/// One monophonic compact sequence track.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompactTrack<'a> {
    /// Compact event stream for this track.
    pub events: &'a [CompactEvent],
    /// Track-local fixed-point gain, where 256 is unity.
    pub gain: MixerVolume,
    /// Initial compact instrument table index for future parser defaults.
    pub initial_instrument_id: u8,
}

/// Compact instrument descriptor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompactInstrument {
    /// P/ECE/MusLib-style built-in sequence instrument id.
    pub builtin_id: u8,
    /// Instrument-local volume, where 255 is full volume.
    pub volume: u8,
    /// Optional linear fade-in length in sequence ticks. Zero disables attack.
    pub attack_ticks: u16,
    /// Optional linear fade-out length in sequence ticks. Zero disables release.
    pub release_ticks: u16,
    /// Optional linear decay length in sequence ticks. Zero disables decay.
    pub decay_ticks: u16,
}

/// Runtime-ready compact sequence event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompactEvent {
    /// Plays a compact-table instrument for a fixed number of sequence ticks.
    Note {
        /// Note frequency in hertz.
        pitch: u16,
        /// Duration in sequence ticks.
        duration_ticks: u16,
        /// Note-local volume, where 255 is full note volume.
        volume: u8,
        /// Index into [`CompactSequence::instruments`].
        instrument_id: u8,
    },
    /// Emits silence for a fixed number of sequence ticks.
    Rest {
        /// Duration in sequence ticks.
        duration_ticks: u16,
    },
    /// Marks the beginning of the single supported loop region.
    LoopStart,
    /// Jumps back to the last loop start.
    LoopEnd {
        /// Extra loop passes, or [`SEQUENCE_REPEAT_INFINITE`].
        repeat_count: u8,
    },
    /// Completes this compact track.
    End,
}

/// Compact sequence tempo metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompactTempo {
    /// Fixed sequence tick rate used for event durations.
    pub tick_rate_hz: u16,
    /// Optional generated-table BPM metadata. Zero means unspecified.
    pub bpm: u16,
    /// Optional generated-table ticks-per-beat metadata. Zero means unspecified.
    pub ticks_per_beat: u16,
}

/// Detailed compact sequence validation error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompactSequenceError {
    /// The compact asset is malformed.
    Malformed,
    /// The compact asset is structurally valid but exceeds the current runtime.
    Unsupported,
    /// No tracks were supplied.
    ZeroTracks,
    /// More tracks were supplied than the sequence voice limit permits.
    TooManyTracks,
    /// A compact instrument points at an unknown or reserved built-in id.
    UnknownBuiltinInstrument,
    /// A track or event references a missing compact instrument table entry.
    UnknownInstrument,
    /// A note or rest has zero duration.
    ZeroDuration,
    /// A loop end appeared without a preceding loop start.
    LoopEndWithoutStart,
    /// A track contains nested loops, which are not supported by M12.
    NestedLoop,
    /// A track did not contain an end event.
    MissingEnd,
}

impl From<CompactSequenceError> for AudioError {
    fn from(error: CompactSequenceError) -> Self {
        match error {
            CompactSequenceError::Unsupported
            | CompactSequenceError::TooManyTracks
            | CompactSequenceError::NestedLoop => Self::UnsupportedOperation,
            _ => Self::MalformedAsset,
        }
    }
}

impl<'a> PolyphonicSequence<'a> {
    /// Creates a polyphonic sequence from a static voice slice.
    pub const fn new(voices: &'a [PolyphonicSequenceVoice<'a>]) -> Self {
        Self { voices }
    }

    /// Validates the bounded polyphonic sequence shape.
    pub fn validate(self, limits: AudioLimits) -> AudioResult<Self> {
        limits.validate()?;
        if self.voices.is_empty() || self.voices.len() > MAX_SEQUENCE_VOICES {
            return Err(AudioError::InvalidArgument);
        }

        for voice in self.voices {
            voice.sequence.validate(limits)?;
        }

        Ok(self)
    }
}

impl<'a> CompactSequence<'a> {
    /// Creates a compact sequence from caller-owned static slices.
    pub const fn new(
        instruments: &'a [CompactInstrument],
        tracks: &'a [CompactTrack<'a>],
        tempo: CompactTempo,
    ) -> Self {
        Self {
            instruments,
            tracks,
            tempo,
        }
    }

    /// Validates the compact sequence boundary.
    pub fn validate(self) -> core::result::Result<Self, CompactSequenceError> {
        validate_compact_sequence(self)?;
        Ok(self)
    }
}

impl<'a> CompactTrack<'a> {
    /// Creates a compact track from a caller-owned event stream.
    pub const fn new(
        events: &'a [CompactEvent],
        gain: MixerVolume,
        initial_instrument_id: u8,
    ) -> Self {
        Self {
            events,
            gain,
            initial_instrument_id,
        }
    }

    /// Adapts this compact track into the existing monophonic sequence shape.
    ///
    /// Converted events are written into `events_out`, which remains
    /// caller-owned. The returned sequence references that output slice and the
    /// fixed built-in instrument table.
    pub fn adapt_to_sequence<'out>(
        self,
        compact: CompactSequence<'_>,
        events_out: &'out mut [SequenceEvent],
    ) -> core::result::Result<Sequence<'out>, CompactSequenceError> {
        validate_compact_sequence(compact)?;
        if events_out.len() < self.events.len() {
            return Err(CompactSequenceError::Unsupported);
        }

        let mut index = 0usize;
        while index < self.events.len() {
            events_out[index] = match self.events[index] {
                CompactEvent::Note {
                    pitch,
                    duration_ticks,
                    volume,
                    instrument_id,
                } => {
                    let compact_instrument = compact
                        .instruments
                        .get(usize::from(instrument_id))
                        .ok_or(CompactSequenceError::UnknownInstrument)?;
                    SequenceEvent::Note {
                        pitch,
                        duration_ticks,
                        volume: scale_u8(volume, compact_instrument.volume),
                        instrument_id: compact_instrument.builtin_id,
                    }
                }
                CompactEvent::Rest { duration_ticks } => SequenceEvent::Rest { duration_ticks },
                CompactEvent::LoopStart => SequenceEvent::LoopStart,
                CompactEvent::LoopEnd { repeat_count } => SequenceEvent::LoopEnd { repeat_count },
                CompactEvent::End => SequenceEvent::End,
            };
            index += 1;
        }

        Ok(Sequence::new(
            &events_out[..self.events.len()],
            &BUILTIN_SEQUENCE_INSTRUMENTS,
            compact.tempo.tick_rate_hz,
        ))
    }
}

impl CompactInstrument {
    /// Creates a compact built-in instrument reference.
    pub const fn builtin(builtin_id: u8, volume: u8) -> Self {
        Self {
            builtin_id,
            volume,
            attack_ticks: 0,
            release_ticks: 0,
            decay_ticks: 0,
        }
    }
}

impl CompactTempo {
    /// Creates compact tempo metadata from a fixed tick rate.
    pub const fn from_tick_rate_hz(tick_rate_hz: u16) -> Self {
        Self {
            tick_rate_hz,
            bpm: 0,
            ticks_per_beat: 0,
        }
    }
}

/// Validates a compact sequence without allocating.
pub fn validate_compact_sequence(
    sequence: CompactSequence<'_>,
) -> core::result::Result<(), CompactSequenceError> {
    if sequence.tempo.tick_rate_hz == 0 {
        return Err(CompactSequenceError::Malformed);
    }
    if sequence.tracks.is_empty() {
        return Err(CompactSequenceError::ZeroTracks);
    }
    if sequence.tracks.len() > MAX_SEQUENCE_VOICES {
        return Err(CompactSequenceError::TooManyTracks);
    }

    for instrument in sequence.instruments {
        if SequenceInstrument::builtin(instrument.builtin_id).is_none() {
            return Err(CompactSequenceError::UnknownBuiltinInstrument);
        }
    }

    for track in sequence.tracks {
        validate_compact_track(track, sequence.instruments.len())?;
    }

    Ok(())
}

fn validate_compact_track(
    track: &CompactTrack<'_>,
    instrument_count: usize,
) -> core::result::Result<(), CompactSequenceError> {
    if usize::from(track.initial_instrument_id) >= instrument_count {
        return Err(CompactSequenceError::UnknownInstrument);
    }

    let mut loop_open = false;
    for event in track.events {
        match *event {
            CompactEvent::Note {
                duration_ticks,
                instrument_id,
                ..
            } => {
                if duration_ticks == 0 {
                    return Err(CompactSequenceError::ZeroDuration);
                }
                if usize::from(instrument_id) >= instrument_count {
                    return Err(CompactSequenceError::UnknownInstrument);
                }
            }
            CompactEvent::Rest { duration_ticks } => {
                if duration_ticks == 0 {
                    return Err(CompactSequenceError::ZeroDuration);
                }
            }
            CompactEvent::LoopStart => {
                if loop_open {
                    return Err(CompactSequenceError::NestedLoop);
                }
                loop_open = true;
            }
            CompactEvent::LoopEnd { .. } => {
                if !loop_open {
                    return Err(CompactSequenceError::LoopEndWithoutStart);
                }
                loop_open = false;
            }
            CompactEvent::End => return Ok(()),
        }
    }

    Err(CompactSequenceError::MissingEnd)
}

impl<'a> Sequence<'a> {
    /// Creates a sequence from static event and instrument slices.
    pub const fn new(
        events: &'a [SequenceEvent],
        instruments: &'a [SequenceInstrument],
        tick_rate_hz: u16,
    ) -> Self {
        Self {
            events,
            instruments,
            tick_rate_hz,
        }
    }

    /// Creates a sequence using a minimal BPM helper.
    pub const fn with_tempo(
        events: &'a [SequenceEvent],
        instruments: &'a [SequenceInstrument],
        tempo: SequenceTempo,
    ) -> Self {
        Self::new(events, instruments, tempo.tick_rate_hz())
    }

    /// Validates that this sequence can be interpreted by the minimal runtime.
    pub fn validate(self, limits: AudioLimits) -> AudioResult<Self> {
        limits.validate()?;
        if self.events.is_empty() || self.tick_rate_hz == 0 {
            return Err(AudioError::MalformedAsset);
        }

        let mut loop_depth = 0u8;
        for event in self.events {
            match *event {
                SequenceEvent::Note {
                    duration_ticks,
                    instrument_id,
                    ..
                } => {
                    if duration_ticks == 0 || usize::from(instrument_id) >= self.instruments.len() {
                        return Err(AudioError::MalformedAsset);
                    }
                    if self.instruments[usize::from(instrument_id)].kind
                        == SequenceInstrumentKind::Reserved
                    {
                        return Err(AudioError::MalformedAsset);
                    }
                }
                SequenceEvent::Rest { duration_ticks } => {
                    if duration_ticks == 0 {
                        return Err(AudioError::MalformedAsset);
                    }
                }
                SequenceEvent::LoopStart => {
                    loop_depth = loop_depth.saturating_add(1);
                    if loop_depth > 1 {
                        return Err(AudioError::UnsupportedOperation);
                    }
                }
                SequenceEvent::LoopEnd { .. } => {
                    if loop_depth == 0 {
                        return Err(AudioError::MalformedAsset);
                    }
                    loop_depth = loop_depth.saturating_sub(1);
                }
                SequenceEvent::End => return Ok(self),
            }
        }

        Err(AudioError::MalformedAsset)
    }
}

/// A single event in a minimal monophonic sequence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SequenceEvent {
    /// Plays a note for a fixed number of sequence ticks.
    ///
    /// `pitch` is interpreted as frequency in hertz. `volume` uses 255 as full
    /// note volume before source/app/master mixer gain is applied.
    Note {
        /// Note frequency in hertz.
        pitch: u16,
        /// Duration in sequence ticks.
        duration_ticks: u16,
        /// Note-local volume, where 255 is full note volume.
        volume: u8,
        /// Index into [`Sequence::instruments`].
        instrument_id: u8,
    },
    /// Emits silence for a fixed number of sequence ticks.
    Rest {
        /// Duration in sequence ticks.
        duration_ticks: u16,
    },
    /// Marks the beginning of the single supported loop region.
    LoopStart,
    /// Jumps back to the last loop start.
    ///
    /// `repeat_count` is the number of extra loop passes. Use
    /// [`SEQUENCE_REPEAT_INFINITE`] for an infinite loop.
    LoopEnd {
        /// Extra loop passes, or [`SEQUENCE_REPEAT_INFINITE`].
        repeat_count: u8,
    },
    /// Completes the source.
    End,
}

impl SequenceEvent {
    /// Creates a note event from a pitch helper.
    pub const fn note(
        pitch: SequencePitch,
        duration_ticks: u16,
        volume: u8,
        instrument_id: u8,
    ) -> Self {
        Self::Note {
            pitch: pitch.frequency_hz,
            duration_ticks,
            volume,
            instrument_id,
        }
    }

    /// Creates a note event from a raw frequency in hertz.
    pub const fn note_hz(
        frequency_hz: u16,
        duration_ticks: u16,
        volume: u8,
        instrument_id: u8,
    ) -> Self {
        Self::note(
            SequencePitch::from_hz(frequency_hz),
            duration_ticks,
            volume,
            instrument_id,
        )
    }

    /// Creates a rest event.
    pub const fn rest(duration_ticks: u16) -> Self {
        Self::Rest { duration_ticks }
    }
}

/// Built-in waveform for the minimal sequence synth.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SequenceWaveform {
    /// Simple square wave.
    Square,
    /// Simple sawtooth wave.
    Saw,
    /// Simple triangle wave.
    Triangle,
}

/// Fixed built-in PCM16 mono drum sequence instrument.
///
/// Built-in drums use static signed PCM16 mono sample slices at the current
/// placeholder table rate. Pitch is ignored, each note starts playback from the
/// first sample, and playback returns silence after the selected slice ends.
/// Normal note, instrument, source, bus, app, and master gain still apply.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SequenceDrum {
    /// Bass drum placeholder.
    BassDrum,
    /// Snare drum 2 placeholder.
    SnareDrum2,
    /// Snare drum 1 placeholder.
    SnareDrum1,
    /// Open hi-hat placeholder.
    OpenHiHat,
    /// Closed hi-hat placeholder.
    ClosedHiHat,
    /// Crash cymbal placeholder.
    CrashCymbal,
    /// High synth tom placeholder.
    SynthTomHigh,
    /// Middle synth tom placeholder.
    SynthTomMid,
    /// Low synth tom placeholder.
    SynthTomLow,
    /// Clap placeholder.
    Clap,
}

impl SequenceDrum {
    #[cfg(not(feature = "sldpcm4-drums"))]
    const fn samples(self) -> &'static [i16] {
        let _sample_rate_hz = builtin_drums::BUILTIN_DRUM_SAMPLE_RATE_HZ;
        builtin_drums::samples(self)
    }

    #[cfg(feature = "sldpcm4-drums")]
    const fn sldpcm4_samples(self) -> builtin_drums::BuiltinDrumSldpcm4 {
        let _sample_rate_hz = builtin_drums::BUILTIN_DRUM_SAMPLE_RATE_HZ;
        builtin_drums::sldpcm4_samples(self)
    }
}

/// Minimal built-in sequence instrument kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SequenceInstrumentKind {
    /// Generated tone waveform.
    Tone {
        /// Generated waveform.
        waveform: SequenceWaveform,
    },
    /// Fixed built-in PCM16 mono drum sample.
    Drum {
        /// Drum sample selector.
        drum: SequenceDrum,
    },
    /// Reserved built-in slot.
    Reserved,
}

/// Minimal built-in sequence instrument.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SequenceInstrument {
    /// Instrument synthesis kind.
    pub kind: SequenceInstrumentKind,
    /// Instrument volume, where 255 is full volume.
    pub volume: u8,
    /// Optional linear fade-in length in sequence ticks. Zero disables attack.
    pub attack_ticks: u16,
    /// Optional linear fade-out length in sequence ticks. Zero disables release.
    pub release_ticks: u16,
    /// Optional linear decay length in sequence ticks. Zero disables decay.
    ///
    /// Reserved for future envelope expansion; the minimal M11 synth currently
    /// uses attack and release only for click reduction.
    pub decay_ticks: u16,
}

impl SequenceInstrument {
    /// Creates an instrument without decay.
    pub const fn new(waveform: SequenceWaveform, volume: u8) -> Self {
        Self::tone(waveform, volume)
    }

    /// Creates a tone instrument without decay.
    pub const fn tone(waveform: SequenceWaveform, volume: u8) -> Self {
        Self {
            kind: SequenceInstrumentKind::Tone { waveform },
            volume,
            attack_ticks: 0,
            release_ticks: 0,
            decay_ticks: 0,
        }
    }

    /// Creates a fixed drum instrument.
    pub const fn drum(drum: SequenceDrum, volume: u8) -> Self {
        Self {
            kind: SequenceInstrumentKind::Drum { drum },
            volume,
            attack_ticks: 0,
            release_ticks: 0,
            decay_ticks: 0,
        }
    }

    /// Creates a reserved built-in slot.
    pub const fn reserved() -> Self {
        Self {
            kind: SequenceInstrumentKind::Reserved,
            volume: 0,
            attack_ticks: 0,
            release_ticks: 0,
            decay_ticks: 0,
        }
    }

    /// Creates an instrument with a linear decay envelope.
    pub const fn with_decay(waveform: SequenceWaveform, volume: u8, decay_ticks: u16) -> Self {
        Self {
            kind: SequenceInstrumentKind::Tone { waveform },
            volume,
            attack_ticks: 0,
            release_ticks: 0,
            decay_ticks,
        }
    }

    /// Creates an instrument with minimal linear attack/release fades.
    pub const fn with_envelope(
        waveform: SequenceWaveform,
        volume: u8,
        attack_ticks: u16,
        release_ticks: u16,
    ) -> Self {
        Self {
            kind: SequenceInstrumentKind::Tone { waveform },
            volume,
            attack_ticks,
            release_ticks,
            decay_ticks: 0,
        }
    }

    /// Small square lead preset for short melodies.
    pub const fn square_lead() -> Self {
        Self::with_envelope(SequenceWaveform::Square, 176, 1, 1)
    }

    /// Softer triangle preset for gentle launcher or puzzle cues.
    pub const fn soft_triangle() -> Self {
        Self::with_envelope(SequenceWaveform::Triangle, 144, 1, 2)
    }

    /// Short square blip preset for compact UI jingles.
    pub const fn short_blip() -> Self {
        Self::with_envelope(SequenceWaveform::Square, 128, 0, 1)
    }

    /// Returns the P/ECE/MusLib-style built-in instrument for an id.
    pub const fn builtin(id: u8) -> Option<Self> {
        match id {
            BUILTIN_INSTRUMENT_SQUARE_FAST => Some(Self::tone(SequenceWaveform::Square, 255)),
            BUILTIN_INSTRUMENT_SAW_FAST => Some(Self::tone(SequenceWaveform::Saw, 255)),
            BUILTIN_INSTRUMENT_TRIANGLE_FAST => Some(Self::tone(SequenceWaveform::Triangle, 255)),
            BUILTIN_INSTRUMENT_SQUARE => Some(Self::tone(SequenceWaveform::Square, 255)),
            BUILTIN_INSTRUMENT_SAW => Some(Self::tone(SequenceWaveform::Saw, 255)),
            BUILTIN_INSTRUMENT_TRIANGLE => Some(Self::tone(SequenceWaveform::Triangle, 255)),
            BUILTIN_INSTRUMENT_BASS_DRUM => Some(Self::drum(SequenceDrum::BassDrum, 255)),
            BUILTIN_INSTRUMENT_SNARE_DRUM_2 => Some(Self::drum(SequenceDrum::SnareDrum2, 255)),
            BUILTIN_INSTRUMENT_SNARE_DRUM_1 => Some(Self::drum(SequenceDrum::SnareDrum1, 255)),
            BUILTIN_INSTRUMENT_OPEN_HI_HAT => Some(Self::drum(SequenceDrum::OpenHiHat, 255)),
            BUILTIN_INSTRUMENT_CLOSED_HI_HAT => Some(Self::drum(SequenceDrum::ClosedHiHat, 255)),
            BUILTIN_INSTRUMENT_CRASH_CYMBAL => Some(Self::drum(SequenceDrum::CrashCymbal, 255)),
            BUILTIN_INSTRUMENT_SYNTH_TOM_HIGH => Some(Self::drum(SequenceDrum::SynthTomHigh, 255)),
            BUILTIN_INSTRUMENT_SYNTH_TOM_MID => Some(Self::drum(SequenceDrum::SynthTomMid, 255)),
            BUILTIN_INSTRUMENT_SYNTH_TOM_LOW => Some(Self::drum(SequenceDrum::SynthTomLow, 255)),
            BUILTIN_INSTRUMENT_CLAP => Some(Self::drum(SequenceDrum::Clap, 255)),
            _ => None,
        }
    }
}

/// P/ECE/MusLib-style built-in instrument table.
///
/// Slot 12 is reserved to preserve the numeric id gap.
pub const BUILTIN_SEQUENCE_INSTRUMENTS: [SequenceInstrument; 17] = [
    SequenceInstrument::tone(SequenceWaveform::Square, 255),
    SequenceInstrument::tone(SequenceWaveform::Saw, 255),
    SequenceInstrument::tone(SequenceWaveform::Triangle, 255),
    SequenceInstrument::tone(SequenceWaveform::Square, 255),
    SequenceInstrument::tone(SequenceWaveform::Saw, 255),
    SequenceInstrument::tone(SequenceWaveform::Triangle, 255),
    SequenceInstrument::drum(SequenceDrum::BassDrum, 255),
    SequenceInstrument::drum(SequenceDrum::SnareDrum2, 255),
    SequenceInstrument::drum(SequenceDrum::SnareDrum1, 255),
    SequenceInstrument::drum(SequenceDrum::OpenHiHat, 255),
    SequenceInstrument::drum(SequenceDrum::ClosedHiHat, 255),
    SequenceInstrument::drum(SequenceDrum::CrashCymbal, 255),
    SequenceInstrument::reserved(),
    SequenceInstrument::drum(SequenceDrum::SynthTomHigh, 255),
    SequenceInstrument::drum(SequenceDrum::SynthTomMid, 255),
    SequenceInstrument::drum(SequenceDrum::SynthTomLow, 255),
    SequenceInstrument::drum(SequenceDrum::Clap, 255),
];

/// Decoder state for the minimal monophonic sequence synth.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SequenceDecoder<'a> {
    sequence: Sequence<'a>,
    sample_rate_hz: u32,
    event_index: usize,
    loop_start: Option<usize>,
    loop_remaining: Option<u8>,
    active: ActiveTone,
    ended: bool,
}

/// Decoder state for the experimental fixed-capacity polyphonic sequence synth.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PolyphonicSequenceDecoder<'a> {
    voices: [Option<PolyphonicVoiceDecoder<'a>>; MAX_SEQUENCE_VOICES],
    active_voice_count: usize,
    ended: bool,
}

impl<'a> PolyphonicSequenceDecoder<'a> {
    pub(crate) fn new(sequence: PolyphonicSequence<'a>, limits: AudioLimits) -> AudioResult<Self> {
        sequence.validate(limits)?;

        let mut voices = [None; MAX_SEQUENCE_VOICES];
        for (slot, voice) in voices.iter_mut().zip(sequence.voices) {
            *slot = Some(PolyphonicVoiceDecoder {
                decoder: SequenceDecoder::new(voice.sequence, limits)?,
                gain: voice.gain,
            });
        }

        Ok(Self {
            voices,
            active_voice_count: sequence.voices.len(),
            ended: false,
        })
    }
}

impl Decoder for PolyphonicSequenceDecoder<'_> {
    #[cfg_attr(feature = "ram-hot-mix", link_section = ".data.koto_audio_mix")]
    fn next_sample(&mut self) -> DecodeResult {
        if self.ended {
            return DecodeResult::End;
        }

        let mut mixed = 0i64;
        let mut emitted_count = 0usize;
        let mut continuing_count = 0usize;

        for voice in self.voices.iter_mut().flatten() {
            match voice.decoder.next_sample() {
                DecodeResult::Sample(sample) => {
                    emitted_count += 1;
                    mixed = mixed.saturating_add(scale_voice_sample(sample, voice.gain));
                }
                DecodeResult::End => {}
            }
            if !voice.decoder.is_ended() {
                continuing_count += 1;
            }
        }

        self.active_voice_count = continuing_count;
        if emitted_count == 0 {
            self.ended = true;
            DecodeResult::End
        } else {
            if continuing_count == 0 {
                self.ended = true;
            }
            DecodeResult::Sample(saturate_i64_to_i16(mixed))
        }
    }

    fn is_ended(&self) -> bool {
        self.ended
    }

    fn completed_loops(&self) -> u32 {
        0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PolyphonicVoiceDecoder<'a> {
    decoder: SequenceDecoder<'a>,
    gain: MixerVolume,
}

impl<'a> SequenceDecoder<'a> {
    pub(crate) fn new(sequence: Sequence<'a>, limits: AudioLimits) -> AudioResult<Self> {
        sequence.validate(limits)?;
        Ok(Self {
            sequence,
            sample_rate_hz: limits.sample_rate_hz,
            event_index: 0,
            loop_start: None,
            loop_remaining: None,
            active: ActiveTone::silent(),
            ended: false,
        })
    }

    fn frames_for_ticks(&self, ticks: u16) -> u32 {
        let frames_per_tick = (self.sample_rate_hz / u32::from(self.sequence.tick_rate_hz)).max(1);
        u32::from(ticks).saturating_mul(frames_per_tick).max(1)
    }

    fn load_next_event(&mut self) {
        while !self.ended && self.active.remaining_frames == 0 {
            let Some(event) = self.sequence.events.get(self.event_index).copied() else {
                self.ended = true;
                break;
            };
            self.event_index = self.event_index.saturating_add(1);

            match event {
                SequenceEvent::Note {
                    pitch,
                    duration_ticks,
                    volume,
                    instrument_id,
                } => {
                    let instrument = self.sequence.instruments[usize::from(instrument_id)];
                    let total_frames = self.frames_for_ticks(duration_ticks);
                    let attack_frames = self.envelope_frames(instrument.attack_ticks, total_frames);
                    let release_frames =
                        self.envelope_frames(instrument.release_ticks, total_frames);
                    self.active = ActiveTone {
                        remaining_frames: total_frames,
                        total_frames,
                        elapsed_frames: 0,
                        phase: 0,
                        phase_step: phase_step(pitch, self.sample_rate_hz),
                        kind: instrument.kind,
                        drum_sample_index: 0,
                        #[cfg(feature = "sldpcm4-drums")]
                        drum_previous_sample: 0,
                        volume: u32::from(volume).saturating_mul(u32::from(instrument.volume)),
                        attack_frames,
                        release_frames,
                        rest: false,
                        scale_env: SCALE_ENV_INVALID,
                        scale_k: 0,
                        scaled_square: 0,
                    };
                }
                SequenceEvent::Rest { duration_ticks } => {
                    let total_frames = self.frames_for_ticks(duration_ticks);
                    self.active = ActiveTone {
                        remaining_frames: total_frames,
                        total_frames,
                        elapsed_frames: 0,
                        rest: true,
                        ..ActiveTone::silent()
                    };
                }
                SequenceEvent::LoopStart => {
                    self.loop_start = Some(self.event_index);
                    self.loop_remaining = None;
                }
                SequenceEvent::LoopEnd { repeat_count } => {
                    if let Some(loop_start) = self.loop_start {
                        if repeat_count == SEQUENCE_REPEAT_INFINITE {
                            self.event_index = loop_start;
                        } else {
                            let remaining = self.loop_remaining.get_or_insert(repeat_count);
                            if *remaining > 0 {
                                *remaining = remaining.saturating_sub(1);
                                self.event_index = loop_start;
                            } else {
                                self.loop_remaining = None;
                            }
                        }
                    } else {
                        self.ended = true;
                    }
                }
                SequenceEvent::End => self.ended = true,
            }
        }
    }

    fn finish_control_events(&mut self) {
        while !self.ended && self.active.remaining_frames == 0 {
            let Some(event) = self.sequence.events.get(self.event_index).copied() else {
                self.ended = true;
                break;
            };

            match event {
                SequenceEvent::LoopStart => {
                    self.event_index = self.event_index.saturating_add(1);
                    self.loop_start = Some(self.event_index);
                    self.loop_remaining = None;
                }
                SequenceEvent::LoopEnd { repeat_count } => {
                    self.event_index = self.event_index.saturating_add(1);
                    if let Some(loop_start) = self.loop_start {
                        if repeat_count == SEQUENCE_REPEAT_INFINITE {
                            self.event_index = loop_start;
                        } else {
                            let remaining = self.loop_remaining.get_or_insert(repeat_count);
                            if *remaining > 0 {
                                *remaining = remaining.saturating_sub(1);
                                self.event_index = loop_start;
                            } else {
                                self.loop_remaining = None;
                            }
                        }
                    } else {
                        self.ended = true;
                    }
                }
                SequenceEvent::End => {
                    self.event_index = self.event_index.saturating_add(1);
                    self.ended = true;
                }
                SequenceEvent::Note { .. } | SequenceEvent::Rest { .. } => break,
            }
        }
    }

    fn envelope_frames(&self, ticks: u16, total_frames: u32) -> u32 {
        if ticks == 0 {
            0
        } else {
            self.frames_for_ticks(ticks).min(total_frames)
        }
    }

    #[allow(dead_code)]
    pub(crate) fn stop_with_release(&mut self) {
        self.load_next_event();
        if self.active.remaining_frames == 0 || self.active.rest {
            self.ended = true;
            return;
        }

        if self.active.release_frames == 0 {
            self.ended = true;
        } else {
            self.active.remaining_frames =
                self.active.remaining_frames.min(self.active.release_frames);
        }
    }
}

impl Decoder for SequenceDecoder<'_> {
    #[cfg_attr(feature = "ram-hot-mix", link_section = ".data.koto_audio_mix")]
    fn next_sample(&mut self) -> DecodeResult {
        self.load_next_event();
        if self.ended {
            return DecodeResult::End;
        }

        let sample = self.active.next_sample();
        self.finish_control_events();
        DecodeResult::Sample(sample)
    }

    fn is_ended(&self) -> bool {
        self.ended
    }

    fn completed_loops(&self) -> u32 {
        0
    }
}

/// Owned polyphonic player for a dynamically compiled runtime cue.
///
/// Unlike [`PolyphonicSequenceDecoder`], this player holds event arrays by
/// value and keeps only indices into them. It is therefore safe to replace
/// from data copied out of PSRAM and never retains a reference to a staging
/// buffer or generated Rust table.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuntimeCuePlayer<const N: usize> {
    cue: RuntimeCue<N>,
    voices: [RuntimeVoiceState; MAX_SEQUENCE_VOICES],
    sample_rate_hz: u32,
    playing: bool,
}

impl<const N: usize> RuntimeCuePlayer<N> {
    /// Creates an idle owned player at the requested output sample rate.
    pub const fn new(sample_rate_hz: u32) -> Self {
        Self {
            cue: RuntimeCue::empty(),
            voices: [RuntimeVoiceState::IDLE; MAX_SEQUENCE_VOICES],
            sample_rate_hz,
            playing: false,
        }
    }

    /// Replaces the current score and starts it from the first event.
    pub fn play(&mut self, cue: RuntimeCue<N>) -> AudioResult<()> {
        if self.sample_rate_hz == 0
            || cue.track_count == 0
            || cue.track_count > MAX_SEQUENCE_VOICES
            || cue.tick_rate_hz == 0
        {
            return Err(AudioError::InvalidArgument);
        }
        for track in &cue.tracks[..cue.track_count] {
            if track.len == 0 || track.events[track.len - 1] != SequenceEvent::End {
                return Err(AudioError::MalformedAsset);
            }
        }
        self.cue = cue;
        self.voices.fill(RuntimeVoiceState::IDLE);
        for voice in &mut self.voices[..self.cue.track_count] {
            voice.ended = false;
        }
        self.playing = true;
        Ok(())
    }

    /// Decodes a pointer-free cue image directly into the owned player and starts it.
    pub fn play_image(&mut self, bytes: &[u8]) -> AudioResult<()> {
        self.stop();
        self.cue
            .decode_into(bytes)
            .map_err(|_| AudioError::MalformedAsset)?;
        if self.cue.track_count == 0 || self.cue.tick_rate_hz == 0 {
            return Err(AudioError::MalformedAsset);
        }
        self.voices.fill(RuntimeVoiceState::IDLE);
        for voice in &mut self.voices[..self.cue.track_count] {
            voice.ended = false;
        }
        self.playing = true;
        Ok(())
    }

    /// Stops the current score immediately.
    pub fn stop(&mut self) {
        self.playing = false;
        self.voices.fill(RuntimeVoiceState::IDLE);
    }

    /// Returns whether at least one voice can still produce samples.
    pub const fn is_playing(&self) -> bool {
        self.playing
    }

    /// Produces the next mixed mono sample, or [`DecodeResult::End`].
    #[cfg_attr(feature = "ram-hot-mix", link_section = ".data.koto_audio_mix")]
    pub fn next_sample(&mut self) -> DecodeResult {
        if !self.playing {
            return DecodeResult::End;
        }
        let mut mixed = 0i64;
        let mut emitted = 0usize;
        let mut continuing = 0usize;
        for index in 0..self.cue.track_count {
            match runtime_voice_next(
                &self.cue.tracks[index],
                self.cue.tick_rate_hz,
                self.sample_rate_hz,
                &mut self.voices[index],
            ) {
                DecodeResult::Sample(sample) => {
                    emitted += 1;
                    mixed = mixed
                        .saturating_add(scale_voice_sample(sample, self.cue.tracks[index].gain));
                }
                DecodeResult::End => {}
            }
            if !self.voices[index].ended {
                continuing += 1;
            }
        }
        if emitted == 0 {
            self.playing = false;
            DecodeResult::End
        } else {
            if continuing == 0 {
                self.playing = false;
            }
            DecodeResult::Sample(saturate_i64_to_i16(mixed))
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RuntimeVoiceState {
    event_index: usize,
    loop_start: Option<usize>,
    loop_remaining: Option<u8>,
    active: ActiveTone,
    ended: bool,
}

impl RuntimeVoiceState {
    const IDLE: Self = Self {
        event_index: 0,
        loop_start: None,
        loop_remaining: None,
        active: ActiveTone::silent(),
        ended: true,
    };
}

fn runtime_voice_next<const N: usize>(
    track: &crate::RuntimeCueTrack<N>,
    tick_rate_hz: u16,
    sample_rate_hz: u32,
    state: &mut RuntimeVoiceState,
) -> DecodeResult {
    while !state.ended && state.active.remaining_frames == 0 {
        let Some(event) = track.events.get(state.event_index).copied() else {
            state.ended = true;
            break;
        };
        state.event_index = state.event_index.saturating_add(1);
        match event {
            SequenceEvent::Note {
                pitch,
                duration_ticks,
                volume,
                instrument_id,
            } => {
                let Some(instrument) = BUILTIN_SEQUENCE_INSTRUMENTS.get(usize::from(instrument_id))
                else {
                    state.ended = true;
                    break;
                };
                let total_frames =
                    runtime_frames_for_ticks(sample_rate_hz, tick_rate_hz, duration_ticks);
                let envelope_frames = |ticks: u16| {
                    if ticks == 0 {
                        0
                    } else {
                        runtime_frames_for_ticks(sample_rate_hz, tick_rate_hz, ticks)
                            .min(total_frames)
                    }
                };
                state.active = ActiveTone {
                    remaining_frames: total_frames,
                    total_frames,
                    elapsed_frames: 0,
                    phase: 0,
                    phase_step: phase_step(pitch, sample_rate_hz),
                    kind: instrument.kind,
                    drum_sample_index: 0,
                    #[cfg(feature = "sldpcm4-drums")]
                    drum_previous_sample: 0,
                    volume: u32::from(volume).saturating_mul(u32::from(instrument.volume)),
                    attack_frames: envelope_frames(instrument.attack_ticks),
                    release_frames: envelope_frames(instrument.release_ticks),
                    rest: false,
                    scale_env: SCALE_ENV_INVALID,
                    scale_k: 0,
                    scaled_square: 0,
                };
            }
            SequenceEvent::Rest { duration_ticks } => {
                let total_frames =
                    runtime_frames_for_ticks(sample_rate_hz, tick_rate_hz, duration_ticks);
                state.active = ActiveTone {
                    remaining_frames: total_frames,
                    total_frames,
                    elapsed_frames: 0,
                    rest: true,
                    ..ActiveTone::silent()
                };
            }
            SequenceEvent::LoopStart => {
                state.loop_start = Some(state.event_index);
                state.loop_remaining = None;
            }
            SequenceEvent::LoopEnd { repeat_count } => {
                runtime_apply_loop(state, repeat_count);
            }
            SequenceEvent::End => state.ended = true,
        }
    }
    if state.ended {
        return DecodeResult::End;
    }
    let sample = state.active.next_sample();
    while !state.ended && state.active.remaining_frames == 0 {
        let Some(event) = track.events.get(state.event_index).copied() else {
            state.ended = true;
            break;
        };
        match event {
            SequenceEvent::LoopStart => {
                state.event_index += 1;
                state.loop_start = Some(state.event_index);
                state.loop_remaining = None;
            }
            SequenceEvent::LoopEnd { repeat_count } => {
                state.event_index += 1;
                runtime_apply_loop(state, repeat_count);
            }
            SequenceEvent::End => {
                state.event_index += 1;
                state.ended = true;
            }
            SequenceEvent::Note { .. } | SequenceEvent::Rest { .. } => break,
        }
    }
    DecodeResult::Sample(sample)
}

fn runtime_apply_loop(state: &mut RuntimeVoiceState, repeat_count: u8) {
    let Some(loop_start) = state.loop_start else {
        state.ended = true;
        return;
    };
    if repeat_count == SEQUENCE_REPEAT_INFINITE {
        state.event_index = loop_start;
    } else {
        let remaining = state.loop_remaining.get_or_insert(repeat_count);
        if *remaining > 0 {
            *remaining = remaining.saturating_sub(1);
            state.event_index = loop_start;
        } else {
            state.loop_remaining = None;
        }
    }
}

fn runtime_frames_for_ticks(sample_rate_hz: u32, tick_rate_hz: u16, ticks: u16) -> u32 {
    let frames_per_tick = (sample_rate_hz / u32::from(tick_rate_hz)).max(1);
    u32::from(ticks).saturating_mul(frames_per_tick).max(1)
}

/// The square oscillator's raw half-cycle amplitude.
const SQUARE_AMPLITUDE: i64 = 8192;
/// Exact denominator of the tone scale chain: note volume (0-255) ×
/// instrument volume (0-255) × envelope (0-255).
const TONE_SCALE_DIVISOR: i64 = 255 * 255 * 255;
/// Sentinel for [`ActiveTone::scale_env`]: no cached scale yet. Envelope
/// values are 0-255, so this can never collide with a real one.
const SCALE_ENV_INVALID: u32 = u32::MAX;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ActiveTone {
    remaining_frames: u32,
    total_frames: u32,
    elapsed_frames: u32,
    phase: u32,
    phase_step: u32,
    kind: SequenceInstrumentKind,
    drum_sample_index: usize,
    /// SLDPCM4 drum predictor: the previously decoded sample.
    #[cfg(feature = "sldpcm4-drums")]
    drum_previous_sample: i16,
    volume: u32,
    attack_frames: u32,
    release_frames: u32,
    rest: bool,
    /// The envelope value `scale_k`/`scaled_square` were computed for
    /// ([`SCALE_ENV_INVALID`] until the first scaled sample). The envelope is
    /// piecewise-constant (it only steps while an attack/release ramp runs;
    /// sustain holds 255), so this cache turns the per-sample 64-bit scale
    /// chain into a compare on the sustain path.
    scale_env: u32,
    /// Cached `volume * envelope` (≤ 255² × 255, fits u32) — the numerator
    /// factor shared by every waveform's scale step.
    scale_k: u32,
    /// Cached scaled output of the square wave's positive half-cycle. The
    /// square emits only ±[`SQUARE_AMPLITUDE`], so with the envelope cached
    /// its samples need no per-sample multiply/divide at all — on Cortex-M0+
    /// those lower to flash-resident compiler-rt calls, which CPU0's XIP
    /// traffic makes expensive (KotoRun SMASH underruns, phase=173).
    scaled_square: i16,
}

impl ActiveTone {
    const fn silent() -> Self {
        Self {
            remaining_frames: 0,
            total_frames: 0,
            elapsed_frames: 0,
            phase: 0,
            phase_step: 0,
            kind: SequenceInstrumentKind::Tone {
                waveform: SequenceWaveform::Square,
            },
            drum_sample_index: 0,
            #[cfg(feature = "sldpcm4-drums")]
            drum_previous_sample: 0,
            volume: 0,
            attack_frames: 0,
            release_frames: 0,
            rest: true,
            scale_env: SCALE_ENV_INVALID,
            scale_k: 0,
            scaled_square: 0,
        }
    }

    #[cfg_attr(feature = "ram-hot-mix", link_section = ".data.koto_audio_mix")]
    fn next_sample(&mut self) -> i16 {
        if self.remaining_frames == 0 {
            return 0;
        }

        self.remaining_frames = self.remaining_frames.saturating_sub(1);
        let elapsed = self.elapsed_frames;
        self.elapsed_frames = self.elapsed_frames.saturating_add(1);

        if self.rest || self.volume == 0 {
            return 0;
        }

        // Refresh the piecewise-constant scale caches only when the envelope
        // actually stepped (attack/release ramps; sustain holds 255). On the
        // sustain path this reduces the scale chain to one compare, and the
        // square wave below to a cached value — bit-identical to the previous
        // per-sample `raw * volume * envelope / 255³` chain (`volume` ≤ 255²
        // and `envelope` ≤ 255, so no intermediate ever overflowed there and
        // the product is associative).
        let envelope = self.envelope(elapsed);
        if envelope != self.scale_env {
            self.scale_env = envelope;
            self.scale_k = self.volume.saturating_mul(envelope);
            self.scaled_square =
                ((SQUARE_AMPLITUDE * i64::from(self.scale_k)) / TONE_SCALE_DIVISOR) as i16;
        }

        let raw = match self.kind {
            SequenceInstrumentKind::Tone { waveform } => {
                if self.phase_step == 0 {
                    return 0;
                }
                if matches!(waveform, SequenceWaveform::Square) {
                    // ±SQUARE_AMPLITUDE scale to ±scaled_square exactly
                    // (truncating division is symmetric around zero), so the
                    // square path emits the cache with no per-sample math.
                    let sample = if self.phase < 0x8000_0000 {
                        self.scaled_square
                    } else {
                        -self.scaled_square
                    };
                    self.phase = self.phase.wrapping_add(self.phase_step);
                    return sample;
                }
                let sample = match waveform {
                    SequenceWaveform::Square => unreachable!("handled above"),
                    SequenceWaveform::Saw => saw_sample(self.phase),
                    SequenceWaveform::Triangle => triangle_sample(self.phase),
                };
                self.phase = self.phase.wrapping_add(self.phase_step);
                sample
            }
            #[cfg(not(feature = "sldpcm4-drums"))]
            SequenceInstrumentKind::Drum { drum } => {
                let samples = drum.samples();
                let sample = if let Some(sample) = samples.get(self.drum_sample_index) {
                    *sample
                } else {
                    0
                };
                if self.drum_sample_index < samples.len() {
                    self.drum_sample_index += 1;
                }
                sample
            }
            #[cfg(feature = "sldpcm4-drums")]
            SequenceInstrumentKind::Drum { drum } => {
                // Incremental SLDPCM4 decode, matching `Sldpcm4Decoder`: sample
                // `i` is the high nibble of payload byte `i / 2` when `i` is
                // even, else the low nibble; each nibble is a delta added to
                // the previous decoded sample with saturation.
                let table = drum.sldpcm4_samples();
                let index = self.drum_sample_index;
                if (index as u32) < table.sample_count {
                    let sample = if let Some(byte) = table.payload.get(index / 2) {
                        let nibble = if index % 2 == 0 {
                            byte >> 4
                        } else {
                            byte & 0x0f
                        };
                        self.drum_previous_sample
                            .saturating_add(crate::codec::SLDPCM4_DELTAS_V0[usize::from(nibble)])
                    } else {
                        0
                    };
                    self.drum_previous_sample = sample;
                    self.drum_sample_index += 1;
                    sample
                } else {
                    0
                }
            }
            SequenceInstrumentKind::Reserved => 0,
        };

        // One 64-bit multiply + one constant divide per sample (saw/triangle/
        // drums); `raw` ≤ 2^15 and `scale_k` ≤ 255² × 255, so the product is
        // exactly the old `raw * volume * envelope` (≤ 2^39, no saturation).
        let scaled = (i64::from(raw) * i64::from(self.scale_k)) / TONE_SCALE_DIVISOR;
        scaled as i16
    }

    fn envelope(&self, elapsed: u32) -> u32 {
        let attack = if self.attack_frames == 0 || elapsed >= self.attack_frames {
            255
        } else {
            elapsed.saturating_mul(255) / self.attack_frames
        };

        let release = if self.release_frames == 0 {
            255
        } else {
            let frames_left = self.remaining_frames.saturating_add(1);
            if frames_left >= self.release_frames {
                255
            } else {
                frames_left.saturating_mul(255) / self.release_frames
            }
        };

        attack.min(release)
    }
}

const fn phase_step(pitch_hz: u16, sample_rate_hz: u32) -> u32 {
    if pitch_hz == 0 || sample_rate_hz == 0 {
        return 0;
    }

    (((pitch_hz as u64) << 32) / sample_rate_hz as u64) as u32
}

fn triangle_sample(phase: u32) -> i16 {
    let quarter = phase >> 30;
    let fraction = ((phase >> 15) & 0x7fff) as i32;
    let value = match quarter {
        0 => fraction,
        1 => 32767 - fraction,
        2 => -fraction,
        _ => -32767 + fraction,
    };
    (value / 4) as i16
}

fn saw_sample(phase: u32) -> i16 {
    let centered = ((phase >> 17) as i32) - 16384;
    (centered / 2) as i16
}

fn scale_voice_sample(sample: i16, gain: MixerVolume) -> i64 {
    i64::from(sample) * i64::from(gain.get()) / i64::from(MixerVolume::UNITY.get())
}

fn scale_u8(value: u8, gain: u8) -> u8 {
    ((u16::from(value) * u16::from(gain)) / 255) as u8
}

fn saturate_i64_to_i16(value: i64) -> i16 {
    if value > i64::from(i16::MAX) {
        i16::MAX
    } else if value < i64::from(i16::MIN) {
        i16::MIN
    } else {
        value as i16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMITS: AudioLimits = AudioLimits {
        sample_rate_hz: 16,
        block_frames: 4,
        max_sfx_sources: 1,
        source_queue_depth: 1,
        event_queue_depth: 4,
    };
    const INST: [SequenceInstrument; 1] = [SequenceInstrument::new(SequenceWaveform::Square, 255)];
    const ENV_INST: [SequenceInstrument; 1] = [SequenceInstrument::with_envelope(
        SequenceWaveform::Square,
        255,
        1,
        1,
    )];

    #[test]
    fn midi_pitch_helper_rounds_known_notes_and_sample_step() {
        assert_eq!(SequencePitch::A4.frequency_hz, 440);
        assert_eq!(SequencePitch::from_midi_note(60).frequency_hz, 262);
        assert_eq!(SequencePitch::from_midi_note(72).frequency_hz, 523);
        assert_eq!(
            SequencePitch::A4.sample_step(44_000),
            phase_step(440, 44_000)
        );
        assert_eq!(SequencePitch::from_hz(0).sample_step(44_000), 0);
    }

    #[test]
    fn tempo_helper_derives_tick_rate_and_note_lengths() {
        const TEMPO: SequenceTempo = SequenceTempo::from_bpm(120, 4);

        assert_eq!(TEMPO.tick_rate_hz(), 8);
        assert_eq!(TEMPO.beat(), 4);
        assert_eq!(TEMPO.quarter(), 4);
        assert_eq!(TEMPO.eighth(), 2);
        assert_eq!(TEMPO.half(), 8);
        assert_eq!(TEMPO.whole(), 16);
    }

    #[test]
    fn note_and_rest_constructors_preserve_event_layout() {
        assert_eq!(
            SequenceEvent::note(SequencePitch::C4, 3, 200, 1),
            SequenceEvent::Note {
                pitch: 262,
                duration_ticks: 3,
                volume: 200,
                instrument_id: 1,
            }
        );
        assert_eq!(
            SequenceEvent::note_hz(330, 2, 180, 0),
            SequenceEvent::Note {
                pitch: 330,
                duration_ticks: 2,
                volume: 180,
                instrument_id: 0,
            }
        );
        assert_eq!(
            SequenceEvent::rest(5),
            SequenceEvent::Rest { duration_ticks: 5 }
        );
    }

    #[test]
    fn sequence_can_be_created_with_tempo_helper() {
        const TEMPO: SequenceTempo = SequenceTempo::from_bpm(120, 4);
        const EVENTS: [SequenceEvent; 2] =
            [SequenceEvent::rest(TEMPO.quarter()), SequenceEvent::End];
        const SEQ: Sequence<'static> = Sequence::with_tempo(&EVENTS, &INST, TEMPO);

        assert_eq!(SEQ.tick_rate_hz, 8);
        assert_eq!(SEQ.events, &EVENTS);
        assert_eq!(SEQ.instruments, &INST);
    }

    #[test]
    fn note_sequence_produces_non_silence() {
        const EVENTS: [SequenceEvent; 2] = [
            SequenceEvent::Note {
                pitch: 4,
                duration_ticks: 1,
                volume: 255,
                instrument_id: 0,
            },
            SequenceEvent::End,
        ];
        let mut decoder = SequenceDecoder::new(Sequence::new(&EVENTS, &INST, 4), LIMITS).unwrap();

        let mut out = [0; 4];
        assert_eq!(decoder.read_samples(&mut out), 4);
        assert!(out.iter().any(|sample| *sample != 0));
    }

    #[test]
    fn preset_instruments_produce_non_silence() {
        const PRESET_INSTRUMENTS: [SequenceInstrument; 3] = [
            SequenceInstrument::square_lead(),
            SequenceInstrument::soft_triangle(),
            SequenceInstrument::short_blip(),
        ];

        let mut instrument_id = 0u8;
        while usize::from(instrument_id) < PRESET_INSTRUMENTS.len() {
            let events = [
                SequenceEvent::note(SequencePitch::from_hz(4), 2, 255, instrument_id),
                SequenceEvent::End,
            ];
            let mut decoder =
                SequenceDecoder::new(Sequence::new(&events, &PRESET_INSTRUMENTS, 4), LIMITS)
                    .unwrap();
            let mut out = [0; 8];
            assert_eq!(decoder.read_samples(&mut out), 8);
            assert!(out.iter().any(|sample| *sample != 0));
            instrument_id += 1;
        }
    }

    #[test]
    fn rest_sequence_produces_silence() {
        const EVENTS: [SequenceEvent; 2] = [
            SequenceEvent::Rest { duration_ticks: 1 },
            SequenceEvent::End,
        ];
        let mut decoder = SequenceDecoder::new(Sequence::new(&EVENTS, &INST, 4), LIMITS).unwrap();

        let mut out = [1; 4];
        assert_eq!(decoder.read_samples(&mut out), 4);
        assert_eq!(out, [0, 0, 0, 0]);
    }

    #[test]
    fn triangle_note_sequence_produces_non_silence() {
        const TRIANGLE_INST: [SequenceInstrument; 1] =
            [SequenceInstrument::new(SequenceWaveform::Triangle, 255)];
        const EVENTS: [SequenceEvent; 2] = [
            SequenceEvent::Note {
                pitch: 4,
                duration_ticks: 1,
                volume: 255,
                instrument_id: 0,
            },
            SequenceEvent::End,
        ];
        let mut decoder =
            SequenceDecoder::new(Sequence::new(&EVENTS, &TRIANGLE_INST, 4), LIMITS).unwrap();

        let mut out = [0; 4];
        assert_eq!(decoder.read_samples(&mut out), 4);
        assert!(out.iter().any(|sample| *sample != 0));
    }

    #[test]
    fn builtin_tone_ids_produce_non_silence() {
        const TONE_IDS: [u8; 6] = [
            BUILTIN_INSTRUMENT_SQUARE_FAST,
            BUILTIN_INSTRUMENT_SAW_FAST,
            BUILTIN_INSTRUMENT_TRIANGLE_FAST,
            BUILTIN_INSTRUMENT_SQUARE,
            BUILTIN_INSTRUMENT_SAW,
            BUILTIN_INSTRUMENT_TRIANGLE,
        ];

        for instrument_id in TONE_IDS {
            let events = [
                SequenceEvent::note_hz(4, 1, 255, instrument_id),
                SequenceEvent::End,
            ];
            let mut decoder = SequenceDecoder::new(
                Sequence::new(&events, &BUILTIN_SEQUENCE_INSTRUMENTS, 4),
                LIMITS,
            )
            .unwrap();
            let mut out = [0; 4];
            assert_eq!(decoder.read_samples(&mut out), 4);
            assert!(out.iter().any(|sample| *sample != 0));
        }
    }

    #[test]
    fn drum_builtin_ignores_pitch_and_produces_non_silence() {
        const EVENTS_LOW: [SequenceEvent; 2] = [
            SequenceEvent::note_hz(1, 1, 255, BUILTIN_INSTRUMENT_BASS_DRUM),
            SequenceEvent::End,
        ];
        const EVENTS_HIGH: [SequenceEvent; 2] = [
            SequenceEvent::note_hz(15, 1, 255, BUILTIN_INSTRUMENT_BASS_DRUM),
            SequenceEvent::End,
        ];
        let mut low = SequenceDecoder::new(
            Sequence::new(&EVENTS_LOW, &BUILTIN_SEQUENCE_INSTRUMENTS, 4),
            LIMITS,
        )
        .unwrap();
        let mut high = SequenceDecoder::new(
            Sequence::new(&EVENTS_HIGH, &BUILTIN_SEQUENCE_INSTRUMENTS, 4),
            LIMITS,
        )
        .unwrap();

        let mut low_out = [0; 4];
        let mut high_out = [0; 4];
        assert_eq!(low.read_samples(&mut low_out), 4);
        assert_eq!(high.read_samples(&mut high_out), 4);

        assert_eq!(low_out, high_out);
        assert!(low_out.iter().any(|sample| *sample != 0));
    }

    #[cfg(not(feature = "sldpcm4-drums"))]
    #[test]
    fn builtin_drum_representation_is_static_pcm16_mono_data() {
        assert_eq!(crate::builtin_drums::BUILTIN_DRUM_SAMPLE_RATE_HZ, 16_000);
        let bass = SequenceDrum::BassDrum.samples();
        let closed_hat = SequenceDrum::ClosedHiHat.samples();

        assert_eq!(bass, crate::builtin_drums_generated::BUILTIN_DRUM_BASS_DRUM);
        assert_eq!(
            closed_hat,
            crate::builtin_drums_generated::BUILTIN_DRUM_CLOSED_HI_HAT
        );
        assert_eq!(core::mem::size_of_val(&bass[0]), 2);
        assert!(bass.iter().any(|sample| *sample != 0));
        assert_eq!(
            SequenceDrum::SnareDrum2.samples(),
            crate::builtin_drums_generated::BUILTIN_DRUM_SNARE_DRUM_1
        );
        assert_eq!(
            SequenceDrum::Clap.samples(),
            crate::builtin_drums_generated::BUILTIN_DRUM_CLAP
        );
    }

    #[cfg(feature = "sldpcm4-drums")]
    #[test]
    fn builtin_drum_representation_is_sldpcm4_payload_data() {
        assert_eq!(crate::builtin_drums::BUILTIN_DRUM_SAMPLE_RATE_HZ, 16_000);
        let bass = SequenceDrum::BassDrum.sldpcm4_samples();

        // Two samples per payload byte (the final byte may pad a nibble).
        assert_eq!(bass.payload.len(), (bass.sample_count as usize).div_ceil(2));
        assert!(bass.sample_count > 0);
        assert_eq!(
            SequenceDrum::SnareDrum2.sldpcm4_samples().payload,
            SequenceDrum::SnareDrum1.sldpcm4_samples().payload
        );
    }

    #[test]
    fn drum_builtin_is_silent_after_sample_end() {
        const EVENTS: [SequenceEvent; 2] = [
            SequenceEvent::note_hz(8, 5, 255, BUILTIN_INSTRUMENT_CLOSED_HI_HAT),
            SequenceEvent::End,
        ];
        let mut decoder = SequenceDecoder::new(
            Sequence::new(&EVENTS, &BUILTIN_SEQUENCE_INSTRUMENTS, 4),
            LIMITS,
        )
        .unwrap();

        let mut out = [1; 20];
        assert_eq!(decoder.read_samples(&mut out), 20);

        assert!(out.iter().any(|sample| *sample != 0));
    }

    #[test]
    fn polyphonic_sequence_can_mix_tone_and_drum() {
        const TONE_EVENTS: [SequenceEvent; 2] = [
            SequenceEvent::note_hz(4, 1, 255, BUILTIN_INSTRUMENT_SQUARE),
            SequenceEvent::End,
        ];
        const DRUM_EVENTS: [SequenceEvent; 2] = [
            SequenceEvent::note_hz(1, 1, 255, BUILTIN_INSTRUMENT_SNARE_DRUM_1),
            SequenceEvent::End,
        ];
        const VOICES: [PolyphonicSequenceVoice<'static>; 2] = [
            PolyphonicSequenceVoice::unity(Sequence::new(
                &TONE_EVENTS,
                &BUILTIN_SEQUENCE_INSTRUMENTS,
                4,
            )),
            PolyphonicSequenceVoice::unity(Sequence::new(
                &DRUM_EVENTS,
                &BUILTIN_SEQUENCE_INSTRUMENTS,
                4,
            )),
        ];
        let mut decoder =
            PolyphonicSequenceDecoder::new(PolyphonicSequence::new(&VOICES), LIMITS).unwrap();

        let mut out = [0; 4];
        assert_eq!(decoder.read_samples(&mut out), 4);

        assert!(out.iter().any(|sample| *sample != 0));
    }

    #[test]
    fn attack_fade_in_starts_below_steady_state() {
        const EVENTS: [SequenceEvent; 2] = [
            SequenceEvent::Note {
                pitch: 1,
                duration_ticks: 2,
                volume: 255,
                instrument_id: 0,
            },
            SequenceEvent::End,
        ];
        let mut decoder =
            SequenceDecoder::new(Sequence::new(&EVENTS, &ENV_INST, 4), LIMITS).unwrap();

        let mut out = [0; 8];
        assert_eq!(decoder.read_samples(&mut out), 8);

        assert_eq!(out[0], 0);
        assert!(out[1].abs() < out[4].abs());
        assert!(out[2].abs() < out[4].abs());
        assert_ne!(out[4], 0);
    }

    #[test]
    fn release_fade_out_approaches_silence_before_rest() {
        const EVENTS: [SequenceEvent; 3] = [
            SequenceEvent::Note {
                pitch: 1,
                duration_ticks: 2,
                volume: 255,
                instrument_id: 0,
            },
            SequenceEvent::Rest { duration_ticks: 1 },
            SequenceEvent::End,
        ];
        let mut decoder =
            SequenceDecoder::new(Sequence::new(&EVENTS, &ENV_INST, 4), LIMITS).unwrap();

        let mut out = [1; 12];
        assert_eq!(decoder.read_samples(&mut out), 12);

        assert!(out[7].abs() < out[5].abs());
        assert_eq!(&out[8..], &[0, 0, 0, 0]);
    }

    #[test]
    fn decoder_stop_can_fade_out_without_immediate_click() {
        const EVENTS: [SequenceEvent; 2] = [
            SequenceEvent::Note {
                pitch: 1,
                duration_ticks: 4,
                volume: 255,
                instrument_id: 0,
            },
            SequenceEvent::End,
        ];
        let mut decoder =
            SequenceDecoder::new(Sequence::new(&EVENTS, &ENV_INST, 4), LIMITS).unwrap();

        let mut lead_in = [0; 5];
        assert_eq!(decoder.read_samples(&mut lead_in), 5);
        assert_ne!(lead_in[4], 0);
        decoder.stop_with_release();

        let mut out = [0; 4];
        assert_eq!(decoder.read_samples(&mut out), 4);

        assert!(out[0].abs() >= out[1].abs());
        assert!(out[1].abs() >= out[2].abs());
        assert!(out[2].abs() >= out[3].abs());
        assert_eq!(decoder.next_sample(), DecodeResult::End);
    }

    #[test]
    fn finite_loop_repeats_then_ends() {
        const EVENTS: [SequenceEvent; 5] = [
            SequenceEvent::LoopStart,
            SequenceEvent::Rest { duration_ticks: 1 },
            SequenceEvent::LoopEnd { repeat_count: 1 },
            SequenceEvent::Rest { duration_ticks: 1 },
            SequenceEvent::End,
        ];
        let mut decoder = SequenceDecoder::new(Sequence::new(&EVENTS, &INST, 4), LIMITS).unwrap();

        let mut out = [1; 12];
        assert_eq!(decoder.read_samples(&mut out), 12);
        assert_eq!(decoder.next_sample(), DecodeResult::End);
    }

    #[test]
    fn one_voice_polyphonic_sequence_matches_monophonic_sequence() {
        const EVENTS: [SequenceEvent; 2] =
            [SequenceEvent::note_hz(4, 1, 255, 0), SequenceEvent::End];
        const SEQUENCE: Sequence<'static> = Sequence::new(&EVENTS, &INST, 4);
        const VOICES: [PolyphonicSequenceVoice<'static>; 1] =
            [PolyphonicSequenceVoice::unity(SEQUENCE)];
        let mut mono = SequenceDecoder::new(SEQUENCE, LIMITS).unwrap();
        let mut poly =
            PolyphonicSequenceDecoder::new(PolyphonicSequence::new(&VOICES), LIMITS).unwrap();

        let mut mono_out = [0; 4];
        let mut poly_out = [0; 4];
        assert_eq!(mono.read_samples(&mut mono_out), 4);
        assert_eq!(poly.read_samples(&mut poly_out), 4);

        assert_eq!(poly_out, mono_out);
        assert!(poly_out.iter().any(|sample| *sample != 0));
    }

    #[test]
    fn four_voice_polyphonic_sequence_generates_non_silence() {
        const EVENTS_A: [SequenceEvent; 2] =
            [SequenceEvent::note_hz(4, 1, 255, 0), SequenceEvent::End];
        const EVENTS_B: [SequenceEvent; 2] =
            [SequenceEvent::note_hz(5, 1, 255, 0), SequenceEvent::End];
        const EVENTS_C: [SequenceEvent; 2] =
            [SequenceEvent::note_hz(6, 1, 255, 0), SequenceEvent::End];
        const EVENTS_D: [SequenceEvent; 2] =
            [SequenceEvent::note_hz(7, 1, 255, 0), SequenceEvent::End];
        const VOICES: [PolyphonicSequenceVoice<'static>; MAX_SEQUENCE_VOICES] = [
            PolyphonicSequenceVoice::unity(Sequence::new(&EVENTS_A, &INST, 4)),
            PolyphonicSequenceVoice::unity(Sequence::new(&EVENTS_B, &INST, 4)),
            PolyphonicSequenceVoice::unity(Sequence::new(&EVENTS_C, &INST, 4)),
            PolyphonicSequenceVoice::unity(Sequence::new(&EVENTS_D, &INST, 4)),
        ];
        let mut decoder =
            PolyphonicSequenceDecoder::new(PolyphonicSequence::new(&VOICES), LIMITS).unwrap();

        let mut out = [0; 4];
        assert_eq!(decoder.read_samples(&mut out), 4);

        assert!(out.iter().any(|sample| *sample != 0));
    }

    #[test]
    fn polyphonic_mix_saturates_without_wrapping() {
        const EVENTS: [SequenceEvent; 2] =
            [SequenceEvent::note_hz(4, 1, 255, 0), SequenceEvent::End];
        const SEQUENCE: Sequence<'static> = Sequence::new(&EVENTS, &INST, 4);
        const HOT_GAIN: MixerVolume = MixerVolume::new(2048);
        const VOICES: [PolyphonicSequenceVoice<'static>; MAX_SEQUENCE_VOICES] = [
            PolyphonicSequenceVoice::new(SEQUENCE, HOT_GAIN),
            PolyphonicSequenceVoice::new(SEQUENCE, HOT_GAIN),
            PolyphonicSequenceVoice::new(SEQUENCE, HOT_GAIN),
            PolyphonicSequenceVoice::new(SEQUENCE, HOT_GAIN),
        ];
        let mut decoder =
            PolyphonicSequenceDecoder::new(PolyphonicSequence::new(&VOICES), LIMITS).unwrap();

        let mut out = [0; 4];
        assert_eq!(decoder.read_samples(&mut out), 4);

        assert_eq!(out, [i16::MAX, i16::MAX, i16::MIN, i16::MIN]);
    }

    #[test]
    fn polyphonic_sequence_rejects_more_than_four_voices() {
        const EVENTS: [SequenceEvent; 2] = [SequenceEvent::rest(1), SequenceEvent::End];
        const SEQUENCE: Sequence<'static> = Sequence::new(&EVENTS, &INST, 4);
        const VOICES: [PolyphonicSequenceVoice<'static>; 5] = [
            PolyphonicSequenceVoice::unity(SEQUENCE),
            PolyphonicSequenceVoice::unity(SEQUENCE),
            PolyphonicSequenceVoice::unity(SEQUENCE),
            PolyphonicSequenceVoice::unity(SEQUENCE),
            PolyphonicSequenceVoice::unity(SEQUENCE),
        ];

        assert_eq!(
            PolyphonicSequence::new(&VOICES).validate(LIMITS),
            Err(AudioError::InvalidArgument)
        );
    }

    #[test]
    fn valid_compact_sequence_passes_validation() {
        const COMPACT_INSTRUMENTS: [CompactInstrument; 1] =
            [CompactInstrument::builtin(BUILTIN_INSTRUMENT_SQUARE, 255)];
        const COMPACT_EVENTS: [CompactEvent; 2] = [
            CompactEvent::Note {
                pitch: 4,
                duration_ticks: 1,
                volume: 200,
                instrument_id: 0,
            },
            CompactEvent::End,
        ];
        const TRACKS: [CompactTrack<'static>; 1] =
            [CompactTrack::new(&COMPACT_EVENTS, MixerVolume::UNITY, 0)];
        const COMPACT: CompactSequence<'static> = CompactSequence::new(
            &COMPACT_INSTRUMENTS,
            &TRACKS,
            CompactTempo::from_tick_rate_hz(4),
        );

        assert_eq!(validate_compact_sequence(COMPACT), Ok(()));
        assert_eq!(COMPACT.validate(), Ok(COMPACT));
    }

    #[test]
    fn zero_compact_tracks_rejected() {
        const COMPACT_INSTRUMENTS: [CompactInstrument; 1] =
            [CompactInstrument::builtin(BUILTIN_INSTRUMENT_SQUARE, 255)];
        const TRACKS: [CompactTrack<'static>; 0] = [];
        const COMPACT: CompactSequence<'static> = CompactSequence::new(
            &COMPACT_INSTRUMENTS,
            &TRACKS,
            CompactTempo::from_tick_rate_hz(4),
        );

        assert_eq!(
            validate_compact_sequence(COMPACT),
            Err(CompactSequenceError::ZeroTracks)
        );
    }

    #[test]
    fn too_many_compact_tracks_rejected() {
        const COMPACT_INSTRUMENTS: [CompactInstrument; 1] =
            [CompactInstrument::builtin(BUILTIN_INSTRUMENT_SQUARE, 255)];
        const COMPACT_EVENTS: [CompactEvent; 2] =
            [CompactEvent::Rest { duration_ticks: 1 }, CompactEvent::End];
        const TRACK: CompactTrack<'static> =
            CompactTrack::new(&COMPACT_EVENTS, MixerVolume::UNITY, 0);
        const TRACKS: [CompactTrack<'static>; MAX_SEQUENCE_VOICES + 1] =
            [TRACK, TRACK, TRACK, TRACK, TRACK];
        const COMPACT: CompactSequence<'static> = CompactSequence::new(
            &COMPACT_INSTRUMENTS,
            &TRACKS,
            CompactTempo::from_tick_rate_hz(4),
        );

        assert_eq!(
            validate_compact_sequence(COMPACT),
            Err(CompactSequenceError::TooManyTracks)
        );
    }

    #[test]
    fn unknown_compact_builtin_instrument_id_rejected() {
        const COMPACT_INSTRUMENTS: [CompactInstrument; 1] = [CompactInstrument::builtin(12, 255)];
        const COMPACT_EVENTS: [CompactEvent; 2] =
            [CompactEvent::Rest { duration_ticks: 1 }, CompactEvent::End];
        const TRACKS: [CompactTrack<'static>; 1] =
            [CompactTrack::new(&COMPACT_EVENTS, MixerVolume::UNITY, 0)];
        const COMPACT: CompactSequence<'static> = CompactSequence::new(
            &COMPACT_INSTRUMENTS,
            &TRACKS,
            CompactTempo::from_tick_rate_hz(4),
        );

        assert_eq!(
            validate_compact_sequence(COMPACT),
            Err(CompactSequenceError::UnknownBuiltinInstrument)
        );
    }

    #[test]
    fn zero_duration_compact_note_and_rest_rejected() {
        const COMPACT_INSTRUMENTS: [CompactInstrument; 1] =
            [CompactInstrument::builtin(BUILTIN_INSTRUMENT_SQUARE, 255)];
        const NOTE_EVENTS: [CompactEvent; 2] = [
            CompactEvent::Note {
                pitch: 4,
                duration_ticks: 0,
                volume: 200,
                instrument_id: 0,
            },
            CompactEvent::End,
        ];
        const REST_EVENTS: [CompactEvent; 2] =
            [CompactEvent::Rest { duration_ticks: 0 }, CompactEvent::End];
        const NOTE_TRACKS: [CompactTrack<'static>; 1] =
            [CompactTrack::new(&NOTE_EVENTS, MixerVolume::UNITY, 0)];
        const REST_TRACKS: [CompactTrack<'static>; 1] =
            [CompactTrack::new(&REST_EVENTS, MixerVolume::UNITY, 0)];

        assert_eq!(
            validate_compact_sequence(CompactSequence::new(
                &COMPACT_INSTRUMENTS,
                &NOTE_TRACKS,
                CompactTempo::from_tick_rate_hz(4),
            )),
            Err(CompactSequenceError::ZeroDuration)
        );
        assert_eq!(
            validate_compact_sequence(CompactSequence::new(
                &COMPACT_INSTRUMENTS,
                &REST_TRACKS,
                CompactTempo::from_tick_rate_hz(4),
            )),
            Err(CompactSequenceError::ZeroDuration)
        );
    }

    #[test]
    fn malformed_compact_loop_rejected() {
        const COMPACT_INSTRUMENTS: [CompactInstrument; 1] =
            [CompactInstrument::builtin(BUILTIN_INSTRUMENT_SQUARE, 255)];
        const END_WITHOUT_START: [CompactEvent; 2] =
            [CompactEvent::LoopEnd { repeat_count: 1 }, CompactEvent::End];
        const NESTED: [CompactEvent; 5] = [
            CompactEvent::LoopStart,
            CompactEvent::LoopStart,
            CompactEvent::Rest { duration_ticks: 1 },
            CompactEvent::LoopEnd { repeat_count: 1 },
            CompactEvent::End,
        ];
        const END_WITHOUT_START_TRACKS: [CompactTrack<'static>; 1] =
            [CompactTrack::new(&END_WITHOUT_START, MixerVolume::UNITY, 0)];
        const NESTED_TRACKS: [CompactTrack<'static>; 1] =
            [CompactTrack::new(&NESTED, MixerVolume::UNITY, 0)];

        assert_eq!(
            validate_compact_sequence(CompactSequence::new(
                &COMPACT_INSTRUMENTS,
                &END_WITHOUT_START_TRACKS,
                CompactTempo::from_tick_rate_hz(4),
            )),
            Err(CompactSequenceError::LoopEndWithoutStart)
        );
        assert_eq!(
            validate_compact_sequence(CompactSequence::new(
                &COMPACT_INSTRUMENTS,
                &NESTED_TRACKS,
                CompactTempo::from_tick_rate_hz(4),
            )),
            Err(CompactSequenceError::NestedLoop)
        );
    }

    #[test]
    fn missing_compact_end_rejected() {
        const COMPACT_INSTRUMENTS: [CompactInstrument; 1] =
            [CompactInstrument::builtin(BUILTIN_INSTRUMENT_SQUARE, 255)];
        const COMPACT_EVENTS: [CompactEvent; 1] = [CompactEvent::Rest { duration_ticks: 1 }];
        const TRACKS: [CompactTrack<'static>; 1] =
            [CompactTrack::new(&COMPACT_EVENTS, MixerVolume::UNITY, 0)];

        assert_eq!(
            validate_compact_sequence(CompactSequence::new(
                &COMPACT_INSTRUMENTS,
                &TRACKS,
                CompactTempo::from_tick_rate_hz(4),
            )),
            Err(CompactSequenceError::MissingEnd)
        );
    }

    #[test]
    fn valid_compact_drum_instrument_id_accepted() {
        const COMPACT_INSTRUMENTS: [CompactInstrument; 1] = [CompactInstrument::builtin(
            BUILTIN_INSTRUMENT_CLOSED_HI_HAT,
            255,
        )];
        const COMPACT_EVENTS: [CompactEvent; 2] = [
            CompactEvent::Note {
                pitch: 1,
                duration_ticks: 1,
                volume: 255,
                instrument_id: 0,
            },
            CompactEvent::End,
        ];
        const TRACKS: [CompactTrack<'static>; 1] =
            [CompactTrack::new(&COMPACT_EVENTS, MixerVolume::UNITY, 0)];

        assert_eq!(
            validate_compact_sequence(CompactSequence::new(
                &COMPACT_INSTRUMENTS,
                &TRACKS,
                CompactTempo::from_tick_rate_hz(4),
            )),
            Ok(())
        );
    }

    #[test]
    fn compact_track_can_adapt_to_existing_sequence_events() {
        const COMPACT_INSTRUMENTS: [CompactInstrument; 1] =
            [CompactInstrument::builtin(BUILTIN_INSTRUMENT_SQUARE, 128)];
        const COMPACT_EVENTS: [CompactEvent; 2] = [
            CompactEvent::Note {
                pitch: 4,
                duration_ticks: 1,
                volume: 254,
                instrument_id: 0,
            },
            CompactEvent::End,
        ];
        const TRACKS: [CompactTrack<'static>; 1] =
            [CompactTrack::new(&COMPACT_EVENTS, MixerVolume::UNITY, 0)];
        const COMPACT: CompactSequence<'static> = CompactSequence::new(
            &COMPACT_INSTRUMENTS,
            &TRACKS,
            CompactTempo::from_tick_rate_hz(4),
        );
        let mut events_out = [SequenceEvent::End; 2];

        let sequence = TRACKS[0]
            .adapt_to_sequence(COMPACT, &mut events_out)
            .unwrap();

        assert_eq!(sequence.tick_rate_hz, 4);
        assert_eq!(sequence.instruments, &BUILTIN_SEQUENCE_INSTRUMENTS);
        assert_eq!(
            sequence.events[0],
            SequenceEvent::Note {
                pitch: 4,
                duration_ticks: 1,
                volume: 127,
                instrument_id: BUILTIN_INSTRUMENT_SQUARE,
            }
        );
        assert_eq!(sequence.events[1], SequenceEvent::End);
    }
}
